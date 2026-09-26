use crate::launch_at_login;
use crate::led::{read_status, with_state, Effect, LedState, STATE};
use crate::preferences;
use crate::schedule::{self, AutoDim};
#[cfg(all(feature = "direct", not(feature = "field-test")))]
use crate::sparkle;
use crate::ui::settings::{self, SettingsRefs};
use crate::ui::tray::update_tray_icon;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAlert, NSApplication, NSMenuItem, NSSlider, NSStatusItem, NSSwitch, NSTextField,
    NSToolbarItem,
};
use objc2_foundation::{NSArray, NSObject, NSObjectProtocol, NSString, NSTimer};
use std::cell::{Cell, RefCell};
#[cfg(feature = "direct")]
use std::time::{SystemTime, UNIX_EPOCH};

const FIRST_ALERT_BUTTON: isize = 1000;
#[cfg(feature = "direct")]
const THIRD_ALERT_BUTTON: isize = 1002;
#[cfg(feature = "direct")]
const SUPPORT_URL: &str = "https://www.buymeacoffee.com/bastiencantet";

pub struct UiRefs {
    pub status_item: Retained<NSStatusItem>,
    pub switch: Retained<NSSwitch>,
    pub slider: Retained<NSSlider>,
    pub status_label: Retained<NSTextField>,
    pub percent_label: Retained<NSTextField>,
    pub autodim_status_item: Retained<NSMenuItem>,
    pub autodim_sunset_item: Retained<NSMenuItem>,
    pub autodim_dim_item: Retained<NSMenuItem>,
    pub launch_at_login_item: Retained<NSMenuItem>,
}

