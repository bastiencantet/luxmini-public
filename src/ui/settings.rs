//! Native toolbar settings window. The singleton is kept alive in the `Handler`
//! ivars; closing hides it and reopening repopulates the controls.

use crate::compat;
use crate::i18n;
use crate::launch_at_login;
use crate::led::read_status;
use crate::preferences;
use crate::schedule::{self, AutoDim};
use crate::ui::handler::Handler;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{msg_send, sel, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAlert, NSBackingStoreType, NSBox, NSBoxType, NSButton, NSColor, NSFont, NSImage, NSImageView,
    NSSlider, NSSwitch, NSTextField, NSTitlePosition, NSToolbar, NSToolbarDisplayMode,
    NSToolbarItem, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString};

/// Handles to the editable controls (read by `saveSettings:` / repopulated by `open`),
/// plus the native toolbar and content panes for section switching.
pub struct SettingsRefs {
    pub window: Retained<NSWindow>,
    pub toolbar: Retained<NSToolbar>,
    pub sunset: Retained<NSSwitch>,
    pub dim: Retained<NSSwitch>,
    pub time: Retained<NSTextField>,
    pub pct: Retained<NSTextField>,
    pub lat: Retained<NSTextField>,
    pub lon: Retained<NSTextField>,
    pub login: Retained<NSSwitch>,
    pub hide: Retained<NSSwitch>,
    pub api: Retained<NSSwitch>,
    pub diagnostics: Retained<NSSwitch>,
    pub led_switch: Retained<NSSwitch>,
    pub led_slider: Retained<NSSlider>,
    pub led_percent: Retained<NSTextField>,
    pub led_status: Retained<NSTextField>,
    pub led_preview_on: Option<Retained<NSImageView>>,
    pub led_preview_off: Option<Retained<NSImageView>>,
    #[cfg(debug_assertions)]
    pub save: Retained<NSButton>,
    pub sections: Vec<Retained<NSView>>,
}

const W: f64 = 800.0;
const H: f64 = 540.0;
const PANE_X: f64 = 0.0;
const PW: f64 = W;
const INSET: f64 = 42.0;
const CONTENT_WIDTH: f64 = PW - INSET * 2.0;

const TOOLBAR_IDS: [&str; 4] = [
    "luxmini.led",
    "luxmini.automation",
    "luxmini.general",
    "luxmini.advanced",
];

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

/// A rect inside a content pane, addressed top-down (`top` = distance from the top edge).
fn prect(top: f64, h: f64, x: f64, w: f64) -> NSRect {
    rect(x, H - top - h, w, h)
}

fn label(mtm: MainThreadMarker, text: &str, frame: NSRect, bold: bool) -> Retained<NSTextField> {
    // SAFETY: initWithFrame: returns the initialized NSTextField.
    let f: Retained<NSTextField> =
        unsafe { msg_send![NSTextField::alloc(mtm), initWithFrame: frame] };
    f.setStringValue(&NSString::from_str(text));
    f.setEditable(false);
    f.setBezeled(false);
    f.setDrawsBackground(false);
    f.setSelectable(false);
    if bold {
        f.setFont(Some(&NSFont::boldSystemFontOfSize(15.0)));
    }
    f
}

fn field(mtm: MainThreadMarker, text: &str, frame: NSRect) -> Retained<NSTextField> {
    // SAFETY: initWithFrame: returns the initialized NSTextField.
    let f: Retained<NSTextField> =
        unsafe { msg_send![NSTextField::alloc(mtm), initWithFrame: frame] };
    f.setStringValue(&NSString::from_str(text));
    f.setEditable(true);
    f.setBezeled(true);
    f.setDrawsBackground(true);
    f
}

fn toggle_row(
    mtm: MainThreadMarker,
    pane: &NSView,
    title: &str,
    description: Option<&str>,
    top: f64,
) -> Retained<NSSwitch> {
    let title_width = CONTENT_WIDTH - 70.0;
    pane.addSubview(&label(
        mtm,
        title,
        prect(top, 24.0, INSET, title_width),
        false,
    ));
    if let Some(description) = description {
        pane.addSubview(&hint(
            mtm,
            description,
            prect(top + 27.0, 30.0, INSET, title_width),
        ));
    }
    // SAFETY: initWithFrame: returns the initialized native macOS switch.
    let toggle: Retained<NSSwitch> = unsafe {
        msg_send![NSSwitch::alloc(mtm), initWithFrame: prect(top, 28.0, PW - INSET - 50.0, 50.0)]
    };
    pane.addSubview(&toggle);
    toggle
}

/// A small secondary-colour caption.
fn hint(mtm: MainThreadMarker, text: &str, frame: NSRect) -> Retained<NSTextField> {
    let f = label(mtm, text, frame, false);
    f.setFont(Some(&NSFont::systemFontOfSize(12.0)));
    f.setTextColor(Some(&NSColor::secondaryLabelColor()));
    f
}

fn heading(mtm: MainThreadMarker, text: &str) -> Retained<NSTextField> {
    let field = label(mtm, text, prect(24.0, 32.0, INSET, CONTENT_WIDTH), false);
    field.setFont(Some(&NSFont::boldSystemFontOfSize(22.0)));
    field
}

fn section_label(mtm: MainThreadMarker, text: &str, top: f64) -> Retained<NSTextField> {
    let field = label(mtm, text, prect(top, 18.0, INSET, CONTENT_WIDTH), false);
    field.setFont(Some(&NSFont::systemFontOfSize_weight(11.0, 0.28)));
    field.setTextColor(Some(&NSColor::secondaryLabelColor()));
    field
}

