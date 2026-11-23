# Soniclaw

Formerly Sonora. Crates/imports use `soniclaw`, and the CLI is `cargo soniclaw`. See the [rename guide](docs/RENAMING.md) for compatibility history.

**A modular Rust framework for native audio applications, instruments and effects.**

See the [release notes](docs/releases/README.md) for each major and minor release,
or the [changelog](CHANGELOG.md) for the chronological change history.

Soniclaw connects processing, devices, routing, DSP, MIDI, recording, state, UI and project tools. **v0.7.0 added the instrument/application SDK:** per-voice modulation, visual/live routing, sample maps, MIDI performance/files, scoped CLAP hosting and diagnostics. This remains pre-1.0; no JUCE parity or untested-platform claim is made.

The eight-feature desktop series is complete; see the [delivery guide](docs/DESKTOP_DELIVERY.md) for all releases, workflows and current limits.

[DSP releases 0.130–0.137](docs/DSP_130_TO_137.md) · [DSP releases 0.122–0.129](docs/DSP_122_TO_129.md) · [DSP releases 0.115–0.121](docs/DSP_115_TO_121.md) · [DSP and instrument releases 0.101–0.114](docs/DSP_101_TO_114.md) · [Eight reusable DSP additions](docs/DSP_NEXT_EIGHT.md) · [DSP roadmap](docs/DSP_ROADMAP.md)

## Added in v0.146.0

**Reusable floating-point SIMD types.** Compose DSP with safe f32 and f64 lane vectors, masks and portable tails using scalar, SSE2, AVX2 and AArch64 NEON backends.

[Feature guide](docs/DSP_SIMD_VECTORS.md) · [Release notes](docs/releases/v0.146.0.md) · [Verification](docs/validation/v0_146_0/REPORT.md)

## Earlier additions in v0.145.0

**End-to-end f64 processing.** Carry native float64 audio through the prepared engine, latency-compensated graph, opt-in Studio clip sessions, WAV I/O and double-precision CLAP/VST3 plug-in paths.

[Feature guide](docs/DSP_ENGINE_PRECISION.md) · [Release notes](docs/releases/v0.145.0.md) · [Verification](docs/validation/v0_145_0/REPORT.md)

## Earlier additions in v0.144.0

**Selectable panning laws.** Pan mono sources or balance stereo sources with seven selectable gain laws and continuous position changes.

[Feature guide](docs/DSP_STEREO_PANNER.md) · [Release notes](docs/releases/v0.144.0.md) · [Verification](docs/validation/v0_144_0/REPORT.md)

[DSP release series 0.138–0.144](docs/DSP_138_TO_144.md)

## Earlier additions in v0.143.0

**Logarithmic ramps.** Shape independent rise and fall transitions with exact endpoint arrival and continuous retargeting.

[Feature guide](docs/DSP_LOGARITHMIC_RAMP.md) · [Release notes](docs/releases/v0.143.0.md) · [Verification](docs/validation/v0_143_0/REPORT.md)

## Earlier additions in v0.142.0

**First-order TPT filters.** Process low-pass, high-pass and all-pass responses with topology-preserving state and audio-rate cutoff modulation.

[Feature guide](docs/DSP_FIRST_ORDER_TPT.md) · [Release notes](docs/releases/v0.142.0.md) · [Verification](docs/validation/v0_142_0/REPORT.md)

## Earlier additions in v0.141.0

**Borrowed audio sub-blocks.** Borrow checked frame and channel rectangles from planar or interleaved sample storage with zero copies.

[Feature guide](docs/DSP_BLOCK_VIEW.md) · [Release notes](docs/releases/v0.141.0.md) · [Verification](docs/validation/v0_141_0/REPORT.md)

## Earlier additions in v0.140.0

**Lookup-table transforms.** Prepare bounded lookup transforms for custom DSP functions, measure interpolation error, and apply them as an allocation-free waveshaper.

[Feature guide](docs/DSP_LOOKUP_TRANSFORM.md) · [Release notes](docs/releases/v0.140.0.md) · [Verification](docs/validation/v0_140_0/REPORT.md)

## Earlier additions in v0.139.0

**Shared convolution update queue.** Share a bounded background worker between reloadable convolution processors while preserving real-time crossfades and deferred kernel retirement.

[Feature guide](docs/DSP_CONVOLUTION_UPDATE_QUEUE.md) · [Release notes](docs/releases/v0.139.0.md) · [Verification](docs/validation/v0_139_0/REPORT.md)

## Earlier additions in v0.138.0

**Half-band filter design and custom oversampling.** Design measured equiripple FIR or elliptic polyphase IIR half-band filters and prepare cascades with independent up/down specifications.

[Feature guide](docs/DSP_HALF_BAND_DESIGN.md) · [Release notes](docs/releases/v0.138.0.md) · [Verification](docs/validation/v0_138_0/REPORT.md)

## Earlier additions in v0.137.0

**Audio-rate feedback patching.** Prepared sample-rate signal graphs with explicit delayed edges and zero-delay cycle rejection.

[Feature guide](docs/DSP_FEEDBACK_PATCH.md) · [Release notes](docs/releases/v0.137.0.md) · [Verification](docs/validation/v0_137_0/REPORT.md)

## Earlier additions in v0.136.0

**First-order Ambisonics.** AmbiX source encoding, soundfield rotation and a SOFA headphone decoder.

[Feature guide](docs/DSP_AMBISONICS.md) · [Release notes](docs/releases/v0.136.0.md) · [Verification](docs/validation/v0_136_0/REPORT.md)

## Earlier additions in v0.135.0

**Tempo-synced live granular effects.** Rolling history, freeze, and beat-scheduled grains on live input.

[Feature guide](docs/DSP_GRANULAR_LIVE.md) · [Release notes](docs/releases/v0.135.0.md) · [Verification](docs/validation/v0_135_0/REPORT.md)

## Earlier additions in v0.134.0

**Harmonic and percussive separation.** Complementary, stereo-linked masks for tonal and transient layers.

[Feature guide](docs/DSP_HPSS.md) · [Release notes](docs/releases/v0.134.0.md) · [Verification](docs/validation/v0_134_0/REPORT.md)

## Earlier additions in v0.133.0

**Cabinet IR alignment and blending.** Polarity-aware delay estimation and fractional alignment for convolution-ready cabinet IR blends.

[Feature guide](docs/DSP_CABINET_BLEND.md) · [Release notes](docs/releases/v0.133.0.md) · [Verification](docs/validation/v0_133_0/REPORT.md)

## Earlier additions in v0.132.0

**Live monophonic pitch correction.** Scale-aware real-time tuning with a confidence gate, retune speed and aligned bypass.

[Feature guide](docs/DSP_PITCH_CORRECTION.md) · [Release notes](docs/releases/v0.132.0.md) · [Verification](docs/validation/v0_132_0/REPORT.md)

## Earlier additions in v0.131.0

**Phase-locked multi-mic sample voices.** A shared take selection and playback clock across independently mixed microphone outputs.

[Feature guide](docs/DSP_MULTIMIC_SAMPLER.md) · [Release notes](docs/releases/v0.131.0.md) · [Verification](docs/validation/v0_131_0/REPORT.md)

## Earlier additions in v0.130.0

**NAM WaveNet playback.** Classic mono NAM WaveNet inference with measured callback cost and upstream comparison.

[Feature guide](docs/DSP_NAM_WAVENET.md) · [Release notes](docs/releases/v0.130.0.md) · [Verification](docs/validation/v0_130_0/REPORT.md)

## Earlier additions in v0.129.0

**Loudness-targeted mastering renders.** Offline LUFS analysis, true-peak-aware limiting and measured output reports.

[Feature guide](docs/DSP_MASTERING.md) · [Release notes](docs/releases/v0.129.0.md) · [Verification](docs/validation/v0_129_0/REPORT.md)

## Earlier additions in v0.128.0

**Reed and wind physical modelling.** A pressure-driven reed and bore waveguide with breath and reed controls.

[Feature guide](docs/DSP_REED_WAVEGUIDE.md) · [Release notes](docs/releases/v0.128.0.md) · [Verification](docs/validation/v0_128_0/REPORT.md)

## Earlier additions in v0.127.0

**Binaural spatial rendering.** SOFA HRTF import and prepared headphone rendering with directional interpolation.

[Feature guide](docs/DSP_BINAURAL.md) · [Release notes](docs/releases/v0.127.0.md) · [Verification](docs/validation/v0_127_0/REPORT.md)

## Earlier additions in v0.126.0

**Polyphonic pitch and onset analysis.** Offline note tracking with harmonic peak selection, onset times and confidence.

[Feature guide](docs/DSP_POLYPHONIC_ANALYSIS.md) · [Release notes](docs/releases/v0.126.0.md) · [Verification](docs/validation/v0_126_0/REPORT.md)

## Earlier additions in v0.125.0

**Two-input spectral morphing.** Prepared cross-synthesis with independently adjustable spectral envelope and fine structure.

[Feature guide](docs/DSP_SPECTRAL_MORPH.md) · [Release notes](docs/releases/v0.125.0.md) · [Verification](docs/validation/v0_125_0/REPORT.md)

## Earlier additions in v0.124.0

**Component-level circuit modelling.** Reusable wave-digital resistor and capacitor components and a nonlinear diode overdrive.

[Feature guide](docs/DSP_WAVE_DIGITAL.md) · [Release notes](docs/releases/v0.124.0.md) · [Verification](docs/validation/v0_124_0/REPORT.md)

## Earlier additions in v0.123.0

**Multi-operator FM and phase modulation.** Polyphonic MIDI voice engine with four operators, envelopes, selectable modulation routing and feedback.