pub struct HandlerIvars {
    pub ui: RefCell<Option<UiRefs>>,
    pub settings: RefCell<Option<SettingsRefs>>,
    /// The in-flight `CLLocationManager`, kept alive for the async request.
    pub location_manager: RefCell<Option<Retained<NSObject>>>,
    /// Coalesces all changes made while one menu session is open into one
    /// meaningful action for the support-prompt policy.
    pub menu_had_manual_action: Cell<bool>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "LedTrayHandler"]
    #[ivars = HandlerIvars]
    pub struct Handler;

    impl Handler {
        #[unsafe(method(helperFeedbackTick:))]
        fn helper_feedback_tick(&self, _timer: &AnyObject) {
            if self.show_control_error_if_any() {
                self.refresh_ui();
            }
        }

        #[unsafe(method(switchToggled:))]
        fn switch_toggled(&self, sender: &AnyObject) {
            // SAFETY: sender is an NSControl (the NSSwitch) responding to -state, which returns an
            // NSControlStateValue (isize here).
            let state: isize = unsafe { msg_send![sender, state] };
            let on = state != 0;
            schedule::note_manual_override(); // don't let auto-dim fight a manual change
            with_state(|s| s.set_on(on));
            self.finish_led_action();
        }

        #[unsafe(method(sliderChanged:))]
        fn slider_changed(&self, sender: &AnyObject) {
            // SAFETY: sender is the NSSlider responding to -doubleValue, which returns a double.
            let value: f64 = unsafe { msg_send![sender, doubleValue] };
            // Round (not truncate) so the slider extremes are reachable and the
            // conversion matches every other f64->int site in the codebase.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped+rounded to 0..=255
            let byte = value.clamp(0.0, 255.0).round() as u8;
            schedule::note_manual_override(); // don't let auto-dim fight a manual change
            with_state(|s| s.set_brightness(byte));
            self.finish_led_action();
        }

        #[unsafe(method(effectNone:))]
        fn effect_none(&self, _sender: &AnyObject) {
            schedule::note_manual_override();
            with_state(LedState::clear_effect);
            self.finish_led_action();
        }

        #[unsafe(method(effectBlink:))]
        fn effect_blink(&self, _sender: &AnyObject) {
            schedule::note_manual_override();
            with_state(|s| s.start_effect(Effect::Blink));
            self.finish_led_action();
        }

        #[unsafe(method(effectBlinkFast:))]
        fn effect_blink_fast(&self, _sender: &AnyObject) {
            schedule::note_manual_override();
            with_state(|s| s.start_effect(Effect::BlinkFast));
            self.finish_led_action();
        }

        #[unsafe(method(effectPulse:))]
        fn effect_pulse(&self, _sender: &AnyObject) {
            schedule::note_manual_override();
            with_state(|s| s.start_effect(Effect::Pulse));
            self.finish_led_action();
        }

        #[unsafe(method(effectSos:))]
        fn effect_sos(&self, _sender: &AnyObject) {
            schedule::note_manual_override();
            with_state(|s| s.start_effect(Effect::Sos));
            self.finish_led_action();
        }

        #[unsafe(method(effectStrobe:))]
        fn effect_strobe(&self, _sender: &AnyObject) {
            schedule::note_manual_override();
            with_state(|s| s.start_effect(Effect::Strobe));
            self.finish_led_action();
        }

        #[unsafe(method(toggleAutoDimSunset:))]
        fn toggle_autodim_sunset(&self, _sender: &AnyObject) {
            let mut d = preferences::load_autodim();
            d.off_at_sunset = !d.off_at_sunset;
            d.enabled = d.off_at_sunset || d.dim_at_time;
            preferences::save_autodim(&d);
            schedule::restart();
            self.note_meaningful_action();
            self.sync_autodim_menu();
        }

        #[unsafe(method(toggleAutoDimDim:))]
        fn toggle_autodim_dim(&self, _sender: &AnyObject) {
            let mut d = preferences::load_autodim();
            d.dim_at_time = !d.dim_at_time;
            d.enabled = d.off_at_sunset || d.dim_at_time;
            preferences::save_autodim(&d);
            schedule::restart();
            self.note_meaningful_action();
            self.sync_autodim_menu();
        }

        #[unsafe(method(disableAutoDim:))]
        fn disable_autodim(&self, _sender: &AnyObject) {
            // Suppress any in-flight scheduler tick's final write (its in-lock re-check
            // sees this) so disabling never causes a surprise jump.
            schedule::note_manual_override();
            let d = AutoDim {
                enabled: false,
                off_at_sunset: false,
                dim_at_time: false,
                ..preferences::load_autodim()
            };
            preferences::save_autodim(&d);
            schedule::stop(); // LED left as-is — no surprise jump
            self.note_meaningful_action();
            self.sync_autodim_menu();
        }

        // NSWorkspaceDidWakeNotification: re-evaluate the schedule the instant the Mac wakes.
        #[unsafe(method(workspaceDidWake:))]
        fn workspace_did_wake(&self, _note: &AnyObject) {
            schedule::kick();
        }

        // NSMenuDelegate: refresh the controls to the live state whenever the menu opens
        // (covers brightness the scheduler changed while the menu was closed).
        #[unsafe(method(menuWillOpen:))]
        fn menu_will_open(&self, _menu: &AnyObject) {
            self.refresh_ui();
            self.sync_autodim_menu();
        }

        // NSMenuDelegate: evaluate the support prompt only after the menu has
        // closed, so it never interrupts an LED adjustment.
        #[unsafe(method(menuDidClose:))]
        fn menu_did_close(&self, _menu: &AnyObject) {
            #[cfg(feature = "direct")]
            {
                if self.ivars().menu_had_manual_action.replace(false) {
                    preferences::record_meaningful_action(unix_timestamp());
                    Self::maybe_show_support_prompt();
                }
            }
        }

        #[unsafe(method(loadPreset1:))]
        fn load_preset_1(&self, _sender: &AnyObject) {
            self.load_preset(1);
        }

        #[unsafe(method(loadPreset2:))]
        fn load_preset_2(&self, _sender: &AnyObject) {
            self.load_preset(2);
        }

        #[unsafe(method(loadPreset3:))]
        fn load_preset_3(&self, _sender: &AnyObject) {
            self.load_preset(3);
        }

        #[unsafe(method(saveToPreset1:))]
        fn save_to_preset_1(&self, _sender: &AnyObject) {
            self.save_to_preset(1);
        }

        #[unsafe(method(saveToPreset2:))]
        fn save_to_preset_2(&self, _sender: &AnyObject) {
            self.save_to_preset(2);
        }

        #[unsafe(method(saveToPreset3:))]
        fn save_to_preset_3(&self, _sender: &AnyObject) {
            self.save_to_preset(3);
        }

        #[unsafe(method(toggleLaunchAtLogin:))]
        fn toggle_launch_at_login_obj(&self, _sender: &AnyObject) {
            self.toggle_launch_at_login();
        }

        #[unsafe(method(showAbout:))]
        fn show_about_obj(&self, _sender: &AnyObject) {
            if let Some(mtm) = MainThreadMarker::new() {
                show_about_panel(mtm);
            }
        }

        #[unsafe(method(sendFeedback:))]
        fn send_feedback(&self, _sender: &AnyObject) {
            open_feedback();
        }

        #[cfg(feature = "direct")]
        #[unsafe(method(supportLuxMini:))]
        fn support_luxmini(&self, _sender: &AnyObject) {
            // An explicit support click is stronger intent than an automatic
            // reminder; never follow it with a prompt when the menu closes.
            preferences::suppress_support_prompts();
            crate::telemetry::emit(
                "support_prompt_result",
                "support",
                &crate::compat::get_mac_model(),
            );
            open_url(SUPPORT_URL);
        }

        #[unsafe(method(openApiDocs:))]
        fn open_api_docs(&self, _sender: &AnyObject) {
            open_url("https://github.com/bastiencantet/luxmini-public#local-control-api");
        }

        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: &AnyObject) {
            if let Some(mtm) = MainThreadMarker::new() {
                settings::open(self, mtm);
            }
        }

        #[unsafe(method(saveSettings:))]
        fn save_settings(&self, _sender: &AnyObject) {
            settings::save(self);
        }

        #[unsafe(method(saveGeneralSettings:))]
        fn save_general_settings(&self, sender: &AnyObject) {
            settings::save_general(self, sender);
        }

        #[unsafe(method(toolbarDefaultItemIdentifiers:))]
        fn toolbar_default_items(&self, _toolbar: &AnyObject) -> *mut NSArray<NSString> {
            settings::toolbar_identifiers()
        }

        #[unsafe(method(toolbarAllowedItemIdentifiers:))]
        fn toolbar_allowed_items(&self, _toolbar: &AnyObject) -> *mut NSArray<NSString> {
            settings::toolbar_identifiers()
        }

        #[unsafe(method(toolbarSelectableItemIdentifiers:))]
        fn toolbar_selectable_items(&self, _toolbar: &AnyObject) -> *mut NSArray<NSString> {
            settings::toolbar_identifiers()
        }

        #[unsafe(method(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn toolbar_item(
            &self,
            _toolbar: &AnyObject,
            identifier: &NSString,
            _inserted: bool,
        ) -> *mut NSToolbarItem {
            let Some(mtm) = MainThreadMarker::new() else {
                return std::ptr::null_mut();
            };
            settings::toolbar_item(self, mtm, identifier)
        }

        #[unsafe(method(selectSettingsPane:))]
        fn select_settings_pane(&self, sender: &AnyObject) {
            // SAFETY: sender is the NSToolbarItem built by settings::toolbar_item.
            let section: isize = unsafe { msg_send![sender, tag] };
            settings::select_section(self, section);
        }

        // Settings "📍 Detect my location" button — kick off a CoreLocation request.
        #[unsafe(method(detectLocation:))]
        fn detect_location(&self, _sender: &AnyObject) {
            crate::location::request(self);
        }

        #[unsafe(method(detectLedAccess:))]
        fn detect_led_access(&self, _sender: &AnyObject) {
            self.run_isolated_setup(crate::onboarding::run_manual);
        }

        #[unsafe(method(runSetupTest:))]
        fn run_setup_test(&self, _sender: &AnyObject) {
            self.retry_led_setup();
        }

        #[cfg(feature = "field-test")]
        #[unsafe(method(checkHelperTest:))]
        fn check_helper_test(&self, _sender: &AnyObject) {
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            match with_state(LedState::test_helper_connection) {
                Some(Ok(())) => {
                    field_test_alert(
                        mtm,
                        "Helper check passed",
                        "The root helper answered and the current SMC profile returned two bytes. Run the visual fade test to confirm the physical LED.",
                        None,
                    );
                }
                Some(Err(error)) => {
                    field_test_alert(mtm, "Helper check failed", &error.to_string(), None);
                }
                None => {
                    field_test_alert(mtm, "Helper check failed", "LED state is unavailable.", None);
                }
            }
            self.refresh_ui();
        }

        #[cfg(feature = "field-test")]
        #[unsafe(method(checkRemoteProfileTest:))]
        fn check_remote_profile_test(&self, _sender: &AnyObject) {
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            match crate::profile::DeviceProfile::test_remote_profile() {
                Ok(crate::profile::RemoteProfileTest::ApprovedMatchesCache) => {
                    field_test_alert(mtm, "Profile fetch passed", "The production API returned a valid profile for this Mac. It matches the cached profile. No LED setting or cache was changed.", None);
                }
                Ok(crate::profile::RemoteProfileTest::ApprovedDiffersFromCache) => {
                    field_test_alert(mtm, "Profile fetch passed", "The production API returned a valid profile for this Mac, but it differs from the locally cached profile. No LED setting or cache was changed. Verify the physical LED before selecting a profile.", None);
                }
                Ok(crate::profile::RemoteProfileTest::CandidateOnly) => {
                    field_test_alert(mtm, "Candidate profile found", "No approved profile was served, but candidate 1 is available for an explicit visual test. No LED setting or cache was changed.", None);
                }
                Err(error) => {
                    field_test_alert(mtm, "Profile fetch failed", error, None);
                }
            }
        }

        #[cfg(feature = "field-test")]
        #[unsafe(method(testHelperRestart:))]
        fn test_helper_restart(&self, _sender: &AnyObject) {
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            if !field_test_alert(
                mtm,
                "Test helper recovery?",
                "This stops the current effect, terminates the helper, and verifies one unattended restart. The automatic restart allowance is then used until LuxMini Field Test is relaunched.",
                Some("Run test"),
            ) {
                return;
            }
            match with_state(LedState::test_helper_restart) {
                Some(Ok(())) => {
                    let note = with_state(LedState::take_control_error).flatten();
                    field_test_alert(
                        mtm,
                        "Helper recovery passed",
                        note.as_deref().unwrap_or("The helper restarted without another administrator prompt and restored the LED value."),
                        None,
                    );
                }
                Some(Err(error)) => {
                    let _ = with_state(LedState::take_control_error);
                    field_test_alert(mtm, "Helper recovery failed", &error.to_string(), None);
                }
                None => {
                    field_test_alert(mtm, "Helper recovery failed", "LED state is unavailable.", None);
                }
            }
            self.refresh_ui();
        }

        // CLLocationManagerDelegate: a fix arrived.
        #[unsafe(method(locationManager:didUpdateLocations:))]
        fn location_did_update(&self, _manager: &AnyObject, locations: &AnyObject) {
            crate::location::handle_update(self, locations);
        }

        // CLLocationManagerDelegate: the request failed.
        #[unsafe(method(locationManager:didFailWithError:))]
        fn location_did_fail(&self, _manager: &AnyObject, error: &AnyObject) {
            crate::location::handle_fail(self, error);
        }

        // CLLocationManagerDelegate: authorization status changed (or was just set).
        #[unsafe(method(locationManagerDidChangeAuthorization:))]
        fn location_did_change_auth(&self, manager: &AnyObject) {
            crate::location::handle_auth_change(self, manager);
        }

        // NSApplicationDelegate: the user re-opened LuxMini (Finder/Launchpad) while it
        // was already running with the icon hidden — bring the icon and Settings back,
        // so a hidden app is always recoverable.
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn app_should_handle_reopen(&self, _app: &AnyObject, _has_visible: bool) -> bool {
            self.reveal_icon();
            true
        }

        #[cfg(all(feature = "direct", not(feature = "field-test")))]
        #[unsafe(method(checkForUpdates:))]
        fn check_for_updates(&self, _sender: &AnyObject) {
            sparkle::check_for_updates();
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: &AnyObject) {
            if let Some(mtm) = MainThreadMarker::new() {
                let app = NSApplication::sharedApplication(mtm);
                app.terminate(None);
            }
        }
    }

    unsafe impl NSObjectProtocol for Handler {}
);

