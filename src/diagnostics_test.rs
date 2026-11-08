//! Debug-only process tests. Only disposable bundle IDs and loopback transport
//! are accepted. This harness returns before profile discovery or LED startup.

use super::{
    apply_choice, ask_to_enable, consent_alert, disable, enqueue, load_state, mpsc,
    pending_erasures, preferences, should_offer_consent, start, Command, Deserialize, Duration,
    MainThreadMarker, Mutex, NSAlert, NSApplication, NSString, OnceLock, WORKER,
};
use objc2_foundation::NSBundle;
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    bundle_id: String,
    base_url: String,
    #[serde(default = "default_version")]
    version: String,
    #[serde(default = "default_day")]
    day: i64,
    #[serde(default)]
    interactive: bool,
    #[serde(default)]
    capture_dir: Option<PathBuf>,
    actions: Vec<Action>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Action {
    kind: String,
    #[serde(default)]
    accepted: bool,
    #[serde(default)]
    existing: bool,
    version: Option<String>,
    day: Option<i64>,
    milliseconds: Option<u64>,
    key: Option<String>,
    value: Option<serde_json::Value>,
}
struct Clock {
    version: String,
    day: i64,
}
static CONFIG: OnceLock<Config> = OnceLock::new();
static CLOCK: OnceLock<Mutex<Clock>> = OnceLock::new();
fn default_version() -> String {
    "0.4.3".into()
}
const fn default_day() -> i64 {
    20_000
}
pub fn active() -> bool {
    CONFIG.get().is_some()
}
pub fn base_url() -> Option<String> {
    CONFIG.get().map(|config| config.base_url.clone())
}
pub fn version() -> Option<String> {
    Some(CLOCK.get()?.lock().ok()?.version.clone())
}
pub fn day() -> Option<i64> {
    Some(CLOCK.get()?.lock().ok()?.day)
}

