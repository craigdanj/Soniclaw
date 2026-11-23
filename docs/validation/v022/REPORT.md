# Soniclaw 0.22.0 validation

Executed 2026-11-17 on Linux x86_64 with Rust 1.100.0, native X11 under Xvfb and Mesa software GL.

- Formatting, all-target/all-feature Clippy with warnings denied, all-feature workspace tests (198 passed including doctests), documentation with warnings denied and native example builds: PASS. Exact commands, exits and timings: [checks](final-checks/checks.json); full logs alongside.
- Independent fixture embedded editor: PASS. Resize/reopen remained 800×600; 341 timer callbacks and one readable FD callback were delivered. [Report](native-final/fixture-editor-demo/report.json).
- Actual workbench CLAP editor: PASS. Resize/scale to 125%, close/reopen remained 1000×750. Captured software-rendered window was visually inspected. [Report](native-final/workbench-editor-demo/report.json), [image](native-final/workbench-editor-demo/embedded.png).
- Host ABI contract, allocation-instrumented direct callback/owner revocation and session state/bypass regression demos: PASS. [Commands](regressions/checks.json).

A first native attempt exposed compounded scaling across reopen. The retained scale is now initialized before the first GUI frame; this final evidence was generated after the fix and subsequent checks. Earlier attempts are not release acceptance.

These are deterministic fixture and framework-product checks. No physical audio/MIDI device, external DAW, Wayland-native parent, Windows or macOS runtime acceptance was performed. The X11 screenshot validates rendering, not every interaction, accessibility or arbitrary third-party plug-in compatibility. Native plug-ins execute trusted code in process.
