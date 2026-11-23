# LuxMini 0.5.4

## Changes

- Fix Settings losing keyboard focus when reopened, while preserving the selected section.
- Choose whether idle brightness follows Mac sleep or display sleep. The beta stays off by default; existing settings retain Mac sleep. At 0%, the light turns off.
- Preserve the idle level across overlapping display and system sleep; restore current automation or previous manual brightness and effects when the selected event ends.
- Offer launch at login after setup. New installations see the option selected; existing installations can opt in. Remember the choice and respect subsequent changes.

## Verification

Both client mirrors pass their Rust tests and strict Clippy checks. Native AppKit fixtures verify exclusive radio buttons, saved preferences, display/system notifications, brightness restoration, and repeated Settings reopening using disposable preferences and a memory-only output. The public client passes 27 isolated diagnostics process scenarios and the Field Test suite.

The release is universal for Apple silicon and Intel, signed with Developer ID, notarized by Apple, and signed for Sparkle. Artifact and publication verification is recorded separately during release.

Physical LED behavior during real Mac/display sleep still requires field validation. The feature remains a beta disabled by default.
