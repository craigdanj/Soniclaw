# VST3 musical integration

The buffered VST3 owner now accepts the framework's prepared tempo map, meter,
loop and punch ranges. It translates expressive notes to native VST3 events and
preserves sample offsets for normalized parameter changes and output events.
This is the buffered instrument path; direct audio mode retains its documented
v0.24 audio/control contract.

## Workflow and API

Inspect a trusted native bundle, select a class ID, and load `vst3::Instance` on
the application's main thread. Configure while stopped, then call
`start_buffered_musical(Config { timeline, recording })`. Move the returned
`Processor` to the audio callback and retain/poll the owner on the main thread.
Workbench's VST3 panel has a “Use project tempo map, meter and loop range”
option. Mixed sessions may save an optional `musical` configuration on each
`Source::Vst3` node; omitted fields preserve the old constant-tempo path.

```rust,no_run
use soniclaw_vst3::{Instance, musical::{Config, ParameterPoint}};
use soniclaw_core::{AudioBuffer, AudioProcessor, ProcessSpec, ProcessContext, Transport};
let spec = ProcessSpec { sample_rate:48000., max_block_frames:256, channels:2 };
// SAFETY: application has explicitly selected trusted ABI-conforming native code.
let mut owner = unsafe { Instance::load(std::path::Path::new("Trusted.vst3"), None, spec) }?;
let mut processor = owner.start_buffered_musical(Config::default())?;
let mut pcm = [0.;512];
let context = ProcessContext {events:&[], transport:Transport::default()};
processor.process_automated(&mut AudioBuffer::new(&mut pcm,2)?, &context,
    &[ParameterPoint {frame:64,id:9,value:0.5}])?; // use a writable ID from owner.parameters()
// Production callbacks run on their own thread; call owner.poll() on main regularly.
owner.stop()?;
# Ok::<(),Box<dyn std::error::Error+Send+Sync>>(())
```

`take_native_input()` transfers one bounded producer to a MIDI/control thread.
`try_sysex(offset, bytes)` accepts a complete F0...F7 message with 7-bit interior
bytes; `try_program_change(offset, channel, program)` uses zero-based channels.
Offsets address the next worker packet (256 frames), not an arbitrary wall clock.
The producer returns `InputError::{Invalid,Full,Stopped}` without allocating.
Retry only while the run is active. `drain_timed_output()` runs on the control
thread and returns owned native events with absolute source-stream timestamps;
`event.sample_offset` remains relative to its original 256-frame packet. Drain
regularly, including after stopping. A new run replaces the output queue.

## Timing, note expression and limits

The callback prepares a clock once and stores metadata with each sample in the
fixed packet. A host-source discontinuity seeks the clock; continuous callbacks
advance it, including wrap within a packet. The worker splits native processing
at tempo/meter/bar/punch/loop boundaries. Ramps can require one-sample spans.
This preserves musical context at additional worker CPU cost. The 1024-frame
bridge and native latency are reported; this is not a latency reduction.

`NoteOnId` preserves explicit IDs. Ordinary notes get stable host IDs. Up to128
live identities are tracked. Wildcard note targets fan out to matching voices.
Tuning maps semitones -120...120 to VST3's normalized 0...1; volume 0...4 maps to
0...1 (unity is .25), pan and brightness are normalized. Pressure uses native
poly pressure. Choke maps to note-off because generic VST3 note events do not
provide CLAP's separate choke semantics. Legacy note-off release velocity is
preserved. MIDI CC, pitch bend, channel pressure and program changes require the
plug-in's native MIDI mappings; unsupported mappings need not affect its DSP.

A callback accepts up to1024 sorted automation points, each finite0...1 and
addressing a writable stable parameter ID. A packet holds128 points and128 core
events. SysEx messages are at most1024 bytes; native ingress128 commands;
output256 events. On audio/automation overflow the run fails silent and must be
recreated. Input queue saturation is reported to the producer. Output queue
saturation increments processing errors and silences that packet. Poll stats
and rebuild after stopping; these bounded queues are not lossless unlimited
recorders. Unknown/read-only automation IDs are rejected before enqueue.

## Ownership, persistence and recovery

Main owns native lifecycle/editor/controller services. The native worker owns
processing, event conversion and any native allocations/locks. The callback
uses prepared memory and bounded SPSC operations only. Stop/drop revokes the
processor and joins the worker before unloading native code. A retained callback
stays silent. Reset cancels the run; create a fresh processor for playback.

`Config` serializes its versioned `Timeline` and recording-arm state. Clock
position, held notes, pending events and DSP history are runtime state and are
not saved. Existing mixed-session schema1 files omit `musical` and still load.
Stop before changing timeline, restoring a native state or reconfiguring buses.
Host restart/layout/latency requests require the existing owner recovery policy.

## Validation and scope

Run `cargo test -p soniclaw-vst3 -p soniclaw --all-features`. Build and execute
an independent native fixture (Linux; pinned SDK is fetched only with `--fetch`):

```sh
python3 tools/build-formats.py --fetch --no-editor --protocol-fixture
cargo run -p soniclaw-musical-host -- build/formats/fixtures/HostProtocolFixture.vst3 new-protocol-report
cargo run -p soniclaw-sdk-tools --features vst3-tools -- --vst3-demo build/formats/Release/SoniclawReference.vst3 new-vst3-report
```

The report directory must not already exist. The protocol executable checks
actual native context snapshots and gain automation, then emits `report.json`. Unit checks cover
loop/punch/meter translation, expression laws, bounded SysEx input with zero
allocations, callback partitioning and revoked-owner silence. Native fixture
checks and their exact results are recorded separately; no physical interface,
third-party DAW or Windows/macOS runtime result is implied. The generic offline
renderer retains its constant-tempo API. The new musical path is buffered/live
and mixed-session hosting.
