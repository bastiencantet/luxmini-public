//! Hardware boundary. Product behavior depends only on this brightness contract.

mod logo;
mod smc;

use self::logo::Logo;
use crate::helper::Helper;
use std::io;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightKind {
    FrontLed,
    RearLogo,
}

impl LightKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FrontLed => "front_led",
            Self::RearLogo => "rear_logo",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupMethod {
    Profile,
    Reconnect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub kind: LightKind,
    pub experimental: bool,
    pub simulated: bool,
}

impl DeviceInfo {
    pub const fn physical(kind: LightKind) -> Self {
        Self {
            kind,
            experimental: matches!(kind, LightKind::RearLogo),
            simulated: false,
        }
    }

    pub const fn setup_method(self) -> SetupMethod {
        if self.simulated || matches!(self.kind, LightKind::RearLogo) {
            SetupMethod::Reconnect
        } else {
            SetupMethod::Profile
        }
    }
}

/// Detection happens once at startup, never from a view or feature implementation.
pub fn detect_device() -> DeviceInfo {
    DeviceInfo::physical(if logo::is_available() {
        LightKind::RearLogo
    } else {
        LightKind::FrontLed
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightValue {
    pub on: bool,
    pub brightness: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputStatus {
    Ready,
    Reconnected,
}

#[cfg(feature = "field-test")]
pub enum Diagnostic {
    Connection,
    Restart,
}

/// Shared normalized brightness, lifecycle and error semantics for every output.
/// Drivers own transport details and bounded recovery; no presets/effects live here.
pub trait LightOutput: Send {
    fn device(&self) -> DeviceInfo;
    fn initial_value(&self) -> LightValue;
    fn write(&mut self, brightness: u8) -> io::Result<OutputStatus>;
    fn check_health(&mut self, _restore: u8) -> io::Result<OutputStatus> {
        Ok(OutputStatus::Ready)
    }
    fn shutdown(&mut self) -> io::Result<()>;

    #[cfg(feature = "field-test")]
    fn diagnose(&mut self, _diagnostic: Diagnostic, _restore: u8) -> io::Result<OutputStatus> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "This output has no helper diagnostics",
        ))
    }
}

pub fn open(device: DeviceInfo) -> io::Result<Box<dyn LightOutput>> {
    if device.simulated {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Use the simulated output factory for a preview",
        ));
    }
    match device.kind {
        LightKind::FrontLed => {
            smc::SmcOutput::open().map(|output| Box::new(output) as Box<dyn LightOutput>)
        }
        LightKind::RearLogo => Logo::open().map(|logo| Box::new(logo) as Box<dyn LightOutput>),
    }
}

pub fn from_helper(helper: Helper) -> Box<dyn LightOutput> {
    Box::new(smc::SmcOutput::from_helper(helper))
}

pub fn simulated(kind: LightKind, value: Arc<AtomicU8>) -> Box<dyn LightOutput> {
    value.store(255, Ordering::Relaxed);
    Box::new(MemoryOutput {
        device: DeviceInfo {
            kind,
            simulated: true,
            experimental: false,
        },
        value,
    })
}

struct MemoryOutput {
    device: DeviceInfo,
    value: Arc<AtomicU8>,
}

impl LightOutput for MemoryOutput {
    fn device(&self) -> DeviceInfo {
        self.device
    }
    fn initial_value(&self) -> LightValue {
        LightValue {
            on: true,
            brightness: 255,
        }
    }
    fn write(&mut self, brightness: u8) -> io::Result<OutputStatus> {
        self.value.store(brightness, Ordering::Relaxed);
        Ok(OutputStatus::Ready)
    }
    fn shutdown(&mut self) -> io::Result<()> {
        Ok(())
    }
}
