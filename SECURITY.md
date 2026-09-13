# Security policy

LuxMini uses a small setuid-root helper for the AppleSMC connection. The helper
source is in `src/bin/led-helper.rs` and is kept separate from UI, networking,
updates, telemetry, and scheduling code.

The helper starts as a child of LuxMini and remains alive only while the app is
running. It is not a `LaunchDaemon`, has no socket or network access, waits on an
anonymous stdin pipe, and exits when its private stdin pipe closes.

LuxMini's optional local control API is disabled by default and binds only to
`127.0.0.1`. Profile fetches, optional anonymous aggregate telemetry, and
Sparkle update checks are outbound HTTPS requests. See the README for the exact
data involved.

## Supported versions

Security fixes are applied to the latest published LuxMini version.

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub Security Advisories:

https://github.com/bastiencantet/luxmini-public/security/advisories/new

Please include the LuxMini version, macOS version, Mac model identifier, impact,
and reproduction steps when possible. Do not include a serial number or other
device identifier.

Please do not open a public issue for an unpatched vulnerability.
