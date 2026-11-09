//! Device presentation previews using the shared light controller and memory driver.
//! Starts before hardware discovery, preferences, telemetry, helper or API startup.

use crate::light::{self, Effect, LightState, STATE};
use crate::output::LightKind;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAppearance, NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSButton,
    NSColor, NSFont, NSSlider, NSTextAlignment, NSTextField, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSTimer};
use std::cell::Cell;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

struct SimulatorIvars {
    value: Arc<AtomicU8>,
    glow: Retained<NSTextField>,
    status: Retained<NSTextField>,
    mode: Cell<&'static str>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "LuxMiniLightSimulator"]
    #[ivars = SimulatorIvars]
    struct Simulator;

    impl Simulator {
        #[unsafe(method(tick:))]
        fn tick(&self, _timer: &AnyObject) {
            let value = self.ivars().value.load(Ordering::Relaxed);
            let level = f64::from(value) / 255.0;
            self.ivars().glow.setAlphaValue(0.08 + level * 0.92);
            self.ivars().status.setStringValue(&NSString::from_str(&format!(
                "{}  ·  {:.0} %", self.ivars().mode.get(), level * 100.0
            )));
            // SAFETY: the text field is layer-backed by panel(); CALayer accepts
            // CGFloat shadowRadius and float shadowOpacity on the main thread.
            unsafe {
                let layer: *mut AnyObject = msg_send![&self.ivars().glow, layer];
                if !layer.is_null() {
                    let _: () = msg_send![layer, setShadowRadius: 10.0 + level * 24.0];
                    #[allow(clippy::cast_possible_truncation)] // normalized finite 0..1
                    let opacity = (level * 0.9) as f32;
                    let _: () = msg_send![layer, setShadowOpacity: opacity];
                }
            }
        }

        #[unsafe(method(brightness:))]
        fn brightness(&self, slider: &NSSlider) {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped byte range
            let value = slider.doubleValue().clamp(0.0, 255.0).round() as u8;
            light::with_state(|state| state.set_brightness(value));
            self.ivars().mode.set("Fixe");
        }

        #[unsafe(method(steady:))]
        fn steady(&self, _sender: &AnyObject) {
            light::with_state(LightState::clear_effect);
            self.ivars().mode.set("Fixe");
        }

        #[unsafe(method(breathe:))]
        fn breathe(&self, _sender: &AnyObject) {
            self.effect(Effect::Pulse, "Respiration");
        }

        #[unsafe(method(blink:))]
        fn blink(&self, _sender: &AnyObject) {
            self.effect(Effect::Blink, "Clignotement");
        }

        #[unsafe(method(sos:))]
        fn sos(&self, _sender: &AnyObject) {
            self.effect(Effect::Sos, "SOS");
        }

        #[unsafe(method(toggle:))]
        fn toggle(&self, _sender: &AnyObject) {
            let on = light::read_status().is_on;
            light::with_state(|state| state.set_on(!on));
            self.ivars().mode.set(if on { "Éteinte" } else { "Fixe" });
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn terminate_after_close(&self, _app: &AnyObject) -> bool { true }

        #[unsafe(method(applicationWillTerminate:))]
        fn will_terminate(&self, _note: &AnyObject) { light::shutdown(); }
    }