fn divider(mtm: MainThreadMarker, frame: NSRect) -> Retained<NSBox> {
    // SAFETY: initWithFrame: initializes the freshly allocated separator.
    let line: Retained<NSBox> = unsafe { msg_send![NSBox::alloc(mtm), initWithFrame: frame] };
    line.setBoxType(NSBoxType::Separator);
    line
}

fn artwork_names(model: &str) -> (&'static str, &'static str) {
    if matches!(model, "Mac16,10" | "Mac16,11" | "Mac18,5" | "Mac17,16") {
        ("welcome-mini-m4-off.jpg", "welcome-mini-m4.png")
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
        ("welcome-studio-off.jpg", "welcome-studio.png")
    } else {
        ("welcome-mini-legacy-off.jpg", "welcome-mini-legacy.png")
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
    NSImage::initWithContentsOfFile(
        NSImage::alloc(),
        &NSString::from_str(&bundled?.to_string_lossy()),
    )
}

fn artwork_view(mtm: MainThreadMarker, image: &NSImage, size: NSSize) -> Retained<NSImageView> {
    let source = image.size();
    let fill = (size.width / source.width).max(size.height / source.height);
    // The wide M4 artwork has generous negative space; bring the hardware
    // closer while keeping the front LED and desk in the frame.
    let scale = fill
        * if source.width / source.height > 1.5 {
            1.5
        } else {
            1.0
        };
    let width = source.width * scale;
    let height = source.height * scale;
    let frame = rect(
        (size.width - width) / 2.0,
        (size.height - height) / 2.0,
        width,
        height,
    );
    // SAFETY: initWithFrame: initializes the image view, and imageScaling: uses
    // NSImageScaleProportionallyUpOrDown to preserve the device's proportions.
    let view: Retained<NSImageView> =
        unsafe { msg_send![NSImageView::alloc(mtm), initWithFrame: frame] };
    view.setImage(Some(image));
    // SAFETY: setImageScaling: takes a valid NSImageScaling enum value.
    unsafe {
        let _: () = msg_send![&view, setImageScaling: 3isize];
    }
    view
}

fn backdrop(mtm: MainThreadMarker, frame: NSRect) -> Retained<NSBox> {
    // SAFETY: initWithFrame: initializes a decorative, noninteractive box.
    let box_view: Retained<NSBox> = unsafe { msg_send![NSBox::alloc(mtm), initWithFrame: frame] };
    box_view.setBoxType(NSBoxType::Custom);
    box_view.setTitlePosition(NSTitlePosition::NoTitle);
    box_view.setBorderWidth(0.0);
    box_view.setCornerRadius(12.0);
    box_view.setFillColor(&NSColor::colorWithCalibratedWhite_alpha(0.055, 1.0));
    box_view
}

/// A rounded push button wired to one of the `Handler`'s selectors.
fn push_button(
    mtm: MainThreadMarker,
    handler: &Handler,
    title: &str,
    action: Sel,
    frame: NSRect,
) -> Retained<NSButton> {
    // SAFETY: initWithFrame: returns the initialized push button.
    let b: Retained<NSButton> = unsafe { msg_send![NSButton::alloc(mtm), initWithFrame: frame] };
    b.setTitle(&NSString::from_str(title));
    // SAFETY: flexible-height native push bezel + target/action; the handler
    // outlives the window.
    unsafe {
        let _: () = msg_send![&b, setBezelStyle: 2isize];
        let _: () = msg_send![&b, setTarget: handler];
        let _: () = msg_send![&b, setAction: action];
    }
    b
}

/// An SF Symbol image (macOS 11+), or `None` if the symbol is unavailable.
fn symbol_icon(name: &str) -> Option<Retained<NSImage>> {
    use objc2::runtime::AnyClass;
    // SAFETY: +[NSImage imageWithSystemSymbolName:accessibilityDescription:] returns an
    // autoreleased NSImage* or nil; Retained::retain takes our own +1 (None on nil).
    unsafe {
        let cls = AnyClass::get(c"NSImage")?;
        let n = NSString::from_str(name);
        let img: *mut NSImage = msg_send![
            cls,
            imageWithSystemSymbolName: &*n,
            accessibilityDescription: std::ptr::null::<AnyObject>(),
        ];
        Retained::retain(img)
    }
}

/// The four fixed settings destinations, shared by the toolbar delegate methods.
pub fn toolbar_identifiers() -> *mut NSArray<NSString> {
    let identifiers: Vec<Retained<NSString>> = TOOLBAR_IDS
        .iter()
        .map(|id| NSString::from_str(id))
        .collect();
    Retained::autorelease_return(NSArray::from_retained_slice(&identifiers))
}

pub fn toolbar_item(
    handler: &Handler,
    mtm: MainThreadMarker,
    identifier: &NSString,
) -> *mut NSToolbarItem {
    let Some(index) = TOOLBAR_IDS
        .iter()
        .position(|id| identifier.to_string() == *id)
    else {
        return std::ptr::null_mut();
    };
    let (title, symbol) = match index {
        0 => ("LED", "lightbulb"),
        1 => (
            i18n::s("Automation", "Automatisation"),
            "clock.arrow.circlepath",
        ),
        2 => (i18n::s("General", "Général"), "gearshape"),
        _ => (i18n::s("Advanced", "Avancé"), "wrench.and.screwdriver"),
    };
    // SAFETY: initWithItemIdentifier: initializes a new native toolbar item.
    let item: Retained<NSToolbarItem> =
        unsafe { msg_send![NSToolbarItem::alloc(mtm), initWithItemIdentifier: identifier] };
    item.setLabel(&NSString::from_str(title));
    item.setPaletteLabel(&NSString::from_str(title));
    item.setToolTip(Some(&NSString::from_str(title)));
    item.setImage(symbol_icon(symbol).as_deref());
    item.setTag(index.cast_signed());
    // SAFETY: the handler outlives the settings window and receives toolbar actions.
    unsafe {
        let _: () = msg_send![&item, setTarget: handler];
        let _: () = msg_send![&item, setAction: sel!(selectSettingsPane:)];
    }
    Retained::autorelease_return(item)
}

/// An empty content pane sized to the full settings area.
fn pane(mtm: MainThreadMarker) -> Retained<NSView> {
    // SAFETY: initWithFrame: returns the initialized NSView.
    unsafe { msg_send![NSView::alloc(mtm), initWithFrame: rect(PANE_X, 0.0, PW, H)] }
}

fn set_state(btn: &NSSwitch, on: bool) {
    // SAFETY: setState: takes an NSControlStateValue (isize), returns void.
    unsafe {
        let _: () = msg_send![btn, setState: isize::from(on)];
    }
}

fn is_on(btn: &NSSwitch) -> bool {
    // SAFETY: -state returns an NSControlStateValue (isize); != 0 means checked.
    let s: isize = unsafe { msg_send![btn, state] };
    s != 0
}

fn fmt_hhmm(minute: u16) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

fn parse_hhmm(s: &str) -> Option<u16> {
    let (h, m) = s.trim().split_once(':')?;
    let h: u16 = h.trim().parse().ok()?;
    let m: u16 = m.trim().parse().ok()?;
    if h > 23 || m > 59 {
        return None;
    }
    Some(h * 60 + m)
}

fn parse_location(lat: &str, lon: &str) -> Result<Option<(f64, f64)>, ()> {
    let (lat, lon) = (lat.trim(), lon.trim());
    if lat.is_empty() && lon.is_empty() {
        return Ok(None);
    }
    let (la, lo) = (
        lat.parse::<f64>().map_err(|_| ())?,
        lon.parse::<f64>().map_err(|_| ())?,
    );
    if !la.is_finite()
        || !lo.is_finite()
        || !(-90.0..=90.0).contains(&la)
        || !(-180.0..=180.0).contains(&lo)
    {
        return Err(());
    }
    Ok(Some((la, lo)))
}

fn invalid_input(message: &str) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(i18n::s(
        "Check your settings",
        "Vérifiez vos réglages",
    )));
    alert.setInformativeText(&NSString::from_str(message));
    alert.runModal();
}