[Feature guide](docs/DSP_FM_OPERATORS.md) · [Release notes](docs/releases/v0.123.0.md) · [Verification](docs/validation/v0_123_0/REPORT.md)

## Earlier additions in v0.122.0

**Impulse-response capture.** Exponential sweep generation, wave capture recovery and convolution-ready IR export.

[Feature guide](docs/DSP_IR_CAPTURE.md) · [Release notes](docs/releases/v0.122.0.md) · [Verification](docs/validation/v0_122_0/REPORT.md)

## Earlier additions in v0.121.0

**Double-precision processing.** Borrowed f64 buffers and native double-precision gain, FIR, IIR and linked dynamics processors.

[Feature guide](docs/DSP_DOUBLE_PRECISION.md) · [Release notes](docs/releases/v0.121.0.md) · [Verification](docs/validation/v0_121_0/REPORT.md)

## Earlier additions in v0.120.0

**Reusable DSP utilities.** Frame-based peak and RMS followers, more dispatched block math, and a scoped denormal guard.

[Feature guide](docs/DSP_UTILITIES.md) · [Release notes](docs/releases/v0.120.0.md) · [Verification](docs/validation/v0_120_0/REPORT.md)

## Earlier additions in v0.119.0

**Fractional-delay interpolation modes.** Integer, third-order Lagrange and first-order Thiran interpolation alongside the existing delay modes.

[Feature guide](docs/DSP_INTERPOLATION.md) · [Release notes](docs/releases/v0.119.0.md) · [Verification](docs/validation/v0_119_0/REPORT.md)

## Earlier additions in v0.118.0

**Real-input FFT and analysis windows.** Prepared real-input transforms, calibrated one-sided magnitudes and configurable analysis windows.

[Feature guide](docs/DSP_REAL_FFT.md) · [Release notes](docs/releases/v0.118.0.md) · [Verification](docs/validation/v0_118_0/REPORT.md)

## Earlier additions in v0.117.0

**Unified oversampling.** A common FIR and allpass IIR oversampling API from bypass through 16×, with quality presets and explicit latency.

[Feature guide](docs/DSP_UNIFIED_OVERSAMPLING.md) · [Release notes](docs/releases/v0.117.0.md) · [Verification](docs/validation/v0_117_0/REPORT.md)

## Earlier additions in v0.116.0

**Advanced FIR and IIR filter design.** Specification-driven Butterworth, Chebyshev and elliptic cascades, plus weighted least-squares and half-band FIR design.

[Feature guide](docs/DSP_ADVANCED_DESIGN.md) · [Release notes](docs/releases/v0.116.0.md) · [Verification](docs/validation/v0_116_0/REPORT.md)

## Earlier additions in v0.115.0

**Processor composition and latency-aware mixing.** Prepared chains, mono duplication and latency-aligned dry/wet mixing, with an automatable CLAP parallel compressor.

[Feature guide](docs/DSP_COMPOSITION.md) · [Release notes](docs/releases/v0.115.0.md) · [Verification](docs/validation/v0_115_0/REPORT.md)

## Earlier additions in v0.114.0

**Coupled physical instruments.** Velocity-sensitive hammer and pluck excitation, multiple waveguide strings, a shared modal soundboard and bounded sympathetic response.

[Feature guide](docs/DSP_COUPLED_MODELS.md) · [Release notes](docs/releases/v0.114.0.md) · [Verification](docs/validation/v0_114_0/REPORT.md)

## Earlier additions in v0.113.0

**Neural amp model playback.** A bounded mono NAM LSTM reader and prepared processor with model-rate validation, adjustable calibration and a reference comparison fixture.

[Feature guide](docs/DSP_NEURAL_AMP.md) · [Release notes](docs/releases/v0.113.0.md) · [Verification](docs/validation/v0_113_0/REPORT.md)

## Earlier additions in v0.112.0

**Streamed sampler articulations.** An SFZ subset for paged WAV instruments with per-channel keyswitches, per-zone release rules and bounded first-page preload.

[Feature guide](docs/DSP_STREAMED_ARTICULATIONS.md) · [Release notes](docs/releases/v0.112.0.md) · [Verification](docs/validation/v0_112_0/REPORT.md)

## Earlier additions in v0.111.0

**Harmonic and residual spectral resynthesis.** A prepared harmonic/residual STFT processor with frequency-guided partial tracking and separate harmonic and residual gain controls.

[Feature guide](docs/DSP_RESYNTHESIS.md) · [Release notes](docs/releases/v0.111.0.md) · [Verification](docs/validation/v0_111_0/REPORT.md)

## Earlier additions in v0.110.0

**Wavetable authoring.** Offline PCM cycle extraction, periodic resampling, seam repair, phase alignment and band-limited bank compilation.

[Feature guide](docs/DSP_WAVETABLE_AUTHOR.md) · [Release notes](docs/releases/v0.110.0.md) · [Verification](docs/validation/v0_110_0/REPORT.md)

## Earlier additions in v0.109.0

**Hybrid polyphonic instrument.** A playable wavetable and sample-layer voice engine with shared envelopes, per-voice filtering and modulation, deterministic voice stealing and preset serialization.

[Feature guide](docs/DSP_HYBRID_VOICE.md) · [Release notes](docs/releases/v0.109.0.md) · [Verification](docs/validation/v0_109_0/REPORT.md)

## Earlier additions in v0.108.0

**Audio restoration primitives.** Learnable spectral noise reduction, harmonic hum notches and isolated-click repair for offline or prepared processing workflows.

[Feature guide](docs/DSP_RESTORATION.md) · [Release notes](docs/releases/v0.108.0.md) · [Verification](docs/validation/v0_108_0/REPORT.md)

## Earlier additions in v0.107.0

**TPDF dither and noise shaping.** Deterministic per-channel TPDF and optional first-order error feedback for 16-bit and 24-bit integer PCM export.

[Feature guide](docs/DSP_DITHER.md) · [Release notes](docs/releases/v0.107.0.md) · [Verification](docs/validation/v0_107_0/REPORT.md)

## Earlier additions in v0.106.0

**General-purpose block DSP kernels.** Checked planar and interleaved buffer views, reusable channel matrix mixing with the existing SIMD backend, and streaming gain ramps.

[Feature guide](docs/DSP_BLOCK_KERNELS.md) · [Release notes](docs/releases/v0.106.0.md) · [Verification](docs/validation/v0_106_0/REPORT.md)

## Earlier additions in v0.105.0

**Linear-phase EQ and crossover.** A complementary three-band FIR crossover with exact delayed recombination and a reusable linear-phase equalizer.

[Feature guide](docs/DSP_LINEAR_PHASE.md) · [Release notes](docs/releases/v0.105.0.md) · [Verification](docs/validation/v0_105_0/REPORT.md)

## Earlier additions in v0.104.0

**Continuous-ratio streaming resampling.** A bounded push/pull multichannel converter with smooth rate changes, interpolated phase/cutoff tables and explicit end-of-stream flushing.

[Feature guide](docs/DSP_STREAM_RESAMPLE.md) · [Release notes](docs/releases/v0.104.0.md) · [Verification](docs/validation/v0_104_0/REPORT.md)

## Earlier additions in v0.103.0

**Polyphase IIR oversampling.** A prepared 2× polyphase allpass oversampler with bounded buffers, a high-rate callback and measured latency and aliasing tradeoffs.

[Feature guide](docs/DSP_IIR_OVERSAMPLING.md) · [Release notes](docs/releases/v0.103.0.md) · [Verification](docs/validation/v0_103_0/REPORT.md)

## Earlier additions in v0.102.0

**Modulatable fractional delay.** A prepared multichannel delay line with linear and cubic interpolation, per-frame slew control and independently readable taps.

[Feature guide](docs/DSP_FRACTIONAL_DELAY.md) · [Release notes](docs/releases/v0.102.0.md) · [Verification](docs/validation/v0_102_0/REPORT.md)

## Earlier additions in v0.101.0

**FIR and Butterworth filter design.** Kaiser-window FIR and Butterworth IIR designs, prepared multichannel filters, and magnitude, phase and group-delay analysis.

[Feature guide](docs/DSP_FILTER_DESIGN.md) · [Release notes](docs/releases/v0.101.0.md) · [Verification](docs/validation/v0_101_0/REPORT.md)

## Earlier additions in v0.100.0

**Prepared resampling kernels and profiling.** Immutable polyphase sinc coefficient banks with checked SIMD dot evaluation, shared-table sampler integration, bounded memory/rate tradeoffs and reproducible accuracy/allocation/timing reports.

[Feature guide](docs/DSP_OPTIMIZATION.md) · [Release notes](docs/releases/v0.100.0.md) · [Verification](docs/validation/v0_100_0/REPORT.md)

## Earlier additions in v0.99.0

**Advanced sampler region playback.** Single-voice resident sample regions with reverse and ping-pong playback, explicit wrap-overlap fades, seek/speed controls, loop release into tails and an offline bounded loop matcher.

[Feature guide](docs/DSP_PLAYBACK.md) · [Release notes](docs/releases/v0.99.0.md) · [Verification](docs/validation/v0_99_0/REPORT.md)

## Earlier additions in v0.98.0

**Flexible linked and keyed dynamics.** Linked peak/RMS compressor, downward expander and upward dynamics with soft knee, hold, external key filtering, lookahead, aligned parallel mix and audio-owner telemetry.

[Feature guide](docs/DSP_DYNAMICS.md) · [Release notes](docs/releases/v0.98.0.md) · [Verification](docs/validation/v0_98_0/REPORT.md)

## Earlier additions in v0.97.0

**True-stereo convolution and hybrid reverberation.** Four-path stereo convolution with explicit cross-channel IR routing and aligned dry blend, plus a parallel early-IR/FDN hybrid reverb with compensated timing.

