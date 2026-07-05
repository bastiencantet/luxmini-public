# Contributing to LuxMini

Thanks for helping make the Mac front-LED controllable in software. Issues and
PRs are welcome — the single most valuable contribution is **device profiles for
more Mac models** (read on).

## Build & run

```sh
git clone https://github.com/bastiencantet/luxmini-public
cd mac-led-tray
make run        # builds both binaries and launches the tray app
```

Requirements: a stable Rust toolchain (via [`rustup`](https://rustup.rs) — the
pinned channel is in `rust-toolchain.toml`) and the Xcode command-line tools.
On first launch the app asks **once** for admin rights to install the setuid
`led-helper`; that is the only privileged step.

## Adding support for a Mac model (device profiles)

LuxMini deliberately keeps the per-model LED addressing out of the source tree:
it is **data**, loaded at runtime from a profile file, not hardcoded. See
[`profile.example`](profile.example) for the `KEY FORMAT MAX` format.

If your Mac's LED isn't supported, **open an issue** with your model identifier
(`sysctl hw.model`) rather than a PR — coordinating keeps the device-profile data
consistent. Please don't commit a real profile to the repo; `/profile` and
`profile.local` are gitignored for exactly this reason.

## Quality bar

Every PR must be green on CI, which runs on macOS:

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo build --release
```

A few standing rules the lint config enforces (`[lints.clippy]` in `Cargo.toml`):

- **No panics in the running app.** `unwrap`, `expect`, `panic!`, `unreachable!`
  and raw indexing/slicing are denied — return `Result`/`Option` or use `.get()`.
  This is a menu-bar daemon; it must never take down the user's session.
- **Every `unsafe` block carries a `// SAFETY:` comment** explaining the
  invariant it upholds (most of the codebase is objc2 / IOKit FFI).
- `arithmetic_side_effects` and `as_conversions` are intentionally *not* enabled
  crate-wide; the few numeric casts are saturating and carry a scoped, justified
  `#[allow]`. Match that style rather than adding new bare `as` casts.

`cargo deny check` (licenses, advisories, bans) also runs in CI; keep the
dependency tree permissive-licensed and advisory-free.

## MSRV

The minimum supported Rust version is **1.82** (declared as `rust-version` in
`Cargo.toml` and exercised by a dedicated CI job). Bumping it is allowed when a
new language feature genuinely helps, but call it out explicitly in the PR and
the changelog.

## Commits & PRs

- Keep commits focused; a short imperative subject (`fix:`, `feat:`, `docs:` …).
- Describe *why*, not just *what*, in the PR body.
- Add or update tests for any behavior change.
- Add a `CHANGELOG.md` entry under **Unreleased** for anything user-visible.

## Security

Please report security issues privately — see [`SECURITY.md`](SECURITY.md).
