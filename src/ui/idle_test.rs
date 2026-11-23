//! Disposable native UI and notification checks. No physical sleep or hardware access.

use super::{handler::Handler, settings};
use crate::{light, output, power, preferences};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSButton, NSSwitch, NSWindow};
use objc2_foundation::{NSBundle, NSString};
use std::io;
use std::path::Path;
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

fn require(condition: bool, message: &str) -> io::Result<()> {
    if condition {
        Ok(())
    } else {
        Err(io::Error::other(message.to_owned()))
    }
}

fn notify(name: &str) -> io::Result<()> {
    let cls =
        AnyClass::get(c"NSWorkspace").ok_or_else(|| io::Error::other("workspace unavailable"))?;
    // SAFETY: these are process-local Foundation notifications sent to the
    // test app's real observers. They do not request OS sleep or display sleep.
    unsafe {
        let workspace: *mut AnyObject = msg_send![cls, sharedWorkspace];
        require(!workspace.is_null(), "workspace singleton unavailable")?;
        let center: *mut AnyObject = msg_send![workspace, notificationCenter];
        require(
            !center.is_null(),
            "workspace notification center unavailable",
        )?;
        let _: () = msg_send![center, postNotificationName: &*NSString::from_str(name), object: std::ptr::null::<AnyObject>()];
    }
    Ok(())
}

fn capture(window: &NSWindow, path: &Path) -> io::Result<()> {
    window.makeKeyAndOrderFront(None);
    let app = NSApplication::sharedApplication(window.mtm());
    // Native switches and radio cells animate their new state. Process this
    // fixture's AppKit events before exporting the settled controls.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
    while std::time::Instant::now() < deadline {
        let until = objc2_foundation::NSDate::dateWithTimeIntervalSinceNow(0.025);
        // SAFETY: Foundation supplies this immutable run-loop mode.
        let mode = unsafe { objc2_foundation::NSDefaultRunLoopMode };
        if let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
            objc2_app_kit::NSEventMask::Any,
            Some(&until),
            mode,
            true,
        ) {
            app.sendEvent(&event);
        }
        app.updateWindows();
    }
    window.display();
    let content = window
        .contentView()
        .ok_or_else(|| io::Error::other("settings content missing"))?;
    content.layoutSubtreeIfNeeded();
    let bitmap = content
        .bitmapImageRepForCachingDisplayInRect(content.bounds())
        .ok_or_else(|| io::Error::other("settings bitmap unavailable"))?;
    content.cacheDisplayInRect_toBitmapImageRep(content.bounds(), &bitmap);
    // SAFETY: PNG encoding accepts an empty properties dictionary.
    let data = unsafe {
        bitmap.representationUsingType_properties(
            objc2_app_kit::NSBitmapImageFileType::PNG,
            &objc2_foundation::NSDictionary::new(),
        )
    }
    .ok_or_else(|| io::Error::other("settings PNG unavailable"))?;
    require(
        data.writeToFile_atomically(&NSString::from_str(&path.to_string_lossy()), true),
        "could not export idle settings",
    )
}

struct Controls {
    window: Retained<NSWindow>,
    enabled: Retained<NSSwitch>,
    system: Retained<NSButton>,
    display: Retained<NSButton>,
}

fn controls(handler: &Handler) -> io::Result<Controls> {
    let refs = handler.ivars().settings.borrow();
    let r = refs
        .as_ref()
        .ok_or_else(|| io::Error::other("idle controls missing"))?;
    Ok(Controls {
        window: r.window.clone(),
        enabled: r.sleep_switch.clone(),
        system: r.sleep_system_radio.clone(),
        display: r.sleep_display_radio.clone(),
    })
}

fn check_notifications(value: &AtomicU8, trigger: power::Trigger) -> io::Result<()> {
    notify("NSWorkspaceScreensDidSleepNotification")?;
    let expected = if trigger == power::Trigger::DisplaySleep {
        0
    } else {
        153
    };
    require(
        value.load(Ordering::Relaxed) == expected,
        "display sleep used the wrong trigger",
    )?;
    notify("NSWorkspaceWillSleepNotification")?;
    require(
        value.load(Ordering::Relaxed) == 0,
        "system sleep did not apply idle brightness",
    )?;
    notify("NSWorkspaceDidWakeNotification")?;
    require(
        value.load(Ordering::Relaxed) == expected,
        "system wake released display mode too early",
    )?;
    notify("NSWorkspaceScreensDidWakeNotification")?;
    require(
        value.load(Ordering::Relaxed) == 153,
        "display wake did not restore manual brightness",
    )
}

