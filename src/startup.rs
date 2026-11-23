//! A one-time, explicit launch-at-login choice after light control is ready.

use std::cell::Cell;
use std::io;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAppearance, NSApplication, NSBackingStoreType, NSBox, NSBoxType, NSButton, NSButtonType,
    NSColor, NSFont, NSTextField, NSTitlePosition, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

const WIDTH: f64 = 520.0;
const HEIGHT: f64 = 350.0;
const CONTINUE: isize = 1000;
const DEFER: isize = 1001;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Offer {
    NewInstallation,
    ExistingInstallation,
}

impl Offer {
    const fn initially_enabled(self) -> bool {
        matches!(self, Self::NewInstallation)
    }
}

const fn offer_for(existing: bool, decided: bool, enabled: bool) -> Option<Offer> {
    if decided || enabled {
        None
    } else if existing {
        Some(Offer::ExistingInstallation)
    } else {
        Some(Offer::NewInstallation)
    }
}

fn apply_choice(
    enabled: bool,
    register: impl FnOnce() -> io::Result<()>,
    remember: impl FnOnce(),
) -> io::Result<()> {
    if enabled {
        register()?;
    }
    remember();
    Ok(())
}

/// The startup choice replaces the old setup completion card when it is pending.
pub fn needs_offer() -> bool {
    !cfg!(feature = "field-test")
        && !crate::preferences::login_offer_decided()
        && !crate::launch_at_login::is_enabled()
}

/// Preview and field-test builds never register a service through this prompt.
pub fn offer_if_needed(mtm: MainThreadMarker, existing: bool) {
    if cfg!(feature = "field-test") || crate::preferences::login_offer_decided() {
        return;
    }
    let enabled = crate::launch_at_login::is_enabled();
    if enabled {
        // Remember the observed system choice so disabling it in macOS later
        // does not cause this application to ask again.
        crate::preferences::mark_login_offer_decided();
        return;
    }
    let Some(offer) = offer_for(existing, false, enabled) else {
        return;
    };
    let ui = StartupWindow::new(mtm, offer);
    loop {
        let response = ui.run_modal(mtm);
        if !matches!(response, CONTINUE | DEFER) {
            // A native Quit is not an answer and must not enable a service.
            break;
        }
        let enabled = response == CONTINUE && ui.is_checked();
        match apply_choice(
            enabled,
            || crate::launch_at_login::set_enabled(true),
            crate::preferences::mark_login_offer_decided,
        ) {
            Ok(()) => break,
            Err(error) => {
                eprintln!("launch-at-login setup failed: {error}");
                ui.show_error();
            }
        }
    }
    ui.window.close();
}

struct StartupActionsIvars {
    response: Cell<isize>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "LuxMiniStartupActions"]
    #[ivars = StartupActionsIvars]
    struct StartupActions;

    impl StartupActions {
        #[unsafe(method(continue:))]
        fn continue_action(&self, _sender: &AnyObject) {
            self.finish(CONTINUE);
        }

        #[unsafe(method(defer:))]
        fn defer_action(&self, _sender: &AnyObject) {
            self.finish(DEFER);
        }
    }

    unsafe impl NSObjectProtocol for StartupActions {}
);

impl StartupActions {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(StartupActionsIvars {
            response: Cell::new(0),
        });
        // SAFETY: forwarding initialization to NSObject for the allocated object.
        unsafe { msg_send![super(this), init] }
    }

    fn finish(&self, response: isize) {
        self.ivars().response.set(response);
        if let Some(mtm) = MainThreadMarker::new() {
            let app = NSApplication::sharedApplication(mtm);
            if app.modalWindow().is_some() {
                app.stopModalWithCode(response);
            }
        }
    }
}

struct StartupWindow {
    window: Retained<NSWindow>,
    checkbox: Retained<NSButton>,
    status: Retained<NSTextField>,
    defer: Retained<NSButton>,
    actions: Retained<StartupActions>,
    #[cfg(debug_assertions)]
    primary: Retained<NSButton>,
}