/// Open (or re-show) the Settings window, populated from the saved settings.
#[allow(
    clippy::too_many_lines, // flat linear AppKit layout builder; splitting hurts readability
    clippy::cast_precision_loss // small loop indices to f64 for layout
)]
pub fn open(handler: &Handler, mtm: MainThreadMarker) {
    if handler.ivars().settings.borrow().is_some() {
        populate(handler);
        if let Some(r) = handler.ivars().settings.borrow().as_ref() {
            // SAFETY: makeKeyAndOrderFront: shows the existing window; nil sender is fine.
            unsafe {
                let _: () =
                    msg_send![&r.window, makeKeyAndOrderFront: std::ptr::null_mut::<AnyObject>()];
            }
        }
        return;
    }

    let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable;
    // SAFETY: initWithContentRect:styleMask:backing:defer: — NSWindow's designated initializer.
    let window: Retained<NSWindow> = unsafe {
        msg_send![
            NSWindow::alloc(mtm),
            initWithContentRect: rect(0.0, 0.0, W, H),
            styleMask: style,
            backing: NSBackingStoreType::Buffered,
            defer: false,
        ]
    };
    window.setTitle(&NSString::from_str(i18n::s(
        "LuxMini Settings",
        "Réglages LuxMini",
    )));
    window.setBackgroundColor(Some(&NSColor::windowBackgroundColor()));
    // SAFETY: a fixed, non-customizable NSToolbar gives AppKit control of
    // titlebar metrics, symbol placement, selection, and OS-specific styling.
    let toolbar: Retained<NSToolbar> = unsafe {
        msg_send![NSToolbar::alloc(mtm), initWithIdentifier: &*NSString::from_str("luxmini.settings")]
    };
    toolbar.setAllowsUserCustomization(false);
    toolbar.setAllowsDisplayModeCustomization(false);
    toolbar.setDisplayMode(NSToolbarDisplayMode::LabelOnly);
    // SAFETY: the handler outlives the toolbar, and AppKit retains the delegate weakly.
    unsafe {
        let _: () = msg_send![&toolbar, setDelegate: handler];
    }
    window.setToolbar(Some(&toolbar));
    // SAFETY: keep our Retained valid when the user clicks the close box (hide, don't dealloc).
    unsafe {
        let _: () = msg_send![&window, setReleasedWhenClosed: false];
    }
    window.center();

    let Some(content) = window.contentView() else {
        return;
    };

    // ── Pane 0: Auto-dim ────────────────────────────────────────────────
    let p_auto = pane(mtm);
    p_auto.addSubview(&heading(
        mtm,
        i18n::s("Automatic dimming", "Variation automatique"),
    ));
    p_auto.addSubview(&hint(
        mtm,
        i18n::s(
            "Choose when LuxMini should adjust the front LED.",
            "Choisissez quand LuxMini ajuste la LED avant.",
        ),
        prect(62.0, 24.0, INSET, CONTENT_WIDTH),
    ));
    p_auto.addSubview(&section_label(mtm, i18n::s("SCHEDULE", "HORAIRES"), 111.0));
    let sunset = toggle_row(
        mtm,
        &p_auto,
        i18n::s("Turn off at sunset", "Éteindre au coucher du soleil"),
        Some(i18n::s(
            "Restores your chosen level at sunrise.",
            "Rétablit votre niveau choisi au lever du soleil.",
        )),
        137.0,
    );
    let dim = toggle_row(
        mtm,
        &p_auto,
        i18n::s("Dim in the evening", "Atténuer le soir"),
        None,
        205.0,
    );
    let time = field(mtm, "21:00", prect(242.0, 28.0, INSET + 56.0, 82.0));
    let pct = field(mtm, "20", prect(242.0, 28.0, INSET + 185.0, 64.0));
    p_auto.addSubview(&label(
        mtm,
        i18n::s("at", "à"),
        prect(244.0, 22.0, INSET + 22.0, 28.0),
        false,
    ));
    p_auto.addSubview(&label(
        mtm,
        i18n::s("to", "à"),
        prect(244.0, 22.0, INSET + 155.0, 24.0),
        false,
    ));
    p_auto.addSubview(&label(
        mtm,
        "%",
        prect(244.0, 22.0, INSET + 255.0, 24.0),
        false,
    ));
    p_auto.addSubview(&divider(mtm, prect(291.0, 1.0, INSET, CONTENT_WIDTH)));
    p_auto.addSubview(&section_label(
        mtm,
        i18n::s("LOCATION", "LOCALISATION"),
        309.0,
    ));
    p_auto.addSubview(&hint(
        mtm,
        i18n::s("Latitude", "Latitude"),
        prect(338.0, 20.0, INSET, 112.0),
    ));
    p_auto.addSubview(&hint(
        mtm,
        i18n::s("Longitude", "Longitude"),
        prect(338.0, 20.0, INSET + 128.0, 112.0),
    ));
    let lat = field(mtm, "", prect(359.0, 28.0, INSET, 112.0));
    let lon = field(mtm, "", prect(359.0, 28.0, INSET + 128.0, 112.0));
    let detect = push_button(
        mtm,
        handler,
        i18n::s("Use my location", "Utiliser ma position"),
        sel!(detectLocation:),
        prect(358.0, 30.0, INSET + 263.0, 191.0),
    );
    p_auto.addSubview(&hint(
        mtm,
        i18n::s(
            "Leave both fields empty to estimate from your time zone.",
            "Laissez les deux champs vides pour une estimation par fuseau horaire.",
        ),
        prect(400.0, 35.0, INSET, CONTENT_WIDTH),
    ));
    for v in [&time, &pct, &lat, &lon] {
        p_auto.addSubview(v);
    }
    p_auto.addSubview(&detect);

    // ── Pane 0: daily LED control ────────────────────────────────────────
    let p_led = pane(mtm);
    p_led.addSubview(&heading(
        mtm,
        i18n::s("Your front LED", "La LED de votre Mac"),
    ));
    p_led.addSubview(&hint(
        mtm,
        i18n::s(
            "Changes take effect on your Mac immediately.",
            "Les changements s'appliquent immédiatement sur votre Mac.",
        ),
        prect(62.0, 24.0, INSET, CONTENT_WIDTH),
    ));
    p_led.addSubview(&backdrop(mtm, prect(100.0, 224.0, INSET, 420.0)));
    #[cfg(debug_assertions)]
    let model = std::env::var("LUXMINI_PREVIEW_MODEL").unwrap_or_else(|_| compat::get_mac_model());
    #[cfg(not(debug_assertions))]
    let model = compat::get_mac_model();
    let (off_name, on_name) = artwork_names(&model);
    let image_frame = prect(105.0, 214.0, INSET + 5.0, 410.0);
    // SAFETY: a layer-backed container masks the photos to the preview card,
    // allowing a centered aspect-fill crop without distorting Mac hardware.
    let image_clip: Retained<NSView> =
        unsafe { msg_send![NSView::alloc(mtm), initWithFrame: image_frame] };
    // SAFETY: NSView owns the created layer; CALayer masking clips only its
    // child artwork and never changes the rest of the settings window.
    unsafe {
        let _: () = msg_send![&image_clip, setWantsLayer: true];
        let layer: *mut AnyObject = msg_send![&image_clip, layer];
        if !layer.is_null() {
            let _: () = msg_send![layer, setMasksToBounds: true];
            let _: () = msg_send![layer, setCornerRadius: 9.0f64];
        }
    }
    p_led.addSubview(&image_clip);
    let led_preview_off = load_artwork(off_name).map(|image| {
        let view = artwork_view(mtm, &image, image_frame.size);
        image_clip.addSubview(&view);
        view
    });
    let led_preview_on = load_artwork(on_name).map(|image| {
        let view = artwork_view(mtm, &image, image_frame.size);
        image_clip.addSubview(&view);
        view
    });

    let control_x = INSET + 446.0;
    p_led.addSubview(&label(
        mtm,
        i18n::s("Front light", "Voyant avant"),
        prect(111.0, 25.0, control_x, 195.0),
        true,
    ));
    let led_status = hint(mtm, "", prect(143.0, 28.0, control_x, 210.0));
    p_led.addSubview(&led_status);
    // SAFETY: initWithFrame: returns a native switch. Its target/action uses the
    // same controller path as the menu-bar switch.
    let led_switch: Retained<NSSwitch> = unsafe {
        msg_send![NSSwitch::alloc(mtm), initWithFrame: prect(111.0, 28.0, PW - INSET - 52.0, 52.0)]
    };
    // SAFETY: the handler outlives the switch and implements the selector.
    unsafe {
        let _: () = msg_send![&led_switch, setTarget: handler];
        let _: () = msg_send![&led_switch, setAction: sel!(switchToggled:)];
    }
    p_led.addSubview(&led_switch);
    p_led.addSubview(&divider(mtm, prect(183.0, 1.0, control_x, 270.0)));
    p_led.addSubview(&label(
        mtm,
        i18n::s("Brightness", "Luminosité"),
        prect(202.0, 23.0, control_x, 155.0),
        false,
    ));
    let led_percent = label(
        mtm,
        "100%",
        prect(202.0, 23.0, control_x + 206.0, 64.0),
        false,
    );
    led_percent.setTextColor(Some(&NSColor::secondaryLabelColor()));
    p_led.addSubview(&led_percent);
    // SAFETY: initWithFrame: returns a native slider. Its action follows the
    // existing brightness path, including helper feedback and error handling.
    let led_slider: Retained<NSSlider> = unsafe {
        msg_send![NSSlider::alloc(mtm), initWithFrame: prect(231.0, 25.0, control_x, 270.0)]
    };
    led_slider.setMinValue(0.0);
    led_slider.setMaxValue(255.0);
    // SAFETY: the handler outlives the slider and implements the selector.
    unsafe {
        let _: () = msg_send![&led_slider, setTarget: handler];
        let _: () = msg_send![&led_slider, setAction: sel!(sliderChanged:)];
        let _: () = msg_send![&led_slider, setContinuous: true];
    }
    p_led.addSubview(&led_slider);
    p_led.addSubview(&hint(
        mtm,
        i18n::s(
            "Effects work while your Mac is awake.",
            "Les effets fonctionnent quand le Mac est éveillé.",
        ),
        prect(285.0, 32.0, control_x, 270.0),
    ));

    p_led.addSubview(&section_label(mtm, i18n::s("EFFECTS", "EFFETS"), 342.0));
    let primary_fx: [(&str, Sel); 3] = [
        (i18n::s("Steady", "Fixe"), sel!(effectNone:)),
        (i18n::s("Blink", "Clignotement"), sel!(effectBlink:)),
        (i18n::s("Pulse", "Pulsation"), sel!(effectPulse:)),
    ];
    for (i, (title, action)) in primary_fx.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)] // i is 0..3
        let x = INSET + i as f64 * 148.0;
        p_led.addSubview(&push_button(
            mtm,
            handler,
            title,
            *action,
            prect(369.0, 34.0, x, 138.0),
        ));
    }
    let extra_fx: [(&str, Sel); 3] = [
        (i18n::s("Fast blink", "Rapide"), sel!(effectBlinkFast:)),
        ("SOS", sel!(effectSos:)),
        (i18n::s("Strobe", "Stroboscope"), sel!(effectStrobe:)),
    ];
    for (i, (title, action)) in extra_fx.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)] // i is 0..3
        let x = INSET + 450.0 + i as f64 * 90.0;
        let button = push_button(mtm, handler, title, *action, prect(369.0, 34.0, x, 86.0));
        // SAFETY: a borderless button still retains a native focus and action
        // path, but reads as a secondary text command rather than another tile.
        unsafe {
            let _: () = msg_send![&button, setBordered: false];
        }
        p_led.addSubview(&button);
    }
    p_led.addSubview(&divider(mtm, prect(426.0, 1.0, INSET, CONTENT_WIDTH)));
    p_led.addSubview(&section_label(
        mtm,
        i18n::s("SAVED SETUPS", "RÉGLAGES ENREGISTRÉS"),
        443.0,
    ));
    let load: [Sel; 3] = [sel!(loadPreset1:), sel!(loadPreset2:), sel!(loadPreset3:)];
    let store: [Sel; 3] = [
        sel!(saveToPreset1:),
        sel!(saveToPreset2:),
        sel!(saveToPreset3:),
    ];
    for (i, (load_action, save_action)) in load.iter().zip(store.iter()).enumerate() {
        #[allow(clippy::cast_precision_loss)] // i is 0..3
        let x = INSET + i as f64 * 238.0;
        p_led.addSubview(&push_button(
            mtm,
            handler,
            &format!("{} {}", i18n::s("Use preset", "Appliquer"), i + 1),
            *load_action,
            prect(474.0, 36.0, x, 151.0),
        ));
        let save_button = push_button(
            mtm,
            handler,
            i18n::s("Save", "Mémoriser"),
            *save_action,
            prect(474.0, 36.0, x + 151.0, 74.0),
        );
        // SAFETY: setBordered: changes only the button's native appearance.
        unsafe {
            let _: () = msg_send![&save_button, setBordered: false];
        }
        p_led.addSubview(&save_button);
    }

    // ── Pane 2: General ─────────────────────────────────────────────────
    let p_gen = pane(mtm);
    p_gen.addSubview(&heading(mtm, i18n::s("General", "Général")));
    p_gen.addSubview(&hint(
        mtm,
        i18n::s(
            "Choose how LuxMini starts and connects.",
            "Choisissez comment LuxMini démarre et se connecte.",
        ),
        prect(62.0, 24.0, INSET, CONTENT_WIDTH),
    ));
    p_gen.addSubview(&section_label(mtm, i18n::s("STARTUP", "DÉMARRAGE"), 106.0));
    let login = toggle_row(
        mtm,
        &p_gen,
        i18n::s("Launch at login", "Lancer à l'ouverture de session"),
        None,
        133.0,
    );
    let hide = toggle_row(
        mtm,
        &p_gen,
        i18n::s(
            "Hide menu-bar icon",
            "Masquer l'icône de la barre des menus",
        ),
        Some(i18n::s(
            "Reopen LuxMini to show the icon again.",
            "Rouvrez LuxMini pour afficher à nouveau l'icône.",
        )),
        174.0,
    );
    p_gen.addSubview(&divider(mtm, prect(243.0, 1.0, INSET, CONTENT_WIDTH)));
    p_gen.addSubview(&section_label(
        mtm,
        i18n::s("INTEGRATIONS & PRIVACY", "INTÉGRATIONS ET CONFIDENTIALITÉ"),
        260.0,
    ));
    let api = toggle_row(
        mtm,
        &p_gen,
        i18n::s(
            "Enable local control API",
            "Activer l'API de contrôle locale",
        ),
        Some(i18n::s(
            "For Shortcuts and scripts. Runs only on this Mac; off by default.",
            "Pour Raccourcis et vos scripts. Disponible sur ce Mac uniquement, désactivée par défaut.",
        )),
        287.0,
    );
    let diagnostics = toggle_row(
        mtm,
        &p_gen,
        i18n::s(
            "Share anonymous diagnostics",
            "Partager des diagnostics anonymes",
        ),
        Some(i18n::s(
            "Sends aggregate usage, Mac model, and macOS major version. No serial number.",
            "Envoie des données d'usage agrégées, le modèle et la version majeure de macOS. Aucun numéro de série.",
        )),
        361.0,
    );
    // General switches are independent preferences, so each change persists
    // immediately instead of waiting for a window-wide Save button.
    for (tag, control) in [&login, &hide, &api, &diagnostics].into_iter().enumerate() {
        // SAFETY: each switch is an NSControl and the handler lives with the window.
        unsafe {
            let _: () = msg_send![control, setTag: tag.cast_signed()];
            let _: () = msg_send![control, setTarget: handler];
            let _: () = msg_send![control, setAction: sel!(saveGeneralSettings:)];
        }
    }
    p_gen.addSubview(&push_button(
        mtm,
        handler,
        i18n::s("API docs\u{2026}", "Doc de l'API\u{2026}"),
        sel!(openApiDocs:),
        prect(434.0, 30.0, INSET, 150.0),
    ));

    // ── Pane 4: Advanced ────────────────────────────────────────────────
    let p_advanced = pane(mtm);
    p_advanced.addSubview(&heading(mtm, i18n::s("Advanced", "Avancé")));
    #[cfg(not(feature = "field-test"))]
    p_advanced.addSubview(&hint(
        mtm,
        i18n::s(
            "Troubleshoot LED access if the light does not respond.",
            "Vérifiez l'accès à la LED si elle ne répond pas.",
        ),
        prect(62.0, 24.0, INSET, CONTENT_WIDTH),
    ));
    #[cfg(not(feature = "field-test"))]
    p_advanced.addSubview(&section_label(
        mtm,
        i18n::s("LED ACCESS", "ACCÈS À LA LED"),
        111.0,
    ));
    #[cfg(not(feature = "field-test"))]
    p_advanced.addSubview(&hint(
        mtm,
        i18n::s(
            "Run setup again to verify the helper and confirm a visible LED fade.",
            "Relancez la configuration pour vérifier l'assistant et confirmer le fondu de la LED.",
        ),
        prect(142.0, 44.0, INSET, CONTENT_WIDTH),
    ));
    #[cfg(not(feature = "field-test"))]
    p_advanced.addSubview(&push_button(
        mtm,
        handler,
        i18n::s("Run LED setup…", "Relancer la configuration LED…"),
        sel!(runSetupTest:),
        prect(204.0, 36.0, INSET, 250.0),
    ));
    #[cfg(not(feature = "field-test"))]
    p_advanced.addSubview(&divider(mtm, prect(265.0, 1.0, INSET, CONTENT_WIDTH)));
    #[cfg(not(feature = "field-test"))]
    p_advanced.addSubview(&section_label(
        mtm,
        i18n::s("PROFILE DISCOVERY", "DÉTECTION DU PROFIL"),
        283.0,
    ));
    #[cfg(not(feature = "field-test"))]
    p_advanced.addSubview(&hint(
        mtm,
        i18n::s(
            "Test up to three allowlisted profiles and keep only the one you confirm.",
            "Teste jusqu'à trois profils autorisés et conserve celui que vous confirmez.",
        ),
        prect(310.0, 40.0, INSET, CONTENT_WIDTH),
    ));
    #[cfg(not(feature = "field-test"))]
    p_advanced.addSubview(&push_button(
        mtm,
        handler,
        i18n::s("Detect LED access…", "Détecter l'accès à la LED…"),
        sel!(detectLedAccess:),
        prect(363.0, 36.0, INSET, 250.0),
    ));
    #[cfg(feature = "field-test")]
    {
        p_advanced.addSubview(&hint(
            mtm,
            "Private build. Tests use this Mac's profile and restore the LED.",
            prect(62.0, 24.0, INSET, CONTENT_WIDTH),
        ));
        p_advanced.addSubview(&section_label(mtm, "DIAGNOSTIC TOOLS", 111.0));
        for (top, title, action) in [
            (145.0, "Check helper and SMC read", sel!(checkHelperTest:)),
            (
                198.0,
                "Check production profile fetch",
                sel!(checkRemoteProfileTest:),
            ),
            (251.0, "Run setup and visual fade", sel!(runSetupTest:)),
            (304.0, "Detect LED profile", sel!(detectLedAccess:)),
            (357.0, "Test one helper restart", sel!(testHelperRestart:)),
        ] {
            p_advanced.addSubview(&push_button(
                mtm,
                handler,
                title,
                action,
                prect(top, 36.0, INSET, 310.0),
            ));
        }
        p_advanced.addSubview(&hint(
            mtm,
            "Remote check does not change the cache. Run helper restart last.",
            prect(420.0, 40.0, INSET, CONTENT_WIDTH),
        ));
    }

    // Automation has multiple related fields, so apply them together. General
    // switches persist immediately and do not need a global Save action.
    let save = push_button(
        mtm,
        handler,
        i18n::s("Apply automation", "Appliquer l'automatisation"),
        sel!(saveSettings:),
        prect(468.0, 36.0, INSET, 230.0),
    );
    p_auto.addSubview(&save);
    let sections = vec![p_led, p_auto, p_gen, p_advanced];
    for s in &sections {
        content.addSubview(s);
    }

    *handler.ivars().settings.borrow_mut() = Some(SettingsRefs {
        window: window.clone(),
        toolbar: toolbar.clone(),
        sunset,
        dim,
        time,
        pct,
        lat,
        lon,
        login,
        hide,
        api,
        diagnostics,
        led_switch,
        led_slider,
        led_percent,
        led_status,
        led_preview_on,
        led_preview_off,
        #[cfg(debug_assertions)]
        save,
        sections,
    });
    populate(handler);
    refresh_led(handler);

    select_section(handler, 0);

    // SAFETY: bring the freshly built window to the front; nil sender is fine.
    unsafe {
        let _: () = msg_send![&window, makeKeyAndOrderFront: std::ptr::null_mut::<AnyObject>()];
    }
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
}