#[allow(clippy::too_many_lines)] // A sequential native fixture keeps event order and assertions explicit.
pub fn run(mtm: MainThreadMarker) -> io::Result<()> {
    let bundle = NSBundle::mainBundle()
        .bundleIdentifier()
        .map(|id| id.to_string())
        .unwrap_or_default();
    require(
        bundle.starts_with("com.bastiencantet.luxmini.settings-test."),
        "disposable settings-test bundle required",
    )?;
    let app = NSApplication::sharedApplication(mtm);
    app.finishLaunching();
    let value = Arc::new(AtomicU8::new(153));
    let mut state = light::LightState::with_output(
        output::simulated(output::LightKind::FrontLed, value.clone()),
        light::Persistence::SessionOnly,
    );
    state.set_brightness(153);
    *light::STATE
        .lock()
        .map_err(|_| io::Error::other("fixture state lock"))? = Some(state);
    let handler = Handler::new(mtm);
    super::menu::observe_power(&handler);
    settings::open(&handler, mtm);
    settings::select_section(&handler, 0);
    let c = controls(&handler)?;
    if let Some(appearance) = objc2_app_kit::NSAppearance::appearanceNamed(&NSString::from_str(
        "NSAppearanceNameDarkAqua",
    )) {
        // SAFETY: setAppearance: retains this fixture window's native appearance.
        unsafe {
            let _: () = msg_send![&c.window, setAppearance: &*appearance];
        }
    }
    if let Some(content) = c.window.contentView() {
        // Include the same native background in the view bitmap; a window's
        // own background is otherwise outside its exported content view.
        let background: Retained<objc2_app_kit::NSBox> = unsafe {
            // SAFETY: a custom box covering this fixture's valid content bounds.
            msg_send![objc2_app_kit::NSBox::alloc(mtm), initWithFrame: content.bounds()]
        };
        background.setBoxType(objc2_app_kit::NSBoxType::Custom);
        background.setTitlePosition(objc2_app_kit::NSTitlePosition::NoTitle);
        background.setFillColor(&objc2_app_kit::NSColor::windowBackgroundColor());
        background.setBorderWidth(0.0);
        content.addSubview_positioned_relativeTo(
            &background,
            objc2_app_kit::NSWindowOrderingMode::Below,
            None,
        );
    }
    require(
        c.system.state() == 1 && c.display.state() == 0,
        "legacy default must be system sleep",
    )?;
    require(
        !c.system.isEnabled() && !c.display.isEnabled(),
        "disabled feature must disable both radios",
    )?;
    // SAFETY: native controls belong to the disposable test app. All light
    // writes are routed through the memory driver installed above.
    unsafe { c.enabled.performClick(None) };
    require(
        c.system.isEnabled() && c.display.isEnabled(),
        "enabling idle brightness did not enable the radios",
    )?;
    require(
        value.load(Ordering::Relaxed) == 153,
        "enabling idle settings must not preview brightness",
    )?;
    check_notifications(&value, power::Trigger::SystemSleep)?;
    let capture_dir = std::env::var_os("LUXMINI_IDLE_PREVIEW_DIR").map(std::path::PathBuf::from);
    if let Some(directory) = &capture_dir {
        std::fs::create_dir_all(directory)?;
        capture(&c.window, &directory.join("system-sleep.png"))?;
    }
    // SAFETY: same isolated native radio action; no physical output is initialized.
    unsafe { c.display.performClick(None) };
    require(
        c.display.state() == 1 && c.system.state() == 0,
        "display radio is not exclusive",
    )?;
    require(
        preferences::load_sleep_settings().trigger == power::Trigger::DisplaySleep,
        "display trigger was not saved",
    )?;
    check_notifications(&value, power::Trigger::DisplaySleep)?;
    c.window.close();
    settings::open(&handler, mtm);
    require(
        c.display.state() == 1 && c.system.state() == 0,
        "reopening settings lost the selected trigger",
    )?;
    if let Some(directory) = &capture_dir {
        capture(&c.window, &directory.join("display-sleep.png"))?;
    }
    // SAFETY: clicking the switch only disables this fixture's own idle setting.
    unsafe { c.enabled.performClick(None) };
    require(
        !preferences::load_sleep_settings().enabled,
        "disabling idle brightness was not saved",
    )?;
    require(
        !c.system.isEnabled() && !c.display.isEnabled(),
        "disabling idle brightness did not disable the radios",
    )?;
    c.window.close();
    light::shutdown();
    println!("PASS: native idle radios, persisted selection, display/system notifications and restoration");
    Ok(())
}