#[allow(clippy::too_many_lines)] // Sequential fixture commands keep process scenarios readable.
pub fn run(mtm: MainThreadMarker) -> Result<(), String> {
    let path = std::env::var_os("LUXMINI_DIAGNOSTICS_TEST_CONFIG").ok_or("missing test config")?;
    let config: Config =
        serde_json::from_slice(&std::fs::read(path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    let actual = NSBundle::mainBundle()
        .bundleIdentifier()
        .map(|id| id.to_string())
        .unwrap_or_default();
    if !config
        .bundle_id
        .starts_with("com.bastiencantet.luxmini.metrics-test.")
        || config.bundle_id != actual
    {
        return Err("disposable test bundle required".into());
    }
    let port = config
        .base_url
        .strip_prefix("http://127.0.0.1:")
        .and_then(|value| value.strip_suffix("/api/v1"))
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| *port != 0);
    if port.is_none() {
        return Err("loopback-only test transport required".into());
    }
    CLOCK
        .set(Mutex::new(Clock {
            version: config.version.clone(),
            day: config.day,
        }))
        .map_err(|_| "test clock already initialized")?;
    CONFIG
        .set(config)
        .map_err(|_| "test config already initialized")?;
    let config = CONFIG.get().ok_or("missing initialized config")?;
    let app = NSApplication::sharedApplication(mtm);
    app.finishLaunching();
    let mut snapshots = Vec::new();
    let mut prompts = 0;
    for action in &config.actions {
        match action.kind.as_str() {
            "seed" => seed(
                action.key.as_deref().ok_or("seed key missing")?,
                action.value.as_ref().ok_or("seed value missing")?,
            )?,
            "offer" => {
                if should_offer_consent(
                    cfg!(feature = "field-test"),
                    preferences::diagnostics_consent_decided(),
                ) {
                    prompts += 1;
                    if config.interactive {
                        ask_to_enable(mtm, action.existing);
                    } else {
                        apply_choice(action.accepted, action.existing);
                    }
                }
            }
            "choose" => apply_choice(action.accepted, action.existing),
            "start" => start(),
            "activity" => enqueue(Command::Activity),
            "disable" => disable(),
            "clock" => {
                let mut clock = CLOCK
                    .get()
                    .ok_or("clock unavailable")?
                    .lock()
                    .map_err(|_| "clock poisoned")?;
                if let Some(version) = &action.version {
                    clock.version.clone_from(version);
                }
                if let Some(day) = action.day {
                    clock.day = day;
                }
            }
            "wait_request" => {
                let route = action.key.as_deref().ok_or("request route missing")?;
                if !matches!(route, "profile-token" | "telemetry") {
                    return Err("unsupported request route".into());
                }
                ureq::get(&format!("{}/test-wait?route={route}", config.base_url))
                    .timeout(Duration::from_secs(10))
                    .call()
                    .map_err(|error| error.to_string())?;
            }
            "wait" => std::thread::sleep(Duration::from_millis(
                action.milliseconds.unwrap_or(0).min(2000),
            )),
            "drain" => {
                let (sender, receiver) = mpsc::channel();
                if let Some(Some(worker)) = WORKER.get() {
                    worker
                        .send(Command::Barrier(sender))
                        .map_err(|_| "worker unavailable")?;
                    receiver
                        .recv_timeout(Duration::from_secs(15))
                        .map_err(|_| "worker did not drain")?;
                }
            }
            "snapshot" => snapshots.push(snapshot()),
            "capture" => {
                let alert = consent_alert(mtm);
                capture_alert(&alert)?;
            }
            _ => return Err("unsupported test action".into()),
        }
    }
    // Command-line processes explicitly synchronize only their disposable domain.
    seed_sync();
    println!(
        "{}",
        serde_json::json!({"prompts": prompts, "snapshots": snapshots,
        "hardware_started": false, "transport": "loopback"})
    );
    if config.interactive {
        let handler = crate::ui::handler::Handler::new(mtm);
        crate::ui::settings::open(&handler, mtm);
        if let Some(directory) = &config.capture_dir {
            crate::ui::settings::export_previews(&handler, directory)
                .map_err(|error| error.to_string())?;
        }
        app.run();
    }
    Ok(())
}

fn snapshot() -> serde_json::Value {
    let state = load_state();
    serde_json::json!({"enabled":preferences::diagnostics_enabled(),
        "decided": preferences::diagnostics_consent_decided(), "has_identity":state.is_some(),
        "last_version":state.as_ref().and_then(|state|state.last_version.clone()),
        "last_day":state.as_ref().and_then(|state|state.last_day),
        "pending_deletions":pending_erasures().len()})
}

fn seed(key: &str, value: &serde_json::Value) -> Result<(), String> {
    use objc2::{
        msg_send,
        runtime::{AnyClass, AnyObject},
    };
    if !matches!(
        key,
        "diagnostics.enabled"
            | "diagnostics.consent_version"
            | "diagnostics.product_state"
            | "diagnostics.pending_deletions"
    ) {
        return Err("unsupported fixture key".into());
    }
    // SAFETY: the bundle guard above selects a disposable preference domain;
    // each selector below matches its primitive Objective-C argument types.
    unsafe {
        let class = AnyClass::get(c"NSUserDefaults").ok_or("defaults class unavailable")?;
        let defaults: *mut AnyObject = msg_send![class, standardUserDefaults];
        let key = NSString::from_str(key);
        if let Some(value) = value.as_bool() {
            let _: () = msg_send![defaults, setBool: value, forKey: &*key];
        } else if let Some(value) = value.as_i64() {
            let _: () = msg_send![defaults, setInteger: value, forKey: &*key];
        } else if let Some(value) = value.as_str() {
            let value = NSString::from_str(value);
            let _: () = msg_send![defaults, setObject: &*value, forKey: &*key];
        } else {
            return Err("unsupported fixture value".into());
        }
    }
    Ok(())
}
fn seed_sync() {
    use objc2::{
        msg_send,
        runtime::{AnyClass, AnyObject},
    };
    // SAFETY: synchronizing the existing test-domain singleton has no arguments.
    unsafe {
        if let Some(class) = AnyClass::get(c"NSUserDefaults") {
            let defaults: *mut AnyObject = msg_send![class, standardUserDefaults];
            let _: bool = msg_send![defaults, synchronize];
        }
    }
}

pub fn capture_alert(alert: &NSAlert) -> Result<(), String> {
    let Some(directory) = CONFIG.get().and_then(|config| config.capture_dir.as_ref()) else {
        return Ok(());
    };
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    alert.layout();
    let window = alert.window();
    window.display();
    let content = window.contentView().ok_or("alert content unavailable")?;
    let data = content.dataWithPDFInsideRect(content.bounds());
    let path = directory.join("consent.pdf");
    if !data.writeToFile_atomically(&NSString::from_str(&path.to_string_lossy()), true) {
        return Err("could not export native alert".into());
    }
    Ok(())
}