impl Handler {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(HandlerIvars {
            ui: RefCell::new(None),
            settings: RefCell::new(None),
            location_manager: RefCell::new(None),
            menu_had_manual_action: Cell::new(false),
        });
        // SAFETY: forwarding -init to the NSObject superclass on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }

    pub fn set_ui(&self, ui: UiRefs) {
        *self.ivars().ui.borrow_mut() = Some(ui);
    }

    pub fn start_helper_feedback_timer(&self) -> Retained<NSTimer> {
        // SAFETY: the selector is implemented by Handler and receives the timer.
        unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                2.0,
                self,
                sel!(helperFeedbackTick:),
                None,
                true,
            )
        }
    }

    /// Show or hide the menu-bar status item (set-and-forget mode).
    pub fn set_icon_hidden(&self, hidden: bool) {
        let ui_ref = self.ivars().ui.borrow();
        let Some(ui) = ui_ref.as_ref() else { return };
        // SAFETY: setVisible: takes a BOOL and returns void (NSStatusItem, macOS 10.12+).
        unsafe {
            let _: () = msg_send![&ui.status_item, setVisible: !hidden];
        }
    }

    /// Re-show the icon (clearing the saved preference) and open Settings — the
    /// recovery path when the app is running with a hidden icon.
    pub fn reveal_icon(&self) {
        preferences::save_hide_icon(false);
        self.set_icon_hidden(false);
        if let Some(mtm) = MainThreadMarker::new() {
            settings::open(self, mtm);
        }
    }

    pub fn refresh_ui(&self) {
        settings::refresh_led(self);
        let status = read_status();
        let is_on = status.is_on;
        let brightness = status.brightness;
        let helper_ready = status.helper_ready;
        let ui_ref = self.ivars().ui.borrow();
        let Some(ui) = ui_ref.as_ref() else { return };
        let target_state = isize::from(is_on);
        // SAFETY: ui.switch responds to setState: (NSControlStateValue / isize); the matching
        // setDoubleValue: call is a safe objc2 binding pulled out of the unsafe block below.
        unsafe {
            let _: () = msg_send![&ui.switch, setState: target_state];
        }
        ui.slider.setDoubleValue(f64::from(brightness));
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // 0..=100
        let percent = ((f64::from(brightness) / 255.0) * 100.0).round() as u32;
        let status = if !helper_ready {
            "ERROR"
        } else if is_on {
            "ON"
        } else {
            "OFF"
        };
        ui.status_label.setStringValue(&NSString::from_str(status));
        let percent_text = if helper_ready {
            format!("{percent}%")
        } else {
            "!".to_owned()
        };
        ui.percent_label
            .setStringValue(&NSString::from_str(&percent_text));
        update_tray_icon(&ui.status_item, is_on, brightness);
    }

    pub fn show_control_error_if_any(&self) -> bool {
        let error = with_state(LedState::take_control_error).flatten();
        if let Some(error) = error {
            if show_led_control_error(&error) {
                self.retry_led_setup();
            }
            true
        } else {
            false
        }
    }

    fn retry_led_setup(&self) {
        self.run_isolated_setup(|mtm, model| {
            crate::welcome::run(mtm, model, crate::compat::is_pending(model))
        });
    }

    fn run_isolated_setup(&self, setup: impl FnOnce(MainThreadMarker, &str) -> Option<LedState>) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        schedule::stop();
        let mut previous_state = match STATE.lock() {
            Ok(mut state) => state.take(),
            Err(error) => {
                eprintln!("LED state lock poisoned before setup: {error}");
                schedule::restart();
                return;
            }
        };
        let previous_preset = previous_state
            .as_mut()
            .map(LedState::pause_for_profile_discovery);
        let model = crate::compat::get_mac_model();
        let mut next_state = setup(mtm, &model).or(previous_state);
        if let (Some(state), Some(preset)) = (next_state.as_mut(), previous_preset) {
            if state.helper_ready() {
                state.apply_preset(&preset);
            }
        }
        if let Ok(mut state) = STATE.lock() {
            *state = next_state;
        } else {
            eprintln!("LED state lock poisoned after setup");
        }
        schedule::restart();
        self.refresh_ui();
    }

    fn finish_led_action(&self) {
        let failed = self.show_control_error_if_any();
        if !failed {
            self.note_meaningful_action();
        }
        self.refresh_ui();
    }

    fn note_meaningful_action(&self) {
        self.ivars().menu_had_manual_action.set(true);
    }

    #[cfg(feature = "direct")]
    fn maybe_show_support_prompt() {
        let now = unix_timestamp();
        if !preferences::support_prompt_due(now) {
            return;
        }
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);

        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(crate::i18n::s(
            "Enjoying LuxMini?",
            "LuxMini vous est utile ?",
        )));
        alert.setInformativeText(&NSString::from_str(crate::i18n::s(
            "LuxMini is free. Your support helps cover Apple signing and notarization, profile hosting, and validation on new Mac hardware.",
            "LuxMini est gratuit. Votre soutien aide à financer la signature et la notarisation Apple, l’hébergement des profils et la validation des nouveaux Mac.",
        )));
        alert.addButtonWithTitle(&NSString::from_str(crate::i18n::s(
            "Support LuxMini",
            "Soutenir LuxMini",
        )));
        alert.addButtonWithTitle(&NSString::from_str(crate::i18n::s(
            "Maybe Later",
            "Plus tard",
        )));
        alert.addButtonWithTitle(&NSString::from_str(crate::i18n::s(
            "Don't Ask Again",
            "Ne plus demander",
        )));

        match alert.runModal() {
            FIRST_ALERT_BUTTON => {
                preferences::suppress_support_prompts();
                crate::telemetry::emit(
                    "support_prompt_result",
                    "support",
                    &crate::compat::get_mac_model(),
                );
                open_url(SUPPORT_URL);
            }
            THIRD_ALERT_BUTTON => {
                preferences::suppress_support_prompts();
                crate::telemetry::emit(
                    "support_prompt_result",
                    "never",
                    &crate::compat::get_mac_model(),
                );
            }
            _ => {
                preferences::defer_support_prompt(now);
                crate::telemetry::emit(
                    "support_prompt_result",
                    "later",
                    &crate::compat::get_mac_model(),
                );
            }
        }
    }

    fn load_preset(&self, slot: u8) {
        if let Some(p) = preferences::load_preset(slot) {
            schedule::note_manual_override();
            with_state(|s| s.apply_preset(&p));
            self.note_meaningful_action();
            self.refresh_ui();
        }
    }

    #[allow(clippy::unused_self)] // mirrors load_preset(&self, ..) for call-site symmetry
    fn save_to_preset(&self, slot: u8) {
        let preset = {
            let g = STATE.lock().ok();
            g.and_then(|g| g.as_ref().map(LedState::capture_preset))
        };
        if let Some(p) = preset {
            preferences::save_preset(slot, &p);
        }
    }

    fn toggle_launch_at_login(&self) {
        let currently = launch_at_login::is_enabled();
        if let Err(e) = launch_at_login::set_enabled(!currently) {
            eprintln!("launch_at_login toggle failed: {e}");
            return;
        }
        self.sync_launch_at_login_checkmark();
    }

    pub fn sync_launch_at_login_checkmark(&self) {
        let ui_ref = self.ivars().ui.borrow();
        let Some(ui) = ui_ref.as_ref() else { return };
        let state: isize = isize::from(launch_at_login::is_enabled());
        // SAFETY: ui.launch_at_login_item is an NSMenuItem responding to setState:
        // (NSControlStateValue / isize); the call returns void.
        unsafe {
            let _: () = msg_send![&ui.launch_at_login_item, setState: state];
        }
    }

    /// Refresh the Auto-dim status line and the two rule checkmarks from prefs.
    pub fn sync_autodim_menu(&self) {
        let ui_ref = self.ivars().ui.borrow();
        let Some(ui) = ui_ref.as_ref() else { return };
        let d = preferences::load_autodim();
        let loc_precise = preferences::load_location().is_some();
        ui.autodim_status_item
            .setTitle(&NSString::from_str(&format_autodim_status(d, loc_precise)));
        let sunset_state = isize::from(d.enabled && d.off_at_sunset);
        let dim_state = isize::from(d.enabled && d.dim_at_time);
        // SAFETY: both are NSMenuItems responding to setState: (NSControlStateValue / isize),
        // returning void; the checkmarks reflect which rules are active.
        unsafe {
            let _: () = msg_send![&ui.autodim_sunset_item, setState: sunset_state];
            let _: () = msg_send![&ui.autodim_dim_item, setState: dim_state];
        }
    }
}