[Feature guide](docs/DSP_SPATIAL.md) · [Release notes](docs/releases/v0.97.0.md) · [Verification](docs/validation/v0_97_0/REPORT.md)

## Earlier additions in v0.96.0

**Modelled pedal and amplifier signal chains.** Oversampled implicit diode/RC processing with asymmetric clipping, supply-envelope sag, tone/DC filtering, parallel mix and an optional base-rate cabinet IR.

[Feature guide](docs/DSP_AMP_MODELS.md) · [Release notes](docs/releases/v0.96.0.md) · [Verification](docs/validation/v0_96_0/REPORT.md)

## Earlier additions in v0.95.0

**Waveguide strings and modal resonators.** Prepared fractional-delay string voices with deterministic pick shaping and friction excitation, plus bounded modal resonator banks for instrument synthesis.

[Feature guide](docs/DSP_PHYSICAL.md) · [Release notes](docs/releases/v0.95.0.md) · [Verification](docs/validation/v0_95_0/REPORT.md)

## Earlier additions in v0.94.0

**Formant-aware pitch and quality stretch controls.** Optional bounded log-spectral-envelope preservation and independent formant shifting, plus explicit pitch/stretch quality presets and allocation-checked synthetic audio examples.

[Feature guide](docs/DSP_PITCH_QUALITY.md) · [Release notes](docs/releases/v0.94.0.md) · [Verification](docs/validation/v0_94_0/REPORT.md)

## Earlier additions in v0.93.0

**Oversampled modulated oscillator engines.** Adds reusable oversampled PWM, FM, phase modulation, phase warp, fractional hard sync and wavetable-morph sources with explicit generated-audio latency alignment and standalone spectral/audio examples.

[Feature guide](docs/DSP_OSCILLATORS.md) · [Release notes](docs/releases/v0.93.0.md) · [Verification](docs/validation/v0_93_0/REPORT.md)

## Earlier additions in v0.92.0

**Compiled modulation and efficient control updates.** Adds a compiled modulation renderer with stable ID binding, owned normalized buffers, audio/control-rate modes, smoothed base-preserving routes, external envelope/CC/voice inputs, live source controls and bounded evaluation counters. Also refines nonlinear-filter drive continuity.

[Feature guide](docs/DSP_MODULATION.md) · [Release notes](docs/releases/v0.92.0.md) · [Verification](docs/validation/v0_92_0/REPORT.md)

## Earlier additions in v0.91.0

**Advanced nonlinear comb and formant filters.** Adds independent nonlinear TPT state-variable filtering with bounded Newton feedback, damped fractional comb filtering and smoothed vowel/formant morphing, with high-rate preparation, checked controls and standalone audio examples.

[Feature guide](docs/DSP_FILTERS.md) · [Release notes](docs/releases/v0.91.0.md) · [Verification](docs/validation/v0_91_0/REPORT.md)

## Earlier additions in v0.90.0

**Anti-aliased nonlinear processing.** Adds reusable ADAA hard clipping, tanh and cubic saturation, stable near-equal-input handling, smoothed controls, exact integer latency/padding, oversampling composition and coherent spectral/WAV comparisons against matched-filter naive processing.

[Feature guide](docs/DSP_NONLINEAR.md) · [Release notes](docs/releases/v0.90.0.md) · [Verification](docs/validation/v0_90_0/REPORT.md)

## Earlier additions in v0.89.0

**Reusable multi-rate oversampling.** Adds independent 2x/4x/8x half-band FIR oversampling, quality settings, integer latency reporting, a generic AudioProcessor wrapper with MIDI/clock retiming, bounded prepared buffers and standalone audio examples.

[Feature guide](docs/DSP_OVERSAMPLING.md) · [Release notes](docs/releases/v0.89.0.md) · [Verification](docs/validation/v0_89_0/REPORT.md)

## Earlier additions in v0.88.0

**Studio plug-in crash isolation.** Adds Studio stereo VST3 child hosting with explicit helper approval, preserved musical context, latency-preserving dry or silent fallback, verified recovery checkpoints, bounded persisted recovery attempts and guarded offline export.

[Feature guide](docs/STUDIO_NATIVE_ISOLATION.md) · [Release notes](docs/releases/v0.88.0.md) · [Verification](docs/validation/v0_88_0/REPORT.md)

## Earlier additions in v0.87.0

**Native sidechains and multiple outputs.** Adds typed native bus requests, dependency-ordered pre-fader sidechain feeds, independent output-return tracks, VST3 live/offline bus adapters and Studio routing controls with negotiated layout validation.

[Feature guide](docs/STUDIO_NATIVE_BUSES.md) · [Release notes](docs/releases/v0.87.0.md) · [Verification](docs/validation/v0_87_0/REPORT.md)

## Earlier additions in v0.86.0

**Native editor automation.** Adds reusable normalized native-editor gestures, VST3 controller and CLAP bracket adapters, grouped history, host-state reconciliation and Touch/Latch recording through Reference Studio.

[Feature guide](docs/NATIVE_EDITOR_AUTOMATION.md) · [Release notes](docs/releases/v0.86.0.md) · [Verification](docs/validation/v0_86_0/REPORT.md)

## Earlier additions in v0.85.0

**Native mixdown and integrated DAW workflows.** Integrates trusted continuous native offline rendering into Studio export and freeze, captures post-strip, bus and return stems in one graph pass with source-specific latency trimming, and provides an end-to-end recording, MIDI recovery, portable session and delivery example.

[Feature guide](docs/DAW_DELIVERY.md) · [Release notes](docs/releases/v0.85.0.md) · [Verification](docs/validation/v0_85_0/REPORT.md)

## Earlier additions in v0.84.0

**Native plug-in and mixer automation.** Adds stable normalized native plug-in controls to Studio read and Touch/Latch automation, bounded sample-timed CLAP/VST3 event adapters, explicit native automation diagnostics and automatable auxiliary returns.

[Feature guide](docs/DAW_AUTOMATION.md) · [Release notes](docs/releases/v0.84.0.md) · [Verification](docs/validation/v0_84_0/REPORT.md)

## Earlier additions in v0.83.0

**Studio VST3 and plug-in management.** Connects trusted CLAP and musical VST3 hosting to Studio, adds supervised discovery, enabled-product caches and quarantine, missing-instance bypass and identity-checked repair, and verified portable native-state resources with history support.

[Feature guide](docs/DAW_PLUGINS.md) · [Release notes](docs/releases/v0.83.0.md) · [Verification](docs/validation/v0_83_0/REPORT.md)

## Earlier additions in v0.82.0

**Cue mixes and hardware outputs.** Add four independent performer cue mixes with pre/post/bus/aux/main/raw input taps, source delay alignment, sample-weighted recording click, stable send automation identities and portable output pairs. Publish bounded multichannel banks through the native driver, negotiate physical channels, and provide a typed reference cue editor plus virtual routing/timing/allocation examples.

[Feature guide](docs/DAW_CUES.md) · [Release notes](docs/releases/v0.82.0.md) · [Verification](docs/validation/v0_82_0/REPORT.md)

## Earlier additions in v0.81.0

**Submix and sidechain routing.** Add nested stereo submixes and master insert chains with per-edge delay compensation, smoothed gain/balance/mute/solo, peak meters and transactional routing drafts. Add current-block stock sidechain ducking with explicit latency constraints, routing-aware track operations, reference controls and measured allocation/timing examples.

[Feature guide](docs/DAW_MIXING.md) · [Release notes](docs/releases/v0.81.0.md) · [Verification](docs/validation/v0_81_0/REPORT.md)

## Earlier additions in v0.80.0

**Transient and grouped elastic editing.** Detect and grid-snap reference transients, render linked mono/stereo microphone groups through one shared-decision multichannel WSOLA stream, apply verified caches atomically with undo/redo, and persist group recipes with portable projects. Add the reference group editor and four-microphone validation example.

[Feature guide](docs/DAW_ELASTIC.md) · [Release notes](docs/releases/v0.80.0.md) · [Verification](docs/validation/v0_80_0/REPORT.md)

## Earlier additions in v0.79.0

**Stock instruments effects and media browser.** Connect 32 automated stock effects, four synth presets and an asset-backed sampler to Studio; add worker media browsing/import/audition, portable one-processor presets, stock insert edits and actual-latency playback/recording preparation.

[Feature guide](docs/DAW_TOOLS.md) · [Release notes](docs/releases/v0.79.0.md) · [Verification](docs/validation/v0_79_0/REPORT.md)

## Earlier additions in v0.78.0

**Arrangement and musical timeline workflow.** Play from musical cursor, guarded tempo/meter edits, portable markers and song sections, and nondestructive multi-region/section duplication with phase and automation-curve preservation.

[Feature guide](docs/DAW_ARRANGEMENT.md) · [Release notes](docs/releases/v0.78.0.md) · [Verification](docs/validation/v0_78_0/REPORT.md)

## Earlier additions in v0.77.0

**Expressive MIDI clips and controller editing.** Portable editable MIDI CC/bend/pressure points play with deterministic resets and bounded seek/sustain chase; loop projections retain expression and the reference inspector provides transactional controller editing.

[Feature guide](docs/DAW_MIDI.md) · [Release notes](docs/releases/v0.77.0.md) · [Verification](docs/validation/v0_77_0/REPORT.md)

## Earlier additions in v0.76.0

**Grouped audio and loop MIDI takes.** Phase-locked multi-stem comp groups, selectable callback-clock MIDI loop passes, and conservative interrupted-audio salvage complete the implemented recording workflow series with portable take selection and undo.

[Feature guide](docs/DAW_TAKES.md) · [Release notes](docs/releases/v0.76.0.md) · [Verification](docs/validation/v0_76_0/REPORT.md)