/// Show the selected settings pane and let `AppKit` highlight its toolbar item.
#[allow(clippy::cast_possible_wrap)] // small section indices to isize
pub fn select_section(handler: &Handler, index: isize) {
    let refs = handler.ivars().settings.borrow();
    let Some(r) = refs.as_ref() else { return };
    let Ok(selected) = usize::try_from(index) else {
        return;
    };
    let Some(identifier) = TOOLBAR_IDS.get(selected) else {
        return;
    };
    for (i, p) in r.sections.iter().enumerate() {
        p.setHidden(i as isize != index);
    }
    r.toolbar
        .setSelectedItemIdentifier(Some(&NSString::from_str(identifier)));
    let title = match selected {
        0 => "LED",
        1 => i18n::s("Automation", "Automatisation"),
        2 => i18n::s("General", "Général"),
        _ => i18n::s("Advanced", "Avancé"),
    };
    r.window.setTitle(&NSString::from_str(title));
}

#[cfg(debug_assertions)]
pub fn save_receives_pointer_hits(handler: &Handler, section: isize) -> bool {
    select_section(handler, section);
    let refs = handler.ivars().settings.borrow();
    let Some(r) = refs.as_ref() else { return false };
    let Some(content) = r.window.contentView() else {
        return false;
    };
    let save_center = NSPoint::new(INSET + 115.0, H - 468.0 - 18.0);
    let save_ptr = (&raw const *r.save).cast::<NSView>();
    content
        .hitTest(save_center)
        .is_some_and(|hit| std::ptr::eq(&raw const *hit, save_ptr))
}

