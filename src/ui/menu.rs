//! Builds the status-bar item, the dropdown menu, and the custom brightness control view.

use super::handler::{Handler, UiRefs};
use super::tray::update_tray_icon;
use crate::i18n;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSFont, NSMenu, NSMenuItem, NSSlider, NSStatusBar, NSStatusItem, NSSwitch, NSTextAlignment,
    NSTextField, NSVariableStatusItemLength, NSView,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

pub struct App {
    /// Kept alive so the status-bar item is not released; not read directly.
    pub _status_item: Retained<NSStatusItem>,
    pub handler: Retained<Handler>,
}

fn make_label(
    mtm: MainThreadMarker,
    text: &str,
    frame: NSRect,
    font_size: f64,
    align_right: bool,
    secondary: bool,
) -> Retained<NSTextField> {
    let alloc = NSTextField::alloc(mtm);
    // SAFETY: initWithFrame: on a freshly alloc'd NSTextField returns the initialized view; the
    // NSRect argument matches the selector signature.
    let field: Retained<NSTextField> = unsafe { msg_send![alloc, initWithFrame: frame] };
    field.setStringValue(&NSString::from_str(text));
    field.setEditable(false);
    field.setBezeled(false);
    field.setDrawsBackground(false);
    field.setSelectable(false);
    let font: Retained<NSFont> = if secondary {
        NSFont::systemFontOfSize(font_size)
    } else {
        NSFont::boldSystemFontOfSize(font_size)
    };
    field.setFont(Some(&font));
    if align_right {
        field.setAlignment(NSTextAlignment::Right);
    }
    if secondary {
        let color = objc2_app_kit::NSColor::secondaryLabelColor();
        field.setTextColor(Some(&color));
    }
    field
}

struct ControlRefs {
    switch: Retained<NSSwitch>,
    slider: Retained<NSSlider>,
    status_label: Retained<NSTextField>,
    percent_label: Retained<NSTextField>,
}

#[allow(clippy::too_many_lines)] // flat linear AppKit layout builder; splitting hurts readability
fn build_control_view(
    mtm: MainThreadMarker,
    handler: &Retained<Handler>,
) -> (Retained<NSView>, ControlRefs) {
    let pad: f64 = 14.0;
    let width: f64 = 260.0;
    let row_h: f64 = 22.0;
    let slider_h: f64 = 22.0;
    let gap: f64 = 6.0;
    let top_pad: f64 = 8.0;
    let bottom_pad: f64 = 10.0;
    let total_h = top_pad + row_h + gap + row_h + gap + slider_h + bottom_pad;

    // SAFETY: initWithFrame: on a freshly alloc'd NSView returns the initialized container view.
    let container: Retained<NSView> = unsafe {
        let alloc = NSView::alloc(mtm);
        msg_send![
            alloc,
            initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, total_h))
        ]
    };

    let row1_y = total_h - top_pad - row_h;
    let led_label = make_label(
        mtm,
        "LED",
        NSRect::new(NSPoint::new(pad, row1_y), NSSize::new(80.0, row_h)),
        13.0,
        false,
        false,
    );
    container.addSubview(&led_label);

    let switch_w: f64 = 40.0;
    let status_w: f64 = 50.0;
    let switch_x = width - pad - switch_w;
    let status_x = switch_x - 6.0 - status_w;

    let status_label = make_label(
        mtm,
        "ON",
        NSRect::new(NSPoint::new(status_x, row1_y), NSSize::new(status_w, row_h)),
        12.0,
        true,
        true,
    );
    container.addSubview(&status_label);

    // SAFETY: initWithFrame: on a freshly alloc'd NSSwitch returns the initialized control.
    let switch: Retained<NSSwitch> = unsafe {
        let alloc = NSSwitch::alloc(mtm);
        msg_send![
            alloc,
            initWithFrame: NSRect::new(
                NSPoint::new(switch_x, row1_y),
                NSSize::new(switch_w, row_h),
            )
        ]
    };
    // SAFETY: switch responds to setState: (NSControlStateValue / isize); setTarget:/setAction:
    // are safe objc2 bindings on the NSSwitch.
    unsafe {
        let _: () = msg_send![&switch, setState: isize::from(true)];
        switch.setTarget(Some(handler));
        switch.setAction(Some(sel!(switchToggled:)));
    }
    container.addSubview(&switch);

    let row2_y = row1_y - gap - row_h;
    let brightness_label = make_label(
        mtm,
        i18n::s("Brightness", "Luminosité"),
        NSRect::new(NSPoint::new(pad, row2_y), NSSize::new(140.0, row_h)),
        12.0,
        false,
        true,
    );
    container.addSubview(&brightness_label);

    let percent_label = make_label(
        mtm,
        "100%",
        NSRect::new(
            NSPoint::new(width - pad - 60.0, row2_y),
            NSSize::new(60.0, row_h),
        ),
        12.0,
        true,
        true,
    );
    container.addSubview(&percent_label);

    #[allow(clippy::suboptimal_flops)] // readability; sub-ULP difference vs mul_add
    // SAFETY: initWithFrame: on a freshly alloc'd NSSlider returns the initialized control.
    let slider: Retained<NSSlider> = unsafe {
        let alloc = NSSlider::alloc(mtm);
        msg_send![
            alloc,
            initWithFrame: NSRect::new(
                NSPoint::new(pad, bottom_pad),
                NSSize::new(width - 2.0 * pad, slider_h),
            )
        ]
    };
    // SAFETY: setMin/Max/DoubleValue and setTarget:/setAction: are safe objc2 bindings on the
    // NSSlider; setContinuous: takes a BOOL and returns void.
    unsafe {
        slider.setMinValue(0.0);
        slider.setMaxValue(255.0);
        slider.setDoubleValue(255.0);
        slider.setTarget(Some(handler));
        slider.setAction(Some(sel!(sliderChanged:)));
        let _: () = msg_send![&slider, setContinuous: true];
    }
    container.addSubview(&slider);

    (
        container,
        ControlRefs {
            switch,
            slider,
            status_label,
            percent_label,
        },
    )
}

