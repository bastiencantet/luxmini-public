//! First-run setup: verify the model profile, privileged helper, and visible LED response.

use std::cell::{Cell, RefCell};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{
    define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSBox, NSBoxType, NSButton, NSColor, NSFont, NSImage,
    NSTextAlignment, NSTextField, NSTitlePosition, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSTimer};

use crate::helper::Helper;
use crate::led::LedState;
use crate::preferences;
use crate::profile::DeviceProfile;

const WIDTH: f64 = 560.0;
const HEIGHT: f64 = 530.0;
const PRIMARY: isize = 1000;
const SECONDARY: isize = 1001;
const FADE_FAILED: isize = 1002;
const FADE_STEPS: u32 = 50;
const FADE_STEP_DELAY: Duration = Duration::from_millis(40);
const ARTWORK_FRAME: NSRect = NSRect::new(NSPoint::new(32.0, 164.0), NSSize::new(496.0, 260.0));

struct FadeShared {
    stop: AtomicBool,
    failed: AtomicBool,
    level: AtomicU8,
}

impl FadeShared {
    const fn new() -> Self {
        Self {
            stop: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            level: AtomicU8::new(255),
        }
    }
}

fn model_name(model: &str) -> &str {
    match model {
        "Mac16,10" => "Mac mini M4",
        "Mac16,11" => "Mac mini M4 Pro",
        "Mac14,3" => "Mac mini M2",
        "Mac14,12" => "Mac mini M2 Pro",
        "Mac18,5" => "Mac mini M6",
        "Mac17,16" => "Mac mini M5 Pro",
        "Mac17,14" | "Mac17,15" => "Mac Studio M5",
        "Mac13,1" | "Mac13,2" | "Mac14,13" | "Mac14,14" | "Mac15,14" | "Mac16,9" => "Mac Studio",
        _ if model.starts_with("Macmini") => "Mac mini",
        _ => "Mac",
    }
}

#[cfg(test)]
mod tests {
    use super::{artwork, fade_level, model_name, FADE_STEPS};

    #[test]
    fn m4_setup_uses_the_matching_mac_mini_artwork() {
        assert_eq!(model_name("Mac16,10"), "Mac mini M4");
        assert_eq!(artwork("Mac16,10").0, "welcome-mini-m4-off.jpg");
        assert_eq!(artwork("Mac16,10").1, "welcome-mini-m4.png");
        assert_eq!(artwork("Mac16,11").0, "welcome-mini-m4-off.jpg");
    }

    #[test]
    fn studio_setup_does_not_show_a_mac_mini() {
        assert_eq!(artwork("Mac16,9").0, "welcome-studio-off.jpg");
        assert_eq!(artwork("Mac16,9").1, "welcome-studio.png");
    }

    #[test]
    fn legacy_mini_uses_matching_led_states() {
        assert_eq!(artwork("Mac14,3").0, "welcome-mini-legacy-off.jpg");
        assert_eq!(artwork("Mac14,3").1, "welcome-mini-legacy.png");
    }

    #[test]
    fn fade_curve_is_smooth_and_wraps() {
        assert_eq!(fade_level(0), 255);
        assert_eq!(fade_level(FADE_STEPS / 2), 0);
        assert_eq!(fade_level(FADE_STEPS), 255);
        assert!(fade_level(1) < fade_level(0));
        assert!(fade_level(FADE_STEPS - 1) < fade_level(0));
    }
}

fn artwork(model: &str) -> (&'static str, &'static str, NSRect) {
    if matches!(model, "Mac16,10" | "Mac16,11" | "Mac18,5" | "Mac17,16") {
        (
            "welcome-mini-m4-off.jpg",
            "welcome-mini-m4.png",
            rect(-392.0, -180.0, 1280.0, 691.0),
        )
    } else if matches!(
        model,
        "Mac13,1"
            | "Mac13,2"
            | "Mac14,13"
            | "Mac14,14"
            | "Mac15,14"
            | "Mac16,9"
            | "Mac17,14"
            | "Mac17,15"
    ) {
        (
            "welcome-studio-off.jpg",
            "welcome-studio.png",
            rect(-128.0, -245.0, 752.0, 752.0),
        )
    } else {
        (
            "welcome-mini-legacy-off.jpg",
            "welcome-mini-legacy.png",
            rect(-128.0, -245.0, 752.0, 752.0),
        )
    }
}

