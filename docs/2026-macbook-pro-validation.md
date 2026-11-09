# MacBook Pro rear-logo beta validation (BAS-1183)

Implementation uses a separate, unprivileged CoreBrightness backend. It detects
`bklb-backlight` through IORegistry, not a broad model-name allowlist. No new SMC
profile is inferred or enabled. Existing Mac mini / Studio behavior stays on the
SMC helper path.

## Local evidence

The development machine is MacBookPro18,3 (M1 Pro). Its IORegistry has no
`bklb-backlight` entry, and the read-only probe refuses to open a logo backend.
Unit tests exercise scaling using a supplied hardware mapping, disabled-mode
restoration, partial failure rollback, and malformed-table rejection. They do
not establish physical support for a new MacBook Pro.

## Required physical validation

Record exact model, OS build, original logo mapping/mode, and visible results.

- Confirm the beta rear-logo heading and absence of an admin prompt.
- Confirm on/off and slider endpoints affect only the rear logo; compare the
  screen and keyboard before/after each command.
- Start with the logo disabled using Apple's supported preference. Launch,
  switch on, then quit: verify the original disabled state returns.
- Start with automatic lighting enabled. Test brightness, effects, presets,
  schedules, and local API/CLI. Quit and verify the original ambient-light
  response returns, rather than leaving a fixed brightness behind.
- Test rapid effect/manual transitions, wake, display sleep, screen lock,
  low-power mode, logout and reopening. Confirm effects do not wake the display
  or keep the logo active when the system would normally extinguish it.
- Check absent properties and rejected CoreBrightness writes produce a visible
  error or HTTP 503 without a false successful state transition.
- Test termination during a pulse and after a failed write. Force quit cannot
  guarantee restoration; confirm how to recover on the target OS.
- Recheck front-LED controls on a supported Mac mini and Mac Studio.

Do not promote this feature as physically verified until these checks pass.
Private framework behavior can change across macOS releases. The owner has
authorized distribution as a beta; this does not establish physical validation.

Apple's documented on/off preference:
https://support.apple.com/en-us/149248


## Apple documentation reviewed on November 9, 2026

[Apple Support article 149248](https://support.apple.com/en-us/149248), published
November 5, applies to MacBook Pro with PROLED XDR Display. Apple documents:

- Normal logo illumination while the Mac is awake, adjusted using screen
  brightness and the ambient-light sensor.
- `sudo defaults write com.apple.CoreBrightness LEBacklightMode -int 0` to disable
  illumination. This preference applies to all accounts, including Recovery,
  and transfers when migrating to another Mac.
- `sudo defaults delete com.apple.CoreBrightness LEBacklightMode` to return to
  system-managed illumination. Reinstalling macOS also resets illumination.

These are user recovery commands, not commands the LuxMini beta runs. LuxMini
changes live properties and restores the exact mapping and mode captured at
launch, including a previously disabled logo. Deleting a user's preference on
quit would not preserve their intent. Force quit cannot guarantee restoration;
the documented delete command restores automatic behavior when troubleshooting.

Apple does not document `BrightnessSystemClient`, the nits mapping, mode 2 or
`DisplayPowerStateEvent` in that article. Those are private implementation details,
not Apple-supported brightness APIs. The article gives no RGB specification.
The beta does not promise independent ambient-light overrides, closed-lid
illumination, or lighting during system sleep.
