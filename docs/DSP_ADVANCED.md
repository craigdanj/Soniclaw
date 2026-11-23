# DSP expansion — v0.5.0

The eight modules live under `soniclaw::dsp::advanced`. Rendering uses prepared
storage. Constructors, FFT/mipmap generation and `prepare` may allocate. Source
algorithms and their tested limits are described below; tests are evidence for
specified behavior, not acoustic or standards certification.

Most new audio processors support 8–192 kHz and 1–64 channels, with a preparation
budget guard of `sample_rate * channels <= 8,000,000`. The existing core and older
processors retain their own contracts. Analysis APIs have their own explicit
format/range bounds; notably `LoudnessMeter` is mono/stereo 48 kHz.

## Automatable effects and projects

```rust
use soniclaw::{prelude::*, dsp::{AutomatedEffect, EffectKind}, plugin::PluginInstance};
let effect = AutomatedEffect::new(EffectKind::Ladder, "filter")?;
let controls = effect.parameters().to_vec();
// Pass controls to PluginInstance, or bind their stable numeric IDs to Engine.
// Then push the effect into the engine before prepare.
```

Sixteen additional `EffectKind` values are available: `StateVariable`, `Ladder`,
`CrossoverLow`, `CrossoverMid`, `CrossoverHigh`, `RmsCompressor`, `LookaheadLimiter`,
`TransientShaper`, `SpectralGate`, `FdnReverb`, `Phaser`, `Flanger`, `Tremolo`,
`AutoPan`, `MultiTapDelay` and `StereoWidth`. Together with the ten older kinds,
there are 26 effect kinds using the common named-control API. Each new kind is
also a `ProcessorKind` in schema-2 projects. Control IDs are `node_id.control_key`;
inspect `parameters()` for names, units, ranges and defaults.

Scalar controls use the existing per-frame 10 ms smoothing. Discrete SVF mode
changes are rounded from the smoothed control, not filter-output crossfades.
FFT-domain changes take effect at analysis-hop boundaries. Keep topology,
lookahead duration, FFT size, tap lists and wavetable-bank creation off the audio
thread. New project nodes reconstruct while deactivated, preserving parameter
identities just as in v0.4. See [project state](PROJECTS.md).

## 1. Filters

`filters::StateVariableFilter` implements a topology-preserving state-variable
filter with low-pass, band-pass, high-pass and notch modes. `set(frequency, q,
drive, mode)` changes controls without allocating or resetting integrator state.
Resonance is expressed as Q. The live wrapper exposes frequency/Q/drive/mode.

`filters::LadderFilter` implements four cascaded TPT one-poles with an analytical
solution to the linear resonance feedback loop. Drive saturates its input. It
is not an iterative nonlinear ladder/circuit model. Resonance is bounded below
self-oscillation. Both filters clamp cutoff below their processing Nyquist.
Input saturation is not oversampled, so drive can introduce aliasing.

## 2. Crossover filters

`crossover::LinkwitzRiley4::new(rate, hz)` produces a mono two-way LR4 kernel;
`split(x)` returns `[low, high]`. Each branch is -6 dB at the crossover and the
sum is flat in magnitude. It has all-pass phase, so recombination is not a
sample-identical copy of dry input.

`ThreeWayCrossover::new(rate, low_hz, high_hz)` returns low/mid/high. The low
branch receives the upper crossover's all-pass alignment. Use one kernel per
channel. `CrossoverBand` and the three corresponding effect kinds expose each
of those phase-aligned three-way bands as an in-place processor. When automating
parallel branches, update their crossover settings together; the framework does
not automatically link three independently created parameter sets.

These are reusable splitters, not a supplied multiband compressor/GUI. Chain an
existing compressor or other processor per band using the graph. Coefficient
updates retain state; static reconstruction tests do not establish perfect
magnitude reconstruction during arbitrary cutoff modulation.

## 3. Dynamics

`dynamics::RmsCompressor` uses a linked-channel exponentially weighted mean-square
detector, adjustable RMS window, attack/release, threshold/ratio and makeup gain.
Its steady-state ratio is tested separately from envelope timing.