fn load_artwork(name: &str) -> Option<Retained<NSImage>> {
    let bundled = std::env::current_exe().ok().and_then(|path| {
        path.parent()?
            .parent()
            .map(|contents| contents.join("Resources").join(name))
    });
    #[cfg(debug_assertions)]
    let bundled = bundled.filter(|path| path.is_file()).or_else(|| {
        Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets")
                .join(name),
        )
    });
    let path = bundled?;
    NSImage::initWithContentsOfFile(
        NSImage::alloc(),
        &NSString::from_str(&path.to_string_lossy()),
    )
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

fn color(white: f64, alpha: f64) -> Retained<NSColor> {
    NSColor::colorWithCalibratedWhite_alpha(white, alpha)
}

fn label(
    mtm: MainThreadMarker,
    text: &str,
    frame: NSRect,
    size: f64,
    strong: bool,
) -> Retained<NSTextField> {
    // SAFETY: initWithFrame: initializes the freshly allocated text field.
    let field: Retained<NSTextField> =
        unsafe { msg_send![NSTextField::alloc(mtm), initWithFrame: frame] };
    field.setStringValue(&NSString::from_str(text));
    field.setEditable(false);
    field.setBezeled(false);
    field.setDrawsBackground(false);
    field.setSelectable(false);
    field.setAlignment(NSTextAlignment::Center);
    field.setTextColor(Some(&color(if strong { 1.0 } else { 0.68 }, 1.0)));
    let font = NSFont::systemFontOfSize_weight(size, if strong { 0.23 } else { 0.0 });
    field.setFont(Some(&font));
    field
}

fn light(mtm: MainThreadMarker, frame: NSRect, radius: f64) -> Retained<NSBox> {
    // SAFETY: initWithFrame: initializes the freshly allocated box.
    let box_view: Retained<NSBox> = unsafe { msg_send![NSBox::alloc(mtm), initWithFrame: frame] };
    box_view.setBoxType(NSBoxType::Custom);
    box_view.setTitlePosition(NSTitlePosition::NoTitle);
    box_view.setBorderWidth(0.0);
    box_view.setCornerRadius(radius);
    box_view.setFillColor(&color(1.0, 0.0));
    box_view
}

struct ArtworkIvars {
    off: Option<Retained<NSImage>>,
    on: Option<Retained<NSImage>>,
    image_frame: NSRect,
    level: Cell<u8>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "LuxMiniWelcomeArtwork"]
    #[ivars = ArtworkIvars]
    struct WelcomeArtwork;

    impl WelcomeArtwork {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let frame = self.ivars().image_frame;
            let source = rect(0.0, 0.0, 0.0, 0.0);
            if let Some(off) = &self.ivars().off {
                // SAFETY: NSImage draws into the current AppKit graphics context;
                // SourceOver is NSCompositingOperation value 2.
                unsafe {
                    let _: () = msg_send![off, drawInRect: frame, fromRect: source, operation: 2usize, fraction: 1.0f64];
                }
            }
            if let Some(on) = &self.ivars().on {
                let alpha = if self.ivars().off.is_some() {
                    f64::from(self.ivars().level.get()) / 255.0
                } else {
                    1.0
                };
                // SAFETY: the second image is composited over the same pixel-aligned
                // frame, so only the LED difference changes during the fade.
                unsafe {
                    let _: () = msg_send![on, drawInRect: frame, fromRect: source, operation: 2usize, fraction: alpha];
                }
            }
        }
    }

    unsafe impl NSObjectProtocol for WelcomeArtwork {}
);

impl WelcomeArtwork {
    fn new(
        mtm: MainThreadMarker,
        off: Option<Retained<NSImage>>,
        on: Option<Retained<NSImage>>,
        image_frame: NSRect,
    ) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(ArtworkIvars {
            off,
            on,
            image_frame,
            level: Cell::new(0),
        });
        // SAFETY: initializes the newly allocated native NSView subclass.
        unsafe { msg_send![super(this), initWithFrame: ARTWORK_FRAME] }
    }

    fn set_level(&self, level: u8) {
        self.ivars().level.set(level);
        self.setNeedsDisplay(true);
    }
}

