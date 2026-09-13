//! Client side of the privileged led-helper IPC: spawns/elevates the setuid helper and sends line-based commands.
//!
//! Wire format (newline-terminated, sent to the helper's stdin):
//!   `WRITE <KEY> <HEXBYTE> <HEXBYTE>` — space-separated lowercase hex; the app always sends
//!   exactly 2 bytes and the helper rejects any other payload size.

use crate::auth;
use crate::profile::DeviceProfile;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

const HELPER_NAME: &str = "led-helper";

pub struct Helper {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    /// Which SMC key and byte layout to use on this Mac.
    profile: DeviceProfile,
}

fn is_setuid_root(path: &Path) -> bool {
    // Security gate: keep the && short-circuit (uid==0 *and* the setuid bit must both hold).
    std::fs::metadata(path).is_ok_and(|m| m.uid() == 0 && (m.mode() & 0o4000) != 0)
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
    pub fn spawn() -> std::io::Result<Self> {
        Self::spawn_with_profile(DeviceProfile::load())
    }

    pub fn spawn_with_profile(profile: Option<DeviceProfile>) -> std::io::Result<Self> {
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

        if !is_setuid_root(&helper_path) {
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
        let stdout = BufReader::new(stdout);

        let mut helper = Self {
            _child: child,
            stdin,
            stdout,
            profile,
        };

        helper.send("PING")?;
        let resp = helper.recv()?;
        if !resp.starts_with("PONG") {
            return Err(std::io::Error::other(format!(
                "bad helper response: {resp}"
            )));
        }
        Ok(helper)
    }

    fn send(&mut self, cmd: &str) -> std::io::Result<()> {
        writeln!(self.stdin, "{cmd}")?;
        self.stdin.flush()
    }

    fn recv(&mut self) -> std::io::Result<String> {
        let mut line = String::new();
        // A 0-byte read = EOF: the helper closed stdout (crashed/exited) → surface as an error.
        if self.stdout.read_line(&mut line)? == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "helper closed stdout",
            ));
        }
        Ok(line.trim().to_string())
    }

    fn write_led_inner(&mut self, value: u8, emit_telemetry: bool) -> std::io::Result<()> {
        // Key and byte layout come from the device profile and are never hardcoded.
        let [b0, b1] = self.profile.bytes(value);
        let cmd = format!("WRITE {} {b0:02x} {b1:02x}", self.profile.key);
        self.send(&cmd)?;
        let resp = self.recv()?;
        if resp.starts_with("OK") {
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
        if response.starts_with("OK") {
            Ok(())
        } else {
            Err(std::io::Error::other(response))
        }
    }
}
