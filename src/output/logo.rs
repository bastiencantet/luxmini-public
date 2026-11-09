//! Experimental rear-logo control. All private Objective-C calls stay on one worker.
//! No SMC keys, privileged helper, or persistent defaults are used by this backend.

use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{msg_send, sel};
use objc2_foundation::{NSArray, NSNumber, NSString};
use std::ffi::{c_char, c_int, c_void};
use std::io;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

const MAPPING: &str = "LEPWMBrightnessNitsMapping";
const MODE: &str = "LEBacklightMode";
const REFRESH: &str = "DisplayPowerStateEvent";
const TIMEOUT: Duration = Duration::from_secs(3);

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IORegistryEntryFromPath(port: u32, path: *const c_char) -> u32;
    fn IOServiceNameMatching(name: *const c_char) -> *mut c_void;
    fn IOServiceGetMatchingService(port: u32, matching: *mut c_void) -> u32;
    fn IOObjectRelease(object: u32) -> c_int;
}
extern "C" {
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
}

/// Probe the logo device itself, rather than assuming all laptops have one.
pub fn is_available() -> bool {
    // SAFETY: static NUL-terminated names; port 0 is kIOMainPortDefault. The
    // matching service call consumes its dictionary; each returned entry is released.
    unsafe {
        let mut entry =
            IORegistryEntryFromPath(0, c"IODeviceTree:/arm-io/pwm1/bklb-backlight".as_ptr());
        if entry == 0 {
            let matching = IOServiceNameMatching(c"bklb-backlight".as_ptr());
            if !matching.is_null() {
                entry = IOServiceGetMatchingService(0, matching);
            }
        }
        if entry == 0 {
            return false;
        }
        IOObjectRelease(entry);
        true
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    mapping: Vec<Vec<f64>>,
    mode: i64,
}

trait Properties {
    fn read(&self) -> io::Result<Snapshot>;
    fn write(&self, snapshot: &Snapshot) -> io::Result<()>;
}

struct Session<P: Properties> {
    properties: P,
    original: Snapshot,
    current: Snapshot,
    maximum: f64,
    dirty: bool,
}

impl<P: Properties> Session<P> {
    fn new(properties: P) -> io::Result<Self> {
        let original = properties.read()?;
        // Refuse unknown/malformed tables: no guessed nits limit on new hardware.
        let maximum = mapping_maximum(&original.mapping)?;
        Ok(Self {
            properties,
            current: original.clone(),
            original,
            maximum,
            dirty: false,
        })
    }

    fn write(&mut self, value: u8) -> io::Result<()> {
        let nits = f64::from(value) / 255.0 * self.maximum;
        let next = Snapshot {
            mapping: vec![vec![0.0, nits], vec![3.4e38, nits]],
            // Backglow's enabled mode is 2; 0 is Apple's documented disabled mode.
            mode: if value == 0 { 0 } else { 2 },
        };
        self.dirty = true; // even a failed write may have partially changed properties
        if let Err(error) = self.properties.write(&next) {
            if let Err(rollback) = self.properties.write(&self.current) {
                return Err(io::Error::other(format!(
                    "{error}; logo rollback failed: {rollback}"
                )));
            }
            return Err(error);
        }
        self.current = next;
        Ok(())
    }

    fn restore(&mut self) -> io::Result<()> {
        if self.dirty {
            self.properties.write(&self.original)?;
            self.dirty = false;
        }
        Ok(())
    }
}

impl<P: Properties> Drop for Session<P> {
    fn drop(&mut self) {
        if let Err(error) = self.restore() {
            eprintln!("Could not restore Apple's logo settings: {error}");
        }
    }
}

fn mapping_maximum(mapping: &[Vec<f64>]) -> io::Result<f64> {
    let valid = mapping.len() >= 2
        && mapping
            .iter()
            .all(|row| row.len() == 2 && row.iter().all(|v| v.is_finite() && *v >= 0.0))
        && mapping
            .windows(2)
            .all(|rows| matches!(rows, [a, b] if a.first() < b.first()));
    let maximum = mapping
        .iter()
        .filter_map(|row| row.get(1))
        .copied()
        .fold(0.0, f64::max);
    if valid && maximum > 0.0 {
        Ok(maximum)
    } else {
        Err(io::Error::other(
            "Logo brightness mapping is unavailable or invalid",
        ))
    }
}

struct CoreProperties {
    client: Retained<AnyObject>,
}

impl CoreProperties {
    fn new() -> io::Result<Self> {
        // SAFETY: load Apple's framework by absolute path. Keep the image loaded
        // for the process lifetime so its Objective-C classes cannot be unloaded.
        let handle = unsafe {
            dlopen(
                c"/System/Library/PrivateFrameworks/CoreBrightness.framework/CoreBrightness"
                    .as_ptr(),
                2,
            )
        };
        if handle.is_null() {
            return Err(io::Error::other("CoreBrightness is unavailable"));
        }
        let class = AnyClass::get(c"BrightnessSystemClient")
            .ok_or_else(|| io::Error::other("BrightnessSystemClient is unavailable"))?;
        // SAFETY: the dynamically resolved class is BrightnessSystemClient; +new
        // returns a retained instance. No instance is shared with another thread.
        let client: Option<Retained<AnyObject>> = unsafe { msg_send![class, new] };
        let client = client.ok_or_else(|| io::Error::other("Cannot create logo client"))?;
        // SAFETY: NSObject respondsToSelector: is queried before calling private methods.
        let supported: bool = unsafe {
            let read: bool = msg_send![&client, respondsToSelector: sel!(copyPropertyForKey:)];
            let write: bool = msg_send![&client, respondsToSelector: sel!(setProperty:forKey:)];
            read && write
        };
        if !supported {
            return Err(io::Error::other(
                "CoreBrightness logo methods are unavailable",
            ));
        }
        Ok(Self { client })
    }

    fn copy(&self, key: &str) -> io::Result<Retained<AnyObject>> {
        let key = NSString::from_str(key);
        // SAFETY: selector availability checked in new(); copy-family return
        // has +1 ownership. Returned objects are checked before typed access.
        let value: Option<Retained<AnyObject>> =
            unsafe { msg_send![&self.client, copyPropertyForKey: &*key] };
        value.ok_or_else(|| io::Error::other(format!("CoreBrightness property missing: {key}")))
    }

    fn set(&self, key: &str, value: &AnyObject) -> io::Result<()> {
        let key = NSString::from_str(key);
        // SAFETY: selector checked in new(); takes an NSObject value and NSString
        // key and returns BOOL, as observed in Backglow's Client.set wrapper.
        let accepted: bool = unsafe { msg_send![&self.client, setProperty: value, forKey: &*key] };
        if accepted {
            Ok(())
        } else {
            Err(io::Error::other(format!("CoreBrightness rejected {key}")))
        }
    }
}

impl Properties for CoreProperties {
    fn read(&self) -> io::Result<Snapshot> {
        let mapping = self.copy(MAPPING)?;
        let rows = mapping
            .downcast_ref::<NSArray<AnyObject>>()
            .ok_or_else(|| io::Error::other("Logo mapping is not an array"))?;
        let mut table = Vec::new();
        for row in rows {
            let row = row
                .downcast_ref::<NSArray<AnyObject>>()
                .ok_or_else(|| io::Error::other("Logo mapping row is not an array"))?;
            let mut values = Vec::new();
            for number in row {
                let number = number
                    .downcast_ref::<NSNumber>()
                    .ok_or_else(|| io::Error::other("Logo mapping entry is not numeric"))?;
                values.push(number.as_f64());
            }
            table.push(values);
        }
        let mode = self.copy(MODE)?;
        let mode = mode
            .downcast_ref::<NSNumber>()
            .ok_or_else(|| io::Error::other("Logo mode is not numeric"))?
            .as_i64();
        Ok(Snapshot {
            mapping: table,
            mode,
        })
    }

    fn write(&self, snapshot: &Snapshot) -> io::Result<()> {
        let rows: Vec<_> = snapshot
            .mapping
            .iter()
            .map(|row| {
                let numbers: Vec<_> = row.iter().map(|n| NSNumber::new_f64(*n)).collect();
                NSArray::from_retained_slice(&numbers)
            })
            .collect();
        let mapping = NSArray::from_retained_slice(&rows);
        self.set(MAPPING, &mapping)?;
        self.set(MODE, &NSNumber::new_i64(snapshot.mode))?;
        self.set(REFRESH, &NSNumber::new_i64(1))
    }
}

enum Command {
    Write(u8),
    Restore,
}
type Request = (Command, Sender<io::Result<()>>);

fn serve<P: Properties>(session: &mut Session<P>, receiver: &mpsc::Receiver<Request>) {
    while let Ok((command, reply)) = receiver.recv() {
        let stop = matches!(command, Command::Restore);
        let result = autoreleasepool(|_| match command {
            Command::Write(value) => session.write(value),
            Command::Restore => session.restore(),
        });
        let disconnected = reply.send(result).is_err();
        if stop || disconnected {
            break;
        }
    }
}

pub struct Logo {
    requests: Sender<Request>,
    initially_on: bool,
}

impl Logo {
    pub fn open() -> io::Result<Self> {
        if !is_available() {
            return Err(io::Error::other("No illuminated rear-logo device found"));
        }
        let (requests, receiver) = mpsc::channel::<Request>();
        let (ready, startup) = mpsc::channel();
        std::thread::Builder::new()
            .name("luxmini-logo".into())
            .spawn(move || {
                let session = autoreleasepool(|_| CoreProperties::new().and_then(Session::new));
                let mut session = match session {
                    Ok(session) => session,
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                if ready.send(Ok(session.original.mode != 0)).is_err() {
                    return;
                }
                serve(&mut session, &receiver);
                // Session::drop also restores on a disconnected client.
                autoreleasepool(|_| drop(session));
            })?;
        let initially_on = startup
            .recv_timeout(TIMEOUT)
            .map_err(|e| io::Error::other(e.to_string()))??;
        Ok(Self {
            requests,
            initially_on,
        })
    }

    pub const fn initially_on(&self) -> bool {
        self.initially_on
    }

    fn request(&self, command: Command) -> io::Result<()> {
        let (reply, response) = mpsc::channel();
        self.requests
            .send((command, reply))
            .map_err(|e| io::Error::other(e.to_string()))?;
        response
            .recv_timeout(TIMEOUT)
            .map_err(|e| io::Error::other(e.to_string()))?
    }

    pub fn write_led(&self, value: u8) -> io::Result<()> {
        self.request(Command::Write(value))
    }
    pub fn restore(&self) -> io::Result<()> {
        self.request(Command::Restore)
    }
}

impl super::LightOutput for Logo {
    fn device(&self) -> super::DeviceInfo {
        super::DeviceInfo::physical(super::LightKind::RearLogo)
    }
    fn initial_value(&self) -> super::LightValue {
        let on = self.initially_on();
        super::LightValue {
            on,
            brightness: if on { 255 } else { 0 },
        }
    }
    fn write(&mut self, brightness: u8) -> io::Result<super::OutputStatus> {
        self.write_led(brightness)
            .map(|()| super::OutputStatus::Ready)
    }
    fn shutdown(&mut self) -> io::Result<()> {
        self.restore()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    struct Fake {
        state: Rc<RefCell<Snapshot>>,
        failures: Cell<usize>,
        writes: Rc<Cell<usize>>,
    }
    impl Properties for Fake {
        fn read(&self) -> io::Result<Snapshot> {
            Ok(self.state.borrow().clone())
        }
        fn write(&self, value: &Snapshot) -> io::Result<()> {
            *self.state.borrow_mut() = value.clone();
            self.writes.set(self.writes.get() + 1);
            let failures = self.failures.get();
            self.failures.set(failures.saturating_sub(1));
            if failures > 0 {
                Err(io::Error::other("rejected refresh"))
            } else {
                Ok(())
            }
        }
    }
    fn original() -> Snapshot {
        Snapshot {
            mapping: vec![vec![0.0, 2.0], vec![100.0, 20.0]],
            mode: 0,
        }
    }
    fn fake() -> Fake {
        Fake {
            state: Rc::new(RefCell::new(original())),
            failures: Cell::new(0),
            writes: Rc::new(Cell::new(0)),
        }
    }

    #[test]
    fn uses_device_range_and_restores_original_disabled_mode_on_drop() -> io::Result<()> {
        let properties = fake();
        let observed = properties.state.clone();
        {
            let mut session = Session::new(properties)?;
            session.write(255)?;
            assert_eq!(observed.borrow().mode, 2);
            assert_eq!(
                observed.borrow().mapping,
                vec![vec![0.0, 20.0], vec![3.4e38, 20.0]]
            );
            session.write(0)?;
            assert_eq!(observed.borrow().mode, 0);
        }
        assert_eq!(*observed.borrow(), original());
        Ok(())
    }

    #[test]
    fn partial_write_failure_rolls_back_last_successful_state() -> io::Result<()> {
        let mut session = Session::new(fake())?;
        session.write(128)?;
        let previous = session.current.clone();
        session.properties.failures.set(1);
        assert!(session.write(0).is_err());
        assert_eq!(*session.properties.state.borrow(), previous);
        session.restore()?;
        assert_eq!(*session.properties.state.borrow(), original());
        Ok(())
    }

    #[test]
    fn failed_restore_is_retried_on_drop() -> io::Result<()> {
        let properties = fake();
        let observed = properties.state.clone();
        {
            let mut session = Session::new(properties)?;
            session.write(255)?;
            session.properties.failures.set(1);
            assert!(session.restore().is_err());
            assert!(session.dirty);
        }
        assert_eq!(*observed.borrow(), original());
        Ok(())
    }

    #[test]
    fn refuses_unknown_ranges_without_writing() {
        for mapping in [
            vec![],
            vec![vec![0.0, 0.0], vec![1.0, 0.0]],
            vec![vec![0.0, f64::NAN], vec![1.0, 1.0]],
            vec![vec![2.0, 1.0], vec![1.0, 2.0]],
        ] {
            assert!(mapping_maximum(&mapping).is_err());
        }
    }

    #[test]
    fn all_brightness_values_are_bounded_monotonic_and_use_the_device_range() -> io::Result<()> {
        for maximum in [0.01, 1.0, 20.0, 500.0] {
            let properties = fake();
            properties.state.borrow_mut().mapping = vec![vec![0.0, 0.0], vec![100.0, maximum]];
            let mut session = Session::new(properties)?;
            let mut previous = 0.0;
            for value in 0..=255 {
                session.write(value)?;
                let nits = session
                    .current
                    .mapping
                    .first()
                    .and_then(|row| row.get(1))
                    .copied();
                let expected = f64::from(value) / 255.0 * maximum;
                assert_eq!(nits, Some(expected));
                assert!(expected >= previous && expected <= maximum);
                assert_eq!(session.current.mode, if value == 0 { 0 } else { 2 });
                previous = expected;
            }
        }
        Ok(())
    }

    #[test]
    fn nonmonotonic_nits_still_uses_the_largest_device_value() -> io::Result<()> {
        let maximum = mapping_maximum(&[vec![0.0, 3.0], vec![50.0, 40.0], vec![100.0, 7.0]])?;
        assert!((maximum - 40.0).abs() < f64::EPSILON);
        Ok(())
    }

    #[test]
    fn malformed_mapping_never_opens_or_writes() {
        for mapping in [
            vec![vec![0.0, 1.0]],
            vec![vec![], vec![1.0, 2.0]],
            vec![vec![0.0, 1.0, 3.0], vec![1.0, 2.0]],
            vec![vec![0.0, 1.0], vec![0.0, 2.0]],
            vec![vec![-1.0, 1.0], vec![1.0, 2.0]],
            vec![vec![0.0, -1.0], vec![1.0, 2.0]],
            vec![vec![0.0, 1.0], vec![f64::INFINITY, 2.0]],
            vec![vec![0.0, 1.0], vec![1.0, f64::INFINITY]],
            vec![vec![f64::NAN, 1.0], vec![1.0, 2.0]],
        ] {
            let properties = fake();
            properties.state.borrow_mut().mapping = mapping;
            let writes = properties.writes.clone();
            assert!(Session::new(properties).is_err());
            assert_eq!(writes.get(), 0);
        }
    }

    #[test]
    fn read_failure_never_attempts_a_write() {
        struct Unreadable(Rc<Cell<usize>>);
        impl Properties for Unreadable {
            fn read(&self) -> io::Result<Snapshot> {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "unavailable property",
                ))
            }
            fn write(&self, _snapshot: &Snapshot) -> io::Result<()> {
                self.0.set(self.0.get() + 1);
                Err(io::Error::other("unexpected write after failed read"))
            }
        }
        let writes = Rc::new(Cell::new(0));
        assert!(
            matches!(Session::new(Unreadable(writes.clone())), Err(error) if error.kind() == io::ErrorKind::PermissionDenied)
        );
        assert_eq!(writes.get(), 0);
    }

    #[test]
    fn unused_session_and_clean_restore_do_not_write() -> io::Result<()> {
        let properties = fake();
        let writes = properties.writes.clone();
        let mut session = Session::new(properties)?;
        session.restore()?;
        drop(session);
        assert_eq!(writes.get(), 0);
        Ok(())
    }

    #[test]
    fn restores_the_exact_original_mapping_and_mode_once() -> io::Result<()> {
        for mode in [0, 1, 2] {
            let properties = fake();
            properties.state.borrow_mut().mode = mode;
            properties
                .state
                .borrow_mut()
                .mapping
                .insert(1, vec![50.0, 7.5]);
            let original = properties.read()?;
            let observed = properties.state.clone();
            let writes = properties.writes.clone();
            let mut session = Session::new(properties)?;
            session.write(12)?;
            session.write(0)?;
            session.restore()?;
            assert_eq!(*observed.borrow(), original);
            assert_eq!(writes.get(), 3);
            session.restore()?;
            drop(session);
            assert_eq!(writes.get(), 3);
        }
        Ok(())
    }

    #[test]
    fn new_write_after_restore_can_be_restored_again() -> io::Result<()> {
        let mut session = Session::new(fake())?;
        session.write(32)?;
        session.restore()?;
        session.write(200)?;
        assert!(session.dirty);
        session.restore()?;
        assert_eq!(session.properties.read()?, original());
        Ok(())
    }

    #[test]
    fn first_write_failure_rolls_back_the_original_state() -> io::Result<()> {
        let mut session = Session::new(fake())?;
        session.properties.failures.set(1);
        assert!(session.write(255).is_err());
        assert_eq!(session.current, original());
        assert_eq!(session.properties.read()?, original());
        session.restore()?;
        Ok(())
    }

    #[test]
    fn failed_rollback_reports_both_errors_and_still_restores_on_drop() -> io::Result<()> {
        let properties = fake();
        let observed = properties.state.clone();
        let writes = properties.writes.clone();
        let mut session = Session::new(properties)?;
        session.write(100)?;
        let confirmed = session.current.clone();
        session.properties.failures.set(2);
        let error = session
            .write(200)
            .err()
            .ok_or_else(|| io::Error::other("expected rejected write"))?;
        assert!(error.to_string().contains("logo rollback failed"));
        assert_eq!(session.current, confirmed);
        assert!(session.dirty);
        drop(session);
        assert_eq!(*observed.borrow(), original());
        assert_eq!(writes.get(), 4);
        Ok(())
    }

    #[test]
    fn worker_disconnect_restores_after_a_successful_write() -> io::Result<()> {
        let properties = fake();
        let observed = properties.state.clone();
        let mut session = Session::new(properties)?;
        let (requests, receiver) = mpsc::channel();
        let (reply, response) = mpsc::channel();
        requests
            .send((Command::Write(128), reply))
            .map_err(io::Error::other)?;
        drop(requests);
        serve(&mut session, &receiver);
        response.recv().map_err(io::Error::other)??;
        drop(session);
        assert_eq!(*observed.borrow(), original());
        Ok(())
    }

    #[test]
    fn abandoned_reply_stops_worker_and_restores_instead_of_processing_more_writes(
    ) -> io::Result<()> {
        let properties = fake();
        let observed = properties.state.clone();
        let writes = properties.writes.clone();
        let mut session = Session::new(properties)?;
        let (requests, receiver) = mpsc::channel();
        let (reply, response) = mpsc::channel();
        drop(response);
        requests
            .send((Command::Write(128), reply))
            .map_err(io::Error::other)?;
        let (reply, response) = mpsc::channel();
        requests
            .send((Command::Write(255), reply))
            .map_err(io::Error::other)?;
        serve(&mut session, &receiver);
        assert_eq!(writes.get(), 1);
        assert!(response.try_recv().is_err());
        drop(session);
        assert_eq!(*observed.borrow(), original());
        Ok(())
    }

    #[test]
    fn restore_command_ends_worker_before_a_queued_write() -> io::Result<()> {
        let mut session = Session::new(fake())?;
        session.write(128)?;
        let (requests, receiver) = mpsc::channel();
        let (reply, response) = mpsc::channel();
        requests
            .send((Command::Restore, reply))
            .map_err(io::Error::other)?;
        let (reply, pending) = mpsc::channel();
        requests
            .send((Command::Write(255), reply))
            .map_err(io::Error::other)?;
        serve(&mut session, &receiver);
        response.recv().map_err(io::Error::other)??;
        assert_eq!(session.properties.read()?, original());
        assert!(pending.try_recv().is_err());
        assert!(!session.dirty);
        Ok(())
    }

    #[test]
    fn hardware_probe_is_read_only() {
        let available = is_available();
        eprintln!("Rear-logo device present: {available}");
        if !available {
            assert!(Logo::open().is_err());
        }
    }
}
