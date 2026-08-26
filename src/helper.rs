//! Client side of the privileged led-helper IPC: spawns/elevates the setuid helper and sends line-based commands.
//!
//! Wire format (newline-terminated, sent to the helper's stdin):
//!   `WRITE <KEY> <HEXBYTE> <HEXBYTE>` — space-separated lowercase hex; the app always sends
//!   exactly 2 bytes, though the helper accepts up to 32.

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
    /// Which SMC key / byte layout to use on this Mac. `None` → no profile
    /// installed for this model yet, so LED writes are no-ops (the UI still works).
    profile: Option<DeviceProfile>,
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
            elevate_helper(&helper_path)?;
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

        if profile.is_none() {
            eprintln!("no device profile installed — LED control unavailable for this Mac model");
        }

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

    pub fn write_led(&mut self, value: u8) -> std::io::Result<()> {
        // Key + byte layout come from the device profile (data), never hardcoded. No profile → no write.
        let cmd = match self.profile.as_ref() {
            Some(p) => {
                let [b0, b1] = p.bytes(value);
                format!("WRITE {} {b0:02x} {b1:02x}", p.key)
            }
            None => return Ok(()),
        };
        self.send(&cmd)?;
        let resp = self.recv()?;
        if resp.starts_with("OK") {
            Ok(())
        } else {
            Err(std::io::Error::other(resp))
        }
    }
}
