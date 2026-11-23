# Trusted CLAP hosting — v0.7

`soniclaw-host` loads native CLAP code inside the application. Enable facade
`features=["hosting"]` for `soniclaw::host`. `soniclaw-clap` instead exports your
processors to other hosts.

Discover explicitly selected `.clap` files/bundles without execution, then use
unsafe `ClapLibrary::load` only for trusted ABI-conforming code. Clone a loaded
handle for multiple instances. Library/instance owners are deliberately !Send.
Instantiate, inspect and activate on main; `run_audio` lends a processing endpoint
to a separate scoped audio OS thread. Run the complete block loop there, join,
service main callbacks, deactivate, save/load state and destroy on main. Buffers
return before teardown; the endpoint cannot outlive its owner. It implements
`AudioProcessor` for borrowed-buffer composition within the scope.

The scope synchronously blocks main until its loop ends. This is a rendering/
testing foundation, with main callbacks serviced between scopes, not a persistent
CPAL/GUI live-host adapter. Hosted editors, timers, thread pools and extended host
services are absent. Rust cannot contain foreign access violations, exceptions or
termination; no sandbox/crash isolation or universal third-party compatibility is
claimed. Panicked scopes require deactivation/reactivation. Native code must
honor pointer lifetimes and all CLAP contracts.

Supported subset:

- One f32 mono/stereo output; zero/one mono/stereo input; ≤8192 frames. Extra buses,
  sidechains, float64 and MIDI-only products without audio output are rejected.
- ≤4096 parameter metadata records and ≤1024 combined incoming events per callback.
  Automation validates IDs/ranges/order before mutation. Parameter values are
  snapshots refreshed on inspection/activation/state loading.
- Native note IDs/expressions if advertised; negotiated MIDI-1 fallback otherwise,
  losing IDs and limiting expressions. All-notes-off is wildcard choke or CC123 on
  all 16 channels. Bounded output parameter/gesture, note/expression and channel
  MIDI events; SysEx/UMP are not retained.
- Tempo/playing/sample-derived beats and seconds; no loop/time-signature automation.
- State streams ≤4 MiB with partial reads/writes. Deactivate first. Failed restore
  rollback belongs to the foreign plug-in.
- Thread-check and parameter/state/latency/restart/process/callback notifications.
  Stop/reconfigure before honoring a restart. Output conversion honors constant
  masks and sanitizes non-finite samples.

The Workbench Host tab scans trusted selected libraries, inspects controls,
renders a two-second preview, saves/restores state and writes `hosted-preview.wav`.
It opens no audio device and can pause control while the scope runs. SDK tools
reproduce it. Four Workbench products and a separate MIDI-only fixture are checked
through actual shared libraries. The fixture enforces distinct main/audio OS
threads for lifecycle/state/destruction, exact timed gain, MIDI fallback, event
echo and bounded input. Heap probing covers the host wrapper only.

See [SDK commands](SDK_070.md), [verification](validation/v070/REPORT.md).
Primary contracts: [lifecycle](https://github.com/free-audio/clap/blob/main/include/clap/plugin.h),
[threads](https://github.com/free-audio/clap/blob/main/include/clap/ext/thread-check.h),
[parameters](https://github.com/free-audio/clap/blob/main/include/clap/ext/params.h),
[state](https://github.com/free-audio/clap/blob/main/include/clap/ext/state.h).
