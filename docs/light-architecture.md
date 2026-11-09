# Shared light architecture

Product behavior is independent of the physical light. Front LEDs, rear Apple
logos and previews share one normalized brightness contract.

## Boundaries

- `src/light.rs`: manual intent, confirmed state, on/off, presets, scheduling
  transitions, effect cancellation, errors and output lifecycle.
- `src/effects.rs` and `src/schedule.rs`: shared effect frames and timing/rules.
- `src/output/mod.rs`: `LightOutput`, initial value, normalized writes,
  connection status, shutdown, device metadata and driver factories.
- `src/output/smc.rs`: privileged helper IPC, SMC profile verification and one
  bounded unattended reconnection attempt. Normal shutdown leaves the selected
  front-LED value in place and closes the helper.
- `src/output/logo.rs`: CoreBrightness property conversion, worker isolation,
  rollback and restoration of the original logo settings.
- Memory driver: normalized writes to an atomic byte, used with explicit
  session-only persistence. It never probes hardware or starts app services.
- `src/ui/device.rs`: localized device wording and artwork selection. Views read
  metadata from the controller; they never rediscover or control hardware.

`LightState` owns one `Box<dyn LightOutput>`, so mutually inconsistent active
SMC, logo and simulation fields cannot coexist. Transport reconnection returns
an outcome to the shared controller, which stops any interrupted effect and
keeps manual intent consistent. A healthy connection does not erase an earlier
rejected write. Clearing a displayed error does not make the output ready.

## Adding a feature

Implement the command once through `LightState` and the shared effect/schedule
paths. Add it to the two-device controller contract tests. Keep byte layout,
physical units and restoration policy in a driver. Device-specific text and
artwork belong in the presentation layer. Driver-specific setup remains a
composition concern, selected through `DeviceInfo::setup_method`.

The contract suite covers both device kinds with the same command traces:
manual brightness, off/on, presets, auto-dim and manual overrides, every effect's
frame/cancellation path, rejected writes, connection recovery, and shutdown.
Driver tests separately cover SMC transport behavior and CoreBrightness mapping
rollback/restoration. These tests do not substitute for physical validation.

## API compatibility

`GET` and `POST /light` are aliases of the existing `/led` routes. Responses
include `ready`, `device`, `experimental` and `simulated`. `helper_ready` remains
a compatibility alias for `ready`; it no longer names the internal controller
state. Existing clients and scripts keep working without a migration.

## Preview

Run `scripts/build-light-simulators.sh` to create the two local apps. The same
binary runs with `--simulate-logo` or `--simulate-led`; only the selected metadata
and presentation differ. Both route controls through the shared controller.