pub struct WelcomeActionsIvars {
    artwork: RefCell<Option<Retained<WelcomeArtwork>>>,
    fade: RefCell<Option<Arc<FadeShared>>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "LuxMiniWelcomeActions"]
    #[ivars = WelcomeActionsIvars]
    struct WelcomeActions;

    impl WelcomeActions {
        #[unsafe(method(primary:))]
        fn primary(&self, _sender: &AnyObject) {
            if let Some(mtm) = MainThreadMarker::new() {
                NSApplication::sharedApplication(mtm).stopModalWithCode(PRIMARY);
            }
        }

        #[unsafe(method(secondary:))]
        fn secondary(&self, _sender: &AnyObject) {
            if let Some(mtm) = MainThreadMarker::new() {
                NSApplication::sharedApplication(mtm).stopModalWithCode(SECONDARY);
            }
        }

        #[unsafe(method(animateLight:))]
        fn animate_light(&self, _timer: &NSTimer) {
            if let Some(fade) = self.ivars().fade.borrow().as_ref() {
                if let Some(image) = self.ivars().artwork.borrow().as_ref() {
                    let level = fade.level.load(Ordering::Relaxed);
                    image.set_level(level);
                }
                if fade.failed.load(Ordering::Acquire) {
                    if let Some(mtm) = MainThreadMarker::new() {
                        NSApplication::sharedApplication(mtm).stopModalWithCode(FADE_FAILED);
                    }
                }
            }
        }
    }

    unsafe impl NSObjectProtocol for WelcomeActions {}
);

impl WelcomeActions {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(WelcomeActionsIvars {
            artwork: RefCell::new(None),
            fade: RefCell::new(None),
        });
        // SAFETY: forwarding init to NSObject on a newly allocated instance.
        unsafe { msg_send![super(this), init] }
    }
}

struct WelcomeWindow {
    window: Retained<NSWindow>,
    status: Retained<NSTextField>,
    primary: Retained<NSButton>,
    secondary: Retained<NSButton>,
    primary_label: Retained<NSTextField>,
    secondary_label: Retained<NSTextField>,
    secondary_box: Retained<NSBox>,
    artwork: Retained<WelcomeArtwork>,
    actions: Retained<WelcomeActions>,
}

