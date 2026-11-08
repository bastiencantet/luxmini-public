# Consent regression checks

The consent feature is local preparation for a future release. These checks do not deploy the API, publish a client, approve profiles, or initialize the LED helper.

## Reproduce

```sh
cargo build --locked
python3 scripts/test-diagnostics-consent.py --binary target/debug/mac-led-tray
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --features field-test
python3 scripts/test-diagnostics-consent.py --field-test --binary target/debug/mac-led-tray
cargo test --locked --features field-test
```

If `CARGO_TARGET_DIR` is set, pass the corresponding binary path. Run on macOS with an available graphical session: the process harness uses real AppKit and NSUserDefaults.

The process suite creates disposable application bundles and preference domains under `com.bastiencantet.luxmini.metrics-test.*`. It only accepts an exact matching bundle identifier and `http://127.0.0.1:<port>/api/v1`. Production destinations and preference domains are rejected before any fixture action. A release build receiving the test configuration exits before device discovery. The debug-only harness is excluded from release builds.

A local HTTP server controls successful, failed, malformed and delayed responses. In-flight tests wait for an observed request before withdrawing consent. No physical hardware is needed. Fixture identifiers are held in memory, never printed. Test domains and bundles are removed on normal exit and assertion failure.

## Automated scenarios

The twenty process scenarios check:

- No decision means no identity or statistics requests.
- Refusal is remembered within a process and after restart.
- New and existing installations have distinct first events.
- IDs are canonical random UUID v4 values; payload keys follow a strict allowlist.
- Old aggregate consent and unrecognized consent versions do not authorize linked statistics.
- Same-day reports are coalesced, including after restart and after clock rollback.
- Upgrades and downgrades preserve the previous version.
- Token failures, malformed token JSON and telemetry failures do not acknowledge local activity; retry can succeed.
- Withdrawal during token retrieval cancels the activity POST.
- Withdrawal during a POST orders erasure after that POST and cannot recreate local state.
- Reacceptance creates a fresh identity, including while an old request is in flight.
- Failed erasure survives restart and retries while collection remains disabled. Erasure retries are idempotent on the API.
- Corrupt JSON preferences fail closed.
- A consented identity persists across process restart.
- The test harness rejects production destinations and production preference domains.

A separate process check exercises the field-test binary: even explicit acceptance through the fixture produces no prompt, identity or request.

Four added unit tests check the complete modal response acceptance policy, all combinations of the prompt policy, clock rollback and erasure payload minimization. Existing candidate/profile and client tests remain in the regular suite. Hardware-dependent tests remain deliberately ignored.

## Native UI review — 2026-11-08

The actual French AppKit dialog and General settings were exercised using an isolated application and loopback HTTP service:

- The first dialog appears before hardware setup.
- Return selects refusal. Escape leaves the dialog open and never accepts.
- Refusal leaves the optional-statistics switch off and sends no statistics.
- Switching on from General opens the full consent dialog again.
- Explicit Allow enables the switch and reports an existing-install baseline.
- Switching off disables collection and requests erasure.

Real screenshots of the dialog and the off/on General states accompany the local product walkthrough. Screenshots use a synthetic model, disposable preferences and no initialized hardware state; they are not evidence of physical LED validation.

## Limits

These tests exercise the client against a controlled loopback server. Server validation, database deduplication, authorization and retention have separate API contract tests; the loopback suite does not replace them. No test can guarantee absence of future regressions. Production backups, log retention and the privacy policy must still be reviewed before activating the feature remotely.

Local results on 2026-11-08: private client 99 Rust tests passed, public mirror 98 passed; two hardware tests intentionally ignored in each suite. Both clients passed all twenty process scenarios and Clippy with warnings denied. The private field-test suite passed 100 Rust tests plus its dedicated no-statistics process check. An optimized private build succeeded and its test-environment guard exited before device discovery.