## Earlier additions in v0.75.0

**Timestamped input and recording calibration.** Backend MIDI timestamps reach bounded intra-block scheduling; explicit impulse/manual calibration applies nondestructive audio and MIDI recording compensation with portable per-take settings.

[Feature guide](docs/DAW_TIMING.md) · [Release notes](docs/releases/v0.75.0.md) · [Verification](docs/validation/v0_75_0/REPORT.md)

## Earlier additions in v0.74.0

**Independent multichannel recording.** Independent hardware-channel routes feed track monitors and native-width recording stems; gap-free linear dry recordings place automatically as editable regions, with input-quality metadata and portable routes.

[Feature guide](docs/DAW_MULTICHANNEL.md) · [Release notes](docs/releases/v0.74.0.md) · [Verification](docs/validation/v0_74_0/REPORT.md)

## Earlier additions in v0.73.0

**Durable in-place sessions and portable performances.** Repeated background Save commits verified immutable project generations; portable raw MIDI performances and bounded pending MIDI/automation journals support explicit crash-prefix recovery.

[Feature guide](docs/DAW_SESSIONS.md) · [Release notes](docs/releases/v0.73.0.md) · [Verification](docs/validation/v0_73_0/REPORT.md)

## Earlier additions in v0.72.0

**DAW project and track workspace.** Blank-project startup, transactional track lifecycle/folders, schema-3 layout persistence and editing any selected MIDI clip in the studio.

[Feature guide](docs/DAW_WORKSPACE.md) · [Release notes](docs/releases/v0.72.0.md) · [Verification](docs/validation/v0_72_0/REPORT.md)

## Earlier additions in v0.71.0

**Studio MIDI performance recording.** Bounded callback-clock MIDI 1 capture, raw take/SMF export, quantized editable note clips and undoable prepared insertion; reference-studio native input and on-screen keyboard controls.

[Feature guide](docs/STUDIO_MIDI_RECORDING.md) · [Release notes](docs/releases/v0.71.0.md) · [Verification](docs/validation/v0_71_0/REPORT.md)

## Earlier additions in v0.70.0

**Media audit and project consolidation.** Conservative media dependency audit and verified consolidated portable copies, stale-plan rejection, freeze/warp dependency retention and preserved original files.

[Feature guide](docs/STUDIO_MEDIA_AUDIT.md) · [Release notes](docs/releases/v0.70.0.md) · [Verification](docs/validation/v0_70_0/REPORT.md)

## Earlier additions in v0.69.0

**Studio Touch and Latch automation recording.** Studio Touch/Latch automation writing through live Gesture controls, isolated pending takes, snapped control-rate capture and transactional undoable publication.

[Feature guide](docs/STUDIO_AUTOMATION_WRITE.md) · [Release notes](docs/releases/v0.69.0.md) · [Verification](docs/validation/v0_69_0/REPORT.md)

## Earlier additions in v0.68.0

**Export sample-rate conversion.** Bounded streaming sinc export conversion at 8..192 kHz, preserved duration/pitch and destination-rate analysis/normalization.

[Feature guide](docs/EXPORT_CONVERSION.md) · [Release notes](docs/releases/v0.68.0.md) · [Verification](docs/validation/v0_68_0/REPORT.md)

## Earlier additions in v0.67.0

**Native plug-in windows in the studio.** Studio-owned/floating CLAP editor windows, focus/resize/scale controls, identity-bound reopen preferences and ordered main-thread teardown/checkpointing.

[Feature guide](docs/STUDIO_PLUGIN_WINDOWS.md) · [Release notes](docs/releases/v0.67.0.md) · [Verification](docs/validation/v0_67_0/REPORT.md)

## Earlier additions in v0.66.0

**Studio routing editor.** Stopped routing matrix for track sends, pre/post taps, auxiliary returns and inserts, transactional preparation and correct automation/control remapping on removal.

[Feature guide](docs/STUDIO_ROUTING.md) · [Release notes](docs/releases/v0.66.0.md) · [Verification](docs/validation/v0_66_0/REPORT.md)

## Earlier additions in v0.65.0

**Studio export centre.** Shared-job mix and pre-mixer stem export, ranges/tails, streamed codec encoding, peak/LUFS analysis, normalization and deterministic integer dither.

[Feature guide](docs/STUDIO_EXPORT.md) · [Release notes](docs/releases/v0.65.0.md) · [Verification](docs/validation/v0_65_0/REPORT.md)

## Earlier additions in v0.64.0

**Shared background-job scheduler.** Bounded typed background scheduler shared by waveform, warp and built-in freeze work, with priorities, cancellation, progress, memory reservations and stale-result rejection.

[Feature guide](docs/STUDIO_JOBS.md) · [Release notes](docs/releases/v0.64.0.md) · [Verification](docs/validation/v0_64_0/REPORT.md)

## Earlier additions in v0.63.0

**Waveform and warp-marker editor.** Waveform zoom, draggable source markers and target-beat controls, background warp rendering, verified transactional application and global undo/redo.

[Feature guide](docs/WAVEFORM_WARP_EDITOR.md) · [Release notes](docs/releases/v0.63.0.md) · [Verification](docs/validation/v0_63_0/REPORT.md)

## Earlier additions in v0.62.0

**Complete studio recording controls.** Prepared per-track arming and dry/rendered sources, input-monitor controls, callback-clock count-in, pre-roll, exact punch and monitor-only metronome.

[Feature guide](docs/STUDIO_RECORDING.md) · [Release notes](docs/releases/v0.62.0.md) · [Verification](docs/validation/v0_62_0/REPORT.md)

## Earlier additions in v0.61.0

**Studio project recovery and migration.** Background portable studio checkpoints with bounded retention, corrupt-latest fallback into independent working copies, stable project identity/schema-1-to-2 migration, exact-checksum missing-media repair and reference-studio recovery controls.

[Feature guide](docs/STUDIO_RECOVERY.md) · [Release notes](docs/releases/v0.61.0.md) · [Verification](docs/validation/v0_61_0/REPORT.md)

## Earlier additions in v0.60.0

**Studio-wide undo and redo.** One bounded project history covers arrangement, mixer, automation, routing, comp/freeze resources and stopped native checkpoints, with grouped gestures, shared immutable PCM, failed-preparation rollback and reference-studio global shortcuts.

[Feature guide](docs/STUDIO_HISTORY.md) · [Release notes](docs/releases/v0.60.0.md) · [Verification](docs/validation/v0_60_0/REPORT.md)

## Earlier additions in v0.59.0

**Dynamic latency and bus reconfiguration.** Stopped, staged native topology replacement preserves latest state, validates explicit bus/channel routes and latency budgets, prepares complete PDC graphs before revision-checked commit, and leaves retired callbacks silent.

[Feature guide](docs/DYNAMIC_RECONFIGURATION.md) · [Release notes](docs/releases/v0.59.0.md) · [Verification](docs/validation/v059/REPORT.md)

## Earlier additions in v0.58.0

**Richer isolated plug-in control.** Isolated CLAP UI parameters, sample-offset automation and timestamped output MIDI/parameter/gesture events, with monotonic packet matching and continuous helper main-thread timer/FD/callback services.

[Feature guide](docs/ISOLATED_CONTROL.md) · [Release notes](docs/releases/v0.58.0.md) · [Verification](docs/validation/v058/REPORT.md)

## Earlier additions in v0.57.0

**Streaming-capable freeze workflow.** Typed offline streamed rendering removes the memory-only freeze limit. Bounded page waits, monitored cancellable worker jobs, source-change rejection, verified publication and reference-studio progress/cancel controls preserve reversible project freezes.

[Feature guide](docs/STREAMING_FREEZE.md) · [Release notes](docs/releases/v0.57.0.md) · [Verification](docs/validation/v057/REPORT.md)

## Earlier additions in v0.56.0

**Tempo-aware streamed audio.** Bounded worker-side warp caches connect mixed-rate WAV sources to the tempo map, with pitch-preserving WSOLA or repitch, exact marker durations, source checksums, cancellation and portable project bindings.

[Feature guide](docs/TEMPO_STREAMING.md) · [Release notes](docs/releases/v0.56.0.md) · [Verification](docs/validation/v056/REPORT.md)

## Earlier additions in v0.55.0

**Advanced nondestructive audio editing.** Shared memory/disk region split and compatible join, source slip, linear/equal-power crossfades, gain envelopes and linked editing, with undo and portable metadata.

[Feature guide](docs/AUDIO_EDITING.md) · [Release notes](docs/releases/v0.55.0.md) · [Verification](docs/validation/v055/REPORT.md)

## Earlier additions in v0.54.0

**Loop recording and take comping.** Sample-exact loop/transport annotations carried with recorded PCM, immutable latency-aligned pass lanes, transactional range comp recipes with crossfades, bounded-memory WAV export, reusable take-lane UI and portable comp-track integration.

[Feature guide](docs/TAKE_COMPING.md) · [Release notes](docs/releases/v0.54.0.md) · [Verification](docs/validation/v054/REPORT.md)

## Earlier additions in v0.53.0

**Scalable arrangement and piano-roll editors.** Reusable virtualized arrangement and full 128-key piano-roll surfaces with both-axis scrolling/zoom, independently resized track rows, marquee and multiple selection, grouped validated moves/deletes and undo/redo, plus saved viewport state and reference-studio integration.

[Feature guide](docs/SCALABLE_EDITORS.md) · [Release notes](docs/releases/v0.53.0.md) · [Verification](docs/validation/v053/REPORT.md)

## Earlier additions in v0.52.0

**Unified reference studio.** A unified desktop studio template with live session playback, reusable portable project folders, arrangement/piano-roll and automation editors, stem recording, reversible freeze, parameter mixing and explicitly trusted CLAP insert ownership/state capture.