#[cfg(debug_assertions)]
pub fn export_previews(handler: &Handler, directory: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let names = ["led", "automation", "general", "advanced"];
    for (index, name) in names.iter().enumerate() {
        select_section(handler, index.cast_signed());
        let refs = handler.ivars().settings.borrow();
        let Some(r) = refs.as_ref() else {
            return Err(std::io::Error::other("settings window unavailable"));
        };
        r.window.display();
        let Some(content) = r.window.contentView() else {
            return Err(std::io::Error::other("settings content unavailable"));
        };
        let data = content.dataWithPDFInsideRect(content.bounds());
        let destination = directory.join(format!("settings-{name}.pdf"));
        if !data.writeToFile_atomically(&NSString::from_str(&destination.to_string_lossy()), true) {
            return Err(std::io::Error::other("could not write settings preview"));
        }
    }
    Ok(())
}

/// Fill the controls from the saved settings.
fn populate(handler: &Handler) {
    let refs = handler.ivars().settings.borrow();
    let Some(r) = refs.as_ref() else { return };
    let d = preferences::load_autodim();
    set_state(&r.sunset, d.off_at_sunset);
    set_state(&r.dim, d.dim_at_time);
    r.time
        .setStringValue(&NSString::from_str(&fmt_hhmm(d.dim_minute)));
    r.pct
        .setStringValue(&NSString::from_str(&d.dim_pct.to_string()));
    if let Some(loc) = preferences::load_location() {
        r.lat
            .setStringValue(&NSString::from_str(&format!("{}", loc.lat)));
        r.lon
            .setStringValue(&NSString::from_str(&format!("{}", loc.lon)));
    } else {
        r.lat.setStringValue(&NSString::from_str(""));
        r.lon.setStringValue(&NSString::from_str(""));
    }
    set_state(&r.login, launch_at_login::is_enabled());
    set_state(&r.hide, preferences::load_hide_icon());
    set_state(&r.api, preferences::api_enabled());
    set_state(&r.diagnostics, preferences::diagnostics_enabled());
}

