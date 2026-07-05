use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{msg_send, MainThreadMarker};
use objc2_app_kit::{NSImage, NSStatusItem};
use objc2_foundation::NSString;

/// Build the menu-bar status icon for the current LED state: a hollow circle when
/// off/zero, otherwise a filled circle whose point size scales with brightness.
#[must_use]
pub fn make_tray_icon(is_on: bool, brightness: u8) -> Option<Retained<NSImage>> {
    let (symbol, point_size) = if !is_on || brightness == 0 {
        ("circle", 11.0f64)
    } else {
        let norm = f64::from(brightness) / 255.0;
        #[allow(clippy::suboptimal_flops)] // readability; sub-ULP difference vs mul_add
        let pt = 8.0 + norm.sqrt() * 8.0;
        ("circle.fill", pt)
    };

    let name_ns = NSString::from_str(symbol);
    let desc_ns = NSString::from_str("LED");
    let img =
        NSImage::imageWithSystemSymbolName_accessibilityDescription(&name_ns, Some(&desc_ns))?;

    // SAFETY: symbol-config looked up via `?`; valid selectors; Retained::retain takes the +0 returns.
    unsafe {
        let config_cls = objc2::runtime::AnyClass::get(c"NSImageSymbolConfiguration")?;
        let config_raw: *mut AnyObject = msg_send![
            config_cls,
            configurationWithPointSize: point_size,
            weight: 0.0f64,
        ];
        if let Some(config) = Retained::retain(config_raw) {
            let configured_raw: *mut NSImage =
                msg_send![&img, imageWithSymbolConfiguration: &*config];
            if let Some(configured) = Retained::retain(configured_raw) {
                configured.setTemplate(true);
                return Some(configured);
            }
        }
        img.setTemplate(true);
    }
    Some(img)
}

/// Refresh the status-bar button image to reflect the current LED state.
pub fn update_tray_icon(status_item: &NSStatusItem, is_on: bool, brightness: u8) {
    if let Some(icon) = make_tray_icon(is_on, brightness) {
        if let Some(mtm) = MainThreadMarker::new() {
            if let Some(button) = status_item.button(mtm) {
                button.setImage(Some(&icon));
            }
        }
    }
}
