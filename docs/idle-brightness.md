# Idle brightness trigger

The LED settings section offers two mutually exclusive native radio buttons:

- **When the Mac sleeps**: apply the temporary level before system sleep;
  restore the current automation or previous manual setting on system wake.
- **When the display turns off**: apply the level when macOS puts its displays
  to sleep, including while background work keeps the Mac awake. Restore only
  when the displays are awake and system sleep has ended.

The independent feature switch remains off by default. Missing or unknown
`sleep.trigger` values retain system sleep; `1` selects display sleep. Existing
enabled sleep settings keep their level and previous behavior. `0%` turns the
light off. Screen locking or a screensaver alone is not display sleep, and
physically disconnecting or powering off an external monitor is not a supported
substitute for a macOS display-sleep notification.

The four NSWorkspace notifications update one shared controller policy, used by
both front LEDs and rear logos. System sleep is prepared before hardware suspend.
In display mode, a system wake reasserts the idle level if the screen is still
asleep. Display-wake-before-system-wake and the reverse order are both supported.
Repeated events do not recapture the original manual setting or effect.

Settings edits while awake do not preview brightness on the hardware. Editing an
active idle level updates that transient level. Disabling the feature or choosing
a currently inactive trigger releases the override. Manual presets remain intact;
current automation and its manual grace period are resolved at restoration.

Display events do not emit system-sleep diagnostic milestones. No new telemetry
fields, event vocabulary, consent or server deployment are introduced. Feedback
drafts include the selected trigger so display sleep and system sleep can be
distinguished in future reports.

## Verification

```sh
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --features field-test --all-targets
cargo build --locked --bin mac-led-tray
python3 scripts/test-idle-settings.py --binary target/debug/mac-led-tray
python3 scripts/test-idle-settings.py --binary target/debug/mac-led-tray --screenshots /tmp/luxmini-idle-preview
python3 scripts/test-settings-reopen.py --binary target/debug/mac-led-tray
```

Controller tests cover both outputs, overlapping and repeated events, both wake
orders, rejected writes, current-rule restoration, preference changes and the
old default. The native fixture uses a uniquely identified disposable app and a
memory light driver. It clicks the native switch/radios, checks saved state and
reopening, posts process-local notifications through the actual observers, and
verifies output restoration. Optional PNGs render the real AppKit controls.
Release builds reject the fixture flag before hardware discovery.

These checks do not put the host Mac or its displays to sleep and do not prove
physical LED behavior across firmware sleep. Real display/system sleep cycles
remain part of the hardware beta checklist before release.