/// Keep the window's primary controls and device preview aligned with the
/// actual LED state, including changes made from the menu or automation.
pub fn refresh_led(handler: &Handler) {
    let refs = handler.ivars().settings.borrow();
    let Some(r) = refs.as_ref() else { return };
    let status = read_status();
    set_state(&r.led_switch, status.is_on);
    r.led_switch.setEnabled(status.helper_ready);
    r.led_slider.setEnabled(status.helper_ready);
    r.led_slider.setDoubleValue(f64::from(status.brightness));
    let percent = (f64::from(status.brightness) / 255.0 * 100.0).round();
    r.led_percent
        .setStringValue(&NSString::from_str(&format!("{percent:.0}%")));
    let text = if !status.helper_ready {
        i18n::s("LED access unavailable", "Accès à la LED indisponible")
    } else if status.is_on {
        i18n::s("On", "Allumé")
    } else {
        i18n::s("Off", "Éteint")
    };
    r.led_status.setStringValue(&NSString::from_str(text));
    let preview_lit = status.is_on && status.brightness > 0 && status.helper_ready;
    if let Some(image) = &r.led_preview_on {
        image.setHidden(!preview_lit);
        image.setAlphaValue(f64::from(status.brightness) / 255.0);
    }
    if let Some(image) = &r.led_preview_off {
        image.setHidden(false);
    }
}