impl StartupWindow {
    #[allow(clippy::too_many_lines)] // One fixed native layout, shared by both first-run states.
    fn new(mtm: MainThreadMarker, offer: Offer) -> Self {
        let actions = StartupActions::new(mtm);
        // SAFETY: valid native window initializer on the main thread.
        let window: Retained<NSWindow> = unsafe {
            msg_send![NSWindow::alloc(mtm),
                initWithContentRect: rect(0.0, 0.0, WIDTH, HEIGHT),
                styleMask: NSWindowStyleMask::Titled,
                backing: NSBackingStoreType::Buffered, defer: false]
        };
        window.setTitle(&NSString::from_str("LuxMini"));
        window.setTitlebarAppearsTransparent(true);
        window.setBackgroundColor(Some(&color(0.035)));
        // SAFETY: this retained window is owned until the modal session ends.
        unsafe { window.setReleasedWhenClosed(false) };
        if let Some(appearance) =
            NSAppearance::appearanceNamed(&NSString::from_str("NSAppearanceNameDarkAqua"))
        {
            // SAFETY: NSWindow implements NSAppearanceCustomization and retains its appearance.
            unsafe {
                let _: () = msg_send![&window, setAppearance: &*appearance];
            }
        }
        window.center();
        // SAFETY: initializes a content view with the window's dimensions.
        let content: Retained<NSView> =
            unsafe { msg_send![NSView::alloc(mtm), initWithFrame: rect(0.0, 0.0, WIDTH, HEIGHT)] };
        window.setContentView(Some(&content));
        // A content background also survives native view exports, unlike the
        // window-only background that AppKit omits when printing the view.
        let background: Retained<NSBox> = unsafe {
            // SAFETY: a native custom box covering the valid content rectangle.
            msg_send![NSBox::alloc(mtm), initWithFrame: rect(0.0, 0.0, WIDTH, HEIGHT)]
        };
        background.setBoxType(NSBoxType::Custom);
        background.setTitlePosition(NSTitlePosition::NoTitle);
        background.setFillColor(&color(0.035));
        background.setBorderWidth(0.0);
        content.addSubview(&background);
        content.addSubview(&label(
            mtm,
            if offer == Offer::NewInstallation {
                "SETUP COMPLETE"
            } else {
                "STARTUP"
            },
            rect(32.0, 291.0, 456.0, 18.0),
            11.0,
            false,
        ));
        content.addSubview(&label(
            mtm,
            if offer == Offer::NewInstallation {
                "Your light is ready."
            } else {
                "Ready after every restart."
            },
            rect(32.0, 249.0, 456.0, 34.0),
            26.0,
            true,
        ));
        content.addSubview(&label(
            mtm,
            "Keep your light settings working when you sign in.",
            rect(32.0, 216.0, 456.0, 24.0),
            14.0,
            false,
        ));

        // SAFETY: native custom box initializer and standard appearance setters.
        let option: Retained<NSBox> =
            unsafe { msg_send![NSBox::alloc(mtm), initWithFrame: rect(28.0, 121.0, 464.0, 80.0)] };
        option.setBoxType(NSBoxType::Custom);
        option.setTitlePosition(NSTitlePosition::NoTitle);
        option.setFillColor(&color(0.085));
        option.setBorderWidth(0.0);
        option.setCornerRadius(11.0);
        content.addSubview(&option);

        // SAFETY: initializes a native checkbox without application-specific side effects.
        let checkbox: Retained<NSButton> = unsafe {
            msg_send![NSButton::alloc(mtm), initWithFrame: rect(43.0, 163.0, 420.0, 25.0)]
        };
        checkbox.setButtonType(NSButtonType::Switch);
        checkbox.setTitle(&NSString::from_str("Launch LuxMini at login"));
        checkbox.setFont(Some(&NSFont::systemFontOfSize(14.0)));
        checkbox.setState(isize::from(offer.initially_enabled()));
        content.addSubview(&checkbox);
        content.addSubview(&label(
            mtm,
            "Apply your settings automatically after a restart.",
            rect(65.0, 134.0, 402.0, 22.0),
            12.0,
            false,
        ));

        let status = label(
            mtm,
            "Starts quietly in the menu bar. Change this anytime in Settings.",
            rect(32.0, 72.0, 456.0, 37.0),
            12.0,
            false,
        );
        content.addSubview(&status);
        let primary = button(
            mtm,
            &actions,
            "Open LuxMini",
            sel!(continue:),
            rect(325.0, 23.0, 169.0, 36.0),
        );
        primary.setKeyEquivalent(&NSString::from_str("\r"));
        content.addSubview(&primary);
        let defer = button(
            mtm,
            &actions,
            "Not now",
            sel!(defer:),
            rect(26.0, 23.0, 112.0, 36.0),
        );
        defer.setKeyEquivalent(&NSString::from_str("\u{1b}"));
        defer.setHidden(offer == Offer::NewInstallation);
        content.addSubview(&defer);
        Self {
            window,
            checkbox,
            status,
            defer,
            actions,
            #[cfg(debug_assertions)]
            primary,
        }
    }

