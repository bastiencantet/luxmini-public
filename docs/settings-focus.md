# Settings reopening focus — issue #23

The first Settings open activated LuxMini after creating its window. Subsequent
opens reused the window and called only `makeKeyAndOrderFront`, leaving an
inactive menu-bar application inactive. The reported behavior is reproducible
with the retained native window: visible, but neither application-active nor key.

Both paths now share `present`: unhide and activate LuxMini, then make the
Settings window key and order it front. `MoveToActiveSpace` and
`FullScreenAuxiliary` keep this utility window on the active desktop/full-screen
space instead of leaving it behind on another desktop. The window stays at its
normal level, and its selected Settings section is preserved.

## Native regression test

```sh
cargo build --locked --bin mac-led-tray
python3 scripts/test-settings-reopen.py --binary target/debug/mac-led-tray
```

The runner copies the debug binary into a disposable, uniquely identified app.
The fixture runs before hardware, networking and login registration. It opens
Settings, closes the retained window, hides and unhides its own test application
without activating it, then reopens Settings. AppKit events are dispatched until
activation and key status settle. Three cycles must retain focus, the same
window and the selected section. The collection-behavior flags are checked too.
No setting is saved and no production preferences are changed. Release builds
reject the fixture flag before hardware discovery.

Before the fix, the first open reported `active=true visible=true key=true`;
the first reopen reported `active=false visible=true key=false same_window=true`.
After the fix, all three reopen cycles report all four values as true.

Actual Stage Manager grouping and another application's full-screen Space still
need an interactive check on the reporter's setup; the native test does not
toggle either macOS setting or put another app into full screen.