/// Re-fill just the latitude/longitude fields from the saved location (called
/// after `CoreLocation` delivers a fix). No-op if the window was never opened.
pub fn refresh_location(handler: &Handler) {
    let refs = handler.ivars().settings.borrow();
    let Some(r) = refs.as_ref() else { return };
    if let Some(loc) = preferences::load_location() {
        r.lat
            .setStringValue(&NSString::from_str(&format!("{}", loc.lat)));
        r.lon
            .setStringValue(&NSString::from_str(&format!("{}", loc.lon)));
    }
}

/// Validate and apply the automation fields as one related group.
pub fn save(handler: &Handler) {
    let refs = handler.ivars().settings.borrow();
    let Some(r) = refs.as_ref() else { return };

    let off_at_sunset = is_on(&r.sunset);
    let dim_at_time = is_on(&r.dim);
    let Some(dim_minute) = parse_hhmm(&r.time.stringValue().to_string()) else {
        invalid_input(i18n::s(
            "Enter a time from 00:00 to 23:59.",
            "Saisissez une heure entre 00:00 et 23:59.",
        ));
        return;
    };
    let Some(dim_pct) = r
        .pct
        .stringValue()
        .to_string()
        .trim()
        .parse::<u8>()
        .ok()
        .filter(|p| *p <= 100)
    else {
        invalid_input(i18n::s(
            "Enter a brightness from 0 to 100%.",
            "Saisissez une luminosité entre 0 et 100 %.",
        ));
        return;
    };
    let lat = r.lat.stringValue().to_string();
    let lon = r.lon.stringValue().to_string();
    let Ok(location) = parse_location(&lat, &lon) else {
        invalid_input(i18n::s(
            "Enter a valid latitude and longitude, or leave both empty.",
            "Saisissez une latitude et une longitude valides, ou laissez les deux champs vides.",
        ));
        return;
    };
    preferences::save_autodim(&AutoDim {
        enabled: off_at_sunset || dim_at_time,
        off_at_sunset,
        dim_at_time,
        dim_minute,
        dim_pct,
    });

    match location {
        Some((la, lo)) => preferences::save_location(la, lo),
        None => preferences::clear_location(),
    }

    schedule::restart();

    // If the user enabled sunset-off but left the location blank, fetch it
    // automatically from CoreLocation (prompts for permission the first time).
    let want_location = off_at_sunset && preferences::load_location().is_none();

    drop(refs);

    if want_location {
        crate::location::request(handler);
    }
}