/// A non-clickable section header. Uses the native `+sectionHeaderWithTitle:`
/// styling on macOS 14+, falling back to a disabled titled item on older systems.
fn make_header(mtm: MainThreadMarker, title: &str) -> Retained<NSMenuItem> {
    let title_ns = NSString::from_str(title);
    // SAFETY: respondsToSelector: guards the macOS 14+ sectionHeaderWithTitle: (autoreleased +0,
    // retained into the return by typed msg_send); the fallback uses only safe objc2 bindings.
    unsafe {
        let cls = <NSMenuItem as objc2::ClassType>::class();
        let responds: bool = msg_send![cls, respondsToSelector: sel!(sectionHeaderWithTitle:)];
        if responds {
            msg_send![cls, sectionHeaderWithTitle: &*title_ns]
        } else {
            let item = NSMenuItem::new(mtm);
            item.setTitle(&title_ns);
            item.setEnabled(false);
            item
        }
    }
}

fn make_action(
    mtm: MainThreadMarker,
    handler: &Retained<Handler>,
    title: &str,
    action: objc2::runtime::Sel,
) -> Retained<NSMenuItem> {
    let item = NSMenuItem::new(mtm);
    item.setTitle(&NSString::from_str(title));
    // SAFETY: setTarget:/setAction: are safe objc2 bindings on the NSMenuItem; the handler outlives
    // the item and the selector is one defined on the Handler class.
    unsafe {
        item.setTarget(Some(handler));
        item.setAction(Some(action));
    }
    item
}

