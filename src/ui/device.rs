//! Device-specific wording and artwork selection, outside the shared controller.

use crate::i18n;
use crate::output::{DeviceInfo, LightKind};

pub struct Presentation {
    pub title: &'static str,
    pub menu_heading: &'static str,
    pub control_label: &'static str,
    pub schedule_hint: &'static str,
    pub preview_caption: &'static str,
    pub shows_logo: bool,
}

pub fn current() -> Presentation {
    for_device(crate::light::read_status().device)
}

pub fn for_device(device: DeviceInfo) -> Presentation {
    match device.kind {
        LightKind::RearLogo => {
            let title = if device.experimental {
                i18n::s("Rear logo (beta)", "Pomme arrière (bêta)")
            } else {
                i18n::s("Rear logo", "Pomme arrière")
            };
            Presentation {
                title,
                menu_heading: title,
                control_label: i18n::s("Rear logo", "Pomme arrière"),
                schedule_hint: i18n::s(
                    "Choose when LuxMini should adjust the rear logo.",
                    "Choisissez quand LuxMini ajuste la pomme arrière.",
                ),
                preview_caption: i18n::s(
                    "Controls the Apple logo on the back of the display.",
                    "Contrôle la pomme au dos de l'écran.",
                ),
                shows_logo: true,
            }
        }
        LightKind::FrontLed => Presentation {
            title: i18n::s("Your front LED", "La LED de votre Mac"),
            menu_heading: i18n::s("Power", "Alimentation"),
            control_label: i18n::s("Front light", "Voyant avant"),
            schedule_hint: i18n::s(
                "Choose when LuxMini should adjust the front LED.",
                "Choisissez quand LuxMini ajuste la LED avant.",
            ),
            preview_caption: i18n::s(
                "Controls the front light on your Mac.",
                "Contrôle le voyant avant de votre Mac.",
            ),
            shows_logo: false,
        },
    }
}