/// Persist independent general preferences when their native switches change.
pub fn save_general(handler: &Handler, sender: &AnyObject) {
    let refs = handler.ivars().settings.borrow();
    let Some(r) = refs.as_ref() else { return };
    // SAFETY: sender is one of the tagged NSSwitch controls above.
    let tag: isize = unsafe { msg_send![sender, tag] };
    match tag {
        0 => {
            if let Err(e) = launch_at_login::set_enabled(is_on(&r.login)) {
                eprintln!("launch_at_login from settings failed: {e}");
            }
        }
        1 => {
            let hide_icon = is_on(&r.hide);
            preferences::save_hide_icon(hide_icon);
            handler.set_icon_hidden(hide_icon);
        }
        2 => {
            // Apply the local API toggle immediately, including closing the
            // socket when disabled.
            let api_was_on = preferences::api_enabled();
            let api_now_on = is_on(&r.api);
            preferences::save_api_enabled(api_now_on);
            if api_now_on && !api_was_on {
                crate::api::maybe_start();
            } else if !api_now_on && api_was_on {
                crate::api::stop();
            }
        }
        3 => {
            let was_on = preferences::diagnostics_enabled();
            let now_on = is_on(&r.diagnostics);
            preferences::save_diagnostics_enabled(now_on);
            if now_on && !was_on {
                crate::telemetry::note_launch(&compat::get_mac_model());
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{artwork_names, parse_hhmm, parse_location};

    #[test]
    fn led_preview_uses_the_matching_device_artwork() {
        assert_eq!(artwork_names("Mac16,10").1, "welcome-mini-m4.png");
        assert_eq!(artwork_names("Mac14,3").1, "welcome-mini-legacy.png");
        assert_eq!(artwork_names("Mac16,9").1, "welcome-studio.png");
    }

    #[test]
    fn settings_time_rejects_out_of_range_values() {
        assert_eq!(parse_hhmm("23:59"), Some(1439));
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("12:60"), None);
    }

    #[test]
    fn settings_location_requires_a_valid_pair() {
        assert_eq!(parse_location("", ""), Ok(None));
        assert_eq!(parse_location("48.8", "2.3"), Ok(Some((48.8, 2.3))));
        assert_eq!(parse_location("48.8", ""), Err(()));
        assert_eq!(parse_location("NaN", "2.3"), Err(()));
        assert_eq!(parse_location("91", "2.3"), Err(()));
    }
}