fn show_led_control_error(detail: &str) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);

    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(crate::i18n::s(
        "LuxMini could not control the LED",
        "LuxMini n’a pas pu contrôler la LED",
    )));
    let message = format!(
        "{}\n\n{}: {detail}",
        crate::i18n::s(
            "The control was not applied. LuxMini has restored the interface to the last confirmed state.",
            "La commande n’a pas été appliquée. LuxMini a restauré l’interface au dernier état confirmé.",
        ),
        crate::i18n::s("Technical detail", "Détail technique"),
    );
    alert.setInformativeText(&NSString::from_str(&message));
    alert.addButtonWithTitle(&NSString::from_str(crate::i18n::s(
        "Check LED Setup",
        "Vérifier la configuration LED",
    )));
    alert.addButtonWithTitle(&NSString::from_str(crate::i18n::s(
        "Send Feedback",
        "Envoyer un retour",
    )));
    alert.addButtonWithTitle(&NSString::from_str(crate::i18n::s("Later", "Plus tard")));
    let response = alert.runModal();
    if response == FIRST_ALERT_BUTTON + 1 {
        open_feedback();
    }
    response == FIRST_ALERT_BUTTON
}

#[cfg(feature = "field-test")]
fn field_test_alert(
    mtm: MainThreadMarker,
    title: &str,
    detail: &str,
    confirm: Option<&str>,
) -> bool {
    let app = NSApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(detail));
    if let Some(label) = confirm {
        alert.addButtonWithTitle(&NSString::from_str(label));
        alert.addButtonWithTitle(&NSString::from_str("Cancel"));
    } else {
        alert.addButtonWithTitle(&NSString::from_str("OK"));
    }
    alert.runModal() == FIRST_ALERT_BUTTON
}

