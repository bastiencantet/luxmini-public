# 2026 Mac mini profile validation

## Known hardware

- Apple announcement: August 25, 2026.
- Customer availability: September 22, 2026.
- `Mac18,5`: Mac mini with M6.
- `Mac17,16`: Mac mini with M5 Pro.

The identifiers come from post-announcement device catalogs and must be
confirmed with `sysctl -n hw.model` on shipping hardware.

## Current safety state

- Both identifiers are recognized by the client as pending hardware.
- They are deliberately excluded from the supported-model list.
- Candidate profiles exist only under `unverified` in the private,
  git-ignored `profiles.local.json`.
- The candidates inherit the M4 and M4 Pro Mac mini profile respectively.
- The normal production profile endpoint returns `404`; the isolated candidate
  endpoint serves them only to the explicit visual onboarding.
- A `Yes` answer caches and enables the profile only on that exact Mac. Reports
  contain only the model and `yes`, `no`, or `technical_error`; they never
  promote a database profile automatically.

## Physical validation checklist

1. Confirm the exact identifier with `sysctl -n hw.model`.
2. Record the chip name and model identifier without collecting a serial
   number or another device identifier.
3. Start LuxMini and accept the clearly labelled LED test. Confirm that the LED
   fades smoothly down and up and is restored to full brightness.
4. Answer `Yes` only if the front status LED visibly changed as described.
5. After local validation, additionally test brightness 0, 1, 128, and 255 and
   confirm that only the front status LED changes and that each value is stable.
6. Repeat on both M6 and M5 Pro hardware; do not infer one result from the
   other.
7. Move only the validated entry from `unverified` to `profiles` in
   `profiles.local.json`.
8. Seed the profiles database idempotently and verify that the profile API
   returns the exact requested model.
9. Move the validated identifier from `PENDING_EXACT` to `SUPPORTED_EXACT`,
   run the full test suite, sync the auditable public client, and publish a new
   signed release.

Never print, commit, or publish an SMC key while performing this checklist.
