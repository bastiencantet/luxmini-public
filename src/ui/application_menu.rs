//! Standard responder-chain editing shortcuts for the native settings fields.

use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::{sel, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem};
use objc2_foundation::NSString;

fn action(mtm: MainThreadMarker, title: &str, selector: Sel, key: &str) -> Retained<NSMenuItem> {
    let item = NSMenuItem::new(mtm);
    item.setTitle(&NSString::from_str(title));
    item.setKeyEquivalent(&NSString::from_str(key));
    // SAFETY: these standard AppKit actions take one sender argument. A nil
    // target lets AppKit find the active text editor through the responder chain.
    unsafe {
        item.setAction(Some(selector));
        item.setTarget(None);
    }
    item
}

/// A status-item menu does not supply the application's text-editing shortcuts.
/// Install a main menu before showing any editable window or modal dialog.
pub fn install(mtm: MainThreadMarker) {
    let menu = NSMenu::new(mtm);
    let application_item = NSMenuItem::new(mtm);
    let application = NSMenu::new(mtm);
    application.addItem(&action(mtm, "Quit LuxMini", sel!(terminate:), "q"));
    application_item.setSubmenu(Some(&application));
    menu.addItem(&application_item);

    let edit_item = NSMenuItem::new(mtm);
    edit_item.setTitle(&NSString::from_str("Edit"));
    let edit = NSMenu::new(mtm);
    edit.setTitle(&NSString::from_str("Edit"));
    for (title, selector, key) in [
        ("Cut", sel!(cut:), "x"),
        ("Copy", sel!(copy:), "c"),
        ("Paste", sel!(paste:), "v"),
        ("Select All", sel!(selectAll:), "a"),
    ] {
        edit.addItem(&action(mtm, title, selector, key));
    }
    edit_item.setSubmenu(Some(&edit));
    menu.addItem(&edit_item);
    NSApplication::sharedApplication(mtm).setMainMenu(Some(&menu));
}
