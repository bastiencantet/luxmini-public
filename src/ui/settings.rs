//! "Réglages" — a sidebar settings window: a left source list of sections and a
//! content pane on the right that swaps with the selection. Singleton kept alive in
//! the `Handler` ivars; closing hides it (`setReleasedWhenClosed: false`), reopening repopulates.

use crate::compat;
use crate::i18n;
use crate::launch_at_login;
use crate::preferences;
use crate::schedule::{self, AutoDim};
use crate::ui::handler::Handler;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSButton, NSColor, NSFont, NSImage, NSImageView, NSScrollView,
    NSTableColumn, NSTableView, NSTextField, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

/// Handles to the editable controls (read by `saveSettings:` / repopulated by `open`),
/// plus the sidebar rows and content panes for section switching.
pub struct SettingsRefs {
    pub window: Retained<NSWindow>,
    pub sunset: Retained<NSButton>,
    pub dim: Retained<NSButton>,
    pub time: Retained<NSTextField>,
    pub pct: Retained<NSTextField>,
    pub lat: Retained<NSTextField>,
    pub lon: Retained<NSTextField>,
    pub login: Retained<NSButton>,
    pub hide: Retained<NSButton>,
    pub api: Retained<NSButton>,
    pub save: Retained<NSButton>,
    pub sections: Vec<Retained<NSView>>,
    /// Sidebar cell views, returned to the source-list table by `viewForTableColumn:row:`.
    pub row_views: Vec<Retained<NSView>>,
}

const W: f64 = 620.0;
const H: f64 = 330.0;
const PANE_X: f64 = 192.0; // left edge of the content area (right of the sidebar)
const PW: f64 = W - PANE_X; // content-pane width

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
        f.setFont(Some(&NSFont::boldSystemFontOfSize(14.0)));
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

fn checkbox(mtm: MainThreadMarker, title: &str, frame: NSRect) -> Retained<NSButton> {
    // SAFETY: initWithFrame: returns the initialized NSButton.
    let b: Retained<NSButton> = unsafe { msg_send![NSButton::alloc(mtm), initWithFrame: frame] };
    b.setTitle(&NSString::from_str(title));
    // SAFETY: setButtonType: 3 = checkbox.
    unsafe {
        let _: () = msg_send![&b, setButtonType: 3isize];
    }
    b
}