#[cfg(feature = "direct")]
fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(0)
}

/// Build the Auto-dim status-line text from the active rules.
fn format_autodim_status(d: AutoDim, loc_precise: bool) -> String {
    use crate::i18n;
    if !d.enabled || (!d.off_at_sunset && !d.dim_at_time) {
        return i18n::s("off", "désactivé").to_string();
    }
    // The tz-derived sunset is approximate until the user sets a precise location.
    let approx = if d.off_at_sunset && !loc_precise {
        " (approx.)"
    } else {
        ""
    };
    let hh = d.dim_minute / 60;
    let mm = d.dim_minute % 60;
    match (d.off_at_sunset, d.dim_at_time) {
        (true, true) if i18n::fr() => format!("Soir {}% · Nuit Off{approx}", d.dim_pct),
        (true, true) => format!("Evening {}% · Night Off{approx}", d.dim_pct),
        (true, false) => i18n::s("Night Off at sunset", "Nuit Off au coucher").to_string() + approx,
        (false, true) if i18n::fr() => format!("Soir {}% dès {hh:02}:{mm:02}", d.dim_pct),
        (false, true) => format!("Evening {}% from {hh:02}:{mm:02}", d.dim_pct),
        (false, false) => i18n::s("off", "désactivé").to_string(),
    }
}