impl WelcomeWindow {
    #[allow(clippy::too_many_lines)] // Fixed native layout for a single setup card.
    fn new(mtm: MainThreadMarker, model: &str) -> Self {
        let actions = WelcomeActions::new(mtm);
        // SAFETY: designated NSWindow initializer with a valid content rectangle.
        let window: Retained<NSWindow> = unsafe {
            msg_send![
                NSWindow::alloc(mtm),
                initWithContentRect: rect(0.0, 0.0, WIDTH, HEIGHT),
                styleMask: NSWindowStyleMask::Titled,
                backing: NSBackingStoreType::Buffered,
                defer: false,
            ]
        };
        window.setTitle(&NSString::from_str("LuxMini Setup"));
        window.setBackgroundColor(Some(&color(0.035, 1.0)));
        window.setTitlebarAppearsTransparent(true);
        window.center();

        let content = window.contentView().unwrap_or_else(|| {
            // SAFETY: a newly initialized NSWindow normally has a content view.
            let view: Retained<NSView> = unsafe {
                msg_send![NSView::alloc(mtm), initWithFrame: rect(0.0, 0.0, WIDTH, HEIGHT)]
            };
            window.setContentView(Some(&view));
            view
        });
        let background = light(mtm, rect(0.0, 0.0, WIDTH, HEIGHT), 0.0);
        background.setFillColor(&color(0.035, 1.0));
        content.addSubview(&background);

        let (off_name, on_name, artwork_frame) = artwork(model);
        // The two supplied images have matching pixel dimensions. A single
        // clipped view draws the off image first, then blends the on image at
        // the current LED level, with no image-view alpha ambiguity.
        let artwork = WelcomeArtwork::new(
            mtm,
            load_artwork(off_name),
            load_artwork(on_name),
            artwork_frame,
        );
        content.addSubview(&artwork);
        let artwork_outline = light(mtm, ARTWORK_FRAME, 14.0);
        artwork_outline.setFillColor(&color(0.0, 0.0));
        artwork_outline.setBorderColor(&color(1.0, 0.12));
        artwork_outline.setBorderWidth(1.0);
        content.addSubview(&artwork_outline);
        actions
            .ivars()
            .artwork
            .borrow_mut()
            .replace(artwork.clone());

        content.addSubview(&label(
            mtm,
            "Welcome to LuxMini",
            rect(40.0, 469.0, 480.0, 37.0),
            27.0,
            true,
        ));
        content.addSubview(&label(
            mtm,
            &format!("A quick check for your {}", model_name(model)),
            rect(40.0, 437.0, 480.0, 23.0),
            14.0,
            false,
        ));

        let status = label(
            mtm,
            "We will check the profile, helper, and real front LED.",
            rect(45.0, 95.0, 470.0, 52.0),
            13.0,
            false,
        );
        content.addSubview(&status);

        let primary_box = light(mtm, rect(326.0, 36.0, 184.0, 36.0), 9.0);
        primary_box.setFillColor(&color(1.0, 1.0));
        content.addSubview(&primary_box);
        let primary_label = label(
            mtm,
            "Get Started",
            rect(326.0, 41.0, 184.0, 22.0),
            14.0,
            true,
        );
        primary_label.setTextColor(Some(&color(0.06, 1.0)));
        content.addSubview(&primary_label);

        let secondary_box = light(mtm, rect(50.0, 36.0, 130.0, 36.0), 9.0);
        secondary_box.setFillColor(&color(0.13, 1.0));
        content.addSubview(&secondary_box);
        let secondary_label = label(mtm, "Quit", rect(50.0, 41.0, 130.0, 22.0), 14.0, true);
        content.addSubview(&secondary_label);

        // SAFETY: initWithFrame: initializes both buttons.
        let primary: Retained<NSButton> = unsafe {
            msg_send![NSButton::alloc(mtm), initWithFrame: rect(326.0, 36.0, 184.0, 36.0)]
        };
        // SAFETY: initWithFrame: initializes the second freshly allocated button.
        let secondary: Retained<NSButton> = unsafe {
            msg_send![NSButton::alloc(mtm), initWithFrame: rect(50.0, 36.0, 130.0, 36.0)]
        };
        primary.setTitle(&NSString::from_str("Get Started"));
        secondary.setTitle(&NSString::from_str("Quit"));
        primary.setTransparent(true);
        secondary.setTransparent(true);
        // SAFETY: both selectors are implemented by the retained WelcomeActions object.
        unsafe {
            let _: () = msg_send![&primary, setBezelStyle: 1isize];
            let _: () = msg_send![&secondary, setBezelStyle: 1isize];
            let _: () = msg_send![&primary, setTarget: &*actions];
            let _: () = msg_send![&primary, setAction: sel!(primary:)];
            let _: () = msg_send![&secondary, setTarget: &*actions];
            let _: () = msg_send![&secondary, setAction: sel!(secondary:)];
        }
        content.addSubview(&primary);
        content.addSubview(&secondary);
        Self {
            window,
            status,
            primary,
            secondary,
            primary_label,
            secondary_label,
            secondary_box,
            artwork,
            actions,
        }
    }

    fn set_status(&self, text: &str) {
        self.status.setStringValue(&NSString::from_str(text));
        self.window.displayIfNeeded();
    }

    fn set_light(&self, value: u8) {
        self.artwork.set_level(value);
        self.window.displayIfNeeded();
    }

    fn set_prompt(&self, status: &str, primary: &str, secondary: Option<&str>) {
        self.set_status(status);
        self.primary.setTitle(&NSString::from_str(primary));
        self.primary_label
            .setStringValue(&NSString::from_str(primary));
        if let Some(secondary) = secondary {
            self.secondary.setTitle(&NSString::from_str(secondary));
            self.secondary_label
                .setStringValue(&NSString::from_str(secondary));
            self.secondary.setHidden(false);
            self.secondary_label.setHidden(false);
            self.secondary_box.setHidden(false);
        } else {
            self.secondary.setHidden(true);
            self.secondary_label.setHidden(true);
            self.secondary_box.setHidden(true);
        }
        self.window.displayIfNeeded();
    }

    fn run_modal(&self) -> isize {
        let Some(mtm) = MainThreadMarker::new() else {
            return SECONDARY;
        };
        let app = NSApplication::sharedApplication(mtm);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        self.window.makeKeyAndOrderFront(None);
        app.runModalForWindow(&self.window)
    }

    fn ask(&self, status: &str, primary: &str, secondary: Option<&str>) -> bool {
        self.set_prompt(status, primary, secondary);
        self.run_modal() == PRIMARY
    }