`TransientShaper` compares fast and slow linked amplitude envelopes. Attack and
sustain controls boost/cut their corresponding transient/decay regions. Both
controls at zero provide unity gain.

`LookaheadLimiter` uses linked 4x FIR peak estimates and a bounded monotonic peak
queue. Set lookahead from 0–20 ms while stopped, then prepare. Declared latency
is `ceil(lookahead_ms * rate / 1000) + 12` frames, including FIR observation
headroom. The default at 48 kHz is **252 frames**. Ceiling and release are live
controls; the common wrapper retains its 10 ms scalar ramp.

The limiter applies a 0.3 dB guard margin below the selected ceiling. The checked
phase-offset high-frequency tone and demo output remain below their selected
ceiling under the supplied estimate. This is not a universal true-peak guarantee
for arbitrary signals, aggressive gain modulation, resampling or downstream
encoding. A full independent compliance-vector suite has not been run.

## 4. Spectral toolkit

`spectral::FftProcessor` owns forward/inverse plans and scratch. Sizes are powers
of two from 16–32768. Inverse scales by `1/N`. `window` supplies periodic Hann,
Hamming and Blackman coefficients. `SpectrumAnalyzer` returns a coherent-gain
normalized one-sided magnitude spectrum; DC/Nyquist are not doubled.

`Stft::new(size, hop)` uses sqrt-Hann analysis/synthesis, rolling input, weighted
overlap-add and prepared FFT storage. Hop must divide N and be at most N/2.
`tick(sample, callback)` lets a caller modify complex bins. Preserve conjugate
symmetry for real output and obey the callback's allocation/thread contract.
Streaming latency is **N samples**. Initial latency outputs are silent; flush
zero input to recover the tail. Identity reconstruction and normalization are
tested at multiple hops.

`SpectralGate` is a 1024-point, 256-hop example. It suppresses bins below a
threshold and aligns its dry/wet paths to 1024-frame latency. Bin thresholding
is a simple creative gate; musical-noise artifacts are possible. Pitch shifting,
time stretching and spectral freeze are not implemented by this foundation.

## 5. Wavetable synthesis

`wavetable::WavetableBank` builds harmonic mipmaps off-thread using FFT filtering
of 1–16 periodic source frames. Frames must have equal power-of-two lengths,
64–4096 samples. DC is removed. `basic(2048)` supplies sine/saw/square morph frames.
`WavetableOscillator` selects a safe harmonic level for static pitch and provides
frame morphing, phase-difference PWM/pulse width and hard sync.

`WavetableSynth<N>` adds fixed note voices, 1–8 unison oscillators per voice,
detune/stereo spread, ADSR, sustain, MIDI pitch/pressure, explicit MPE zones and
note-ID expression. Its named morph/PWM/width/detune/sync/gain controls can bind
to an engine/plug-in. Project kind `Wavetable` uses a 2048-sample basic bank,
16 voices and four unison oscillators; arbitrary custom bank contents are not
serialized by that built-in project node.

Static-wave mipmaps are band-limited. Rapid morph/PWM, hard sync and changing
mipmap levels are not alias-free or guaranteed click-free. Sync frequencies
are bounded below Nyquist. Oldest-voice stealing has no retiring crossfade.

## 6. Richer reverberation

`reverb::FdnReverb` uses eight damped delay lines with an orthogonal Householder
feedback mix. Controls include RT60, wet mix, damping frequency, predelay,
input diffusion and fractional-delay modulation. Early reflections and two
input all-pass diffusers complement the late network. Predelay is 0–200 ms;
decay is 0.1–15 seconds. Input/output remain in place, with no artificial
latency declaration for musical predelay or reverberation tails.

The late field is shared stereo; additional channels contribute their own early
reflections and receive alternating late outputs. It is not a multichannel room
or binaural renderer. RT60 is a delay-feedback target; damping, diffusion and
modulation affect the measured decay. Reset clears delay/filter history.

