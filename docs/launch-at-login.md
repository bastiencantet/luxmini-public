# Launch-at-login setup

LuxMini offers automatic startup only after the shared light controller is ready.
The offer works for both front LEDs and rear logos. It does not depend on
diagnostic consent and emits no new analytics event.

- New installations see a checked, editable **Launch LuxMini at login** option.
  Registration happens only after **Open LuxMini** is pressed with the option checked.
- Existing installations see the same option unchecked, with a **Not now** action.
- Continuing with the option unchecked or choosing **Not now** remembers the decision.
- An already enabled login item is left alone and remembered. Disabling it in
  macOS later does not cause another offer or automatic re-registration.
- Successful changes from Settings or the menu also remember the decision.
- Registration errors remain visible, with retry and defer choices. They do not
  prevent use of LuxMini. Field Test builds do not show this offer.

The local preference `startup.login_offer_decided` records that the choice has
been handled, not whether automatic startup is currently enabled. The system
login-item status remains authoritative. No update resets this preference.

When the startup offer is pending, it replaces the previous setup completion
card, so the user does not have to confirm **Open LuxMini** twice.

## Verification

```sh
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --features field-test --all-targets
cargo clippy --locked --features field-test --all-targets -- -D warnings
cargo check --locked --release --bin mac-led-tray
```

Debug-only previews run before hardware discovery, preference access, telemetry,
helper startup and login registration. The native control check exercises the
checkbox, hit regions and button targets without applying a login choice:

```sh
cargo build --locked --bin mac-led-tray
LUXMINI_PREVIEW_STARTUP=check target/debug/mac-led-tray
LUXMINI_PREVIEW_STARTUP=new target/debug/mac-led-tray
LUXMINI_PREVIEW_STARTUP=existing target/debug/mac-led-tray
LUXMINI_PREVIEW_STARTUP=new LUXMINI_PREVIEW_STARTUP_EXPORT=/tmp/startup-new.png target/debug/mac-led-tray
LUXMINI_PREVIEW_STARTUP=existing LUXMINI_PREVIEW_STARTUP_EXPORT=/tmp/startup-existing.png target/debug/mac-led-tray
```

The export is a bitmap of the actual AppKit view. `disabled` and `error` are
additional preview states. Preview controls never call the registration path.
Actual login-item registration and a real logout/login still require verification
with a packaged app before release; previews do not change the host's login items.
