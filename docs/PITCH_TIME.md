# Streaming pitch and time processing (0.43)

## Runnable comparison and workflow

`cargo run -p soniclaw-pitch-time -- NEW_DIRECTORY` synthesizes original harmonic/attack material, exports the source, independent-phase and phase-locked pitch WAVs, and stereo time-stretched WAVs at 0.75x/1.5x duration. It writes settings, measured target pitch, transient diagnostics, exact frame counts, block-partition comparison and waveform statistics. Pitch files are latency-trimmed after explicitly flushing zeros. The A/B files support listening review; mathematical checks are not a perceptual quality score.

The new APIs are `soniclaw::dsp::streaming::{PitchShifter,PitchSettings,TimeStretcher,StretchSettings}`. They are opt-in: existing SpectralEffect/project PitchShift nodes and offline pitch/time helpers retain their original behavior. Both new implementations allocate all plans, rings, overlap buffers and scratch before processing.

## Fixed-I/O pitch shifter

Construct `PitchShifter::new(semitones,settings)`, call AudioProcessor `prepare`, then process interleaved mono/stereo audio as usual. It uses a causal normalized STFT with quarter-window hop. Destination magnitudes/frequencies are interpolated from source bins; phase locking ties bins to nearby spectral peaks, preserving within-channel partial phase relationships. Positive spectral flux can reset synthesis phase on attacks. Initial priming also initializes phase but does not count as a transient. `PitchStats::transient_channel_resets` counts channel reset operations, not unique musical onsets; sensitivity can trigger several resets on a complex attack.

Default settings: schema 1, 1024-frame window, phase locking on, transient threshold 0.5 and fully wet mix. Disable phase locking and transient resets for an independent-phase comparison. Ratios cover -12 to +12 semitones. `set_semitones` changes at the next spectral frame; abrupt changes are not smoothed and are not a substitute for product-specific automation ramps.

Reported latency is exactly the window size; the dry path uses the same delay. Flush at least that many silent frames before trimming a finite render. `reset` clears histories/overlaps/voices of this effect, leaving the preparation/settings; call at an owned engine boundary when seeking/restarting. Invalid/unprepared blocks clear output, increment invalid-block diagnostics and leave the valid processing history untouched. Effects ignore MIDI events. Phase handling is per channel: this is not a fully linked stereo phase-vocoder image-preservation algorithm.

## Variable-I/O time stretcher

`TimeStretcher` implements prepared waveform-similarity overlap-add (WSOLA). Its fixed duration ratio is output frames / input frames, from 0.5 to 2.0, with 50% overlap. A normalized waveform search aligns each grain around its expected source position; the expected clock advances independently of the selected offset, avoiding accumulated tempo drift. One shared stereo candidate preserves interchannel alignment. Similarity uses every fourth frame to bound CPU work; difficult high-frequency/polyphonic material can still produce artifacts. Unity ratio copies source frames exactly.

```rust
use soniclaw::dsp::streaming::{TimeStretcher, StretchSettings};
let mut stretch = TimeStretcher::new(StretchSettings {
    channels: 1, ratio: 1.5, ..Default::default()
}).unwrap();
let input = [0.1; 1024];
assert_eq!(stretch.push(&input).unwrap(), 1024);
stretch.finish();
let mut block = [0.0; 1024];
let mut total = 0;
while !stretch.is_drained() {
    total += stretch.pull(&mut block, 8).unwrap();
}
assert_eq!(total, 1536);
```

For a live/long stream, repeatedly push bounded input and pull output. `push` returns accepted *frames*, which can be fewer than supplied when the input ring is full; retain/resubmit the remainder after pulling. `pull` clears output and returns valid produced frames; zero means more input, pending compute budget or final drainage, distinguished by your input state and `is_drained`. Supply a grain budget from 0–64 per pull; zero drains only already prepared pending output. Do not treat cleared, unproduced output as valid stream audio.

After the last accepted input, call `finish` once (repeated calls are harmless) and pull until `is_drained`. Final windows use explicit zero padding, and total output is exactly `ceil(total_input_frames * ratio)`. Further push calls fail until reset. Empty streams drain immediately. `reset` reuses buffers and clears counters/finish state. Invalid shapes/nonfinite samples/budgets fail without advancing the stream; invalid pull output is cleared. Input/output counters, free ring space and transient-lock count are observable.

Energy-ratio transient locking can choose the predicted grain directly and bypass a long crossfade on a large attack. This can retain attack sharpness but can also click, duplicate or move transients; its threshold is configurable/disableable. WSOLA has no universal transient or formant quality guarantee. `priming_frames` reports window plus search lookahead; the first grain can be available after one window. This is buffered source lookahead, not a fixed AudioProcessor latency or automatically inserted silence. Variable-duration processing needs application transport/FIFO scheduling and cannot be dropped into a fixed-I/O effect chain without an adapter.

## Ownership, persistence, bounds and recovery

Prepare/create, JSON serialization, rebuilds and destruction occur off the audio callback. The processing owner has exclusive mutable access. Push/pull/pitch processing do no allocation, locking, I/O or FFT planning. Bounded work does not guarantee a deadline on every CPU: correlation cost grows with window/search/channels and grain budget. Profile with the intended hardware/buffer size, apply backpressure, and move heavy rendering to a worker when needed.

`PitchSettings` and `StretchSettings` serialize as strict schema-1 values; validate settings before replacing an instance. Persist them in a custom node/application payload alongside pitch amount. Raw overlap/FIFO/phase history is not serialized. No existing project schema or native plug-in ABI changes are introduced. Old projects retain legacy processing; choose/register the new processors explicitly. Time-stretch ratio is immutable for a stream; drain/reset and construct a new configuration to change it. Realtime ratio automation is not implemented.

Bounds: pitch 8–192 kHz, mono/stereo, blocks <=8192 and power-of-two windows 256–4096; mix 0–1 and transient threshold 0.05–4 or disabled. WSOLA mono/stereo, power-of-two window 256–4096, search <= one quarter window, blocks 1–8192, duration ratio 0.5–2, energy ratio 1.1–100 or disabled. Ring capacity is four windows plus maximum block; total input <=2^48 frames. Samples must be finite and magnitude <=1,000,000. Extreme/tiny files may be dominated by padding; musical quality needs longer material.

## Validation and scope

Tests measure octave-shift pitch, zero callback allocations, bit-identical output across unequal input/output partitions, unity wet reconstruction/dry latency, attack reset diagnostics and silent invalid-call handling. WSOLA tests cover ratios 0.5/0.75/1/1.5/2, exact duration, preserved test-tone pitch, linked stereo, unity identity, no processing allocations, backpressure, finish/drain, invalid-call non-mutation, empty streams and reset. The runnable demo additionally checks a +7-semitone harmonic signal and attack material. See the release verification report for executed measurements and full workspace regression results.

This release does not add formant preservation, commercial vocal stretching, transient editing, arbitrary live ratio/pitch automation, phase-coherent multichannel pitch shifting or a fixed-rate time-stretch host adapter. Listening acceptance, physical-device/DAW and Windows/macOS runtime tests remain separate work.