/// A small secondary-colour caption.
fn hint(mtm: MainThreadMarker, text: &str, frame: NSRect) -> Retained<NSTextField> {
    let f = label(mtm, text, frame, false);
    f.setFont(Some(&NSFont::systemFontOfSize(11.0)));
    f.setTextColor(Some(&NSColor::secondaryLabelColor()));
    f
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
    // SAFETY: rounded bezel + target/action; the handler outlives the window.
    unsafe {
        let _: () = msg_send![&b, setBezelStyle: 1isize];
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

/// A sidebar source-list cell: SF Symbol icon + title. Returned to the `NSTableView`
/// by the handler's `viewForTableColumn:row:`.
fn sidebar_cell_view(mtm: MainThreadMarker, title: &str, symbol: &str) -> Retained<NSView> {
    // SAFETY: initWithFrame: returns the initialized NSView.
    let v: Retained<NSView> =
        unsafe { msg_send![NSView::alloc(mtm), initWithFrame: rect(0.0, 0.0, 170.0, 28.0)] };
    if let Some(img) = symbol_icon(symbol) {
        // SAFETY: initWithFrame: returns the initialized NSImageView.
        let iv: Retained<NSImageView> = unsafe {
            msg_send![NSImageView::alloc(mtm), initWithFrame: rect(8.0, 5.0, 18.0, 18.0)]
        };
        iv.setImage(Some(&img));
        v.addSubview(&iv);
    }
    v.addSubview(&label(mtm, title, rect(34.0, 4.0, 130.0, 20.0), false));
    v
}

/// An empty, transparent content pane sized to the right-hand area.
fn pane(mtm: MainThreadMarker) -> Retained<NSView> {
    // SAFETY: initWithFrame: returns the initialized NSView.
    unsafe { msg_send![NSView::alloc(mtm), initWithFrame: rect(PANE_X, 0.0, PW, H)] }
}

fn set_state(btn: &NSButton, on: bool) {
    // SAFETY: setState: takes an NSControlStateValue (isize), returns void.
    unsafe {
        let _: () = msg_send![btn, setState: isize::from(on)];
    }
}

fn is_on(btn: &NSButton) -> bool {
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

/// Open (or re-show) the Settings window, populated from the saved settings.
#[allow(
    clippy::too_many_lines, // flat linear AppKit layout builder; splitting hurts readability
    clippy::cast_precision_loss, // small loop indices (0..6) to f64 for layout
    clippy::cast_possible_wrap // small loop indices to isize for tags/section ids
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
        "LuxMini — Settings",
        "LuxMini — Réglages",
    )));
    // SAFETY: keep our Retained valid when the user clicks the close box (hide, don't dealloc).
    unsafe {
        let _: () = msg_send![&window, setReleasedWhenClosed: false];
    }
    window.center();

    let Some(content) = window.contentView() else {
        return;
    };

    // ── Sidebar: native NSTableView in source-list mode ─────────────────
    let labels = [
        (i18n::s("Auto-dim", "Auto-dim"), "sun.max"),
        (i18n::s("Effects", "Effets"), "sparkles"),
        (i18n::s("Presets", "Presets"), "square.stack"),
        (i18n::s("General", "Général"), "gearshape"),
        (i18n::s("About", "À propos"), "info.circle"),
    ];
    let row_views: Vec<Retained<NSView>> = labels
        .iter()
        .map(|(title, sym)| sidebar_cell_view(mtm, title, sym))
        .collect();

    let scroll: Retained<NSScrollView> =
        // SAFETY: initWithFrame: returns the initialized NSScrollView.
        unsafe { msg_send![NSScrollView::alloc(mtm), initWithFrame: rect(0.0, 0.0, 180.0, H)] };
    let table: Retained<NSTableView> =
        // SAFETY: initWithFrame: returns the initialized NSTableView.
        unsafe { msg_send![NSTableView::alloc(mtm), initWithFrame: rect(0.0, 0.0, 180.0, H)] };
    // SAFETY: initWithIdentifier: returns the initialized column.
    let col: Retained<NSTableColumn> = unsafe {
        msg_send![NSTableColumn::alloc(mtm), initWithIdentifier: &*NSString::from_str("section")]
    };
    // SAFETY: standard NSTableView/NSScrollView setters; selectionHighlightStyle 1 = SourceList;
    // setDataSource:/setDelegate: take the handler (it implements the table data-source/delegate
    // selectors) and the handler outlives the window.
    unsafe {
        let _: () = msg_send![&col, setWidth: 180.0];
        let _: () = msg_send![&table, addTableColumn: &*col];
        let _: () = msg_send![&table, setHeaderView: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![&table, setSelectionHighlightStyle: 1isize];
        let _: () = msg_send![&table, setRowHeight: 28.0];
        let _: () = msg_send![&table, setDataSource: handler];
        let _: () = msg_send![&table, setDelegate: handler];
        let _: () = msg_send![&scroll, setDocumentView: &*table];
        let _: () = msg_send![&scroll, setDrawsBackground: false];
        let _: () = msg_send![&scroll, setHasVerticalScroller: false];
    }
    content.addSubview(&scroll);

    // ── Pane 0: Auto-dim ────────────────────────────────────────────────
    let p_auto = pane(mtm);
    p_auto.addSubview(&label(
        mtm,
        i18n::s("Auto-dim", "Auto-dim"),
        prect(18.0, 22.0, 16.0, 300.0),
        true,
    ));
    let sunset = checkbox(
        mtm,
        i18n::s("Turn off at sunset", "Éteindre au coucher du soleil"),
        prect(56.0, 22.0, 16.0, 396.0),
    );
    let dim = checkbox(
        mtm,
        i18n::s("Dim in the evening", "Atténuer le soir"),
        prect(92.0, 22.0, 16.0, 150.0),
    );
    let time = field(mtm, "21:00", prect(91.0, 24.0, 172.0, 58.0));
    let arrow = label(mtm, "→", prect(92.0, 20.0, 236.0, 18.0), false);
    let pct = field(mtm, "20", prect(91.0, 24.0, 258.0, 44.0));
    let pctlbl = label(mtm, "%", prect(92.0, 20.0, 306.0, 20.0), false);
    let loclbl = label(
        mtm,
        i18n::s("Location", "Lieu"),
        prect(126.0, 18.0, 16.0, 200.0),
        false,
    );
    let lat = field(mtm, "", prect(148.0, 24.0, 16.0, 92.0));
    let lon = field(mtm, "", prect(148.0, 24.0, 116.0, 92.0));
    let detect = push_button(
        mtm,
        handler,
        i18n::s("\u{1F4CD} Detect", "\u{1F4CD} Détecter"),
        sel!(detectLocation:),
        prect(147.0, 26.0, 216.0, 150.0),
    );
    let lochint = hint(
        mtm,
        i18n::s(
            "Empty = approximate from time zone.",
            "Vide = approximation par fuseau horaire.",
        ),
        prect(180.0, 14.0, 16.0, 396.0),
    );
    for v in [&*arrow, &*pctlbl, &*loclbl, &*lochint] {
        p_auto.addSubview(v);
    }
    for v in [&time, &pct, &lat, &lon] {
        p_auto.addSubview(v);
    }
    for b in [&sunset, &dim, &detect] {
        p_auto.addSubview(b);
    }

    // ── Pane 1: Effects (Fun stuff) ─────────────────────────────────────
    let p_fx = pane(mtm);
    p_fx.addSubview(&label(
        mtm,
        i18n::s("Effects", "Effets"),
        prect(18.0, 22.0, 16.0, 300.0),
        true,
    ));
    let fx: [(&str, Sel); 6] = [
        (i18n::s("Stop", "Stop"), sel!(effectNone:)),
        ("Blink", sel!(effectBlink:)),
        (
            i18n::s("Blink Fast", "Blink rapide"),
            sel!(effectBlinkFast:),
        ),
        ("Pulse", sel!(effectPulse:)),
        ("SOS", sel!(effectSos:)),
        ("Strobe", sel!(effectStrobe:)),
    ];
    for (i, (title, action)) in fx.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)] // i is 0..6
        let (col, row_i) = (i % 2, i / 2);
        let x = 16.0 + col as f64 * 200.0;
        let top = 56.0 + row_i as f64 * 38.0;
        p_fx.addSubview(&push_button(
            mtm,
            handler,
            title,
            *action,
            prect(top, 30.0, x, 188.0),
        ));
    }

    // ── Pane 2: Presets ─────────────────────────────────────────────────
    let p_preset = pane(mtm);
    p_preset.addSubview(&label(
        mtm,
        i18n::s("Presets", "Presets"),
        prect(18.0, 22.0, 16.0, 300.0),
        true,
    ));
    p_preset.addSubview(&hint(
        mtm,
        i18n::s("Load", "Charger"),
        prect(52.0, 16.0, 16.0, 200.0),
    ));
    let load: [Sel; 3] = [sel!(loadPreset1:), sel!(loadPreset2:), sel!(loadPreset3:)];
    let store: [Sel; 3] = [
        sel!(saveToPreset1:),
        sel!(saveToPreset2:),
        sel!(saveToPreset3:),
    ];
    for (i, action) in load.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)] // i is 0..3
        let x = 16.0 + i as f64 * 130.0;
        p_preset.addSubview(&push_button(
            mtm,
            handler,
            &format!("Preset {}", i + 1),
            *action,
            prect(74.0, 30.0, x, 120.0),
        ));
    }
    p_preset.addSubview(&hint(
        mtm,
        i18n::s("Save current to", "Enregistrer l'actuel dans"),
        prect(116.0, 16.0, 16.0, 300.0),
    ));
    for (i, action) in store.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)] // i is 0..3
        let x = 16.0 + i as f64 * 130.0;
        p_preset.addSubview(&push_button(
            mtm,
            handler,
            &format!("Preset {}", i + 1),
            *action,
            prect(138.0, 30.0, x, 120.0),
        ));
    }

    // ── Pane 3: General ─────────────────────────────────────────────────
    let p_gen = pane(mtm);
    p_gen.addSubview(&label(
        mtm,
        i18n::s("General", "Général"),
        prect(18.0, 22.0, 16.0, 300.0),
        true,
    ));
    let login = checkbox(
        mtm,
        i18n::s("Launch at login", "Lancer au login"),
        prect(56.0, 22.0, 16.0, 396.0),
    );
    let hide = checkbox(
        mtm,
        i18n::s(
            "Hide menu-bar icon (reopen LuxMini to show it)",
            "Masquer l'icône (rouvrir LuxMini pour la réafficher)",
        ),
        prect(92.0, 22.0, 16.0, 396.0),
    );
    let api = checkbox(
        mtm,
        i18n::s(
            "Enable local control API (localhost, off by default)",
            "Activer l'API locale de contrôle (localhost, désactivée par défaut)",
        ),
        prect(128.0, 22.0, 16.0, 396.0),
    );
    p_gen.addSubview(&login);
    p_gen.addSubview(&hide);
    p_gen.addSubview(&api);
    p_gen.addSubview(&hint(
        mtm,
        i18n::s(
            "Lets Shortcuts, Home Assistant, and scripts drive the LED. Disabling takes effect after a restart.",
            "Permet à Shortcuts, Home Assistant et vos scripts de piloter la LED. La désactivation prend effet au redémarrage.",
        ),
        prect(150.0, 32.0, 34.0, 372.0),
    ));
    p_gen.addSubview(&push_button(
        mtm,
        handler,
        i18n::s("API docs\u{2026}", "Doc de l'API\u{2026}"),
        sel!(openApiDocs:),
        prect(188.0, 28.0, 34.0, 150.0),
    ));

    // ── Pane 4: About ───────────────────────────────────────────────────
    let p_about = pane(mtm);
    p_about.addSubview(&label(
        mtm,
        i18n::s("About", "À propos"),
        prect(18.0, 22.0, 16.0, 300.0),
        true,
    ));
    p_about.addSubview(&label(
        mtm,
        &format!("LuxMini {}", env!("CARGO_PKG_VERSION")),
        prect(54.0, 20.0, 16.0, 396.0),
        false,
    ));
    p_about.addSubview(&hint(
        mtm,
        &format!(
            "{} {}",
            i18n::s("Model:", "Modèle :"),
            compat::get_mac_model()
        ),
        prect(80.0, 16.0, 16.0, 396.0),
    ));
    p_about.addSubview(&push_button(
        mtm,
        handler,
        i18n::s(
            "Check for Updates\u{2026}",
            "Vérifier les mises à jour\u{2026}",
        ),
        sel!(checkForUpdates:),
        prect(112.0, 30.0, 16.0, 220.0),
    ));
    p_about.addSubview(&push_button(
        mtm,
        handler,
        i18n::s("Send Feedback\u{2026}", "Envoyer un retour\u{2026}"),
        sel!(sendFeedback:),
        prect(150.0, 30.0, 16.0, 220.0),
    ));
    p_about.addSubview(&hint(
        mtm,
        "Made with ❤️ by Bastien CANTET",
        prect(196.0, 16.0, 16.0, 396.0),
    ));

    // Global Save button (shown only on the form sections).
    let save = push_button(
        mtm,
        handler,
        i18n::s("Save", "Enregistrer"),
        sel!(saveSettings:),
        rect(W - 16.0 - 120.0, 16.0, 120.0, 32.0),
    );
    content.addSubview(&save);

    let sections = vec![p_auto, p_fx, p_preset, p_gen, p_about];
    for s in &sections {
        content.addSubview(s);
    }

    *handler.ivars().settings.borrow_mut() = Some(SettingsRefs {
        window: window.clone(),
        sunset,
        dim,
        time,
        pct,
        lat,
        lon,
        login,
        hide,
        api,
        save,
        sections,
        row_views,
    });
    populate(handler);

    // Load the source list (dataSource = handler, now installed) and select Auto-dim.
    // SAFETY: reloadData / selectRowIndexes:byExtendingSelection: are standard NSTableView calls;
    // the index set holds row 0. Programmatic selection does not fire the delegate, so we also
    // switch the pane explicitly.
    unsafe {
        let _: () = msg_send![&table, reloadData];
        if let Some(is_cls) = objc2::runtime::AnyClass::get(c"NSIndexSet") {
            let set: *mut AnyObject = msg_send![is_cls, indexSetWithIndex: 0usize];
            let _: () = msg_send![&table, selectRowIndexes: set, byExtendingSelection: false];
        }
    }
    select_section(handler, 0);

    // SAFETY: bring the freshly built window to the front; nil sender is fine.
    unsafe {
        let _: () = msg_send![&window, makeKeyAndOrderFront: std::ptr::null_mut::<AnyObject>()];
    }
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
}

