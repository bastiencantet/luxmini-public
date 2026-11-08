# LuxMini Field Test on a Mac mini

This private build is for hardware validation, not distribution. It has a separate app bundle identifier, preferences, and profile cache. Automatic updates are disabled. It does not replace the installed LuxMini app. Both apps still control the same physical LED, so quit the regular app before starting a test.

The candidate has passed the direct and field-test unit suites, strict Clippy, universal arm64/x86_64 builds, bundle signature verification, ZIP integrity, and the Settings Save button hit test. The two hardware-only helper tests remain for the Mac mini. A successful build is not proof of visible LED control or of a working production profile fetch. Do not publish this ad-hoc signed package as a release.

## Install and start

1. Copy the current fade-and-crop candidate, `dist/field-test/fade-crop-2026-09-20/LuxMini Field Test-0.4.0.zip`, to the Mac mini. This local test package is ad-hoc signed, not notarized. A DMG may also be present when the build host supports disk-image creation.
2. Quit both the regular LuxMini app and any older Field Test app. Expand the ZIP and move the new `LuxMini Field Test.app` into `/Applications`, replacing only the older Field Test app. Do not run the privileged helper from Downloads.
3. Launch the test app from `/Applications` (or run `open '/Applications/LuxMini Field Test.app'`). Approve its own helper installation once if macOS asks. The normal app's helper remains untouched.
4. Complete the welcome test. Confirm **Yes** only if the physical front LED visibly fades, not merely when the on-screen illustration changes.
5. Open **Settings > Advanced** for the five test actions. Run the restart test last; it consumes the one automatic restart allowed in that app session.

## Hardware checklist

Record the Mac model, macOS version, test build version, each result, and any error text. Do not share SMC keys or passwords in a public issue.

| Test | Expected result |
| --- | --- |
| First launch and helper setup | One administrator approval; no repeated password prompt on the next launch. |
| Check helper and SMC read | Reports that the root helper responds and the profile returns exactly two bytes. This alone does not prove that the LED moves. |
| Check production profile fetch | Reports an approved profile matching or differing from the cache, or candidate 1 for a model awaiting validation. It must not change the cache or LED. Record a failure exactly as displayed. |
| Run setup and visual fade | The real front LED keeps fading until Yes or No is selected. The on-screen image follows the physical LED test. After either answer, the original LED bytes are restored. |
| Detect LED profile | Tries at most three server-allowlisted candidates, shows their numbered pulses, restores the previous LED value after each test, and selects only a visually confirmed profile. Cancel keeps the previous profile. |
| LED switch and slider | Off, on, minimum, middle, and maximum visibly match the controls and remain correct after reopening the menu. |
| Effects | Blink, fast blink, pulse, SOS, and strobe work; stopping an effect restores the manual LED state. |
| Presets | Save and recall all three slots. Quit and reopen the app; the last manual state returns. |
| Automatic dimming | Save a near-future dim time, verify the dim level, then disable it and verify restoration. Check sunset behavior only when the time/location makes it testable. |
| Sleep and wake | The chosen LED state resumes correctly without a password prompt. |
| Local API | Enable it in General, then run `curl -i http://127.0.0.1:4470/healthz` and `curl -i http://127.0.0.1:4470/led`. A working helper gives 200 on `/healthz`; a confirmed LED write failure must give 503. The API is off by default. |
| Test one helper restart | After confirmation, the helper is terminated and restarted without another administrator prompt. The LED returns to its previous value. Relaunch the test app before repeating. |
| Launch at login | If tested, enable only the test app's setting, log out and in, verify it starts, then disable the setting. |

## Full interface and failure review

Work through the app in this order while the Mac mini is still available. For every step, record **Pass**, **Fail**, or **Not tested**, a short observation, and a screenshot of any broken layout or error. Keep the Field Test app separate from the normal LuxMini app.

1. **Welcome:** Check the model and illustration, text alignment, primary button, helper-install prompt, continuous physical fade, Yes/No behavior, and restored LED state. Choose Yes only after seeing the real front light change. Relaunch the app and confirm it does not reopen the welcome screen or ask for the administrator password again.
2. **Menu bar:** Check that the icon appears, the menu opens, every label is readable, the switch and slider remain synchronized with the Settings window, and a manual change is preserved after closing and reopening the menu. Check the menu after changing the system appearance if both Light and Dark modes are available.
3. **LED settings:** Check image/model alignment, LED indicator alignment, switch, slider at 0/middle/maximum, all effects, stop effect, and three preset save/recall actions. Confirm the physical light follows the UI, not just the on-screen illustration. Quit and relaunch to check the last state.
4. **Automation settings:** Try a valid near-future time and brightness, click **Apply automation**, close and reopen Settings, then observe the scheduled change. Disable the rule and confirm the LED returns to manual control. Try invalid time and invalid latitude/longitude values; they must show a clear error and must not silently save. Restore any test location afterward.
5. **General settings:** Toggle launch at login, menu icon visibility, local API, and optional diagnostics independently. Confirm each setting survives reopening Settings. Verify hiding the icon is recoverable by opening the app from Finder. Disable any option you do not want to keep. With the API enabled, check `/healthz` and `/led` on localhost; disable it afterward.
6. **Advanced settings:** Run helper check, production profile fetch, visual fade, and profile discovery. Decline a candidate at least once and confirm the original LED state returns. Confirm no candidate is selected without an explicit visible Yes. Run the helper restart test last and verify no password prompt appears.
7. **Recovery:** With a safe manual LED state, quit/relaunch, sleep/wake, then reboot the Mac mini. Check that the app recovers the correct state and does not ask for the administrator password again. If a test fails, preserve its exact error text and avoid repeatedly approving a profile that did not visibly work.
8. **Layout:** At the Mac mini's normal display scaling, inspect every pane for clipped text, misaligned icons, overlapping controls, dead buttons, incorrect focus, and poor contrast in both appearances. In particular, confirm **Apply automation** responds to an ordinary pointer click.

