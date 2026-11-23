# Reusable stereo mixer (0.37)

## Workflow and example

`cargo run -p soniclaw-mixer-studio -- NEW_DIRECTORY` renders a two-second stereo mix plus its post-fader send stem and persists mixer targets/report. Add `--gui` for reusable channel-strip and group controls; change gain/pan/mute/solo/sends then render again. No audio device is opened; this small demo renders synchronously on its UI thread. Production apps should render on a worker.

Create `soniclaw::mixer::Model` with stable strip IDs, labels, groups and pre/post-fader `Tap`s. Validate and `compile(rate,max_frames)` off the callback. Borrow the prepared `Mixer` on the processing thread and share cloned `Arc<Controls>`/`Arc<GroupControls>` with the UI. `ui::mixer_controls` presents the same handles. Inputs are interleaved stereo slices in declaration order; `process(inputs,main,sends,changes)` clears and writes the main bus and one output per configured send. Sends are separate outputs: route them through prepared effects and return buses explicitly to avoid a feedback cycle.

## Signal and automation contract

Each strip applies constant-power stereo balance (centre -3 dB on each side), a common mute/solo gate, gain times optional group gain, and independent send levels. Pre-fader sends omit channel/group gain but respect pan and mute/solo; post-fader sends include gain. Any soloed strip suppresses non-soloed strips; mute and group mute still win. Solo is global to this mixer, including a muted solo strip. Gain, pan, send and gate changes use a sample-rate-derived 5 ms exponential smoothing coefficient. This is smoothing, not a fixed-length ramp; state persists across uneven block partitions.

`Change {frame,strip,value}` applies up to 1024 sorted sample-offset automation points per call. All points are validated before control targets or DSP state mutate. Automation updates shared targets as well as the local callback snapshot. Multiple UI target writes are individually atomic, not a multi-field transaction; publish whole prepared mixers at engine boundaries for structural changes. A UI write racing a scheduled automation point has last-writer semantics.

`Controls::take_peaks` atomically consumes accumulated post-fader absolute L/R maxima. No callback locks, allocation, formatting or I/O occur. GUI meters are independent of the audio buffers. Strip controls can outlive a mixer but then change only detached targets. Mixer preparation/destruction, JSON and snapshots belong on the control thread.

## Persistence, bounds and integration

Schema 1 `Model::to_json/from_json` validates stable IDs and values, rejects oversized input and unknown fields. `Mixer::snapshot` captures current targets, excluding meter/smoothing history. This is additive to existing project formats: apps persist it in their own project payload and reconnect source processors by stable strip ID. Keep a previous valid model on errors; changing strip/group/send topology requires recompile.

Bounds: 1–64 stereo strips, 16 gain/mute groups, 4 sends, 8–384 kHz, 1–8192 maximum frames, linear gain/send 0–4, pan -1–1, JSON 128 KiB. Finite input samples must have magnitude <= 1,000,000, bounding intermediate arithmetic. Invalid shapes/automation/input clear every provided output and return an error without applying automation. Zero-frame blocks accept no events.

Route mono/multichannel sources through explicit adapters before mixing. Existing `soniclaw::bus` graph delay compensation belongs upstream; this mixer adds no delay and does not discover processor latency. It supplies no automatic insert loading, feedback routing, master limiting, dB-law faders, loudness metering or native DAW mixer automation. Summed signals can exceed unity; add headroom/limiting before hardware/file conversion. Pre/post sends and solo policies are deliberate and should remain stable for a product.

## Validation

Tests check constant-power balance, send gain, atomic peak consumption, zero callback allocations, sample-offset automation partition invariance, rejected-automation rollback/silence, group mute and schema roundtrip. The runnable demo exports actual mix and send WAV files from two sources.
