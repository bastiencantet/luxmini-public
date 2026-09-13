<div align="center">

# LuxMini

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
   control the front LED.

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
1. runs a small **setuid-root helper** (`led-helper`) that talks to `AppleSMC` via IOKit;
2. loads a **device profile** that says *how* to address the LED on your specific Mac model
   (the addressing differs across Intel / T2 / Apple Silicon / Studio);
3. writes the brightness; the app itself stays unprivileged.

The per-model addressing, meaning which SMC channel each model uses, lives in a **device profile**
loaded at runtime rather than hardcoded, so the app stays clean and supports a new model by
shipping a profile instead of a new build.

### Privileged helper security

`led-helper` starts as a child of LuxMini and remains alive only while LuxMini is
running. It is not installed as a `LaunchDaemon`, does not listen on a socket,
does not access the network, and exits when its private stdin pipe closes.

Its protocol contains only ping, LED read, and LED write operations. It cannot
enumerate SMC keys. It rejects malformed key names and accepts only the two-byte
payload used by LuxMini LED profiles. The app reads the current two bytes before
an interactive candidate test and restores those exact bytes afterward.

The current profile cache is bound to the exact Mac model and stored with mode
`0600`. It is not yet cryptographically signed. Signing profiles in the API and
verifying them inside the privileged helper is the remaining hardening step
before claiming that the helper itself enforces the server allowlist.

## Local control API

LuxMini can expose a tiny **local HTTP API** so Apple Shortcuts,
`AppleScript`, Home Assistant, and shell scripts can read and drive the LED. It is
**off by default**, **binds `127.0.0.1` only** (never the network), and there is
no cloud or account involved. Turn it on in **Settings › General → “Enable local
control API”**; optionally set a bearer token (`api.token`).

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

LuxMini has no account, advertising SDK, or cross-app tracking. The client does
not generate or send a serial number or installation identifier.

Network calls are limited to update checks, requesting profiles matching the Mac
model, and submitting a bounded candidate number with a `yes`, `no`, `selected`,
or `technical_error` result after an explicit visual test. The profile service
can aggregate requests by model, candidate number, and validation outcome. The
selected profile is cached in
`~/Library/Application Support/LuxMini/profile`.

Anonymous product diagnostics are optional and disabled by default. If enabled
in Settings, LuxMini sends only a closed event name, its outcome, the Mac model,
and the macOS major version. It does not create or send an installation ID, and
the server stores aggregate counters rather than per-device event histories.

LED control, schedules, presets, support-reminder choices, and the optional
localhost API stay on the Mac.

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
