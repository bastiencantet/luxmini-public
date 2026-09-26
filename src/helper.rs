//! Client side of the privileged led-helper IPC: spawns/elevates the setuid helper and sends line-based commands.
//!
//! Wire format (newline-terminated, sent to the helper's stdin):
//!   `WRITE <KEY> <HEXBYTE> <HEXBYTE>` — space-separated lowercase hex; the app always sends
//!   exactly 2 bytes and the helper rejects any other payload size.

use crate::auth;
use crate::helper_protocol::ROOT_HANDSHAKE;
use crate::profile::DeviceProfile;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const HELPER_NAME: &str = "led-helper";
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RESPONSE_BYTES: usize = 256;

pub struct Helper {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
    /// Which SMC key and byte layout to use on this Mac.
    profile: DeviceProfile,
}

fn is_setuid_root(path: &Path) -> bool {
    // Security gate: keep the && short-circuit (uid==0 *and* the setuid bit must both hold).
    std::fs::metadata(path).is_ok_and(|m| m.uid() == 0 && (m.mode() & 0o4000) != 0)
}

fn check_handshake(response: &str) -> std::io::Result<()> {
    match response {
        ROOT_HANDSHAKE => Ok(()),
        "PONG" => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "LED helper is outdated; reinstall LuxMini",
        )),
        "ERR ROOT_REQUIRED" => Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "LED helper is not running as root",
        )),
        message if message.starts_with("ERR SMC_ACCESS ") => Err(std::io::Error::other(format!(
            "LED controller access denied: {message}"
        ))),
        message if message.starts_with("ERR SMC_UNAVAILABLE ") => Err(std::io::Error::other(
            format!("LED controller unavailable: {message}"),
        )),
        message if message.starts_with("ERR ") => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message.to_owned(),
        )),
        _ => Err(std::io::Error::other(format!(
            "unexpected LED helper response: {response}"
        ))),
    }
}

fn read_response<R: Read + AsRawFd>(stdout: &mut R, timeout: Duration) -> std::io::Result<String> {
    let deadline = Instant::now() + timeout;
    let mut line = Vec::with_capacity(32);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "LED helper did not respond in time",
            ));
        }
        let mut descriptor = libc::pollfd {
            fd: stdout.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        #[allow(clippy::cast_possible_truncation)] // The production deadline is only five seconds.
        let timeout_ms = remaining.as_millis() as i32;
        // SAFETY: descriptor points to one valid pollfd and the count matches.
        let ready = unsafe { libc::poll(&raw mut descriptor, 1, timeout_ms) };
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if ready == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "LED helper did not respond in time",
            ));
        }
        let mut byte = [0u8; 1];
        stdout.read_exact(&mut byte).map_err(|error| {
            if error.kind() == std::io::ErrorKind::UnexpectedEof {
                std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "LED helper exited before responding",
                )
            } else {
                error
            }
        })?;
        if byte[0] == b'\n' {
            return String::from_utf8(line)
                .map(|text| text.trim_end_matches('\r').to_owned())
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error));
        }
        if line.len() == MAX_RESPONSE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "LED helper response is too long",
            ));
        }
        line.push(byte[0]);
    }
}

fn elevate_helper(path: &Path) -> std::io::Result<()> {
    if let Some(result) = auth::try_elevate_setuid(path) {
        return result;
    }

    let p = path
        .to_str()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad path"))?;
    if p.contains('\'') || p.contains('"') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path contains quote",
        ));
    }
    let script = format!(
        "do shell script \"chown root '{p}' && chmod u+s '{p}'\" with administrator privileges \
         with prompt \"LuxMini needs to install a helper tool to control the LED.\""
    );
    let status = Command::new("osascript").arg("-e").arg(&script).status()?;
    if !status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "admin prompt cancelled",
        ));
    }
    Ok(())
}