    fn ask_fade_confirmation(&self, fade: &Arc<FadeShared>) -> isize {
        self.set_prompt(
            "Watch the real front LED. Does it keep fading?",
            "Yes, it worked",
            Some("No"),
        );
        self.actions.ivars().fade.borrow_mut().replace(fade.clone());
        // SAFETY: WelcomeActions implements animateLight: and outlives the modal timer.
        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                0.04,
                &self.actions,
                sel!(animateLight:),
                None,
                true,
            )
        };
        let response = self.run_modal();
        timer.invalidate();
        self.actions.ivars().fade.borrow_mut().take();
        response
    }

    fn close(&self) {
        self.window.close();
    }
}

#[cfg(debug_assertions)]
pub fn preview(mtm: MainThreadMarker) {
    let model = std::env::var("LUXMINI_PREVIEW_MODEL").unwrap_or_else(|_| "Mac16,10".to_owned());
    let ui = WelcomeWindow::new(mtm, &model);
    if std::env::var("LUXMINI_PREVIEW_SCENARIO").as_deref() == Ok("helper_permission") {
        let error = io::Error::from(io::ErrorKind::PermissionDenied);
        ui.set_prompt(helper_failure_message(&error), "Retry", Some("Quit"));
        ui.set_light(0);
    }
    if let Ok(value) = std::env::var("LUXMINI_PREVIEW_LIGHT") {
        if let Ok(value) = value.parse::<u8>() {
            ui.set_light(value);
        }
    }
    if let Some(path) = std::env::var_os("LUXMINI_PREVIEW_EXPORT") {
        ui.window.makeKeyAndOrderFront(None);
        ui.window.displayIfNeeded();
        if let Some(content) = ui.window.contentView() {
            let data = content.dataWithPDFInsideRect(content.bounds());
            let path = NSString::from_str(&path.to_string_lossy());
            if !data.writeToFile_atomically(&path, true) {
                eprintln!("could not export welcome preview");
            }
        }
        ui.close();
        return;
    }
    let _ = ui.ask(
        "We will check the profile, helper, and real front LED.",
        "Get Started",
        Some("Close"),
    );
    ui.close();
}

fn fade_level(step: u32) -> u8 {
    let phase = f64::from(step % FADE_STEPS) * std::f64::consts::TAU / f64::from(FADE_STEPS);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    // The cosine expression is bounded to the inclusive range 0..=255.
    let level = ((phase.cos() + 1.0) * 127.5).round() as u8;
    level
}

fn fade_loop(helper: &mut Helper, original: &[u8], shared: &FadeShared) -> io::Result<()> {
    let mut step = 0;
    let result = (|| {
        while !shared.stop.load(Ordering::Acquire) {
            let level = fade_level(step);
            helper.write_led_test(level)?;
            shared.level.store(level, Ordering::Release);
            step = (step + 1) % FADE_STEPS;
            for _ in 0..4 {
                if shared.stop.load(Ordering::Acquire) {
                    break;
                }
                thread::sleep(FADE_STEP_DELAY / 4);
            }
        }
        Ok(())
    })();
    let restore = helper.restore_profile_raw(original);
    if result.is_err() || restore.is_err() {
        shared.failed.store(true, Ordering::Release);
    }
    match (result, restore) {
        (Err(write), Err(restore)) => Err(io::Error::other(format!(
            "LED fade failed: {write}; restoration failed: {restore}"
        ))),
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

/// Keep fading the allowlisted LED until the user answers, then restore its exact bytes.
/// Only `AppKit` and the image timer run on the main thread; SMC IPC runs on a worker.
fn test_led_until_response(helper: &mut Helper, ui: &WelcomeWindow) -> io::Result<isize> {
    let original = helper.read_profile_raw()?;
    if original.len() != 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "LED profile is not two bytes",
        ));
    }
    let shared = Arc::new(FadeShared::new());
    let (response, result) = thread::scope(|scope| {
        let worker_shared = Arc::clone(&shared);
        let original_ref = &original;
        let worker_helper = &mut *helper;
        let worker = scope.spawn(move || fade_loop(worker_helper, original_ref, &worker_shared));
        let response = ui.ask_fade_confirmation(&shared);
        shared.stop.store(true, Ordering::Release);
        let result = worker
            .join()
            .unwrap_or_else(|_| Err(io::Error::other("LED fade worker panicked")));
        (response, result)
    });
    ui.set_light(0);
    if result.is_err() {
        // A second restoration attempt also covers a worker panic.
        helper.restore_profile_raw(&original)?;
    }
    result?;
    if response == FADE_FAILED {
        return Err(io::Error::other("LED fade stopped unexpectedly"));
    }
    Ok(response)
}