/// Build the status-bar item, menu, and handler. The returned `App` owns the
/// status item; dropping it would remove the icon from the menu bar.
#[allow(clippy::too_many_lines)] // flat linear menu assembly; splitting hurts readability
#[must_use]
pub fn build_app(mtm: MainThreadMarker) -> App {
    let handler = Handler::new(mtm);

    let status_bar = NSStatusBar::systemStatusBar();
    let status_item = status_bar.statusItemWithLength(NSVariableStatusItemLength);

    update_tray_icon(&status_item, true, 0xff);

    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);

    menu.addItem(&make_header(mtm, i18n::s("Power", "Alimentation")));

    let (control_view, control_refs) = build_control_view(mtm, &handler);
    let control_item = NSMenuItem::new(mtm);
    control_item.setView(Some(&control_view));
    menu.addItem(&control_item);

    // Auto-dim — the flagship utility: off/dim by sunset and by time. The status
    // line shows the active rule; the two items below toggle each rule.
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&make_header(mtm, "Auto-dim"));
    let autodim_status_item = NSMenuItem::new(mtm);
    autodim_status_item.setEnabled(false); // informational status line, not clickable
    menu.addItem(&autodim_status_item);
    let autodim_sunset_item = make_action(
        mtm,
        &handler,
        i18n::s("Off at sunset", "Off au coucher du soleil"),
        sel!(toggleAutoDimSunset:),
    );
    menu.addItem(&autodim_sunset_item);
    let autodim_dim_item = make_action(
        mtm,
        &handler,
        i18n::s("Evening dim", "Atténuation du soir"),
        sel!(toggleAutoDimDim:),
    );
    menu.addItem(&autodim_dim_item);
    menu.addItem(&make_action(
        mtm,
        &handler,
        i18n::s("Disable auto-dim", "Désactiver l'auto-dim"),
        sel!(disableAutoDim:),
    ));

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    // "Fun stuff" — the animations are a gadget, deliberately tucked into a
    // submenu so the main menu stays a clean utility (off / dim / presets).
    let fun_item = NSMenuItem::new(mtm);
    fun_item.setTitle(&NSString::from_str(i18n::s("Fun stuff", "Pour le fun")));
    let fun_submenu = NSMenu::new(mtm);
    fun_submenu.setAutoenablesItems(false);
    let effects: &[(&str, objc2::runtime::Sel)] = &[
        ("Stop", sel!(effectNone:)),
        ("Blink", sel!(effectBlink:)),
        ("Blink Fast", sel!(effectBlinkFast:)),
        ("Pulse", sel!(effectPulse:)),
        ("SOS", sel!(effectSos:)),
        ("Strobe", sel!(effectStrobe:)),
    ];
    for (title, sel) in effects {
        fun_submenu.addItem(&make_action(mtm, &handler, title, *sel));
    }
    fun_item.setSubmenu(Some(&fun_submenu));
    menu.addItem(&fun_item);

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    menu.addItem(&make_header(mtm, i18n::s("Presets", "Préréglages")));
    menu.addItem(&make_action(mtm, &handler, "Preset 1", sel!(loadPreset1:)));
    menu.addItem(&make_action(mtm, &handler, "Preset 2", sel!(loadPreset2:)));
    menu.addItem(&make_action(mtm, &handler, "Preset 3", sel!(loadPreset3:)));

    let save_item = NSMenuItem::new(mtm);
    save_item.setTitle(&NSString::from_str(i18n::s(
        "Save current\u{2026}",
        "Enregistrer l'actuel\u{2026}",
    )));
    let save_submenu = NSMenu::new(mtm);
    save_submenu.setAutoenablesItems(false);
    save_submenu.addItem(&make_action(
        mtm,
        &handler,
        i18n::s("Save to Preset 1", "Enregistrer dans Preset 1"),
        sel!(saveToPreset1:),
    ));
    save_submenu.addItem(&make_action(
        mtm,
        &handler,
        i18n::s("Save to Preset 2", "Enregistrer dans Preset 2"),
        sel!(saveToPreset2:),
    ));
    save_submenu.addItem(&make_action(
        mtm,
        &handler,
        i18n::s("Save to Preset 3", "Enregistrer dans Preset 3"),
        sel!(saveToPreset3:),
    ));
    save_item.setSubmenu(Some(&save_submenu));
    menu.addItem(&save_item);

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    menu.addItem(&make_action(
        mtm,
        &handler,
        i18n::s("Settings\u{2026}", "Réglages\u{2026}"),
        sel!(openSettings:),
    ));
    menu.addItem(&make_action(
        mtm,
        &handler,
        i18n::s("About LuxMini", "À propos de LuxMini"),
        sel!(showAbout:),
    ));
    menu.addItem(&make_action(
        mtm,
        &handler,
        i18n::s("Send Feedback\u{2026}", "Envoyer un retour\u{2026}"),
        sel!(sendFeedback:),
    ));
    let launch_at_login_item = make_action(
        mtm,
        &handler,
        i18n::s("Launch at Login", "Lancer au login"),
        sel!(toggleLaunchAtLogin:),
    );
    menu.addItem(&launch_at_login_item);
    menu.addItem(&make_action(
        mtm,
        &handler,
        i18n::s(
            "Check for Updates\u{2026}",
            "Vérifier les mises à jour\u{2026}",
        ),
        sel!(checkForUpdates:),
    ));
    menu.addItem(&make_action(
        mtm,
        &handler,
        i18n::s("Quit", "Quitter"),
        sel!(quit:),
    ));

    handler.set_ui(UiRefs {
        status_item: status_item.clone(),
        switch: control_refs.switch,
        slider: control_refs.slider,
        status_label: control_refs.status_label,
        percent_label: control_refs.percent_label,
        autodim_status_item,
        autodim_sunset_item,
        autodim_dim_item,
        launch_at_login_item,
    });
    handler.sync_launch_at_login_checkmark();
    handler.sync_autodim_menu();

    // Handler is the menu delegate (menuWillOpen: refreshes controls) and observes system wake.
    // SAFETY: setDelegate: takes an NSMenuDelegate (Handler implements menuWillOpen:, outlives the
    // menu); addObserver:selector:name:object: registers it for the wake notification by name.
    unsafe {
        let _: () = msg_send![&menu, setDelegate: &*handler];
        if let Some(ws_cls) = objc2::runtime::AnyClass::get(c"NSWorkspace") {
            let ws: *mut AnyObject = msg_send![ws_cls, sharedWorkspace];
            if !ws.is_null() {
                let nc: *mut AnyObject = msg_send![ws, notificationCenter];
                if !nc.is_null() {
                    let name = NSString::from_str("NSWorkspaceDidWakeNotification");
                    let _: () = msg_send![
                        nc,
                        addObserver: &*handler,
                        selector: sel!(workspaceDidWake:),
                        name: &*name,
                        object: std::ptr::null::<AnyObject>(),
                    ];
                }
            }
        }
    }

    status_item.setMenu(Some(&menu));

    App {
        _status_item: status_item,
        handler,
    }
}