[Feature guide](docs/REFERENCE_STUDIO.md) · [Release notes](docs/releases/v0.52.0.md) · [Verification](docs/validation/v052/REPORT.md)

## Earlier additions in v0.51.0

**Native PipeWire integration.** Optional native Linux PipeWire DSP filter backend with named per-channel input/output ports, explicit non-lingering graph links, bounded registry snapshots, prepared callback buffer splitting and driver/processing timing telemetry; includes a private software-server acceptance probe.

[Feature guide](docs/PIPEWIRE.md) · [Release notes](docs/releases/v0.51.0.md) · [Verification](docs/validation/v051/REPORT.md)

## Earlier additions in v0.50.0

**Accessible audio controls.** AccessKit semantics and assistive value actions for custom audio controls, keyboard clip/note editing with accessible object lists, independent XY-axis controls, descriptive mixer labels and enabled eframe platform adapters.

[Feature guide](docs/ACCESSIBILITY.md) · [Release notes](docs/releases/v0.50.0.md) · [Verification](docs/validation/v050/REPORT.md)

## Earlier additions in v0.49.0

**Broader plug-in isolation.** Explicit CLAP helper-process hosting for main/auxiliary input-output ports, a timestamped callback bridge, bounded RPC timeouts, opaque checkpoints and fresh-process recovery with a restart budget; existing VST3 isolation remains available.

[Feature guide](docs/EXPANDED_ISOLATION.md) · [Release notes](docs/releases/v0.49.0.md) · [Verification](docs/validation/v049/REPORT.md)

## Earlier additions in v0.48.0

**Multicore graph processing.** Dependency-level DSP worker execution with fixed node ownership, deterministic fan-in and existing delay compensation, exposed through a bounded timestamped callback bridge with explicit latency, silence-on-miss and atomic deadline telemetry.

[Feature guide](docs/PARALLEL_GRAPH.md) · [Release notes](docs/releases/v0.48.0.md) · [Verification](docs/validation/v048/REPORT.md)

## Earlier additions in v0.47.0

**Track freeze and bounce-in-place.** Control-side pre-mixer stem rendering with latency trimming, explicit tails, SHA-256 source/cache validation, processor-bypassing frozen playback, reversible restoration and bounce-in-place document generation.

[Feature guide](docs/TRACK_FREEZE.md) · [Release notes](docs/releases/v0.47.0.md) · [Verification](docs/validation/v047/REPORT.md)

## Earlier additions in v0.46.0

**Multitrack recording and takes.** Synchronized float32 track, master and input recording, bounded pre-roll, punch intervals, independent arming, immutable take manifests and non-destructive take selection, with live-session capture and off-callback retirement.

[Feature guide](docs/MULTITRACK_RECORDING.md) · [Release notes](docs/releases/v0.46.0.md) · [Verification](docs/validation/v046/REPORT.md)

## Earlier additions in v0.45.0

**Disk-streamed arrangement clips.** Disk-backed arrangement regions with bounded independent caches/workers, source trims, loops/fades, seek-generation protection, explicit prefetch/telemetry and unified-session playback integration.

[Feature guide](docs/STREAMED_CLIPS.md) · [Release notes](docs/releases/v0.45.0.md) · [Verification](docs/validation/v045/REPORT.md)

## Earlier additions in v0.44.0

**Unified live session engine.** Shared live track/clip/instrument/insert/mixer/aux/automation playback, bounded transport commands, latency alignment, parameter snapshots, existing recorder integration and prepared crossfaded project swaps.

[Feature guide](docs/LIVE_SESSIONS.md) · [Release notes](docs/releases/v0.44.0.md) · [Verification](docs/validation/v044/REPORT.md)

## Earlier additions in v0.43.0

**Improved streaming pitch and time processing.** Opt-in prepared phase-locked/transient-aware pitch shifting and stereo-linked streaming WSOLA time stretching with bounded work, explicit backpressure/finish/drain, exact durations and audible comparison renders.

[Feature guide](docs/PITCH_TIME.md) · [Release notes](docs/releases/v0.43.0.md) · [Verification](docs/validation/v043/REPORT.md)

## Earlier additions in v0.42.0

**Audio-quality regression suite.** Deterministic offline distortion/noise/IMD/alias/response/stability measurements, versioned regression thresholds and a twelve-probe executable suite with failure exit codes and release-workflow integration.

[Feature guide](docs/DSP_QUALITY.md) · [Release notes](docs/releases/v0.42.0.md) · [Verification](docs/validation/v042/REPORT.md)

## Earlier additions in v0.41.0

**Optimized DSP backends.** Safe prepared scalar/SSE2/AVX2/ARM64-NEON gain, mix, dot and interleave kernels with tail/unaligned handling, existing Gain integration and a repeatable release microbenchmark.

[Feature guide](docs/DSP_KERNELS.md) · [Release notes](docs/releases/v0.41.0.md) · [Verification](docs/validation/v041/REPORT.md)

## Earlier additions in v0.40.0

**Hardware controller integration.** Prepared controller banks, relative encoder modes, soft takeover, toggles and coalesced MIDI feedback with bounded-queue retries and sent-echo suppression, demonstrated through a virtual MIDI output worker.

[Feature guide](docs/CONTROLLERS.md) · [Release notes](docs/releases/v0.40.0.md) · [Verification](docs/validation/v040/REPORT.md)

## Earlier additions in v0.39.0

**Microtuning support.** Validated absolute tuning tables, bounded Scala SCL/KBM imports, synth/sampler frequency integration, unmapped-key handling and optional project persistence, with an audible just-intonation demo.

[Feature guide](docs/MICROTUNING.md) · [Release notes](docs/releases/v0.39.0.md) · [Verification](docs/validation/v039/REPORT.md)

## Earlier additions in v0.38.0

**Sample-instrument authoring tools.** Sample filename/velocity/round-robin mapping, monophonic root estimation, peak-normalized copies, zero-crossing/trim/loop helpers and playable native-project/SFZ exports with sequence roundtrips.

[Feature guide](docs/SAMPLE_AUTHORING.md) · [Release notes](docs/releases/v0.38.0.md) · [Verification](docs/validation/v038/REPORT.md)

## Earlier additions in v0.37.0

**Reusable mixer toolkit.** Prepared stereo channel strips with smoothed gain/pan, mute/solo, groups, pre/post sends, sample-offset automation, atomic meters and reusable UI controls with a stem-rendering example.

[Feature guide](docs/MIXER.md) · [Release notes](docs/releases/v0.37.0.md) · [Verification](docs/validation/v037/REPORT.md)

## Earlier additions in v0.36.0

**Application command framework.** Stable application commands, validated configurable keymaps, shared menu/toolbar/shortcut adapters and bounded transactional undo/redo, integrated into the clip editor.

[Feature guide](docs/COMMANDS.md) · [Release notes](docs/releases/v0.36.0.md) · [Verification](docs/validation/v036/REPORT.md)

## Earlier additions in v0.35.0

**Audio and MIDI clip toolkit.** Validated audio/MIDI clip editing, looped frame/beat playback, seek recovery, latency alignment and reusable arrangement/piano-roll views, with a native editor and WAV rendering demo.

[Feature guide](docs/CLIPS.md) · [Release notes](docs/releases/v0.35.0.md) · [Verification](docs/validation/v035/REPORT.md)

## Earlier additions in v0.34.0

**Waveform thumbnail cache.** Streaming multiresolution waveform peaks, fingerprinted atomic disk caches with corruption repair/eviction, latest-request background loading/cancellation, bounded PCM detail reads and a cached native waveform view.

[Feature guide](docs/WAVEFORM_CACHE.md) · [Release notes](docs/releases/v0.34.0.md) · [Verification](docs/validation/v034/REPORT.md)

## Earlier additions in v0.33.0

**Themeable audio UI toolkit.** Serializable audio UI themes, normalized knobs/faders with fine and stepped interaction, modulation rings, typed toggles/enums, prepared SVG and filmstrip assets, and a native theme gallery with framebuffer snapshots.

[Feature guide](docs/UI_TOOLKIT.md) · [Release notes](docs/releases/v0.33.0.md) · [Verification](docs/validation/v033/REPORT.md)

## Earlier additions in v0.32.0

**Typed parameter SDK.** Typed float/integer/bool/choice/Rust-enum declarations, shared normalization/text/step metadata and editor controls, macro-generated parameter sets and revision-checked snapshots, with a real exported typed effect.

[Feature guide](docs/TYPED_PARAMETERS.md) · [Release notes](docs/releases/v0.32.0.md) · [Verification](docs/validation/v032/REPORT.md)

## Earlier additions in v0.31.0

**Platform validation and release tooling.** Complete schema2 staged-release inventories, ELF/PE/Mach-O architecture and checksum auditing, manifest-specific target-machine acceptance, and a read-only cross-platform native export evidence workflow.

[Feature guide](docs/PLATFORM_RELEASES.md) · [Release notes](docs/releases/v0.31.0.md) · [Verification](docs/validation/v031/REPORT.md)

## Earlier additions in v0.30.0

**Desktop editor integration.** Owned CLAP HWND/AppKit parents with ordered GUI teardown, deferred Windows close/resize/DPI handling and desktop focus; fixes native VST3 flag ABI casts and validates Windows/macOS host code by cross-target Clippy.

[Feature guide](docs/DESKTOP_EDITORS.md) · [Release notes](docs/releases/v0.30.0.md) · [Verification](docs/validation/v030/REPORT.md)

## Earlier additions in v0.29.0

**Plug-in crash isolation.** Opt-in VST3 child-process hosting with bounded response watchdog, explicit checkpoint/parameter recovery, permanently revoked stale callbacks and per-owner recovery quarantine.

