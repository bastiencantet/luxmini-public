# Security policy

LuxMini uses a small setuid-root helper for the AppleSMC connection. The helper
source is in `src/bin/led-helper.rs` and is kept separate from UI, networking,
updates, telemetry, and scheduling code.

The helper starts as a child of LuxMini and remains alive only while the app is
running. It is not a `LaunchDaemon`, has no socket or network access, waits on an
anonymous stdin pipe, and exits when that pipe closes. Its protocol supports
only ping, two-byte LED reads, and two-byte LED writes. It cannot enumerate SMC
keys and rejects malformed key names and payload sizes.

LuxMini's optional local control API is disabled by default and binds only to
`127.0.0.1`. Profile fetches, optional anonymous aggregate telemetry, and
Sparkle update checks are outbound HTTPS requests. See the README for the exact
data involved.

Device profiles are bound to the exact Mac model and cached with mode `0600`.
Profiles are not yet cryptographically signed. Signing them at the API and
verifying that signature inside the privileged helper is the remaining boundary
hardening work.

## Supported versions

Security fixes are applied to the latest published LuxMini version.

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub Security Advisories:

https://github.com/bastiencantet/luxmini-public/security/advisories/new

Please include the LuxMini version, macOS version, Mac model identifier, impact,
and reproduction steps when possible. Do not include a serial number or other
device identifier.

Please do not open a public issue for an unpatched vulnerability.
