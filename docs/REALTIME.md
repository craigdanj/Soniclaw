# Real-time contract

1. Construct processors, parameters, loaded samples, graphs and queues on a control thread.
2. Negotiate actual stream rate/layout, then prepare processors and storage.
3. Give one rendering owner exclusive access to mutable DSP state.
4. Communicate using scalar atomics, timed events and bounded SPSC queues.
5. Stop before graph changes, device reconfiguration or resource destruction.

`AudioProcessor` is a contract, not a compiler proof. User processors must not allocate, lock, block, log, do I/O, call GUI functions or drop the last owner of heap resources during processing. Built-in processing is tested under an allocation counter; this is not a hard real-time certification of OS/drivers/dependencies.

## Callback bounds

Native GUI/MIDI queues consume a bounded per-block event budget. The original output host reserves a panic event and handles queue overflow with all-notes-off. The duplex input frame queue is bounded and has separate input overflow/underrun counters. Initial prefill produces silence. Drift correction consumes at most two frames per output frame with the current ±0.2% bound.

Automation splits reuse prepared scratch storage and validate before mutation. Same-frame parameter updates occur before the next render segment. Oversized lists are rejected. Suspension clears DSP state and freezes processing/transport; events during suspension are discarded, not replayed on resume.

`RecordTap` uses fixed 256-frame stereo packets and does not wake the worker through a blocking channel. The worker polls commands/queue and performs WAV I/O. Start/stop/finalization and waiting for in-flight capture happen only on the control thread. Keep recording queues reasonably sized; the default demo uses 128 packets. Full queues cause reported silent gaps in the file rather than time compression.

CLAP activation allocates audio/event buffers. Its processing callback copies host buffers, translates bounded events, applies stable staged state and runs the engine without locks. Host callbacks and custom processors must obey the documented CLAP threading contract. Necessary raw FFI is confined to the adapter/export/host boundary.

## Audio boundaries

DSP can exceed unity when voices or buses sum. Native output sanitizes non-finite audio and hard-saturates to [-1,1]. The output meter measures before saturation/mute. WAV PCM16 export saturates; the recording worker replaces non-finite samples with silence. These boundaries are not transparent limiting or dithering. Use sensible gains.

## Current signal limitations

- Voice stealing/retriggering can click; pitch bend, sustain and aftertouch are not implemented by the demo synth.
- Saw/square use PolyBLEP, but sampler transposition and the duplex bridge use linear interpolation and can alias.
- Cutoff coefficient changes are not smoothed. Delay controls are constructor settings.
- Recording supports stereo PCM16, not multi-track/disk-streamed instruments or automatic disk failure recovery.
- The native host has no ASIO integration, measured latency report, arbitrary-rate clock conversion or seamless device recovery guarantee.
- The CLAP adapter has fixed stereo f32 main ports and no embedded GUI, float64, sidechain port or per-note modulation.

## Tests

`crates/soniclaw/tests/realtime.rs` measures the prepared synth/effect chain, sampler/graph and automated processing with recording enqueue. It checks MIDI and automation output across different block partitions. Other suites cover exact event offsets, malformed input, sidechain isolation, latency alignment across block boundaries, bridge starvation/recovery and simulated clock drift, file duration under overflow, concurrent record stop/restart and UI shape generation.

The independent CLAP host and upstream validator load actual exported binaries. Driver latency, physical audio/MIDI, UI interaction and native platform DAW behavior require separate hardware sessions.

## v0.3 additions

All new DSP, multisampler and FFT paths allocate their storage before processing. The multisampler owns immutable PCM for the bank lifetime, with fixed current/retiring voice slots. StreamHandle owns its worker join and must stay on the control thread; StreamingPlayer only consumes bounded queue frames. Editor producers may lock on the GUI thread; the audio consumer never takes that lock. CLAP output gesture backpressure is retained across callbacks. New reference processing and the exported CLAP ABI are probed for heap allocation/free operations independently.

Effect configuration constructors are not an automation API: rebuild/reconfigure with exclusive ownership while stopped unless a specific processor exposes an atomic parameter. Host state currently covers parameter values, not arbitrary sample-bank/rack replacement.

The CLAP audio entry scopes FTZ/DAZ on x86-64 and FZ on AArch64, restoring the host control word before returning (including caught panics). The x86-64 behavior and restoration are tested; AArch64 source is supplied but not executed on this machine. Portable DSP called outside the CLAP adapter does not implicitly change the floating-point environment.

The upstream validator's denormal test includes random subnormal input generation outside the callback. If it reports a timing warning, use `tools/compat/denormal-probe.cpp` to isolate callback CPU: it constructs input before timing, checks that gain flushes subnormal samples, alternates normal/denormal batches and verifies the host control bits afterward. The report preserves upstream warnings rather than excluding that test.