[Feature guide](docs/PLUGIN_ISOLATION.md) · [Release notes](docs/releases/v0.29.0.md) · [Verification](docs/validation/v029/REPORT.md)

## Earlier additions in v0.28.0

**Multichannel hosting and delay compensation.** Explicit mono/stereo/multichannel bus DAGs with per-channel feed-forward latency compensation, native VST3 sidechain/auxiliary activation and a bounded multiple-bus worker bridge.

[Feature guide](docs/HOST_CHANNELS.md) · [Release notes](docs/releases/v0.28.0.md) · [Verification](docs/validation/v028/REPORT.md)

## Earlier additions in v0.27.0

**VST3 musical integration.** Prepared tempo/meter/loop/punch context, expressive note identities, bounded SysEx/program ingress, timed native output and callback-offset normalized automation for buffered VST3 instruments and mixed sessions.

[Feature guide](docs/VST3_MUSICAL.md) · [Release notes](docs/releases/v0.27.0.md) · [Verification](docs/validation/v027/REPORT.md)

## Earlier additions in v0.26.0

**Continuous offline rendering.** Continuous scoped VST3/CLAP offline processing, tail-inclusive rendering, latency trimming, cancellation and transactional aligned WAV stem export.

[Feature guide](docs/HOST_RENDERING.md) · [Release notes](docs/releases/v0.26.0.md) · [Verification](docs/validation/v026/REPORT.md)

## Earlier additions in v0.25.0

**Unified plug-in graphs and sessions.** Versioned mixed native/CLAP/VST3 graph sessions with explicit module trust, compensated stereo routing, opaque-state capture, aligned bypass and preserved missing nodes.

[Feature guide](docs/MIXED_SESSIONS.md) · [Release notes](docs/releases/v0.25.0.md) · [Verification](docs/validation/v025/REPORT.md)

## Earlier additions in v0.24.0

**Direct VST3 processing.** Prepared native f32 stereo VST3 audio callbacks with zero worker bridge delay, bounded normalized controls and owner-thread lease revocation; buffered hosting remains available.

[Feature guide](docs/VST3_DIRECT.md) · [Release notes](docs/releases/v0.24.0.md) · [Verification](docs/validation/v024/REPORT.md)

## Earlier additions in v0.23.0

**VST3 hosting.** Common CLAP/VST3 product controls, trusted factory scanning, native VST3 state and timed parameters, a bounded stereo audio/MIDI worker bridge, native editors and a workbench VST3 panel.

Read the [guide](docs/VST3_HOSTING.md), [release notes](docs/releases/v0.23.0.md) and [verification](docs/validation/v023/REPORT.md).

## Earlier additions in v0.22.0

**Expanded hosted editors.** Native parent attachment, owned X11 editor windows, resize and scale negotiation, stable reopening, persisted editor preferences and bounded CLAP timer/POSIX FD host services.

Read the [guide](docs/HOSTED_EDITORS.md), [release notes](docs/releases/v0.22.0.md) and [verification](docs/validation/v022/REPORT.md).

## Earlier additions in v0.21.0

**Shared musical transport and tempo maps.** Analytic tempo steps/ramps, PPQ and meter/bar positions, loop/punch metadata, persisted project timelines, mapped automation and native CLAP musical transport.

Read the [guide](docs/MUSICAL_TRANSPORT.md), [release notes](docs/releases/v0.21.0.md) and [verification](docs/validation/v021/REPORT.md).

## Earlier additions in v0.20.0

**MIDI I/O and sample scheduling.** Bounded full MIDI 1 wire packets and SysEx, explicit driver-clock mapping, allocation-free sample queues, scheduled native/virtual output workers and persisted CC learning.

Read the [guide](docs/MIDI_IO.md), [release notes](docs/releases/v0.20.0.md) and [verification](docs/validation/v020/REPORT.md).

## Earlier additions in v0.19.0

**Audio format manager.** A common optional libsndfile-backed WAV/AIFF/FLAC API with encoding, seekable decoding, metadata, bounded asset loading and project/bundle integration; portable WAV APIs remain available.

Read the [guide](docs/AUDIO_FORMATS.md), [release notes](docs/releases/v0.19.0.md) and [verification](docs/validation/v019/REPORT.md).

## Earlier additions in v0.18.0

**Flexible audio buses.** Named multichannel layouts, exact negotiation, explicit auxiliary/sidechain channel patching and scoped native multi-port CLAP processing with advertised configuration selection.

Read the [guide](docs/FLEXIBLE_BUSES.md), [release notes](docs/releases/v0.18.0.md) and [verification](docs/validation/v018/REPORT.md).

## Earlier additions in v0.17.0

**Complete hosted plug-in sessions.** Versioned native rack sessions with opaque state/checksums, parameter fallback, latency-aligned bypass, floating editor preferences and preserved missing plug-in slots.

Read the [guide](docs/HOST_SESSIONS.md), [release notes](docs/releases/v0.17.0.md) and [verification](docs/validation/v017/REPORT.md).

## Earlier additions in v0.16.0

**Low-latency hosting and diagnostics.** Direct trusted CLAP callbacks with zero bridge delay, safe owner revocation, configurable buffered lookahead, deadline counters and offline impulse alignment.

Read the [guide](docs/LOW_LATENCY.md), [release notes](docs/releases/v0.16.0.md) and [verification](docs/validation/v016/REPORT.md).

## Earlier additions in v0.15.0

**Compatibility and release verification suite.** Structured compatibility reports, child-process CLAP probes, repeated exported ABI reopening, workflow/audit coverage, source ZIP hash verification, target-machine acceptance records and stricter performance regression input checks.

Read the [guide](docs/COMPATIBILITY_SUITE.md), [release notes](docs/releases/v0.15.0.md) and [verification](docs/validation/v015/REPORT.md).

## Earlier additions in v0.14.0

**Live CLAP rack and floating editors.** A main-thread-owned serial CLAP rack with a bounded stereo callback bridge, live parameter controls, output-event observation, floating-editor lifecycle and workbench device integration.

Read the [guide](docs/LIVE_CLAP_RACK.md), [release notes](docs/releases/v0.14.0.md) and [verification](docs/validation/v014/REPORT.md).

## Earlier additions in v0.13.0

**Automation editing toolkit.** Editable beat lanes with four curve types, snapping, looping, Touch/Latch recording, prepared playback and project persistence.

Read the [guide](docs/AUTOMATION.md), [release notes](docs/releases/v0.13.0.md) and [verification](docs/validation/v013/REPORT.md).

## Earlier additions in v0.12.0

**Unified input/output management.** Added shared input/output selection and channel maps, stereo duplex processing/MIDI delivery, muted-by-default monitoring, managed recording and coordinated close/reconnect with retained take errors.

Read the [guide](docs/UNIFIED_AUDIO.md), [release notes](docs/releases/v0.12.0.md) and [verification](docs/validation/v012/REPORT.md).

## Earlier additions in v0.11.0

**Advanced sampler and SFZ subset.** Added velocity crossfades, sustain-aware release samples, energy-onset slicing, a strict scoped SFZ subset importer and worker-backed sample-map integration.

Read the [detailed guide](docs/ADVANCED_SAMPLER.md), [release notes](docs/releases/v0.11.0.md) and [verification](docs/validation/v011/REPORT.md).

## Earlier additions in v0.10.0

**Preset and asset browser.** Added searchable preset catalogs, tags, favourites, banks, background-rendered previews and audible audition, missing-asset checks, scoped relocation and portable preset export.

Read the [detailed guide](docs/PRESET_BROWSER.md), [release notes](docs/releases/v0.10.0.md) and [verification](docs/validation/v010/REPORT.md).

## Earlier additions in v0.9.0

| Area | Implementation |
|---|---|
| Audio output management | Saved preferences, device/backend/rate/buffer selection, reconnect policy, explicit fallback and reusable settings UI |
| Custom DSP ecosystem | Compiled processor registry, parameter factories, state migrations, custom/fallback editors and project/automation/modulation integration |
| Demonstration | Separate Tape Colour processor crate, registered menu, seventh preset and portable audio/state walkthrough |

```sh
cargo run -p soniclaw-workbench -- --tab 9
cargo run -p soniclaw-workbench -- --preset 6
cargo run -p soniclaw-sdk-tools -- --registry-demo /new/registry-demo
```

Read [audio settings](docs/DEVICE_MANAGER.md), [custom DSP](docs/REGISTRY.md), [release notes](docs/releases/v0.9.0.md) and [verification](docs/validation/v090/REPORT.md). Reconnection currently manages output streams; physical hardware testing remains pending.

## Earlier additions in v0.8.0

Portable `.soniclaw` project files, checksummed recovery checkpoints, opt-in autosave and background resource loading are integrated into the native workbench.

```sh
cargo run -p soniclaw-workbench
cargo run -p soniclaw-sdk-tools -- --workflow-demo /new/workflow-folder
```

Read the [project workflow guide](docs/PROJECT_WORKFLOWS.md), [release notes](docs/releases/v0.8.0.md) and [verification](docs/validation/v080/REPORT.md). Recovery snapshots retain external asset paths; bundles include the WAV files.

## Earlier additions in v0.7.0

| Area | Implementation |
|---|---|
| Instruments | Independent note envelopes/LFOs, source routing and Poly Motion |
| Routing | Visual cables/layout, prepared graph warmup/crossfade/retirement |
| Sampler | Key/velocity/loop/region editor, slices and memory-sampler fades |
| MIDI | Arpeggiator, chords/scales, swing/humanization, sequences and PPQN files |
| Hosting | Trusted CLAP discovery, scoped separate-thread audio, parameters/state |
| Validation | Rate/block/heap/state stress audits, callback diagnostics and independent host fixture |