## 7. Creative effects

`creative::Phaser` uses six modulated all-pass stages with feedback. `Flanger`
uses a fractional short delay, LFO and signed feedback. `AmplitudeModulator`
provides tremolo/autopan plus optional beat sync through `set_tempo_sync`.
Stereo autopan uses constant-power gains; mono uses amplitude modulation.

`MultiTapDelay` provides up to eight signed-gain/panned taps, bounded feedback,
tempo-following beat times and stereo ping-pong feedback. It reserves four
seconds per channel in prepare. Requested tap times outside the available
range clamp to that storage; changing tempo does not allocate a larger buffer.
Feedback comes from the first tap, keeping loop gain independent of the sum of
output tap gains. Taps and beat sync configuration are stopped-time operations;
beat length, feedback and mix are live controls. Tempo comes from ProcessContext,
including through the common effect wrapper.

## 8. Analysis and stereo utilities

`analysis::PitchDetector` supplies monophonic YIN estimates and confidence for a
configured window/range. Silence and weak/no threshold matches return None.
Supported windows are 64–8192, with at least two periods at the configured low
frequency. Storage is fixed; CPU work is proportional to window times maximum
lag. Run at a suitable analysis cadence/on a worker, not blindly every sample.
It is not a polyphonic note/chord detector.

`TruePeakDetector` uses the published BS.1770 four-phase FIR coefficients and
also includes raw sample peaks. Feed complete frames, then 12 zero frames at EOF
to include its filter tail. Read cumulative linear or dBTP peak; reset starts a
new measurement. It is a finite-oversampling estimate, not an exact continuous
waveform maximum.

`LoudnessMeter` supports **48 kHz mono/stereo** K-weighting, 400 ms momentary,
3-second short-term and integrated loudness with -70 LUFS absolute/-10 LU
relative gating and 75% block overlap. Other sample rates must be converted
first. The caller chooses exact gate-history capacity; when full, integrated
history freezes and `history_full` reports it while current windows continue.
Incomplete windows return None. Integrated-history scans should run outside
the tight callback budget. No surround/LFE mapping, LRA or independent EBU
certification is supplied.

`StereoMeter` measures RMS and correlation; silence gives correlation zero.
`encode_mid_side`/`decode_mid_side` are inverse transforms. `stereo_width` and
the automatable `StereoWidth` preserve the mid/mono sum; widened peaks can
need more headroom. Width processing affects the first stereo pair.

## Examples and verification

```sh
cargo run -p soniclaw-dsp-lab -- /new/output/directory
cargo build -p soniclaw-dsp-lab
# Linux; substitute the DLL/dylib path on the corresponding target
cargo run -p soniclaw-clap-smoke -- target/debug/libsoniclaw_dsp_lab.so --dsp
```

The walkthrough writes audio for all eight areas, crossover stems, FFT CSV,
analysis JSON and project documents. The plug-in library exports separate DSP
FX and Wavetable Lab products, using new product IDs. Older products keep their
identities. These are demonstrations, not a graphical editor for every module.
See [current validation](validation/v050/REPORT.md) and [release notes](releases/v0.5.0.md).

## Algorithm references

Implementations are original Rust code; numerical algorithms are grounded in:

- [Robert Bristow-Johnson / W3C Audio EQ Cookbook](https://www.w3.org/TR/audio-eq-cookbook/)
  for biquad design.
- [Vadim Zavalishin, The Art of VA Filter Design](https://www.native-instruments.com/fileadmin/ni_media/downloads/pdf/VAFilterDesign_2.0.0a.pdf)
  for TPT SVF/ladder structures and the analytical feedback formulation.
- [ITU-R BS.1770-5](https://www.itu.int/rec/R-REC-BS.1770/en)
  for K-weighting, gating and the four-phase peak-filter coefficients.
- [Julius O. Smith, FDN reverberation](https://ccrma.stanford.edu/~jos/pasp/FDN_Reverberation.html)
  for feedback-network background. These references do not confer certification.
