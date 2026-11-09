//! Privileged SMC transport and its single unattended recovery attempt.

use super::{DeviceInfo, LightKind, LightOutput, LightValue, OutputStatus};
use crate::helper::Helper;
use crate::profile::DeviceProfile;
use std::io;

pub(super) struct SmcOutput {
    helper: Option<Helper>,
    restart_used: bool,
}

impl SmcOutput {
    pub fn open() -> io::Result<Self> {
        let mut helper = Helper::spawn()?;
        if let Err(error) = verify(&mut helper) {
            let _ = helper.terminate();
            return Err(error);
        }
        Ok(Self::from_helper(helper))
    }

    pub const fn from_helper(helper: Helper) -> Self {
        Self {
            helper: Some(helper),
            restart_used: false,
        }
    }

    fn reconnect(&mut self, error: &io::Error, value: u8) -> io::Result<OutputStatus> {
        let profile = self.helper.as_ref().map(Helper::profile);
        let termination = self
            .helper
            .take()
            .map_or(Ok(()), |mut helper| helper.terminate());
        let restart = if let Err(error) = termination {
            Err(error)
        } else if self.restart_used {
            Err(io::Error::other("automatic restart limit reached"))
        } else {
            self.restart_used = true;
            profile.map_or_else(
                || Err(io::Error::other("LED helper profile unavailable")),
                |profile| start_verified(profile, value),
            )
        };
        match restart {
            Ok(helper) => {
                self.helper = Some(helper);
                Ok(OutputStatus::Reconnected)
            }
            Err(restart) => Err(io::Error::other(format!(
                "{error}; automatic recovery failed: {restart}"
            ))),
        }
    }
}

fn verify(helper: &mut Helper) -> io::Result<()> {
    if helper.read_profile_raw()?.len() != 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "LED profile key must contain exactly two bytes",
        ));
    }
    Ok(())
}

fn start_verified(profile: DeviceProfile, value: u8) -> io::Result<Helper> {
    let mut helper = Helper::spawn_unattended(profile)?;
    if let Err(error) = verify(&mut helper).and_then(|()| helper.write_led(value)) {
        let _ = helper.terminate();
        return Err(error);
    }
    Ok(helper)
}

fn transport_failed(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::BrokenPipe
            | io::ErrorKind::UnexpectedEof
            | io::ErrorKind::TimedOut
            | io::ErrorKind::InvalidData
    )
}

impl LightOutput for SmcOutput {
    fn device(&self) -> DeviceInfo {
        DeviceInfo::physical(LightKind::FrontLed)
    }
    fn initial_value(&self) -> LightValue {
        LightValue {
            on: true,
            brightness: 255,
        }
    }

    fn write(&mut self, brightness: u8) -> io::Result<OutputStatus> {
        let result = self
            .helper
            .as_mut()
            .ok_or_else(|| io::Error::other("LED helper unavailable"))?
            .write_led(brightness);
        match result {
            Ok(()) => Ok(OutputStatus::Ready),
            Err(error) if transport_failed(&error) => self.reconnect(&error, brightness),
            Err(error) => Err(error),
        }
    }

    fn check_health(&mut self, restore: u8) -> io::Result<OutputStatus> {
        if let Some(helper) = self.helper.as_mut() {
            if let Err(error) = helper.check_running() {
                return self.reconnect(&error, restore);
            }
        }
        Ok(OutputStatus::Ready)
    }

    fn shutdown(&mut self) -> io::Result<()> {
        self.helper
            .take()
            .map_or(Ok(()), |mut helper| helper.terminate())
    }

    #[cfg(feature = "field-test")]
    fn diagnose(&mut self, diagnostic: super::Diagnostic, restore: u8) -> io::Result<OutputStatus> {
        match diagnostic {
            super::Diagnostic::Connection => {
                let helper = self
                    .helper
                    .as_mut()
                    .ok_or_else(|| io::Error::other("LED helper unavailable"))?;
                helper.ping()?;
                verify(helper)?;
                Ok(OutputStatus::Ready)
            }
            super::Diagnostic::Restart => {
                if self.restart_used {
                    return Err(io::Error::other(
                        "automatic restart already used in this app session",
                    ));
                }
                self.helper
                    .as_mut()
                    .ok_or_else(|| io::Error::other("LED helper unavailable"))?
                    .terminate()?;
                self.check_health(restore)
            }
        }
    }
}

impl Drop for SmcOutput {
    fn drop(&mut self) {
        if let Err(error) = self.shutdown() {
            eprintln!("LED helper shutdown failed: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_write_does_not_trigger_transport_recovery() -> io::Result<()> {
        let mut output = SmcOutput::from_helper(Helper::spawn_test_echo()?);
        assert!(output.write(128).is_err());
        assert!(!output.restart_used);
        assert!(output.helper.is_some());
        output.shutdown()
    }

    #[test]
    #[ignore = "requires a Mac mini with an approved profile and a root-owned setuid helper"]
    fn live_helper_restarts_once_after_crash() -> io::Result<()> {
        let mut output = SmcOutput::open()?;
        let helper = output
            .helper
            .as_mut()
            .ok_or_else(|| io::Error::other("helper missing"))?;
        let original = helper.read_profile_raw()?;
        if original != [255, 255] {
            return Err(io::Error::other(
                "recovery test requires full initial brightness",
            ));
        }
        helper.terminate()?;
        assert_eq!(output.check_health(255)?, OutputStatus::Reconnected);
        assert!(output.restart_used);
        let helper = output
            .helper
            .as_mut()
            .ok_or_else(|| io::Error::other("helper missing after restart"))?;
        assert_eq!(helper.read_profile_raw()?, original);
        helper.terminate()?;
        assert!(output.check_health(255).is_err());
        output.shutdown()
    }
}
