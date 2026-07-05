<div align="center">

# LuxMini

**Turn off, dim, or schedule the front LED on your Mac.**
Source-available (Fair Source) · lightweight menu-bar app · set-and-forget.

[Install](#install) · [How it works](#how-it-works) · [Why is it unsigned?](#why-is-it-unsigned) · [Donate](#support)

</div>

---

The Mac mini's (and Studio's) front power LED is **bright**, always on, and macOS gives
you **no way to turn it off** — people resort to a piece of tape. LuxMini fixes that in
software: switch it **off**, **dim** it, or have it **auto-dim at night / after sunset**.

> **Is this even real?** Yes. macOS doesn't expose the front LED, but it's driven by the
> Mac's **SMC** (System Management Controller) — the same chip that runs the fans and
> sensors. LuxMini writes to it through a tiny privileged helper. The source is right here
> so you can check for yourself.

## Features

- **Off / On** and **brightness** for the front LED.
- **Auto-dim** *(in progress)* — lower it at night or after sunset, set-and-forget.
- **Multi-model** *(in progress)* — Mac mini (Intel, T2, Apple Silicon) and Studio via
  per-model device profiles.
- Menu-bar app, discreet, launches at login, auto-updates.
- **Fun stuff** *(optional, tucked away)* — blink / pulse / SOS. Most people don't want
  these on a status light; they live in their own submenu.

## Install

LuxMini is **not notarized yet** (that needs a paid Apple Developer account — see
[below](#why-is-it-unsigned) and [Support](#support)). Pick whichever you trust most:

**Build it yourself** *(most trustworthy — you compile the exact source):*
```sh
git clone https://github.com/bastiencantet/luxmini-public
cd mac-led-tray
make run          # builds + runs; first launch asks once to install the helper
```
Requires Rust (`rustup`) and Xcode command-line tools.

**Homebrew** *(coming):*
```sh
brew install bastiencantet/tap/luxmini    # build-from-source formula (no Gatekeeper prompt)
```

**Download the .app/.dmg** from [Releases](https://github.com/bastiencantet/luxmini-public/releases):
because it's unsigned, first launch needs one manual step — see below.

## Why is it unsigned?

Apple's notarization requires a **$99/year Developer Program** membership I don't have yet.
That's the *only* reason for the "unidentified developer" warning — **not** anything shady:
the full source is in this repo, and you can build it yourself.

To run an unsigned app the first time:
- **Right-click** the app → **Open** → **Open** (only needed once), or
- in Terminal: `xattr -d com.apple.quarantine /Applications/LuxMini.app`

Every release ships with **SHA-256 checksums**. Want notarized builds? See [Support](#support) —
it's the first thing community funding will pay for.

## How it works

The front LED is a PWM channel on the Mac's **SMC**. LuxMini:
1. runs a small **setuid-root helper** (`led-helper`) that talks to `AppleSMC` via IOKit —
   the only privileged part, kept minimal and auditable;
2. loads a **device profile** that says *how* to address the LED on your specific Mac model
   (the addressing differs across Intel / T2 / Apple Silicon / Studio);
3. writes the brightness; the app itself stays unprivileged.

The per-model addressing — *which* SMC channel each model uses — lives in a **device profile**
loaded at runtime rather than hardcoded, so the app stays clean and supports a new model by
shipping a profile instead of a new build.

## Privacy

No tracking, no analytics. The only network calls are: checking for updates, and (planned)
fetching your Mac's device profile once, then cached locally. Everything is in the source.

## Support

LuxMini is free and its source is open to read, build, and audit. If it saved you a piece of tape:
- ⭐ Star the repo
- ☕ [Sponsor / donate](https://github.com/sponsors/bastiencantet) — **first goal: the $99
  Apple Developer account so builds get notarized** (no more Gatekeeper warning for everyone).

## Contributing

Issues and PRs welcome — especially **device profiles for more Mac models** (if your Mac's
LED isn't supported, open an issue with your model identifier).

## License

[FSL-1.1-Apache-2.0](LICENSE) © Bastien Cantet — **Functional Source License**
([fair.io](https://fair.io)): the source is open to read, build, modify, and use
for any purpose **except** building a competing product or reselling it. Each
release automatically converts to **Apache 2.0** two years after it ships, so the
code does become fully open over time.

---

<sub>Parts of this README were drafted with AI assistance and reviewed by the author.</sub>
