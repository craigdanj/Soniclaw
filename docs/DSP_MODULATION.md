# Compiled modulation and efficient controls — 0.92.0

## Application workflow

Use `soniclaw_dsp::modulation::compiled` as an owned modulation renderer in an instrument, effect or per-voice engine. Describe sources, destinations and routes with stable string IDs on a control thread, compile once, cache indices, then render into prepared normalized destination buffers. The callback neither searches strings nor changes global `Parameter` atomics. Existing global `ModulationMatrix` and note-local `VoiceModulationConfig` remain compatible and unchanged.

Built-in sources are constants and sine/triangle/saw/square/sample-hold LFOs. External scalar/audio buffers allow existing ADSRs, voice pressure, CCs and custom generators to use the same routing engine. Each voice can own a compiled instance with different external inputs. This release does not replace existing MIDI/envelope generators or automatically connect every processor; applications bind the generated buffers to their DSP controls.

Run `cargo run -p soniclaw-dsp-modulation --offline -- NEW_DIRECTORY`. It drives the new nonlinear filter from compiled cutoff modulation, writes audio, compares audio/control-rate evaluation counts and measures callback allocations. A second fixture feeds an external envelope buffer. The report's operation counts are deterministic; elapsed timings on a shared machine are not CPU speedup guarantees.

## API and timing

`CompiledMatrix::new(config, sample_rate, maximum_frames)` validates schema/IDs, resolves routes to numeric source/destination indices and groups them by destination. Cache `source_index`/`destination_index` results before playback. `render(frames, tempo, playing, inputs)` fills `buffer(destination_index)`, which borrows the last block of normalized samples. Consume those samples before rendering the next block. No extra output-copy buffer is required.

`Rate::Audio` evaluates sources/routes every sample. `Rate::Control { quantum }` evaluates them at stream-anchored intervals (1–1024 frames) and holds the target between updates. Destination smoothing still runs each sample. Quanta continue across callback boundaries, so callback size does not reset modulation timing. LFO phase integrates the supplied block tempo; playing=false freezes phase while base/external control updates can continue. Reset starts phase, deterministic sample-hold seeds and counters afresh.

Every external source must appear exactly once per render as `Input { source: cached_index, values: &[...] }`. A scalar slice has length one; an audio slice has exactly `frames` entries. Inputs are finite in −1–1. Duplicate, unknown, non-external and missing bindings are rejected before histories advance. Audio mode is required for sample-accurate external changes; control mode quantizes their observation to its update boundaries.

Routes apply signed depth −2–2 and Direct, BipolarToUnipolar or SignedSquare transforms, sum into a normalized base, then clamp targets to 0–1. Multiple routes may intentionally accumulate. Destination smoothing keeps its accumulator in f64 so long/high-rate transitions do not stall at f32 resolution. It uses a prepared one-pole coefficient for 0–10000 ms. `set_base(index, value)` validates 0–1 without changing the modulation offset; its effect is observed at the next update boundary. It runs on the audio owner or via an application command queue. It updates the stored target configuration for subsequent persistence. `set_constant` and `set_lfo_rate` provide checked source updates without changing topology; changing an LFO rate preserves phase.

## Lifecycle, resources and persistence

The processor is single-owner and independent of UI/device/Studio threads. Construction allocates configuration, route arrays and output storage. `render`, set_base, buffer and reset allocate nothing. The application remains responsible for thread-safe delivery of commands and external sources. LFO/table evaluation is bounded by the chosen rate, while phase integration and destination smoothing still have per-sample cost. Stats report frames and source/route evaluations, rather than claim that every operation becomes quantum-times cheaper.

Schema-1 serde `Config` persists stable IDs, routes, base values, timing mode and smoothing. Recreate on reopen; phases, held random values and buffers are not preset state. Limits are 64 sources, 256 destinations, 2048 routes, 8192 maximum frames and 1,048,576 destination-buffer samples. Rate is 8 kHz–1.536 MHz. Duplicate/invalid IDs, unresolved routes, nonfinite values, oversized work and invalid tempo are rejected. A failed render leaves timing/state and the previous completed block intact; callers must handle its error rather than play stale modulation accidentally.

## Validation

Executed checks and examples are recorded under `docs/validation/v0_92_0/`. Tests cover exact callback partition equivalence across control quanta, base preservation, malformed binding transactions, reset, normalized bounds, operation-count reductions and zero allocations. The integrated filter example demonstrates using the output in DSP independently of Studio. No generic third-party plug-in automation mapping, GPU processing, universal CPU improvement or full shared/polyphonic synthesizer integration is claimed.

LFO shapes here are control/modulation sources; saw/square are not advertised as band-limited synthesis oscillators. The stronger-oscillator enhancement remains a separate pending item.