impl Helper {
    #[cfg(test)]
    pub(crate) fn spawn_test_echo() -> std::io::Result<Self> {
        let mut child = Command::new("/bin/cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("test child stdin missing"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("test child stdout missing"))?;
        Ok(Self {
            child,
            stdin,
            stdout,
            profile: DeviceProfile {
                key: "TEST".to_owned(),
                replicate: true,
                max: 255,
            },
        })
    }

    pub fn spawn() -> std::io::Result<Self> {
        Self::spawn_with_profile(DeviceProfile::load())
    }

    pub fn spawn_with_profile(profile: Option<DeviceProfile>) -> std::io::Result<Self> {
        Self::spawn_with_profile_internal(profile, true)
    }

    pub fn spawn_unattended(profile: DeviceProfile) -> std::io::Result<Self> {
        Self::spawn_with_profile_internal(Some(profile), false)
    }

    fn spawn_with_profile_internal(
        profile: Option<DeviceProfile>,
        allow_elevation: bool,
    ) -> std::io::Result<Self> {
        let profile = profile.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "device profile unavailable for this Mac model",
            )
        })?;
        let exe_dir = std::env::current_exe()?
            .parent()
            .map(std::path::Path::to_path_buf)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no exe dir"))?;
        let helper_path = exe_dir.join(HELPER_NAME);

        if !helper_path.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("helper missing: {}", helper_path.display()),
            ));
        }

        if !is_setuid_root(&helper_path) && allow_elevation {
            eprintln!("helper is not setuid root, asking for admin...");
            if let Err(error) = elevate_helper(&helper_path) {
                let outcome = if error.kind() == std::io::ErrorKind::PermissionDenied {
                    "cancelled"
                } else {
                    "error"
                };
                crate::telemetry::emit_once(
                    "helper_install_result",
                    outcome,
                    &crate::compat::get_mac_model(),
                );
                return Err(error);
            }
            crate::telemetry::emit_once(
                "helper_install_result",
                "success",
                &crate::compat::get_mac_model(),
            );
        }

        if !is_setuid_root(&helper_path) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "LED helper does not have root ownership and setuid permissions",
            ));
        }

        let mut child = Command::new(&helper_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("helper stdin pipe missing"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("helper stdout pipe missing"))?;
        let mut helper = Self {
            child,
            stdin,
            stdout,
            profile,
        };

        if let Err(error) = helper.ping() {
            let _ = helper.terminate();
            return Err(error);
        }
        Ok(helper)
    }

    pub fn profile(&self) -> DeviceProfile {
        self.profile.clone()
    }

    pub fn terminate(&mut self) -> std::io::Result<()> {
        let _ = self.child.kill();
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            if self.child.try_wait()?.is_some() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "LED helper did not exit after termination",
                ));
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn ping(&mut self) -> std::io::Result<()> {
        self.check_running()?;
        self.send("PING")?;
        check_handshake(&self.recv()?)
    }

    /// Check process liveness without waiting for an IPC response. The initial
    /// handshake and every LED command still validate the helper over IPC.
    pub fn check_running(&mut self) -> std::io::Result<()> {
        if let Some(status) = self.child.try_wait()? {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                format!("LED helper exited: {status}"),
            ));
        }
        Ok(())
    }

    fn send(&mut self, cmd: &str) -> std::io::Result<()> {
        writeln!(self.stdin, "{cmd}")?;
        self.stdin.flush()
    }

    fn recv(&mut self) -> std::io::Result<String> {
        read_response(&mut self.stdout, RESPONSE_TIMEOUT)
    }

    fn write_led_inner(&mut self, value: u8, emit_telemetry: bool) -> std::io::Result<()> {
        // Key and byte layout come from the device profile and are never hardcoded.
        let [b0, b1] = self.profile.bytes(value);
        let cmd = format!("WRITE {} {b0:02x} {b1:02x}", self.profile.key);
        self.send(&cmd)?;
        let resp = self.recv()?;
        if resp == "OK" {
            if emit_telemetry {
                crate::telemetry::emit_once(
                    "led_activation_result",
                    "success",
                    &crate::compat::get_mac_model(),
                );
            }
            Ok(())
        } else {
            if emit_telemetry {
                crate::telemetry::emit_once(
                    "led_activation_result",
                    "error",
                    &crate::compat::get_mac_model(),
                );
            }
            Err(std::io::Error::other(resp))
        }
    }

    pub fn write_led(&mut self, value: u8) -> std::io::Result<()> {
        self.write_led_inner(value, true)
    }

    /// Write a discovery pulse without counting it as successful normal LED
    /// activation. Visual confirmation is recorded separately by onboarding.
    pub fn write_led_test(&mut self, value: u8) -> std::io::Result<()> {
        self.write_led_inner(value, false)
    }

    /// Read the candidate key before a discovery pulse so the exact original
    /// bytes can be restored even when the candidate is rejected.
    pub fn read_profile_raw(&mut self) -> std::io::Result<Vec<u8>> {
        self.send(&format!("READ {}", self.profile.key))?;
        let response = self.recv()?;
        let hex = response
            .strip_prefix("OK ")
            .ok_or_else(|| std::io::Error::other(response.clone()))?;
        hex.split_whitespace()
            .map(|byte| {
                u8::from_str_radix(byte, 16)
                    .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
            })
            .collect()
    }

    /// Restore bytes previously returned by `read_profile_raw`.
    pub fn restore_profile_raw(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        if bytes.len() != 2 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid SMC restore length",
            ));
        }
        let hex = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .join(" ");
        self.send(&format!("WRITE {} {hex}", self.profile.key))?;
        let response = self.recv()?;
        if response == "OK" {
            Ok(())
        } else {
            Err(std::io::Error::other(response))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{check_handshake, read_response, Helper};
    use std::io::{self, ErrorKind, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    #[test]
    fn missing_profile_cannot_start_a_noop_helper() {
        assert!(matches!(
            Helper::spawn_with_profile(None),
            Err(error) if error.kind() == ErrorKind::NotFound
        ));
    }

    #[test]
    fn liveness_check_does_not_wait_for_an_ipc_response() -> io::Result<()> {
        let mut helper = Helper::spawn_test_echo()?;

        helper.check_running()?;
        helper.terminate()
    }

    #[test]
    fn helper_handshake_requires_current_version_and_root() {
        assert!(check_handshake("PONG 2 ROOT").is_ok());
        assert!(check_handshake("PONG").is_err());
        assert!(check_handshake("PONG 2 USER").is_err());
        assert!(check_handshake("ERR ROOT_REQUIRED")
            .is_err_and(|error| error.kind() == ErrorKind::PermissionDenied));
        assert!(
            check_handshake("ERR SMC_ACCESS open failed: -1").is_err_and(|error| error
                .to_string()
                .starts_with("LED controller access denied:"))
        );
    }

    #[test]
    fn response_reader_accepts_a_complete_line() -> io::Result<()> {
        let (mut writer, mut reader) = UnixStream::pair()?;
        writer.write_all(b"PONG 2 ROOT\r\n")?;
        assert_eq!(
            read_response(&mut reader, Duration::from_millis(50))?,
            "PONG 2 ROOT"
        );
        Ok(())
    }

    #[test]
    fn partial_response_times_out() -> io::Result<()> {
        let (mut writer, mut reader) = UnixStream::pair()?;
        writer.write_all(b"PONG")?;
        let error = read_response(&mut reader, Duration::from_millis(20))
            .err()
            .ok_or_else(|| io::Error::other("partial response unexpectedly succeeded"))?;
        assert_eq!(error.kind(), ErrorKind::TimedOut);
        Ok(())
    }

    #[test]
    fn closed_response_reports_a_helper_exit() -> io::Result<()> {
        let (writer, mut reader) = UnixStream::pair()?;
        drop(writer);
        let error = read_response(&mut reader, Duration::from_millis(50))
            .err()
            .ok_or_else(|| io::Error::other("closed response unexpectedly succeeded"))?;
        assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
        assert!(error.to_string().contains("exited"));
        Ok(())
    }

    #[test]
    #[ignore = "requires a Mac with an approved LED profile and a root-owned setuid helper"]
    fn live_helper_roundtrip() -> io::Result<()> {
        let mut helper = Helper::spawn()?;
        let original = helper.read_profile_raw()?;
        if original.len() != 2 {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "LED profile is not two bytes",
            ));
        }
        helper.restore_profile_raw(&original)?;
        assert_eq!(helper.read_profile_raw()?, original);
        Ok(())
    }
}