/// Show the content pane for `index` (the source-list table owns the row highlight).
/// Save is shown only on the form sections (Auto-dim = 0, General = 3).
#[allow(clippy::cast_possible_wrap)] // small section indices to isize
pub fn select_section(handler: &Handler, index: isize) {
    let refs = handler.ivars().settings.borrow();
    let Some(r) = refs.as_ref() else { return };
    for (i, p) in r.sections.iter().enumerate() {
        p.setHidden(i as isize != index);
    }
    r.save.setHidden(!(index == 0 || index == 3));
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

/// Read the controls, persist everything, restart the scheduler, hide the window.
pub fn save(handler: &Handler) {
    let refs = handler.ivars().settings.borrow();
    let Some(r) = refs.as_ref() else { return };

    let off_at_sunset = is_on(&r.sunset);
    let dim_at_time = is_on(&r.dim);
    let dim_minute = parse_hhmm(&r.time.stringValue().to_string()).unwrap_or(21 * 60);
    let dim_pct = r
        .pct
        .stringValue()
        .to_string()
        .trim()
        .parse::<u8>()
        .map_or(20, |p| p.min(100));
    preferences::save_autodim(&AutoDim {
        enabled: off_at_sunset || dim_at_time,
        off_at_sunset,
        dim_at_time,
        dim_minute,
        dim_pct,
    });

    let lat = r.lat.stringValue().to_string();
    let lon = r.lon.stringValue().to_string();
    match (lat.trim().parse::<f64>(), lon.trim().parse::<f64>()) {
        (Ok(la), Ok(lo)) if (-90.0..=90.0).contains(&la) && (-180.0..=180.0).contains(&lo) => {
            preferences::save_location(la, lo);
        }
        _ => preferences::clear_location(), // empty/invalid → timezone approximation
    }

    if let Err(e) = launch_at_login::set_enabled(is_on(&r.login)) {
        eprintln!("launch_at_login from settings failed: {e}");
    }
    let hide_icon = is_on(&r.hide);
    preferences::save_hide_icon(hide_icon);
    handler.set_icon_hidden(hide_icon);

    // Local API: persist the toggle and, if it was just switched on, start it
    // now (turning it off only takes effect on the next launch — the bound
    // socket is released when the app quits).
    let api_was_on = preferences::api_enabled();
    let api_now_on = is_on(&r.api);
    preferences::save_api_enabled(api_now_on);
    if api_now_on && !api_was_on {
        crate::api::maybe_start();
    } else if !api_now_on && api_was_on {
        crate::api::stop();
    }

    schedule::restart();

    // If the user enabled sunset-off but left the location blank, fetch it
    // automatically from CoreLocation (prompts for permission the first time).
    let want_location = off_at_sunset && preferences::load_location().is_none();

    // SAFETY: orderOut: hides the window (kept alive via ivars); nil sender is fine.
    unsafe {
        let _: () = msg_send![&r.window, orderOut: std::ptr::null_mut::<AnyObject>()];
    }
    drop(refs);

    if want_location {
        crate::location::request(handler);
    }
}