    unsafe impl NSObjectProtocol for Simulator {}
);

impl Simulator {
    fn effect(&self, effect: Effect, name: &'static str) {
        light::with_state(|state| state.start_effect(effect));
        self.ivars().mode.set(name);
    }
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

fn color(red: f64, green: f64, blue: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(red, green, blue, 1.0)
}

fn label(mtm: MainThreadMarker, text: &str, frame: NSRect, size: f64) -> Retained<NSTextField> {
    // SAFETY: initializes an allocated NSTextField with a valid NSRect.
    let field: Retained<NSTextField> =
        unsafe { msg_send![NSTextField::alloc(mtm), initWithFrame: frame] };
    field.setStringValue(&NSString::from_str(text));
    field.setFont(Some(&NSFont::systemFontOfSize(size)));
    field.setEditable(false);
    field.setSelectable(false);
    field.setBezeled(false);
    field.setDrawsBackground(false);
    field.setTextColor(Some(&color(0.86, 0.88, 0.93)));
    field
}

fn panel(view: &NSView, fill: &NSColor, radius: f64) {
    // SAFETY: view is an NSView on the main thread; its layer is created by
    // setWantsLayer:. NSColor's CGColor is borrowed for the setter, which retains it.
    unsafe {
        let _: () = msg_send![view, setWantsLayer: true];
        let layer: *mut AnyObject = msg_send![view, layer];
        if !layer.is_null() {
            let cg_color = fill.CGColor();
            let _: () = msg_send![layer, setBackgroundColor: &*cg_color];
            let _: () = msg_send![layer, setCornerRadius: radius];
        }
    }
}

fn button(
    mtm: MainThreadMarker,
    target: &Simulator,
    title: &str,
    action: objc2::runtime::Sel,
    frame: NSRect,
) -> Retained<NSButton> {
    // SAFETY: allocated NSButton initialized with frame; action selectors belong
    // to Simulator and each takes exactly one object argument.
    unsafe {
        let button: Retained<NSButton> = msg_send![NSButton::alloc(mtm), initWithFrame: frame];
        button.setTitle(&NSString::from_str(title));
        button.setTarget(Some(target));
        button.setAction(Some(action));
        let _: () = msg_send![&button, setBezelStyle: 1usize];
        button
    }
}

/// Start a self-contained preview. No regular app startup runs after this returns.
#[allow(clippy::too_many_lines)] // linear native window layout
pub fn run(mtm: MainThreadMarker, kind: LightKind) {
    let shows_logo = kind == LightKind::RearLogo;
    let value = Arc::new(AtomicU8::new(255));
    let Ok(mut state) = STATE.lock() else {
        return;
    };
    *state = Some(LightState::with_output(
        crate::output::simulated(kind, value.clone()),
        crate::light::Persistence::SessionOnly,
    ));
    drop(state);

    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    // SAFETY: standard AppKit window/view initializers; objects stay retained
    // until app.run returns and the window is not released on close.
    let (window, content, lid, base): (
        Retained<NSWindow>,
        Retained<NSView>,
        Retained<NSView>,
        Retained<NSView>,
    ) = unsafe {
        let window = msg_send![NSWindow::alloc(mtm),
            initWithContentRect: rect(0.0, 0.0, 880.0, 650.0),
            styleMask: NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable,
            backing: NSBackingStoreType::Buffered, defer: false];
        (
            window,
            msg_send![NSView::alloc(mtm), initWithFrame: rect(0.0, 0.0, 880.0, 650.0)],
            msg_send![NSView::alloc(mtm), initWithFrame: if shows_logo { rect(120.0, 215.0, 640.0, 315.0) } else { rect(120.0, 300.0, 640.0, 145.0) }],
            msg_send![NSView::alloc(mtm), initWithFrame: rect(100.0, 202.0, 680.0, 12.0)],
        )
    };
    if let Some(appearance) =
        NSAppearance::appearanceNamed(&NSString::from_str("NSAppearanceNameDarkAqua"))
    {
        // SAFETY: NSWindow implements NSAppearanceCustomization; AppKit retains the appearance.
        unsafe {
            let _: () = msg_send![&window, setAppearance: &*appearance];
        }
    }
    window.setTitle(&NSString::from_str(if shows_logo {
        "LuxMini — Simulateur de pomme arrière"
    } else {
        "LuxMini — Simulateur de LED avant"
    }));
    // SAFETY: retained locally until the application loop exits; AppKit must not release it on close.
    unsafe {
        window.setReleasedWhenClosed(false);
    }
    panel(&content, &color(0.045, 0.055, 0.08), 0.0);
    panel(&lid, &color(0.19, 0.205, 0.23), 22.0);
    panel(&base, &color(0.28, 0.30, 0.33), 6.0);
    content.addSubview(&lid);
    if shows_logo {
        content.addSubview(&base);
    }
    content.addSubview(&label(mtm, "LuxMini", rect(42.0, 591.0, 500.0, 37.0), 28.0));
    content.addSubview(&label(
        mtm,
        if shows_logo {
            "Simulateur de pomme arrière"
        } else {
            "Simulateur de LED avant"
        },
        rect(43.0, 563.0, 650.0, 26.0),
        17.0,
    ));
    let badge = label(mtm, "SIMULATION", rect(691.0, 593.0, 150.0, 23.0), 12.0);
    badge.setTextColor(Some(&color(0.44, 0.87, 0.73)));
    content.addSubview(&badge);
    let glow = label(
        mtm,
        if shows_logo { "\u{f8ff}" } else { "●" },
        if shows_logo {
            rect(220.0, 75.0, 200.0, 180.0)
        } else {
            rect(535.0, 50.0, 60.0, 45.0)
        },
        if shows_logo { 140.0 } else { 28.0 },
    );
    glow.setAlignment(NSTextAlignment::Center);
    glow.setTextColor(Some(&NSColor::whiteColor()));
    // Clear background creates a layer for the glyph's glow without a filled rectangle.
    panel(&glow, &NSColor::clearColor(), 0.0);
    // SAFETY: glow is layer-backed; CALayer retains its shadow CGColor.
    unsafe {
        let layer: *mut AnyObject = msg_send![&glow, layer];
        if !layer.is_null() {
            let white = NSColor::whiteColor().CGColor();
            let _: () = msg_send![layer, setShadowColor: &*white];
            let _: () = msg_send![layer, setShadowOffset: NSSize::new(0.0, 0.0)];
        }
    }
    lid.addSubview(&glow);
    let status = label(mtm, "Fixe · 100 %", rect(295.0, 163.0, 290.0, 28.0), 17.0);
    status.setAlignment(NSTextAlignment::Center);
    content.addSubview(&status);

    let allocated = Simulator::alloc(mtm).set_ivars(SimulatorIvars {
        value,
        glow,
        status,
        mode: Cell::new("Fixe"),
    });
    // SAFETY: initialize the NSObject superclass after assigning all ivars.
    let controller: Retained<Simulator> = unsafe { msg_send![super(allocated), init] };
    for (index, (title, action)) in [
        ("Fixe", sel!(steady:)),
        ("Respiration", sel!(breathe:)),
        ("Clignoter", sel!(blink:)),
        ("SOS", sel!(sos:)),
        ("Marche / Arrêt", sel!(toggle:)),
    ]
    .into_iter()
    .enumerate()
    {
        let x = 90.0 + f64::from(u32::try_from(index).unwrap_or_default()) * 140.0;
        content.addSubview(&button(
            mtm,
            &controller,
            title,
            action,
            rect(x, 112.0, 140.0, 34.0),
        ));
    }
    content.addSubview(&label(
        mtm,
        "Luminosité fixe",
        rect(100.0, 70.0, 170.0, 23.0),
        13.0,
    ));
    // SAFETY: initialize the native slider; its callback takes NSSlider*.
    let slider: Retained<NSSlider> =
        unsafe { msg_send![NSSlider::alloc(mtm), initWithFrame: rect(265.0, 68.0, 515.0, 25.0)] };
    slider.setMinValue(0.0);
    slider.setMaxValue(255.0);
    slider.setDoubleValue(255.0);
    slider.setContinuous(true);
    // SAFETY: controller implements brightness: and outlives the slider.
    unsafe {
        slider.setTarget(Some(&controller));
        slider.setAction(Some(sel!(brightness:)));
    }
    content.addSubview(&slider);
    content.addSubview(&label(
        mtm,
        "Aperçu visuel uniquement · aucun accès au matériel ni enregistrement des réglages",
        rect(100.0, 23.0, 720.0, 24.0),
        12.0,
    ));
    // SAFETY: controller implements both application delegate selectors, and tick:
    // has the NSTimer target signature. Retained locals outlive app.run().
    let timer = unsafe {
        let _: () = msg_send![&app, setDelegate: &*controller];
        NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
            1.0 / 30.0,
            &controller,
            sel!(tick:),
            None,
            true,
        )
    };
    window.setContentView(Some(&content));
    window.center();
    window.makeKeyAndOrderFront(None);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    app.run();
    timer.invalidate();
    light::shutdown();
}

/// Preview selection is explicit and occurs before any hardware or service startup.
pub fn requested_kind() -> Option<LightKind> {
    std::env::args_os().find_map(|arg| {
        if arg == "--simulate-logo" {
            Some(LightKind::RearLogo)
        } else if arg == "--simulate-led" {
            Some(LightKind::FrontLed)
        } else {
            None
        }
    })
}