```sh
cargo run -p soniclaw-workbench
cargo run -p soniclaw-sdk-tools -- --demo /new/output/folder
cargo run -p soniclaw-sdk-tools -- --audit /new/audit.json
```

See [SDK guide](docs/SDK_070.md), [hosting](docs/HOSTING.md), [migration](docs/MIGRATION_070.md), [release notes](docs/releases/v0.7.0.md), [evidence](docs/validation/v070/REPORT.md). Workbench has six presets/nine panels. Scoped hosting is a render foundation, not persistent device/editor hosting. Live graphs use a fixed latency budget and fresh history.

## Earlier additions in v0.6.0

| Area | Implementation |
|---|---|
| Modulation | Tempo LFOs, curved multistage envelopes, macros, MIDI sources and bounded routing |
| Multiband | Three-band linked compression/saturation/gain/mute/solo and dynamic EQ |
| Granular | Fixed grain/voice pools, seeded clouds, position/scan/pitch/spread, sustain/expression |
| Spectral | Freeze, robotisation, frequency/pitch shift; offline phase-vocoder time stretch |
| Convolution | Mono/stereo IRs, zero/uniform latency, two-tier long IRs and deferred-retirement crossfades |
| Native workbench | Rack/controls, modulation editor, preview/live analysis, presets/assets, undo/redo and portable export |

```sh
cargo run -p soniclaw-workbench
cargo run -p soniclaw-workbench --no-default-features -- --render /new/output/folder
cargo build --release -p soniclaw-workbench
```

Read the [workbench guide](docs/WORKBENCH.md), [new DSP APIs](docs/DSP_MODULATION.md), [migration](docs/MIGRATION_060.md), [release notes](docs/releases/v0.6.0.md) and [current evidence](docs/validation/v060/REPORT.md). Time stretching is offline; spectral transforms are basic phase-vocoder implementations. Linux source/ABI/virtual-display checks do not establish physical-device or macOS/Windows DAW acceptance.

## Earlier additions in v0.5.0

| DSP area | New implementation |
|---|---|
| Filters | TPT SVF and four-pole analytical ladder, resonance/drive |
| Crossovers | LR4 two-way and phase-aligned three-way kernels/band effects |
| Dynamics | RMS compressor, lookahead peak-estimate limiter, transient shaper |
| Spectral | FFT, windows, STFT/weighted overlap-add, spectrum analyzer, spectral gate |
| Synthesis | Mipmapped wavetable banks, morph/PWM/sync, unison and note expression |
| Reverb | Eight-line FDN, early reflections, predelay, diffusion, damping/decay and delay modulation |
| Creative effects | Phaser, flanger, tremolo, autopan and tempo-following multitap/ping-pong delay |
| Analysis/stereo | Monophonic pitch, 48 kHz mono/stereo loudness, peak estimates, RMS/correlation and mid/side/width |

```sh
cargo run -p soniclaw-dsp-lab -- /new/output/directory
cargo build -p soniclaw-dsp-lab
```

The new library exports DSP Lab FX and Wavetable Lab as CLAP, using complete project state and named controls. Read the [DSP expansion guide](docs/DSP_ADVANCED.md), [release notes](docs/releases/v0.5.0.md) and [current validation](docs/validation/v050/REPORT.md) for actual bounds and evidence. Most new processors support 8–192 kHz; loudness measurement is specifically 48 kHz mono/stereo. These implementations have functional tests, not universal audio-quality or standards certification.

## Earlier additions in v0.4.0

| Area | Addition |
|---|---|
| Project state | Schema-2 documents for assets, sample maps, rack/routing and controls; v1 migration, stopped reconstruction and bounded undo/redo |
| Automation | Ten effect kinds with named host/UI parameters and 10 ms scalar smoothing |
| Resampling | Selectable linear/windowed-sinc interpolation, offline file conversion and sampler integration |
| Disk sampling | Seekable/looping transport, independent voice caches, background page reads, PCM budgets and telemetry |
| Expression | Explicit MIDI-1 MPE zones, note IDs, tuning, pressure, timbre, pan/volume and an expressive synth |

See [projects](docs/PROJECTS.md), [DSP](docs/DSP.md), [disk streaming](docs/STREAMING.md), [expression](docs/EXPRESSION.md), and [v0.4.0 release notes](docs/releases/v0.4.0.md).

```sh
# Hardware-free walkthrough; output directory must not already exist
cargo run -p soniclaw-ecosystem -- /tmp/soniclaw-v040-demo
# Export a complete-state, automatable rack as CLAP
cargo build --release -p soniclaw-project-plugin
```

The new Project Rack has a new product ID. Existing reference products keep their IDs and fixed factories. The CLAP demo synth now uses the expressive instrument; its timbre can differ. Current Linux source/ABI validation and earlier native-format results are distinguished in [validation](docs/VALIDATION.md). Native macOS/Windows acceptance remains pending.

## Earlier additions in v0.3.0

| Area | Working addition |
|---|---|
| Performance | Settled gain specialization, cached low-pass coefficients, streamlined CLAP buffer conversion; frozen v0.2 baseline and reproducible before/after batch CPU measurements |
| Embedded editors | Shared egui controls inside a host-parented window, resize/scale/focus handling and bounded UI-to-host gestures with backpressure retention |
| Compatibility | Exported-ABI host for state, buses, real sidechain audio, automation, in-place/constant buffers, callback stress and heap instrumentation; optional real X11 mouse interaction |
| Effects | Parametric EQ, compressor, peak limiter, gate, chorus, Schroeder reverb, partitioned FFT convolution and 2× oversampled drive |
| Instruments | Immutable multisample banks, key/velocity zones, round robin, sustain, channel pitch bend and crossfaded stealing; separate bounded background WAV transport |
| Formats and tools | Statically wrapped VST3, macOS AUv2 recipe, app/effect/sampler templates, command-line checks, license collection, archive/installer staging, documentation and platform CI configuration |

**New examples:** `soniclaw-reference` exports Gain + Filter, Sidechain Ducker, Multisample Drums and Effects Rack. `soniclaw-reference-studio` hosts the same drum factory standalone, with a hardware-free render mode. See [formats](docs/FORMATS.md), [DSP](docs/DSP.md), [sampling/streaming](docs/SAMPLER.md), [release tools](docs/RELEASE.md) and the [validation report](docs/VALIDATION.md).

Linux CLAP/VST3 builds and embedded X11 interaction were checked for v0.3.0; those historical results do not establish fresh v0.6.0 wrapper/UI acceptance. Real DAW project round trips, audio-device loopback/unplug tests, native macOS/Windows runtime acceptance and AUv2 compilation remain pending. The AUv2/installer recipes and CI jobs are supplied, but unexecuted targets are not presented as verified products.

## Earlier additions in v0.2.0


| Addition | Implementation |
|---|---|
| Audio input and duplex processing | Native capture/playback, selected mono/stereo input channels, stereo output mapping, muted-by-default monitoring, bypass, bounded input bridge with prefill and adaptive clock-drift correction |
| Device/session management | Supported configuration enumeration, requested rate/buffer validation, device settings UI, JSON configuration, control-thread device-loss detection and retry |
| Recording | Dry input or processed output to stereo PCM16 WAV on a worker thread; bounded enqueue, counted loss, silence inserted for dropped frames, explicit finalization and protection against overwriting takes |
| Automation and buses | Sample-offset parameter events, stable numeric IDs, coherent same-frame groups, separate input/output and sidechain layouts, sidechain compressor, graph latency compensation, suspend/resume |
| CLAP export | Real exported entry/factory and plug-in binaries; gain-effect and synth examples; parameter automation, state, audio/MIDI ports, transport context, latency/tail/render extensions; project template and bundling helper |
| Audio UI toolkit | Parameter knobs/sliders, XY pad, waveform zoom/pan, piano keyboard, envelope editor, low-pass response editor and meter ballistics/peak hold |

**Examples:** native synthesizer, Live FX & Recorder, UI component gallery, offline WAV rendering and a CLAP library containing two plug-ins.

Current checks and limitations are recorded in [validation](docs/VALIDATION.md); original v0.2 reports are retained as historical evidence.

## Build and run

Use stable Rust **1.100+** (validated with 1.100.0). Open a terminal in this extracted `soniclaw` directory.

| Platform | Prerequisites |
|---|---|
| macOS | Xcode command-line tools (`xcode-select --install`) and Rust |
| Windows | Rust MSVC toolchain and Visual Studio C++ Build Tools with the Windows SDK |
| Debian/Ubuntu Linux | `sudo apt-get install build-essential pkg-config libasound2-dev libx11-dev libx11-xcb-dev libxcb1-dev libxcursor-dev libxi-dev libxrandr-dev libxinerama-dev libgl1-mesa-dev libxcb-dri2-0-dev libxcb-icccm4-dev libwayland-dev libxkbcommon-dev libegl1-mesa-dev` |

```sh
# No audio hardware needed: six-second synth render and parameter preset
cargo run -p soniclaw-render -- soniclaw-demo.wav

# Native synth: on-screen piano and computer keyboard
cargo run -p soniclaw-desktop --release

# Microphone/guitar processing, device settings, monitoring and WAV recording
cargo run -p soniclaw-live-fx --release

# New native multisample drums (starts muted)
cargo run -p soniclaw-reference-studio --release
# Same factory rendered without audio hardware
cargo run -p soniclaw-reference-studio -- --render soniclaw-drums.wav

# Interactive controls without opening an audio device
cargo run -p soniclaw-ui-gallery --release
```