### Result record

```text
Mac model:
macOS version:
Field Test bundle version/date:
Welcome / helper / physical fade:
Menu bar / LED controls / effects / presets:
Automation / invalid inputs:
General / localhost API / login item:
Advanced / remote profile / discovery / helper restart:
Relaunch / sleep-wake / reboot:
Light and Dark appearance / display scaling:
Failures with exact message and reproduction steps:
``` 

If the LED does not visibly change, do not mark that profile as validated. A successful SMC response only proves that the command was accepted, not that the front light moved.

The private test build does not test Sparkle updates or notarization. Those require a separate signed release candidate. A helper that remains alive but stops answering is detected when the next command reaches its timeout; the idle monitor checks process liveness only.

## Icon and Save regression checks

The app icon source must contain real alpha transparency outside its silhouette.
Run `python3 scripts/make_icon.py` to rebuild all ten PNG representations and
`assets/Icon.icns`. The generator rejects opaque/empty sources, checks transparent
corners and a preserved white LED, and uses alpha compositing without applying
the edge alpha twice. The content occupies 824 pixels on the 1024-pixel canvas.

The Save button reported in issue #4 is now **Apply automation**. General
preferences are saved immediately by their switches. Verify its native mouse
target without starting the helper or submitting any preferences:

```sh
cargo build --bin mac-led-tray --locked
LUXMINI_CHECK_SAVE_HIT_TEST=1 target/debug/mac-led-tray
```

Expected: `apply_automation_hit=true`. This debug-only path exits before normal
startup, profile resolution, helper installation, scheduling, or LED access.
It checks that the button is enabled and wired to `saveSettings:`, and that nine
points (center, edge midpoints, and inset corners) hit the button rather than an
overlapping view. It does not invoke Save or modify the user's preferences.
The pre-existing center-only check and the expanded nine-point check both passed
locally on November 8, 2026. Actual pointer clicks, preference persistence,
and invalid-input dialogs were subsequently checked as recorded below.

### Issue #4 reproduction record (November 8, 2026)

Source: https://github.com/bastiencantet/luxmini-public/issues/4
The reporter used LuxMini 0.4.0 on a Mac Studio M4 Max with macOS 26.6.2.
They reported that pointer clicks did nothing, while a System Events AppleScript
click saved preferences and closed Settings. The issue contains no exact script
and had no comments when checked. The current interface uses **Apply automation**
and intentionally keeps Settings open after saving.

The local check used the current 0.4.1-based client with the local icon changes
and expanded hit-test assertion, on macOS 26.5 (25F71), in English and French.
It did not run the historical 0.4.0 binary or reproduce the reporter's OS/hardware.

Safety and isolation:

- Copy the debug executable into a separate test bundle with identifier
  `com.bastiencantet.luxmini.ui-regression` and a distinct executable name.
- Launch that executable with `LUXMINI_PREVIEW_SETTINGS_SHOW=1`. This debug path
  enters Settings before profile resolution, helper startup, or LED-state setup.
  Do not launch the test bundle through Finder without that environment variable.
- Keep both automation switches off and both location fields empty. The test
  must neither start an enabled schedule nor request location access.
- Inspect only the test bundle's preferences using
  `defaults read com.bastiencantet.luxmini.ui-regression`.
  Normal LuxMini preferences are not used.

Observed results:

| Check | Result |
| --- | --- |
| English pointer click on Apply automation | Saved 22:17 / 37%; persisted minute 1337 and percentage 37 with automation disabled. |
| Full process restart | Settings reloaded 22:17 / 37%. |
| Invalid time 25:99 | Validation dialog; persisted settings unchanged. |
| Invalid brightness 101% | Validation dialog; persisted settings unchanged. |
| Invalid latitude 91, longitude 0 | Validation dialog; persisted settings unchanged. |
| Subsequent valid save | Saved 23:08 / 41%; another restart reloaded both values. |
| French pointer click on Appliquer l'automatisation | Saved 21:36 / 29%; persisted minute 1296 and percentage 29. |
| System Events AppleScript click | Not verified: macOS denied osascript assistive access (-1728). No accessibility permissions were changed. |
| Physical LED / helper startup / reboot | Not tested in this UI-only session. |

The pointer-only failure was not reproduced in this environment. This is not
confirmation from the reporter and does not justify closing the issue by itself.
Repeat on the reported Mac/OS, with explicit approval for any physical LED test.
The remaining AppleScript comparison requires existing authorized assistive
access; do not bypass or silently expand macOS permissions.
