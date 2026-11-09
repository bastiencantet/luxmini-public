<div align="center">

# LuxMini

[![README views](https://hits.sh/github.com/bastiencantet/luxmini-public.svg?label=README%20views&color=38bdf8&style=flat-square)](https://hits.sh/github.com/bastiencantet/luxmini-public/)

**Turn off, dim, or schedule the front LED on your Mac.**
Source-available (Fair Source) · lightweight menu-bar app · set-and-forget.

[Install](#install) · [How it works](#how-it-works) · [Privacy](#privacy) · [Support](#support)

</div>

---

The Mac mini's (and Studio's) front power LED is **bright**, always on, and macOS gives
you **no way to turn it off**. People resort to a piece of tape. LuxMini fixes that in
software: switch it **off**, **dim** it, or have it **auto-dim at night / after sunset**.

> **Is this even real?** Yes. macOS doesn't expose the front LED, but it's driven by the
> Mac's **SMC** (System Management Controller), the same chip that runs the fans and
> sensors. LuxMini writes to it through a tiny privileged helper. The
> [client source](https://github.com/bastiencantet/luxmini-public) is public so
> you can inspect how the app, helper, profile validation, and local API work.

Beta development support also detects the illuminated rear Apple logo
on compatible MacBook Pro models. It uses a separate unprivileged backend;
physical validation on the new hardware is still pending.

## Features

- **Off / On** and **brightness** for the front LED.
- **Auto-dim**: lower the LED at a chosen time or turn it off after sunset.
- **Three presets**: restore a saved power, brightness, and effect state.
- **Multi-model**: Mac mini and community-tested Mac Studio profiles, with a
  reversible visual test for recognised hardware awaiting physical validation.
- **Local control API**: optional, off by default, and bound to localhost only.
- Menu-bar app, discreet, launches at login, and checks for updates with Sparkle.
- **Fun stuff** *(optional, tucked away)*: blink / pulse / SOS. Most people don't want
  these on a status light; they live in their own submenu.

## Install

Download the latest DMG from the
[LuxMini website](https://luxmini.bastiencantet.com/download) or the
[public releases page](https://github.com/bastiencantet/luxmini-public/releases).
Each GitHub release includes a SHA-256 checksum.

1. Open the DMG and drag `LuxMini.app` into Applications.
2. Launch LuxMini normally. Release builds are signed with Developer ID and
   notarized by Apple.
3. Approve the standard administrator prompt once so the bundled helper can
   control the front LED. The experimental rear-logo backend needs no administrator prompt.

You can also build the public client source yourself:

```sh
git clone https://github.com/bastiencantet/luxmini-public
cd luxmini-public
make run
```

Building requires Rust through `rustup` and the Xcode command-line tools. The
model-specific SMC profile data is delivered by the private profile service and
is intentionally not published in the client repository.

## How it works

The front LED is a PWM channel on the Mac's **SMC**. LuxMini:
1. runs a small **setuid-root helper** (`led-helper`) that talks to `AppleSMC` via IOKit,
   the only privileged part, kept minimal and auditable;
2. loads a **device profile** that says *how* to address the LED on your specific Mac model
   (the addressing differs across Intel / T2 / Apple Silicon / Studio);
3. writes the brightness; the app itself stays unprivileged.

The per-model addressing, meaning which SMC channel each model uses, lives in a **device profile**
loaded at runtime rather than hardcoded, so the app stays clean and supports a new model by
shipping a profile instead of a new build.

### MacBook Pro rear logo (beta)

On machines with an actual `bklb-backlight` device, LuxMini uses Apple's private
CoreBrightness interface instead of the SMC helper. The same switch, brightness
slider, presets, effects, schedule, and local API target the rear logo. The menu
labels this backend as beta. Older MacBooks without this device remain
unsupported; no screen-brightness fallback is used.

Brightness is scaled to the maximum in the device's existing logo mapping.
macOS ambient-light restrictions remain in force. On normal quit/logout, LuxMini
restores the mapping and on/off mode captured at launch, including an originally
disabled logo. Force-quitting or killing the process cannot guarantee restoration.
Avoid running another logo controller simultaneously.

This beta backend uses undocumented properties and is not yet physically validated.
A macOS update may make it unavailable; missing properties or rejected writes
surface as errors. The local API returns HTTP 503 when a control operation fails.
Apple documents the standard on/off preference in its
[illuminated-logo support article](https://support.apple.com/en-us/149248).

The beta limitations and hardware checks are documented in
[the rear-logo checklist](docs/2026-macbook-pro-validation.md).

### Shared light engine

Front LEDs and rear logos use one controller for brightness, on/off, effects,
presets, scheduling and local API commands. Transport-specific work lives behind
`LightOutput`: an SMC driver, a CoreBrightness driver and an in-memory driver.
Device wording and artwork live in the UI presentation layer.

### Light simulators

Run `scripts/build-light-simulators.sh`, then open `dist/LuxMini Simulator.app`
for the rear logo or `dist/LuxMini LED Simulator.app` for the front LED.
Alternatively, use `cargo run --bin mac-led-tray -- --simulate-logo` or
`cargo run --bin mac-led-tray -- --simulate-led`.
The native preview offers on/off, a brightness slider, and the same steady,
pulse, blink and SOS effects as the app. It uses an in-memory output only:
no hardware probing, helper, CoreBrightness, preferences, API or telemetry startup.
This visual preview does not validate the physical logo or its nits response.

## Local control API

LuxMini can expose a tiny **local HTTP API** so Apple Shortcuts,
`AppleScript`, Home Assistant, and shell scripts can read and drive the LED. It is
**off by default**, **binds `127.0.0.1` only** (never the network), and there is
no cloud or account involved. Turn it on in **Settings › General → “Enable local
control API”**; optionally set a bearer token (`api.token`).

`/light` and the existing `/led` path accept the same commands. Status includes
`ready`, `device` (`front_led` or `rear_logo`), `experimental`, and `simulated`.
`helper_ready` remains an alias of `ready` for existing clients.

Endpoints (default port `4470`):

| Method & path | Body | Result |
|---|---|---|
| `GET /led` | None | `{ "on": bool, "brightness": 0-255, "max": 255 }` |
| `POST /led` | `{ "on": bool }` / `{ "brightness": 0-255 }` / `{ "effect": "blink\|blinkfast\|pulse\|sos\|strobe\|none" }` | applies it, returns the new state |
| `GET /healthz` | None | `{ "status": "ok" }` |

The bundled **`luxmini` CLI** wraps it:

```sh
luxmini get
luxmini brightness 128
luxmini effect blink
luxmini off
```

Or hit it directly:

```sh
curl -s http://127.0.0.1:4470/led
curl -s -X POST http://127.0.0.1:4470/led -d '{"brightness":128}'
```

Ready-to-use recipes for Home Assistant, AppleScript, and the shell are in
[`examples/`](examples/). For Apple Shortcuts, use **Run Shell Script**
(`luxmini …`) or **Get Contents of URL** against the endpoints above.

## Privacy

LuxMini has no account or advertising SDK. Essential network calls check for
updates, request the profile matching the Mac model, and submit a bounded
hardware-validation answer after the explicit visual test. These calls do not
include an installation ID. LED values, schedules, presets and local API
requests stay on the Mac.

Optional product statistics are **off until a fresh, explicit consent choice**.
The earlier aggregate-diagnostics switch does not authorize linked statistics.
Refusing does not limit the app, and the choice is remembered without repeatedly
asking. Settings > General can enable or disable the option later.

After consent only, LuxMini creates a random, app-specific installation ID.
Daily activity is linked to this ID along with the app version, previous version
on a version change, Mac model, macOS major version and a coarse installation
origin (newly observed or already existing). These statistics are **pseudonymous**,
not fully anonymous: an installation is recognizable over time but is not a
person, account or serial number. They never identify a website visitor or get
shared with an advertising provider. Technical counters remain aggregate-only;
support-reminder choices are no longer collected by this client.

Disabling statistics stops collection, clears the local ID and requests deletion
of linked server data. Offline erasure requests are retried; accepting again
creates a different ID. Linked activity is retained for at most 90 days, and
installation metadata is removed after at most 90 days of inactivity, provided
the required daily retention task is operating. Already anonymous aggregate
counters cannot be linked back to an installation. The service receives the
connection IP for transport, so proxy logging and backup retention must also be
reviewed before rollout; the product-statistics payload and tables contain no IP.

Privacy contact: [bastiencantet@outlook.com](mailto:bastiencantet@outlook.com).
Copy the statistics ID from Settings > General if it is needed to locate your
records privately; never post it or a hardware profile in a public issue.

Optional installation statistics are available from LuxMini 0.4.3. The public
privacy notice, erasure flow and daily retention task have been verified for
this rollout. These statistics cannot retrospectively recover users, updates
or unique downloads from releases that did not send them.

## Support

LuxMini is free and its source is open to read, build, and audit. If it saved you a piece of tape:
- ⭐ Star the repo
- ☕ [Support development](https://www.buymeacoffee.com/bastiencantet). Donations
  help fund the Apple Developer membership, profile hosting, and physical testing
  on new Mac hardware.

## Contributing

Issues and PRs are welcome in the
[public client repository](https://github.com/bastiencantet/luxmini-public),
especially compatibility reports for new Mac models. Never post an SMC key,
serial number, or another device identifier.

## License

[FSL-1.1-Apache-2.0](LICENSE) © Bastien Cantet. **Functional Source License**
([fair.io](https://fair.io)): the source is open to read, build, modify, and use
for any purpose **except** building a competing product or reselling it. Each
release automatically converts to **Apache 2.0** two years after it ships, so the
code does become fully open over time.