A generated six-second `soniclaw-demo.wav` and preset are included. For external MIDI in the synth demo, enumerate with `cargo run -p soniclaw-desktop -- --list-devices`, then launch with `-- --midi 0` and optionally `--device 0`.

## Live FX & Recorder

1. Expand **Audio devices and session**, choose input/output devices and channels, and select a supported rate/buffer. Channel indices start at zero; `[0,0]` duplicates a mono input across stereo processing.
2. Click **Apply & start / restart**. A changed setting takes effect on restart. Unsupported combinations return an error; there is no silent rate fallback.
3. Enable **Monitor output** when ready. Monitoring starts muted. Use headphones with a live microphone. Meters display captured input and processed output before the monitor mute.
4. Shape the sound with the cutoff/gain knobs, XY pad or filter-response display. Bypass sends dry input through the output path.
5. Choose a new WAV filename, select dry input or processed output, and click **Record**. Recording works while monitoring is muted. **Stop & finalize take** finishes the file and shows a waveform preview for takes up to 20 MiB.
6. Save/load `soniclaw-session.json` with the settings controls. Stream errors or a missing named device stop the session and, when enabled, trigger one-second retries. Recovery creates a fresh engine, finalizes the current recording and mutes monitoring. Recording does not automatically resume.

Recording always uses the **active stream's** rate, even if pending settings have been edited. Overflow produces counted silent gaps rather than shortening the take. Capture/write/loss counters and the last finalized take's summary remain available. A write/finalization error is reported; a recording with a disk failure must not be treated as a successful take.

The requested hardware buffer is distinct from the internal 256-frame processing maximum. The duplex bridge adds a software prefill and can correct small clock differences (bounded to ±0.2%) using linear interpolation. It is not a general high-quality sample-rate converter or a promise of measured round-trip latency. Start with input/output on the same audio interface. See [native session details](docs/SESSIONS.md).

## Build CLAP plug-ins

```sh
cargo build -p soniclaw-clap-demo --release
```

Package the library built for the current platform:

```sh
# Linux
python3 tools/bundle-clap.py --library target/release/libsoniclaw_clap_demo.so

# Windows
python tools/bundle-clap.py --library target/release/soniclaw_clap_demo.dll

# macOS
python3 tools/bundle-clap.py --library target/release/libsoniclaw_clap_demo.dylib
```

This produces `dist/SoniclawDemo.clap`, a file on Windows/Linux and a bundle on macOS. The library contains **Soniclaw Gain** and **Soniclaw Synth**. Open the packaged plug-in through a CLAP-compatible host's plug-in scanner. No plug-in is installed into system directories by these commands.

The CLAP adapter supports stereo f32 main audio, an optional separate stereo sidechain key, MIDI 1, parameters/state/transport and optional embedded UI. Reference plug-ins enable the editor; the original gain/synth demo remains headless by default. The [opt-in stereo f64 path](docs/DSP_ENGINE_PRECISION.md) is available for precision products. Native Wayland GUI handles, full MPE/note-expression routing and arbitrary bus layouts are outside the scope of this adapter. See [CLAP](docs/CLAP.md) and [multi-format builds](docs/FORMATS.md).

```sh
cargo build -p soniclaw-reference --release
python3 tools/bundle-clap.py --library target/release/libsoniclaw_reference.so --name SoniclawReference
# Explicitly download pinned format dependencies, then build VST3:
python3 tools/build-formats.py --fetch
# macOS only, also build AUv2 (not validated on Linux):
# python3 tools/build-formats.py --fetch --au
```

For an ABI integration check, build debug and run the bundled independent host:

```sh
cargo build -p soniclaw-clap-demo
cargo run -p soniclaw-clap-smoke
```

See [CLAP adapter](docs/CLAP.md) for the export API, safety boundary and validation instructions.

## Create a project

```sh
cargo install --path tools/cargo-soniclaw
cargo soniclaw new my-instrument --framework /absolute/path/to/soniclaw
cargo soniclaw new my-effect --kind clap --framework /absolute/path/to/soniclaw
cargo soniclaw new my-sampler --kind sampler --framework /absolute/path/to/soniclaw
cargo soniclaw doctor --framework /absolute/path/to/soniclaw
```

The app template is a native tone generator that starts muted. The CLAP template exports a gain effect with an editor; the sampler template exports a mapped sample instrument. Each generated project has an independent workspace boundary and local dependency paths. Keep the checkout available, quote paths containing spaces, and change the provisional plug-in namespace before distributing a product. Existing directories are refused.

## New module APIs

| Crate/API | Scope |
|---|---|
| `soniclaw-editor` | Host-parented native editor, shared widgets, bounded parameter-edit producer |
| `soniclaw-sampler` | Bank/zone validation and fixed-voice multisampling |
| `soniclaw-stream` | Forward transport, seekable player, paged disk sampler and telemetry |
| `soniclaw-dsp` additions | Effects, FFT convolution and oversampled drive |
| `soniclaw-plugin` additions | Sidechain processing and `HostedProcessor` for shared standalone factories |

## Existing modules

| Crate | Responsibility |
|---|---|
| `soniclaw-core` | Borrowed buffers, processor lifecycle, MIDI/events, transport, atomic parameters, smoothing and automation events |
| `soniclaw-dsp` | Oscillators, ADSR, fixed-voice synth and preloaded sampler, gain, low-pass and delay |
| `soniclaw-engine` | Serial chains, automation splitting, event validation, transport and offline rendering |
| `soniclaw-graph` | Prepared DAG routing, parallel branches, fan-in/output latency alignment |
| `soniclaw-bus` | Explicit main-input/output/sidechain layouts, in-place adapter and sidechain compressor |
| `soniclaw-midi` | MIDI 1 streaming parser and optional native input |
| `soniclaw-device` | CPAL output/duplex hosts, channel maps, clock bridge, queues and telemetry |
| `soniclaw-session` | Portable persisted configuration; optional native controller/recovery |
| `soniclaw-record` | Audio-thread recording tap and background WAV worker |
| `soniclaw-ui` | Reusable egui audio controls and displays |
| `soniclaw-io` | WAV import/export |
| `soniclaw-state` | Parameter/project documents, migration and undo/redo |
| `soniclaw-plugin` | Format-independent lifecycle and stable parameter-ID binding |
| `soniclaw-clap` | CLAP ABI/host-buffer/event/state translation |
| `soniclaw` | Facade/prelude; native devices, native MIDI and UI remain optional |

`cargo-soniclaw`, `soniclaw-clap-smoke` and the Python bundle helper provide developer tooling. Libraries depend on mature CPAL, egui/eframe, midir, rtrb, hound, serde and clap-sys implementations. The core/DSP/engine/device/portable UI code forbids unsafe code; raw ABI handling is confined to `soniclaw-clap`, its export entry point, the native-handle boundary in `soniclaw-editor`, and independent host tools.

## Processing and automation

`AudioProcessor::prepare` can allocate; `process` must not allocate, lock, log or perform I/O. An in-place effect modifies input; an instrument replaces it. Prepared storage supports varying block lengths. Edit graphs and recreate devices on a control thread.

```rust
use soniclaw::prelude::*;
use std::sync::Arc;

fn automate_gain() -> Result<Vec<f32>, Error> {
    let gain = Arc::new(Parameter::new(ParameterInfo {
        id: "master.gain", name: "Gain", min: 0.0, max: 1.0,
        default: 0.5, unit: "",
    })?);
    let mut engine = Engine::default();
    engine.push(PolySynth::<16>::default());
    engine.push(Gain::new(gain.clone()));
    engine.bind_parameter(7, gain)?;
    engine.prepare(ProcessSpec {
        sample_rate: 48_000.0, max_block_frames: 256, channels: 2,
    })?;
    let notes = [Event { frame: 0, message: MidiMessage::NoteOn {
        channel: 0, note: 69, velocity: 100,
    }}];
    let automation = [AutomationEvent { frame: 12_000,
        parameter: 7, value: 0.2,
    }];
    soniclaw::engine::render_automated(&mut engine, 48_000, &notes, &automation)
}
```

Use `process_automated` with block-relative offsets in real time; use `render_automated` with absolute render-relative positions offline. Events must be ordered and parameters registered. Same-frame parameter groups are applied before rendering that sample. Gain still ramps over 10 ms: automation timing and DSP smoothing are distinct. `PluginInstance` binds parameters using stable FNV-1a IDs derived from their persistent string IDs and rejects collisions.

`soniclaw-bus::BusEngine` accepts separate main input/output and sidechain slices. `InPlaceBus` adapts an engine with mono/stereo mapping; `SidechainDucker` demonstrates a linked-channel compressor. The graph compensates declared processor latencies at fan-in and output sums; an intentional echo is not declared as processing latency. Compensation changes require preparation and are bounded to 262,144 frames per delay line.

## Checks and status

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo doc --workspace --all-features --no-deps --locked
```

Full workspace checks require native prerequisites. Portable checks can use `cargo test -p soniclaw` without native features. The test suite includes processing allocation checks, MIDI/automation partition invariance, sidechain behavior, latency alignment, record loss/duration, concurrent stop/restart and UI rendering without a window.

[VALIDATION.md](docs/VALIDATION.md) records what actually ran. CI is configured for native Linux/macOS/Windows, but those remote jobs have not been executed for this archive. The source is not a signed installer and the project is not yet a production-equivalent replacement for JUCE.

Read [architecture](docs/ARCHITECTURE.md), [real-time contract](docs/REALTIME.md), [roadmap](docs/ROADMAP.md) and [contributing](CONTRIBUTING.md). Soniclaw is **MIT OR Apache-2.0**; dependencies keep their own licenses.

## Reusable DSP roadmap

[Implemented DSP batch and pending enhancements](docs/DSP_ROADMAP.md). These modules work independently of Studio.
