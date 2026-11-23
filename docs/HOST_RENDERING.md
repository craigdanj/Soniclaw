# Continuous offline rendering and WAV stems

Version 0.26.0. `soniclaw::rendering` is available without native-host features for ordinary processors. Enable `vst3-hosting` for the VST3 adapter; `hosting` supplies the CLAP adapter. These APIs keep one processing stream alive across a render instead of restarting a hosted instrument for each block.

## Workflow and runnable examples

```sh
cargo build -p soniclaw-sdk-tools -p soniclaw-host-fixture --features soniclaw-sdk-tools/vst3-tools
python3 tools/build-formats.py --fetch
# Each output directory must be new:
target/debug/soniclaw-sdk-tools --offline-host-demo \
  build/formats/Release/SoniclawReference.vst3 /tmp/offline-vst3
target/debug/soniclaw-sdk-tools --offline-clap-demo \
  target/debug/libsoniclaw_host_fixture.so /tmp/offline-clap
```

The VST3 demo renders a native drum voice at block sizes 64 and 256, compares the complete PCM, checks energy after the nominal clip boundary and writes a tail-inclusive WAV. It also exports two native processor stems. The independent CLAP fixture verifies continuous processing on a separate audio thread, arbitrary final-block size and a zero-input tail.

## Generic renderer API

`rendering::Plan` schema 1 specifies sample rate, channels, main frames, tail frames, block size, tempo and whether to compensate declared latency. `render(processor,plan,input,events)` returns a `Rendered` buffer plus frame count, actual processing-block count and trimmed latency. `Rendered::write_wav` writes a WAV using the existing file API.

```rust,ignore
let plan=soniclaw::rendering::Plan{
    frames:48_000,tail_frames:96_000,block_frames:256,
    ..Default::default()
};
let result=soniclaw::rendering::render(&mut processor,&plan,&[],&midi)?;
result.write_wav(output_path)?;
```

Empty input is an instrument/silent-input render; otherwise input contains exactly `frames * channels` interleaved samples. MIDI `Event.frame` values are absolute offsets in the main region, sorted according to the core event contract. Events are rebased for each processing block. The main stream is followed by zero input for the requested tail. There is no automatic tail-length estimate or implicit note-off; supply note-off/panic events as appropriate for the instrument.

With latency compensation enabled, the renderer processes extra trailing frames and trims the processor's declared latency from the front, producing exactly main+tail frames. It does not alter the input event timeline. All stems can therefore share an aligned output duration. A buffered asynchronous host is not suitable for rendering as fast as possible: use a continuous native offline lease, not a worker bridge whose deadlines depend on wall time.

## Continuous native VST3

`Instance::with_offline(callback)` requires live direct/buffered processing to be stopped. It closes the editor, selects native offline mode, activates/starts on the GUI/control thread, creates one scoped processing worker and stops/deactivates after the callback returns. The owner services native run-loop work on its thread; structural restart notifications request cancellation rather than reconfiguring buffers during the render. Native failure is returned even if a caller ignored an `AudioProcessor::process` failure inside the callback.

```rust,ignore
let result=owner.with_offline(|audio|{
    soniclaw::rendering::render(audio,&plan,&input,&events)
        .map_err(|e|e.to_string().into())
})?;
```

`OfflineAudio` implements `AudioProcessor`, accepts changing callback lengths up to the prepared maximum and preserves native voices/filter histories across calls. `process_block(data,events,changes,position,bpm)` additionally supports sorted per-block normalized parameter changes with explicit quarter-note position. Parameter metadata is cached before processing. This is an allocating/locking offline API, not a device-callback adapter. Core MIDI 1 messages and AllNotesOff are supported; unsupported expression messages return errors. Native GUI-domain controller updates are deferred to the control thread by the backend.

The earlier `process_offline` isolated-block convenience method remains unchanged. Choose `with_offline` when continuity matters. Restoring opaque state restores the plug-in's documented saved state, not a guarantee of reproducing transient voices, round-robin counters or nondeterministic vendor DSP.

## Continuous native CLAP

Use the existing `ClapInstance::activate` and `run_audio` scope. Query latency on the owner thread after activation, then create `HostedAudio::processor(latency)` inside the audio scope. It returns a borrowed `ScopedProcessor` which implements the generic processor contract without taking module ownership. Check `processor.error()` after rendering because the generic trait reports processing errors through silence/diagnostics rather than a return value. The lease cannot escape its scope. Deactivate/drop the instance on the owner thread afterward.

This retains CLAP's existing main/audio-thread lifecycle and supports native parameter/state APIs independently; the simple generic renderer does not introduce a new automation document. For explicit per-block CLAP automation, the underlying scoped `process_block`/`process_musical` APIs remain available.

## Stem export

`export_stems(&mut [Stem],plan,new_directory)` renders each named processor independently with its own input/events. Names are ASCII alphanumeric, period, underscore or hyphen, up to 64 characters, and unique ignoring ASCII case. There are 1–128 stems. It writes WAV files plus `stems.json` containing the plan and each stem's compensated latency/frame count.

Files are staged in a uniquely named sibling directory and the complete directory is renamed only after every stem succeeds. Failure removes the staging directory; an existing destination is rejected. DSP state already consumed by a failed export is not rolled back. Reconstruct/reload processors before retrying. Each supplied processor must be fresh/preparable and appropriate for offline execution.

## Ownership, bounds and cancellation

Render on a worker/control thread. The generic renderer allocates buffers, prepares once and calls process continuously; it does not reset between blocks. `render_cancellable` accepts an `AtomicBool` checked at block boundaries. Cancellation returns `RenderError::Cancelled`, with no partially returned PCM. A native call which itself hangs cannot be interrupted by that flag.

Plans support core sample-rate/channel bounds, block sizes 1–8192 and tempo 1–1000 BPM. Main+tail+latency working output is capped at 64 Mi samples, declared latency at 1,048,576 frames and total MIDI events at 1,048,576. At most 1024 events may fall in a processing block. Invalid input sizes, MIDI order/density, nonfinite output, processor preparation errors, cancellation and file errors are explicit failures. WAV sample rates are rounded to integer Hz; choose integer rates when the file header must match exactly.

## Persistence and migration

Render plans and stem manifests use schema 1. Existing project, CLAP-session and VST3-state schemas are unchanged. The render plan stores rendering choices, not processor ownership or delay/voice history. Stems are audio artifacts, not a project/session backup. Existing WAV APIs and in-memory/offline engine rendering remain available.

## Validation and limits

Tests cover continuous state, tail duration, block-partition invariance, cancellation and invalid input. Native VST3 and CLAP demonstrations exercise actual binaries; native drum PCM is compared across block sizes and its tail is measured. See the [release report](validation/v026/REPORT.md). The renderer currently produces WAV stems, uses a constant-tempo generic context and does not render a whole mixed asynchronous graph faster than realtime. Physical hardware, external DAW and other-platform runtime acceptance remain pending.