fn helper_failure_message(error: &io::Error) -> &'static str {
    let detail = error.to_string();
    if detail.starts_with("LED controller access denied:") {
        return "The helper is running as root, but macOS denied LED controller access. Retry or send feedback.";
    }
    if detail.starts_with("LED controller unavailable:") {
        return "No LED controller is accessible on this Mac. Send feedback with your Mac model.";
    }
    match error.kind() {
        io::ErrorKind::PermissionDenied => {
            "macOS did not grant LED helper access. Approve the admin prompt, then retry."
        }
        io::ErrorKind::NotFound => "The LED helper is missing. Reinstall LuxMini, then retry.",
        io::ErrorKind::InvalidData => {
            "The LED helper does not match this app. Reinstall LuxMini, then retry."
        }
        io::ErrorKind::TimedOut => "The LED helper did not respond. Retry or restart LuxMini.",
        _ => "The LED helper stopped or could not start. Retry or send feedback.",
    }
}

#[cfg(test)]
mod feedback_tests {
    use super::helper_failure_message;
    use std::io;

    #[test]
    fn permission_failure_has_an_actionable_message() {
        let error = io::Error::from(io::ErrorKind::PermissionDenied);
        assert!(helper_failure_message(&error).contains("admin prompt"));
    }

    #[test]
    fn version_mismatch_requests_reinstallation() {
        let error = io::Error::from(io::ErrorKind::InvalidData);
        assert!(helper_failure_message(&error).contains("Reinstall"));
    }

    #[test]
    fn smc_failure_does_not_request_an_admin_password() {
        let error = io::Error::other("LED controller access denied: open failed");
        let message = helper_failure_message(&error);
        assert!(message.contains("running as root"));
        assert!(!message.contains("admin prompt"));
    }
}

/// A pending Mac can try its existing approved profile first; failed visual
/// confirmation falls through to the bounded candidate discovery flow.
pub fn run(mtm: MainThreadMarker, model: &str, pending: bool) -> Option<LedState> {
    preferences::mark_setup_pending(model);
    let ui = WelcomeWindow::new(mtm, model);
    if !ui.ask(
        "We will check the profile, helper, and real front LED.",
        "Get Started",
        Some("Quit"),
    ) {
        ui.close();
        return None;
    }

    'setup: loop {
        ui.set_status("Checking the device profile...");
        let Some(profile) = DeviceProfile::load() else {
            if pending
                && ui.ask(
                    "This Mac needs LED profile detection.",
                    "Detect LED",
                    Some("Quit"),
                )
            {
                ui.close();
                let state = crate::onboarding::run(mtm, model)?;
                preferences::mark_setup_completed(model);
                return Some(state);
            }
            if pending
                || !ui.ask(
                    "The profile is unavailable. Check your connection and retry.",
                    "Retry",
                    Some("Quit"),
                )
            {
                ui.close();
                return None;
            }
            continue;
        };

        ui.set_status("Checking the privileged LED helper...");
        let mut helper = match Helper::spawn_with_profile(Some(profile)) {
            Ok(helper) => helper,
            Err(error) => {
                eprintln!("setup helper check failed: {error}");
                if ui.ask(helper_failure_message(&error), "Retry", Some("Quit")) {
                    continue;
                }
                ui.close();
                return None;
            }
        };

        ui.set_status("The helper has root access. Watch the real front LED now...");
        let response = match test_led_until_response(&mut helper, &ui) {
            Ok(response) => response,
            Err(error) => {
                eprintln!("setup LED test failed: {error}");
                if ui.ask(
                    "The LED profile could not be read or written. Retry the test?",
                    "Retry",
                    Some("Quit"),
                ) {
                    continue 'setup;
                }
                ui.close();
                return None;
            }
        };
        if response == PRIMARY {
            if pending {
                preferences::save_validated_profile_model(model);
            }
            preferences::mark_setup_completed(model);
            let _ = ui.ask(
                "LuxMini is ready. Your LED is verified.",
                "Open LuxMini",
                None,
            );
            ui.close();
            return Some(LedState::with_helper(helper));
        }

        if ui.ask(
            "No visible change. Try another known LED profile?",
            "Detect LED",
            Some("Quit"),
        ) {
            ui.close();
            let state = crate::onboarding::run(mtm, model)?;
            preferences::mark_setup_completed(model);
            return Some(state);
        }
        ui.close();
        return None;
    }
}
