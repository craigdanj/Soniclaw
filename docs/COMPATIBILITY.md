# Compatibility and acceptance work

## Automated checks included

- Portable behavior tests, preset/session validation and callback allocation checks.
- Independent `tools/clap-smoke` host for the original gain/synth plug-ins.
- `tools/compat/host.cpp`: exported CLAP lifecycle, bus topology, actual sidechain ducking/release, in-place buffers, constant-channel expansion, analytic gain, timed automation, partial-stream state restoration, malformed state rejection, variable/zero-size blocks, sample-rate reactivation and thread-local glibc heap instrumentation.
- Optional real X11 host-parented editor attachment, XTest knob drags, balanced gestures under a one-event output budget, state restoration while an edit is pending, resize, scaling, hide/show and teardown checks. GUI verification runs separately from the primary performance results.
- Pinned upstream CLAP validator and Steinberg VST3 validator reports where actually run.

```sh
cargo test --workspace --all-features --locked
cargo build --release -p soniclaw-reference
python3 tools/run-compat.py --library target/release/libsoniclaw_reference.so --clap-sdk vendor/formats/clap
# On an X11 display:
python3 tools/run-compat.py --library target/release/libsoniclaw_reference.so --clap-sdk vendor/formats/clap --gui --output benchmarks/results/gui
```

X11 interaction tests require the XTest runtime (`libxtst6` on Debian/Ubuntu). The glibc allocation probe intercepts heap allocation/free operations on the rendering thread, and deliberately allocates in a self-test before trusting zero counts. It does not count stack memory. Other operating systems use the portable Rust allocation tests until native exported-ABI instrumentation is added.

## Target-machine matrix (not yet completed)

| Target | Manual/physical acceptance checks |
|---|---|
| Linux DAWs | Scan CLAP and VST3, instantiate each product, play/record automation, save/reopen a project, route the sidechain, reopen/resize the editor, render and compare audio |
| Windows | Repeat in representative DAWs; test Win32 focus/DPI, ASIO/WASAPI availability through selected backends, device changes and installer uninstall |
| macOS | Build AUv2/VST3/CLAP, run `auval -v aufx SnGF Snra`, scan in Logic/other hosts, verify Cocoa parenting/DPI and signing/notarization |
| Real interfaces | Duplex loopback measurement, requested buffer changes, output/channel maps, unplug/replug while monitoring and recording, device-name persistence, clock drift |
| Storage failures | Real disk-full/write-protection during recording and native recovery; verify failed takes are reported and not resumed silently |

This release includes tools and reproducible fixtures, not a declaration that these manual or hardware tests passed. There is no physical audio interface in the Linux validation environment. CI workflows become evidence only after their remote jobs run successfully.
