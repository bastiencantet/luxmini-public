# Security

LuxMini's only privileged component is **`led-helper`**, a tiny setuid-root binary whose
sole job is to write the LED brightness to the SMC via IOKit. It accepts one line-based
command on stdin and nothing else. Its full source is in
[`src/bin/led-helper.rs`](src/bin/led-helper.rs) — short on purpose, so you can audit it.

The main app runs **unprivileged**. There is no network server, no telemetry. Update checks
and (planned) device-profile fetches are the only outbound calls, and are visible in the source.

Found something? Open a private security advisory on GitHub, or email the address on
https://luxmini.bastiencantet.com.
