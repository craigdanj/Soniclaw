# Prepared direct VST3 processing

Version 0.24.0. Enable `soniclaw/vst3-hosting`. This adds an opt-in direct audio path beside the 0.23 buffered adapter. The first direct path targets trusted f32 stereo effects; it does not turn the backend's existing realtime wrapper into a hard-realtime engine.

## Workflow and runnable example

In the workbench VST3 tab, inspect a trusted effect, select Direct audio mode and start audio. Stop direct audio before opening an editor. Leave direct mode off for the existing MIDI/editor workflow.

```sh
cargo build -p soniclaw-sdk-tools --features vst3-tools
python3 tools/build-formats.py --fetch
# Select a NEW output directory:
target/debug/soniclaw-sdk-tools --vst3-direct-demo \
  build/formats/Release/SoniclawReference.vst3 /tmp/vst3-direct
```

The native demo selects Gain + Filter, sets gain-only mode and gain 0.4, checks callback lengths 1, 7, 64, 255, 257 and 512, changes gain while running, instruments allocations and destroys the owner before calling the retained processor again.

## Public API

```rust,ignore
let mut owner=unsafe { soniclaw::vst3::Instance::load(path,Some(class_id),spec)? };
let mut callback=owner.start_direct()?;
callback.prepare(spec)?;
// Give callback to your device engine. On the control thread:
owner.set_parameter(gain_id,0.8)?;
// Stop/revoke before native state/editor work:
owner.stop()?;
let state=owner.save_state()?;
```

`start_direct` returns `direct::DirectProcessor`, implementing `AudioProcessor`. Preparation must match the owner's sample rate/stereo layout and not exceed its prepared frame maximum. `latency_frames` reports native latency without a worker bridge. `direct_errors()` counts rejected/native-failed blocks while this run remains attached. `reset()` revokes the run; stop/recreate to resume. The existing buffered `start_buffered` stays available.

## Callback and native ABI ownership

A separate prepared adapter leases the native `IAudioProcessor` interface while the main-thread `Instance` retains the module/component. It uses fixed planar buffers and fixed COM parameter-queue pools. It bypasses the backend's processing queues, GUI-edit drain and level-meter mutex. There are no framework allocations, blocking locks, module operations or logging in the normal direct callback path. The callback takes an atomic try-guard; contention produces silence rather than waiting.

The host does not guarantee that arbitrary native plug-in code or its calls into host services are allocation-free or bounded. Test actual products. This first path keeps editors closed and limits `poll()` to draining restart flags while attached, and revokes the run if a native restart is requested. Plug-ins requiring run-loop servicing during playback should use buffered mode. MIDI events are explicitly rejected with silence in this initial direct path; the existing buffered path remains available for instruments. Native output event/parameter capture is not connected to this first direct adapter.

The native owner is `!Send` and stays on its GUI/control thread. Starting activates native processing there. The callback is the only thread invoking the direct processor. Stopping sets revocation first, waits off the callback for an in-flight call, releases the native processor lease on the owner thread and stops/deactivates the backend. A retained callback stays silent after owner destruction. Never call native processor methods outside these ownership rules or keep a third-party raw lease beyond the owner.

## Parameters, persistence and migration

At most 64 queued control commands are consumed per callback. Repeated values for an ID coalesce to its last point at frame zero; queue saturation is an explicit error. ID/read-only/range checks happen on the control thread. Accepted changes mirror into the native edit controller there, while native processor changes arrive through the direct input queues. Controller display can lead the next callback by one block, as normal for queued UI changes.

No state schema change is required. Existing VST3 schema-1 state and buffered workflows remain readable. Stop direct processing before save/restore or opening an editor; no native pointer or prepared buffer is serialized. The workbench direct checkbox is a runtime setting, not a conversion of existing CLAP/VST3 state formats.

## Bounds, errors and recovery

The existing owner permits 1–8192 prepared frames, one stereo output and zero/one stereo input. Direct mode additionally requires native f32 processing. Invalid channels, oversized blocks, nonfinite/out-of-range tempo, MIDI input, contention, a revoked lease or native processing failure produce silence. Error counters expose rejected/native failures; the contention/revocation branch need not count each silent callback. Stop/reconstruct after reset or a native failure. A hang inside trusted native code cannot be forcibly recovered in this in-process path.

## Validation

The native demo measures zero callback allocations across variable lengths and a live normalized gain change, reports zero worker bridge frames and verifies owner-first revocation. Unit tests verify bounded parameter-pool reuse and allocation-free revoked callbacks. See the [release report](validation/v024/REPORT.md) for executed checks. Native timing is not physical round-trip latency, a universal realtime guarantee or a JUCE benchmark. Windows/macOS and external DAW/hardware acceptance remain pending.