    fn is_checked(&self) -> bool {
        self.checkbox.state() == 1
    }

    fn run_modal(&self, mtm: MainThreadMarker) -> isize {
        self.actions.ivars().response.set(0);
        let app = NSApplication::sharedApplication(mtm);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        self.window.makeKeyAndOrderFront(None);
        app.runModalForWindow(&self.window)
    }

    fn show_error(&self) {
        self.status.setStringValue(&NSString::from_str(
            "macOS could not enable automatic startup. Try again, or choose Not now.",
        ));
        self.defer.setHidden(false);
        self.window.displayIfNeeded();
    }
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

fn color(white: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(white, white, white, 1.0)
}

fn label(
    mtm: MainThreadMarker,
    text: &str,
    frame: NSRect,
    size: f64,
    bold: bool,
) -> Retained<NSTextField> {
    // SAFETY: initializes a native, noneditable label with a valid frame.
    let field: Retained<NSTextField> =
        unsafe { msg_send![NSTextField::alloc(mtm), initWithFrame: frame] };
    field.setStringValue(&NSString::from_str(text));
    let font = if bold {
        NSFont::boldSystemFontOfSize(size)
    } else {
        NSFont::systemFontOfSize(size)
    };
    field.setFont(Some(&font));
    field.setEditable(false);
    field.setSelectable(false);
    field.setBezeled(false);
    field.setDrawsBackground(false);
    field.setTextColor(Some(&color(if bold { 0.97 } else { 0.75 })));
    field
}

fn button(
    mtm: MainThreadMarker,
    target: &StartupActions,
    title: &str,
    action: objc2::runtime::Sel,
    frame: NSRect,
) -> Retained<NSButton> {
    // SAFETY: native button initialization and a retained target implementing the selector.
    unsafe {
        let button: Retained<NSButton> = msg_send![NSButton::alloc(mtm), initWithFrame: frame];
        button.setTitle(&NSString::from_str(title));
        button.setBezelStyle(objc2_app_kit::NSBezelStyle::Push);
        button.setTarget(Some(target));
        button.setAction(Some(action));
        button
    }
}

/// Export or inspect the real native views without preferences, hardware, network or login changes.
#[cfg(debug_assertions)]
pub fn preview(mtm: MainThreadMarker) {
    let scenario = std::env::var("LUXMINI_PREVIEW_STARTUP").unwrap_or_default();
    if scenario == "check" {
        if let Err(error) = check_native_controls(mtm) {
            eprintln!("startup native control checks failed: {error}");
            std::process::exit(2);
        }
        println!("PASS: native checkbox toggles, button targets, hit regions and existing-install defaults");
        return;
    }
    let offer = if scenario == "existing" {
        Offer::ExistingInstallation
    } else {
        Offer::NewInstallation
    };
    let ui = StartupWindow::new(mtm, offer);
    if scenario == "disabled" {
        ui.checkbox.setState(0);
    }
    if scenario == "error" {
        ui.show_error();
    }
    if let Some(path) = std::env::var_os("LUXMINI_PREVIEW_STARTUP_EXPORT") {
        ui.window.makeKeyAndOrderFront(None);
        ui.window.display();
        if let Some(content) = ui.window.contentView() {
            content.layoutSubtreeIfNeeded();
            if let Some(bitmap) = content.bitmapImageRepForCachingDisplayInRect(content.bounds()) {
                content.cacheDisplayInRect_toBitmapImageRep(content.bounds(), &bitmap);
                // SAFETY: PNG accepts an empty properties dictionary; AppKit
                // owns the pixels captured from this preview's own native view.
                let data = unsafe {
                    bitmap.representationUsingType_properties(
                        objc2_app_kit::NSBitmapImageFileType::PNG,
                        &objc2_foundation::NSDictionary::new(),
                    )
                };
                if !data.is_some_and(|data| {
                    data.writeToFile_atomically(&NSString::from_str(&path.to_string_lossy()), true)
                }) {
                    eprintln!("could not export startup preview");
                }
            }
        }
    } else {
        let response = ui.run_modal(mtm);
        println!(
            "startup_preview_response={response} enabled={}",
            ui.is_checked()
        );
    }
    ui.window.close();
}

#[cfg(debug_assertions)]
fn check_native_controls(mtm: MainThreadMarker) -> io::Result<()> {
    fn require(value: bool, message: &str) -> io::Result<()> {
        if value {
            Ok(())
        } else {
            Err(io::Error::other(message.to_owned()))
        }
    }

    let ui = StartupWindow::new(mtm, Offer::NewInstallation);
    require(ui.is_checked(), "new-install checkbox must start checked")?;
    let content = ui
        .window
        .contentView()
        .ok_or_else(|| io::Error::other("missing startup content"))?;
    for control in [&ui.checkbox, &ui.primary] {
        let bounds = control.bounds();
        let point = control.convertPoint_toView(
            NSPoint::new(bounds.size.width / 2.0, bounds.size.height / 2.0),
            Some(&content),
        );
        require(
            content
                .hitTest(point)
                .is_some_and(|hit| hit.isDescendantOf(control)),
            "startup control is obscured",
        )?;
    }
    // SAFETY: these preview-only controls target local callbacks that only
    // update the modal response; they cannot reach login registration.
    unsafe {
        ui.checkbox.performClick(None);
        require(!ui.is_checked(), "native checkbox click did not uncheck")?;
        ui.primary.performClick(None);
        require(
            ui.actions.ivars().response.get() == CONTINUE,
            "primary button target did not fire",
        )?;
    }
    ui.window.close();

    let existing = StartupWindow::new(mtm, Offer::ExistingInstallation);
    require(
        !existing.is_checked(),
        "existing-install checkbox must start unchecked",
    )?;
    require(
        !existing.defer.isHidden(),
        "existing-install defer action must be visible",
    )?;
    // SAFETY: this preview-only defer target only records DEFER, without storage or services.
    unsafe { existing.defer.performClick(None) };
    require(
        existing.actions.ivars().response.get() == DEFER,
        "defer button target did not fire",
    )?;
    existing.window.close();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{apply_choice, offer_for, Offer};
    use std::cell::Cell;
    use std::io;

    #[test]
    fn new_installations_start_with_a_visible_checked_option() {
        assert_eq!(offer_for(false, false, false), Some(Offer::NewInstallation));
        assert!(Offer::NewInstallation.initially_enabled());
    }

    #[test]
    fn existing_installations_require_an_explicit_enable_choice() {
        assert_eq!(
            offer_for(true, false, false),
            Some(Offer::ExistingInstallation)
        );
        assert!(!Offer::ExistingInstallation.initially_enabled());
    }

    #[test]
    fn completed_choices_and_enabled_services_are_not_offered_again() {
        assert_eq!(offer_for(false, true, false), None);
        assert_eq!(offer_for(true, true, false), None);
        assert_eq!(offer_for(true, false, true), None);
    }

    #[test]
    fn declining_records_the_choice_without_touching_login_items() {
        let registered = Cell::new(false);
        let remembered = Cell::new(false);
        let result = apply_choice(
            false,
            || {
                registered.set(true);
                Ok(())
            },
            || remembered.set(true),
        );
        assert!(result.is_ok());
        assert!(!registered.get());
        assert!(remembered.get());
    }

    #[test]
    fn failed_registration_does_not_record_a_successful_choice() {
        let remembered = Cell::new(false);
        let result = apply_choice(
            true,
            || Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            || remembered.set(true),
        );
        assert!(result.is_err());
        assert!(!remembered.get());
    }

    #[test]
    fn successful_registration_records_the_choice() {
        let registered = Cell::new(false);
        let remembered = Cell::new(false);
        let result = apply_choice(
            true,
            || {
                registered.set(true);
                Ok(())
            },
            || remembered.set(true),
        );
        assert!(result.is_ok());
        assert!(registered.get());
        assert!(remembered.get());
    }
}
