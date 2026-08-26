# Changelog

All notable changes to LuxMini are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.2] - 2026-08-26

### Added

- Safe hardware-validation onboarding for pending Mac models. LuxMini explains
  the test, fades the front LED, restores full brightness, and asks the user to
  confirm what they saw.
- Recognition and isolated candidate profiles for the 2026 Mac mini identifiers
  (`Mac18,5` M6 and `Mac17,16` M5 Pro). A successful visual test enables the
  exact profile locally and sends only the model and Yes/No outcome; it never
  promotes support globally without owner review.

## [0.3.1] - 2026-08-20

### Added
- **Automatic device-profile delivery**: LuxMini fetches only the profile matching
  the current Mac from the profiles API, validates it, and caches it locally.
- **Local control API** *(opt-in, localhost-only)*: a tiny dependency-free HTTP
  server so local tools — Home Assistant, Apple Shortcuts, AppleScript, shell
  scripts — can read and drive the LED with no cloud or account. `GET /led`,
  `POST /led` (`on` / `brightness` / `effect`), `GET /healthz`. Off by default;
  enable via the `api.enabled` preference, with an optional bearer token
  (`api.token`). Binds `127.0.0.1` only.

### Fixed
- Disabling the local API now releases the listener immediately instead of
  requiring an app restart. Requests have bounded I/O, header and body handling,
  and the CLI now reports non-success HTTP responses as errors.

## [0.3.0]

### Added
- **Auto-dim**: automatically turn the front LED off or dim it at sunset or at a
  time you set — location-based, set-and-forget.
- **Settings window** with **French / English** localization, an option to hide
  the menu-bar icon, and auto-dim location controls.
- **Mac Studio support** (M1 through M4) alongside the Mac mini.
- **Runtime device profiles**: the per-model SMC addressing is loaded at runtime,
  so a new Mac model can be supported by shipping a profile rather than a new
  build.
- `led-helper`: `READ` / `LIST` commands and verification of the SMC result byte.
- Project hygiene for going public: GitHub Actions CI (fmt, clippy `-D warnings`,
  test, release build, MSRV 1.82 job, `cargo-deny`), `rust-toolchain.toml`,
  `deny.toml`, Dependabot, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, and crate
  metadata (`license`, `repository`, `rust-version`).

### Changed
- Released the source under the **Functional Source License** (`FSL-1.1-Apache-2.0`):
  source-available to read, build, modify, and use for any non-competing purpose;
  each release converts to Apache 2.0 two years after it ships.
- Effects (blink / pulse / SOS / strobe) moved into their own **“Fun stuff”**
  submenu to keep the main menu focused.
- Hardened the codebase for the public release: enforced panic-freedom via
  clippy (`unwrap_used`, `expect_used`, `panic`, `unreachable`, `indexing_slicing`
  denied), rewrote the last raw indexing/slicing sites in `led-helper` as bounded
  iterator forms, and externalized the per-model SMC key out of the source.

## [0.2.2]

### Added
- Menu-bar app to turn **off**, **dim** (brightness slider), or run effects on
  the Mac's front LED, via a minimal setuid-root IOKit helper.
- Effects submenu (blink / pulse / SOS / strobe), launch-at-login, and Sparkle
  auto-updates.

[Unreleased]: https://github.com/bastiencantet/luxmini-public/compare/v0.3.2...HEAD
[0.3.2]: https://github.com/bastiencantet/luxmini-public/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/bastiencantet/luxmini-public/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/bastiencantet/luxmini-public/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/bastiencantet/luxmini-public/releases/tag/v0.2.2