/// Open the web feedback form, carrying the app version and Mac model as query
/// params so reports arrive with the diagnostics already filled in.
fn open_feedback() {
    let model = crate::compat::get_mac_model();
    let url = format!(
        "https://luxmini.bastiencantet.com/feedback?v={}&model={}",
        env!("CARGO_PKG_VERSION"),
        urlencode(&model),
    );
    open_url(&url);
}

/// Minimal percent-encoding for a query-parameter value (keeps it dependency-free).
fn urlencode(s: &str) -> String {
    fn hex_digit(n: u8) -> char {
        match n {
            0..=9 => (b'0' + n) as char,
            _ => (b'A' + n - 10) as char,
        }
    }
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push(hex_digit(b >> 4));
                out.push(hex_digit(b & 0x0f));
            }
        }
    }
    out
}

/// Open a URL in the user's default browser via `NSWorkspace`.
fn open_url(url: &str) {
    use objc2::runtime::AnyClass;
    // SAFETY: +[NSURL URLWithString:] builds an NSURL* (nil for a malformed string, which we
    // null-check); -[NSWorkspace sharedWorkspace] / -openURL: match their selector signatures.
    unsafe {
        let Some(url_cls) = AnyClass::get(c"NSURL") else {
            return;
        };
        let s = NSString::from_str(url);
        let nsurl: *mut AnyObject = msg_send![url_cls, URLWithString: &*s];
        if nsurl.is_null() {
            return;
        }
        let Some(ws_cls) = AnyClass::get(c"NSWorkspace") else {
            return;
        };
        let ws: *mut AnyObject = msg_send![ws_cls, sharedWorkspace];
        if ws.is_null() {
            return;
        }
        let _: bool = msg_send![ws, openURL: nsurl];
    }
}

fn show_about_panel(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);

    // SAFETY: about-panel options dict (+0 factory) holding an alloc/init'd attributed string
    // (+1 wrapped in Retained so the dict retains it and our +1 drops at scope end). Main thread.
    unsafe {
        let Some(dict_cls) = objc2::runtime::AnyClass::get(c"NSMutableDictionary") else {
            return;
        };
        let dict: *mut AnyObject = msg_send![dict_cls, dictionary];
        if dict.is_null() {
            return;
        }

        let credits_text = NSString::from_str("Made with ❤️ by Bastien CANTET");
        let Some(attrstr_cls) = objc2::runtime::AnyClass::get(c"NSAttributedString") else {
            return;
        };
        let attr_alloc: *mut AnyObject = msg_send![attrstr_cls, alloc];
        if attr_alloc.is_null() {
            return;
        }
        let credits_raw: *mut AnyObject = msg_send![attr_alloc, initWithString: &*credits_text];
        let Some(credits) = Retained::from_raw(credits_raw) else {
            return;
        };

        let credits_key = NSString::from_str("Credits");
        let _: () = msg_send![dict, setObject: &*credits, forKey: &*credits_key];

        let _: () = msg_send![&app, orderFrontStandardAboutPanelWithOptions: dict];
    }
}
