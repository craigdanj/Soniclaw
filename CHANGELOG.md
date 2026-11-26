# Changelog

Detailed [release notes](docs/releases/README.md) are maintained for every major
and minor release, including pre-1.0 versions.

## [0.146.0] — 2026-11-21

- Reusable floating-point SIMD types: Compose DSP with safe f32 and f64 lane vectors, masks and portable tails using scalar, SSE2, AVX2 and AArch64 NEON backends.
- Detailed API, migration and validation: `docs/DSP_SIMD_VECTORS.md`.

## [0.145.0] — 2026-11-21

- End-to-end f64 processing: Carry native float64 audio through the prepared engine, latency-compensated graph, opt-in Studio clip sessions, WAV I/O and double-precision CLAP/VST3 plug-in paths.
- Detailed API, migration and validation: `docs/DSP_ENGINE_PRECISION.md`.

## [0.144.0] — 2026-11-21

- Selectable panning laws: Pan mono sources or balance stereo sources with seven selectable gain laws and continuous position changes.
- Detailed API, migration and validation: `docs/DSP_STEREO_PANNER.md`.

## [0.143.0] — 2026-11-21

- Logarithmic ramps: Shape independent rise and fall transitions with exact endpoint arrival and continuous retargeting.
- Detailed API, migration and validation: `docs/DSP_LOGARITHMIC_RAMP.md`.

## [0.142.0] — 2026-11-21

- First-order TPT filters: Process low-pass, high-pass and all-pass responses with topology-preserving state and audio-rate cutoff modulation.
- Detailed API, migration and validation: `docs/DSP_FIRST_ORDER_TPT.md`.

## [0.141.0] — 2026-11-21

- Borrowed audio sub-blocks: Borrow checked frame and channel rectangles from planar or interleaved sample storage with zero copies.
- Detailed API, migration and validation: `docs/DSP_BLOCK_VIEW.md`.

## [0.140.0] — 2026-11-21

- Lookup-table transforms: Prepare bounded lookup transforms for custom DSP functions, measure interpolation error, and apply them as an allocation-free waveshaper.
- Detailed API, migration and validation: `docs/DSP_LOOKUP_TRANSFORM.md`.

## [0.139.0] — 2026-11-21

- Shared convolution update queue: Share a bounded background worker between reloadable convolution processors while preserving real-time crossfades and deferred kernel retirement.
- Detailed API, migration and validation: `docs/DSP_CONVOLUTION_UPDATE_QUEUE.md`.

## [0.138.0] — 2026-11-21

- Half-band filter design and custom oversampling: Design measured equiripple FIR or elliptic polyphase IIR half-band filters and prepare cascades with independent up/down specifications.
- Detailed API, migration and validation: `docs/DSP_HALF_BAND_DESIGN.md`.

## [0.137.0] — 2026-11-21

- Audio-rate feedback patching: Prepared sample-rate signal graphs with explicit delayed edges and zero-delay cycle rejection.
- Detailed API, migration and validation: `docs/DSP_FEEDBACK_PATCH.md`.

## [0.136.0] — 2026-11-21

- First-order Ambisonics: AmbiX source encoding, soundfield rotation and a SOFA headphone decoder.
- Detailed API, migration and validation: `docs/DSP_AMBISONICS.md`.

## [0.135.0] — 2026-11-21

- Tempo-synced live granular effects: Rolling history, freeze, and beat-scheduled grains on live input.
- Detailed API, migration and validation: `docs/DSP_GRANULAR_LIVE.md`.

## [0.134.0] — 2026-11-21

- Harmonic and percussive separation: Complementary, stereo-linked masks for tonal and transient layers.
- Detailed API, migration and validation: `docs/DSP_HPSS.md`.

## [0.133.0] — 2026-11-21

- Cabinet IR alignment and blending: Polarity-aware delay estimation and fractional alignment for convolution-ready cabinet IR blends.
- Detailed API, migration and validation: `docs/DSP_CABINET_BLEND.md`.

## [0.132.0] — 2026-11-21

- Live monophonic pitch correction: Scale-aware real-time tuning with a confidence gate, retune speed and aligned bypass.
- Detailed API, migration and validation: `docs/DSP_PITCH_CORRECTION.md`.

## [0.131.0] — 2026-11-21

- Phase-locked multi-mic sample voices: A shared take selection and playback clock across independently mixed microphone outputs.
- Detailed API, migration and validation: `docs/DSP_MULTIMIC_SAMPLER.md`.

## [0.130.0] — 2026-11-21

- NAM WaveNet playback: Classic mono NAM WaveNet inference with measured callback cost and upstream comparison.
- Detailed API, migration and validation: `docs/DSP_NAM_WAVENET.md`.

## [0.129.0] — 2026-11-21

- Loudness-targeted mastering renders: Offline LUFS analysis, true-peak-aware limiting and measured output reports.
- Detailed API, migration and validation: `docs/DSP_MASTERING.md`.

## [0.128.0] — 2026-11-21

- Reed and wind physical modelling: A pressure-driven reed and bore waveguide with breath and reed controls.
- Detailed API, migration and validation: `docs/DSP_REED_WAVEGUIDE.md`.

## [0.127.0] — 2026-11-21

- Binaural spatial rendering: SOFA HRTF import and prepared headphone rendering with directional interpolation.
- Detailed API, migration and validation: `docs/DSP_BINAURAL.md`.

## [0.126.0] — 2026-11-21

- Polyphonic pitch and onset analysis: Offline note tracking with harmonic peak selection, onset times and confidence.
- Detailed API, migration and validation: `docs/DSP_POLYPHONIC_ANALYSIS.md`.

## [0.125.0] — 2026-11-21

- Two-input spectral morphing: Prepared cross-synthesis with independently adjustable spectral envelope and fine structure.
- Detailed API, migration and validation: `docs/DSP_SPECTRAL_MORPH.md`.

## [0.124.0] — 2026-11-21

- Component-level circuit modelling: Reusable wave-digital resistor and capacitor components and a nonlinear diode overdrive.
- Detailed API, migration and validation: `docs/DSP_WAVE_DIGITAL.md`.

## [0.123.0] — 2026-11-21

- Multi-operator FM and phase modulation: Polyphonic MIDI voice engine with four operators, envelopes, selectable modulation routing and feedback.
- Detailed API, migration and validation: `docs/DSP_FM_OPERATORS.md`.

## [0.122.0] — 2026-11-21

- Impulse-response capture: Exponential sweep generation, wave capture recovery and convolution-ready IR export.
- Detailed API, migration and validation: `docs/DSP_IR_CAPTURE.md`.

## [0.121.0] — 2026-11-21

- Double-precision processing: Borrowed f64 buffers and native double-precision gain, FIR, IIR and linked dynamics processors.
- Detailed API, migration and validation: `docs/DSP_DOUBLE_PRECISION.md`.

## [0.120.0] — 2026-11-21

- Reusable DSP utilities: Frame-based peak and RMS followers, more dispatched block math, and a scoped denormal guard.
- Detailed API, migration and validation: `docs/DSP_UTILITIES.md`.

## [0.119.0] — 2026-11-21

- Fractional-delay interpolation modes: Integer, third-order Lagrange and first-order Thiran interpolation alongside the existing delay modes.
- Detailed API, migration and validation: `docs/DSP_INTERPOLATION.md`.

## [0.118.0] — 2026-11-21

- Real-input FFT and analysis windows: Prepared real-input transforms, calibrated one-sided magnitudes and configurable analysis windows.
- Detailed API, migration and validation: `docs/DSP_REAL_FFT.md`.

## [0.117.0] — 2026-11-21

- Unified oversampling: A common FIR and allpass IIR oversampling API from bypass through 16×, with quality presets and explicit latency.
- Detailed API, migration and validation: `docs/DSP_UNIFIED_OVERSAMPLING.md`.

## [0.116.0] — 2026-11-21

- Advanced FIR and IIR filter design: Specification-driven Butterworth, Chebyshev and elliptic cascades, plus weighted least-squares and half-band FIR design.
- Detailed API, migration and validation: `docs/DSP_ADVANCED_DESIGN.md`.

## [0.115.0] — 2026-11-21

- Processor composition and latency-aware mixing: Prepared chains, mono duplication and latency-aligned dry/wet mixing, with an automatable CLAP parallel compressor.
- Detailed API, migration and validation: `docs/DSP_COMPOSITION.md`.

## [0.114.0] — 2026-11-21

- Coupled physical instruments: Velocity-sensitive hammer and pluck excitation, multiple waveguide strings, a shared modal soundboard and bounded sympathetic response.
- Detailed API, migration and validation: `docs/DSP_COUPLED_MODELS.md`.

## [0.113.0] — 2026-11-21

- Neural amp model playback: A bounded mono NAM LSTM reader and prepared processor with model-rate validation, adjustable calibration and a reference comparison fixture.
- Detailed API, migration and validation: `docs/DSP_NEURAL_AMP.md`.

## [0.112.0] — 2026-11-21

- Streamed sampler articulations: An SFZ subset for paged WAV instruments with per-channel keyswitches, per-zone release rules and bounded first-page preload.
- Detailed API, migration and validation: `docs/DSP_STREAMED_ARTICULATIONS.md`.

## [0.111.0] — 2026-11-21

- Harmonic and residual spectral resynthesis: A prepared harmonic/residual STFT processor with frequency-guided partial tracking and separate harmonic and residual gain controls.
- Detailed API, migration and validation: `docs/DSP_RESYNTHESIS.md`.

## [0.110.0] — 2026-11-21

- Wavetable authoring: Offline PCM cycle extraction, periodic resampling, seam repair, phase alignment and band-limited bank compilation.
- Detailed API, migration and validation: `docs/DSP_WAVETABLE_AUTHOR.md`.

## [0.109.0] — 2026-11-21

- Hybrid polyphonic instrument: A playable wavetable and sample-layer voice engine with shared envelopes, per-voice filtering and modulation, deterministic voice stealing and preset serialization.
- Detailed API, migration and validation: `docs/DSP_HYBRID_VOICE.md`.

## [0.108.0] — 2026-11-21

- Audio restoration primitives: Learnable spectral noise reduction, harmonic hum notches and isolated-click repair for offline or prepared processing workflows.
- Detailed API, migration and validation: `docs/DSP_RESTORATION.md`.

## [0.107.0] — 2026-11-21

- TPDF dither and noise shaping: Deterministic per-channel TPDF and optional first-order error feedback for 16-bit and 24-bit integer PCM export.
- Detailed API, migration and validation: `docs/DSP_DITHER.md`.

## [0.106.0] — 2026-11-21

- General-purpose block DSP kernels: Checked planar and interleaved buffer views, reusable channel matrix mixing with the existing SIMD backend, and streaming gain ramps.
- Detailed API, migration and validation: `docs/DSP_BLOCK_KERNELS.md`.

## [0.105.0] — 2026-11-21

- Linear-phase EQ and crossover: A complementary three-band FIR crossover with exact delayed recombination and a reusable linear-phase equalizer.
- Detailed API, migration and validation: `docs/DSP_LINEAR_PHASE.md`.

## [0.104.0] — 2026-11-21

- Continuous-ratio streaming resampling: A bounded push/pull multichannel converter with smooth rate changes, interpolated phase/cutoff tables and explicit end-of-stream flushing.
- Detailed API, migration and validation: `docs/DSP_STREAM_RESAMPLE.md`.

## [0.103.0] — 2026-11-21

- Polyphase IIR oversampling: A prepared 2× polyphase allpass oversampler with bounded buffers, a high-rate callback and measured latency and aliasing tradeoffs.
- Detailed API, migration and validation: `docs/DSP_IIR_OVERSAMPLING.md`.

## [0.102.0] — 2026-11-21

- Modulatable fractional delay: A prepared multichannel delay line with linear and cubic interpolation, per-frame slew control and independently readable taps.
- Detailed API, migration and validation: `docs/DSP_FRACTIONAL_DELAY.md`.

## [0.101.0] — 2026-11-21

- FIR and Butterworth filter design: Kaiser-window FIR and Butterworth IIR designs, prepared multichannel filters, and magnitude, phase and group-delay analysis.
- Detailed API, migration and validation: `docs/DSP_FILTER_DESIGN.md`.

## [0.100.0] — 2026-11-21

- Prepared resampling kernels and profiling: Immutable polyphase sinc coefficient banks with checked SIMD dot evaluation, shared-table sampler integration, bounded memory/rate tradeoffs and reproducible accuracy/allocation/timing reports.
- Detailed API, migration and validation: `docs/DSP_OPTIMIZATION.md`.

## [0.99.0] — 2026-11-21

- Advanced sampler region playback: Single-voice resident sample regions with reverse and ping-pong playback, explicit wrap-overlap fades, seek/speed controls, loop release into tails and an offline bounded loop matcher.
- Detailed API, migration and validation: `docs/DSP_PLAYBACK.md`.

## [0.98.0] — 2026-11-21

- Flexible linked and keyed dynamics: Linked peak/RMS compressor, downward expander and upward dynamics with soft knee, hold, external key filtering, lookahead, aligned parallel mix and audio-owner telemetry.
- Detailed API, migration and validation: `docs/DSP_DYNAMICS.md`.

## [0.97.0] — 2026-11-20

- True-stereo convolution and hybrid reverberation: Four-path stereo convolution with explicit cross-channel IR routing and aligned dry blend, plus a parallel early-IR/FDN hybrid reverb with compensated timing.
- Detailed API, migration and validation: `docs/DSP_SPATIAL.md`.

## [0.96.0] — 2026-11-20

- Modelled pedal and amplifier signal chains: Oversampled implicit diode/RC processing with asymmetric clipping, supply-envelope sag, tone/DC filtering, parallel mix and an optional base-rate cabinet IR.
- Detailed API, migration and validation: `docs/DSP_AMP_MODELS.md`.

## [0.95.0] — 2026-11-20

- Waveguide strings and modal resonators: Prepared fractional-delay string voices with deterministic pick shaping and friction excitation, plus bounded modal resonator banks for instrument synthesis.
- Detailed API, migration and validation: `docs/DSP_PHYSICAL.md`.

## [0.94.0] — 2026-11-20

- Formant-aware pitch and quality stretch controls: Optional bounded log-spectral-envelope preservation and independent formant shifting, plus explicit pitch/stretch quality presets and allocation-checked synthetic audio examples.
- Detailed API, migration and validation: `docs/DSP_PITCH_QUALITY.md`.

## [0.93.0] — 2026-11-20

- Oversampled modulated oscillator engines: Adds reusable oversampled PWM, FM, phase modulation, phase warp, fractional hard sync and wavetable-morph sources with explicit generated-audio latency alignment and standalone spectral/audio examples.
- Detailed API, migration and validation: `docs/DSP_OSCILLATORS.md`.

## [0.92.0] — 2026-11-20

- Compiled modulation and efficient control updates: Adds a compiled modulation renderer with stable ID binding, owned normalized buffers, audio/control-rate modes, smoothed base-preserving routes, external envelope/CC/voice inputs, live source controls and bounded evaluation counters. Also refines nonlinear-filter drive continuity.
- Detailed API, migration and validation: `docs/DSP_MODULATION.md`.

## [0.91.0] — 2026-11-20

- Advanced nonlinear comb and formant filters: Adds independent nonlinear TPT state-variable filtering with bounded Newton feedback, damped fractional comb filtering and smoothed vowel/formant morphing, with high-rate preparation, checked controls and standalone audio examples.
- Detailed API, migration and validation: `docs/DSP_FILTERS.md`.

## [0.90.0] — 2026-11-20

- Anti-aliased nonlinear processing: Adds reusable ADAA hard clipping, tanh and cubic saturation, stable near-equal-input handling, smoothed controls, exact integer latency/padding, oversampling composition and coherent spectral/WAV comparisons against matched-filter naive processing.
- Detailed API, migration and validation: `docs/DSP_NONLINEAR.md`.

## [0.89.0] — 2026-11-20

- Reusable multi-rate oversampling: Adds independent 2x/4x/8x half-band FIR oversampling, quality settings, integer latency reporting, a generic AudioProcessor wrapper with MIDI/clock retiming, bounded prepared buffers and standalone audio examples.
- Detailed API, migration and validation: `docs/DSP_OVERSAMPLING.md`.

## [0.88.0] — 2026-11-20

- Studio plug-in crash isolation: Adds Studio stereo VST3 child hosting with explicit helper approval, preserved musical context, latency-preserving dry or silent fallback, verified recovery checkpoints, bounded persisted recovery attempts and guarded offline export.
- Detailed API, migration and validation: `docs/STUDIO_NATIVE_ISOLATION.md`.

## [0.87.0] — 2026-11-20

- Native sidechains and multiple outputs: Adds typed native bus requests, dependency-ordered pre-fader sidechain feeds, independent output-return tracks, VST3 live/offline bus adapters and Studio routing controls with negotiated layout validation.
- Detailed API, migration and validation: `docs/STUDIO_NATIVE_BUSES.md`.

## [0.86.0] — 2026-11-20

- Native editor automation: Adds reusable normalized native-editor gestures, VST3 controller and CLAP bracket adapters, grouped history, host-state reconciliation and Touch/Latch recording through Reference Studio.
- Detailed API, migration and validation: `docs/NATIVE_EDITOR_AUTOMATION.md`.

## [0.85.0] — 2026-11-20

- Native mixdown and integrated DAW workflows: Integrates trusted continuous native offline rendering into Studio export and freeze, captures post-strip, bus and return stems in one graph pass with source-specific latency trimming, and provides an end-to-end recording, MIDI recovery, portable session and delivery example.
- Detailed API, migration and validation: `docs/DAW_DELIVERY.md`.

## [0.84.0] — 2026-11-20

- Native plug-in and mixer automation: Adds stable normalized native plug-in controls to Studio read and Touch/Latch automation, bounded sample-timed CLAP/VST3 event adapters, explicit native automation diagnostics and automatable auxiliary returns.
- Detailed API, migration and validation: `docs/DAW_AUTOMATION.md`.

## [0.83.0] — 2026-11-20

- Studio VST3 and plug-in management: Connects trusted CLAP and musical VST3 hosting to Studio, adds supervised discovery, enabled-product caches and quarantine, missing-instance bypass and identity-checked repair, and verified portable native-state resources with history support.
- Detailed API, migration and validation: `docs/DAW_PLUGINS.md`.

## [0.82.0] — 2026-11-20

- Cue mixes and hardware outputs: Add four independent performer cue mixes with pre/post/bus/aux/main/raw input taps, source delay alignment, sample-weighted recording click, stable send automation identities and portable output pairs. Publish bounded multichannel banks through the native driver, negotiate physical channels, and provide a typed reference cue editor plus virtual routing/timing/allocation examples.
- Detailed API, migration and validation: `docs/DAW_CUES.md`.

## [0.81.0] — 2026-11-20

- Submix and sidechain routing: Add nested stereo submixes and master insert chains with per-edge delay compensation, smoothed gain/balance/mute/solo, peak meters and transactional routing drafts. Add current-block stock sidechain ducking with explicit latency constraints, routing-aware track operations, reference controls and measured allocation/timing examples.
- Detailed API, migration and validation: `docs/DAW_MIXING.md`.

## [0.80.0] — 2026-11-20

- Transient and grouped elastic editing: Detect and grid-snap reference transients, render linked mono/stereo microphone groups through one shared-decision multichannel WSOLA stream, apply verified caches atomically with undo/redo, and persist group recipes with portable projects. Add the reference group editor and four-microphone validation example.
- Detailed API, migration and validation: `docs/DAW_ELASTIC.md`.

## [0.79.0] — 2026-11-19

- Stock instruments effects and media browser: Connect 32 automated stock effects, four synth presets and an asset-backed sampler to Studio; add worker media browsing/import/audition, portable one-processor presets, stock insert edits and actual-latency playback/recording preparation.
- Detailed API, migration and validation: `docs/DAW_TOOLS.md`.

## [0.78.0] — 2026-11-19

- Arrangement and musical timeline workflow: Play from musical cursor, guarded tempo/meter edits, portable markers and song sections, and nondestructive multi-region/section duplication with phase and automation-curve preservation.
- Detailed API, migration and validation: `docs/DAW_ARRANGEMENT.md`.

## [0.77.0] — 2026-11-19

- Expressive MIDI clips and controller editing: Portable editable MIDI CC/bend/pressure points play with deterministic resets and bounded seek/sustain chase; loop projections retain expression and the reference inspector provides transactional controller editing.
- Detailed API, migration and validation: `docs/DAW_MIDI.md`.

## [0.76.0] — 2026-11-19

- Grouped audio and loop MIDI takes: Phase-locked multi-stem comp groups, selectable callback-clock MIDI loop passes, and conservative interrupted-audio salvage complete the implemented recording workflow series with portable take selection and undo.
- Detailed API, migration and validation: `docs/DAW_TAKES.md`.

## [0.75.0] — 2026-11-19

- Timestamped input and recording calibration: Backend MIDI timestamps reach bounded intra-block scheduling; explicit impulse/manual calibration applies nondestructive audio and MIDI recording compensation with portable per-take settings.
- Detailed API, migration and validation: `docs/DAW_TIMING.md`.

## [0.74.0] — 2026-11-19

- Independent multichannel recording: Independent hardware-channel routes feed track monitors and native-width recording stems; gap-free linear dry recordings place automatically as editable regions, with input-quality metadata and portable routes.
- Detailed API, migration and validation: `docs/DAW_MULTICHANNEL.md`.

## [0.73.0] — 2026-11-19

- Durable in-place sessions and portable performances: Repeated background Save commits verified immutable project generations; portable raw MIDI performances and bounded pending MIDI/automation journals support explicit crash-prefix recovery.
- Detailed API, migration and validation: `docs/DAW_SESSIONS.md`.

## [0.72.0] — 2026-11-19

- DAW project and track workspace: Blank-project startup, transactional track lifecycle/folders, schema-3 layout persistence and editing any selected MIDI clip in the studio.
- Detailed API, migration and validation: `docs/DAW_WORKSPACE.md`.

## [0.71.0] — 2026-11-19

- Studio MIDI performance recording: Bounded callback-clock MIDI 1 capture, raw take/SMF export, quantized editable note clips and undoable prepared insertion; reference-studio native input and on-screen keyboard controls.
- Detailed API, migration and validation: `docs/STUDIO_MIDI_RECORDING.md`.

## [0.70.0] — 2026-11-19

- Media audit and project consolidation: Conservative media dependency audit and verified consolidated portable copies, stale-plan rejection, freeze/warp dependency retention and preserved original files.
- Fixed completed scheduler tickets retaining replacement keys after their results were consumed.
- Fixed streamed recording startup by priming disk pages at the selected recording start; added `Document::compile_streamed_at` for stopped preparation at a chosen position.
- Detailed API, migration and validation: `docs/STUDIO_MEDIA_AUDIT.md`.

## [0.69.0] — 2026-11-19

- Studio Touch and Latch automation recording: Studio Touch/Latch automation writing through live Gesture controls, isolated pending takes, snapped control-rate capture and transactional undoable publication.
- Detailed API, migration and validation: `docs/STUDIO_AUTOMATION_WRITE.md`.

## [0.68.0] — 2026-11-19

- Export sample-rate conversion: Bounded streaming sinc export conversion at 8..192 kHz, preserved duration/pitch and destination-rate analysis/normalization.
- Detailed API, migration and validation: `docs/EXPORT_CONVERSION.md`.

## [0.67.0] — 2026-11-19

- Native plug-in windows in the studio: Studio-owned/floating CLAP editor windows, focus/resize/scale controls, identity-bound reopen preferences and ordered main-thread teardown/checkpointing.
- Detailed API, migration and validation: `docs/STUDIO_PLUGIN_WINDOWS.md`.

## [0.66.0] — 2026-11-19

- Studio routing editor: Stopped routing matrix for track sends, pre/post taps, auxiliary returns and inserts, transactional preparation and correct automation/control remapping on removal.
- Detailed API, migration and validation: `docs/STUDIO_ROUTING.md`.

## [0.65.0] — 2026-11-19

- Studio export centre: Shared-job mix and pre-mixer stem export, ranges/tails, streamed codec encoding, peak/LUFS analysis, normalization and deterministic integer dither.
- Detailed API, migration and validation: `docs/STUDIO_EXPORT.md`.

## [0.64.0] — 2026-11-19

- Shared background-job scheduler: Bounded typed background scheduler shared by waveform, warp and built-in freeze work, with priorities, cancellation, progress, memory reservations and stale-result rejection.
- Detailed API, migration and validation: `docs/STUDIO_JOBS.md`.

## [0.63.0] — 2026-11-19

- Waveform and warp-marker editor: Waveform zoom, draggable source markers and target-beat controls, background warp rendering, verified transactional application and global undo/redo.
- Detailed API, migration and validation: `docs/WAVEFORM_WARP_EDITOR.md`.

## [0.62.0] — 2026-11-19

- Complete studio recording controls: Prepared per-track arming and dry/rendered sources, input-monitor controls, callback-clock count-in, pre-roll, exact punch and monitor-only metronome.
- Detailed API, migration and validation: `docs/STUDIO_RECORDING.md`.

## [0.61.0] — 2026-11-19

- Studio project recovery and migration: Background portable studio checkpoints with bounded retention, corrupt-latest fallback into independent working copies, stable project identity/schema-1-to-2 migration, exact-checksum missing-media repair and reference-studio recovery controls.
- Detailed API, migration and validation: `docs/STUDIO_RECOVERY.md`.

## [0.60.0] — 2026-11-19

- Studio-wide undo and redo: One bounded project history covers arrangement, mixer, automation, routing, comp/freeze resources and stopped native checkpoints, with grouped gestures, shared immutable PCM, failed-preparation rollback and reference-studio global shortcuts.
- Detailed API, migration and validation: `docs/STUDIO_HISTORY.md`.

## [0.59.0] — 2026-11-18

- Fixed a streamed-freeze page-boundary dropout: skip zero-weight interpolation reads, preflight full lookahead and fail offline renders on observed disk underruns.

- Dynamic latency and bus reconfiguration: Stopped, staged native topology replacement preserves latest state, validates explicit bus/channel routes and latency budgets, prepares complete PDC graphs before revision-checked commit, and leaves retired callbacks silent.
- Detailed API, migration and validation: `docs/DYNAMIC_RECONFIGURATION.md`.

## [0.58.0] — 2026-11-18

- Richer isolated plug-in control: Isolated CLAP UI parameters, sample-offset automation and timestamped output MIDI/parameter/gesture events, with monotonic packet matching and continuous helper main-thread timer/FD/callback services.
- Detailed API, migration and validation: `docs/ISOLATED_CONTROL.md`.

## [0.57.0] — 2026-11-18

- Streaming-capable freeze workflow: Typed offline streamed rendering removes the memory-only freeze limit. Bounded page waits, monitored cancellable worker jobs, source-change rejection, verified publication and reference-studio progress/cancel controls preserve reversible project freezes.
- Detailed API, migration and validation: `docs/STREAMING_FREEZE.md`.

## [0.56.0] — 2026-11-18

- Tempo-aware streamed audio: Bounded worker-side warp caches connect mixed-rate WAV sources to the tempo map, with pitch-preserving WSOLA or repitch, exact marker durations, source checksums, cancellation and portable project bindings.
- Detailed API, migration and validation: `docs/TEMPO_STREAMING.md`.

## [0.55.0] — 2026-11-18

- Advanced nondestructive audio editing: Shared memory/disk region split and compatible join, source slip, linear/equal-power crossfades, gain envelopes and linked editing, with undo and portable metadata.
- Detailed API, migration and validation: `docs/AUDIO_EDITING.md`.

## [0.54.0] — 2026-11-18

- Loop recording and take comping: Sample-exact loop/transport annotations carried with recorded PCM, immutable latency-aligned pass lanes, transactional range comp recipes with crossfades, bounded-memory WAV export, reusable take-lane UI and portable comp-track integration.
- Detailed API, migration and validation: `docs/TAKE_COMPING.md`.

## [0.53.0] — 2026-11-18

- Scalable arrangement and piano-roll editors: Reusable virtualized arrangement and full 128-key piano-roll surfaces with both-axis scrolling/zoom, independently resized track rows, marquee and multiple selection, grouped validated moves/deletes and undo/redo, plus saved viewport state and reference-studio integration.
- Detailed API, migration and validation: `docs/SCALABLE_EDITORS.md`.

## [0.52.0] — 2026-11-18

- Unified reference studio: A unified desktop studio template with live session playback, reusable portable project folders, arrangement/piano-roll and automation editors, stem recording, reversible freeze, parameter mixing and explicitly trusted CLAP insert ownership/state capture.
- Detailed API, migration and validation: `docs/REFERENCE_STUDIO.md`.

## [0.51.0] — 2026-11-18

- Native PipeWire integration: Optional native Linux PipeWire DSP filter backend with named per-channel input/output ports, explicit non-lingering graph links, bounded registry snapshots, prepared callback buffer splitting and driver/processing timing telemetry; includes a private software-server acceptance probe.
- Detailed API, migration and validation: `docs/PIPEWIRE.md`.

## [0.50.0] — 2026-11-18

- Accessible audio controls: AccessKit semantics and assistive value actions for custom audio controls, keyboard clip/note editing with accessible object lists, independent XY-axis controls, descriptive mixer labels and enabled eframe platform adapters.
- Detailed API, migration and validation: `docs/ACCESSIBILITY.md`.

## [0.49.0] — 2026-11-18

- Broader plug-in isolation: Explicit CLAP helper-process hosting for main/auxiliary input-output ports, a timestamped callback bridge, bounded RPC timeouts, opaque checkpoints and fresh-process recovery with a restart budget; existing VST3 isolation remains available.
- Detailed API, migration and validation: `docs/EXPANDED_ISOLATION.md`.

## [0.48.0] — 2026-11-18

- Multicore graph processing: Dependency-level DSP worker execution with fixed node ownership, deterministic fan-in and existing delay compensation, exposed through a bounded timestamped callback bridge with explicit latency, silence-on-miss and atomic deadline telemetry.
- Detailed API, migration and validation: `docs/PARALLEL_GRAPH.md`.

## [0.47.0] — 2026-11-18

- Track freeze and bounce-in-place: Control-side pre-mixer stem rendering with latency trimming, explicit tails, SHA-256 source/cache validation, processor-bypassing frozen playback, reversible restoration and bounce-in-place document generation.
- Detailed API, migration and validation: `docs/TRACK_FREEZE.md`.

## [0.46.0] — 2026-11-18

- Multitrack recording and takes: Synchronized float32 track, master and input recording, bounded pre-roll, punch intervals, independent arming, immutable take manifests and non-destructive take selection, with live-session capture and off-callback retirement.
- Detailed API, migration and validation: `docs/MULTITRACK_RECORDING.md`.

## [0.45.0] — 2026-11-18

- Disk-streamed arrangement clips: Disk-backed arrangement regions with bounded independent caches/workers, source trims, loops/fades, seek-generation protection, explicit prefetch/telemetry and unified-session playback integration.
- Detailed API, migration and validation: `docs/STREAMED_CLIPS.md`.

## [0.44.0] — 2026-11-18

- Unified live session engine: Shared live track/clip/instrument/insert/mixer/aux/automation playback, bounded transport commands, latency alignment, parameter snapshots, existing recorder integration and prepared crossfaded project swaps.
- Detailed API, migration and validation: `docs/LIVE_SESSIONS.md`.

## [0.43.0] — 2026-11-18

- Improved streaming pitch and time processing: Opt-in prepared phase-locked/transient-aware pitch shifting and stereo-linked streaming WSOLA time stretching with bounded work, explicit backpressure/finish/drain, exact durations and audible comparison renders.
- Detailed API, migration and validation: `docs/PITCH_TIME.md`.

## [0.42.0] — 2026-11-18

- Audio-quality regression suite: Deterministic offline distortion/noise/IMD/alias/response/stability measurements, versioned regression thresholds and a twelve-probe executable suite with failure exit codes and release-workflow integration.
- Detailed API, migration and validation: `docs/DSP_QUALITY.md`.

## [0.41.0] — 2026-11-18

- Optimized DSP backends: Safe prepared scalar/SSE2/AVX2/ARM64-NEON gain, mix, dot and interleave kernels with tail/unaligned handling, existing Gain integration and a repeatable release microbenchmark.
- Detailed API, migration and validation: `docs/DSP_KERNELS.md`.

## [0.40.0] — 2026-11-18

- Hardware controller integration: Prepared controller banks, relative encoder modes, soft takeover, toggles and coalesced MIDI feedback with bounded-queue retries and sent-echo suppression, demonstrated through a virtual MIDI output worker.
- Detailed API, migration and validation: `docs/CONTROLLERS.md`.

## [0.39.0] — 2026-11-18

- Microtuning support: Validated absolute tuning tables, bounded Scala SCL/KBM imports, synth/sampler frequency integration, unmapped-key handling and optional project persistence, with an audible just-intonation demo.
- Detailed API, migration and validation: `docs/MICROTUNING.md`.

## [0.38.0] — 2026-11-18

- Sample-instrument authoring tools: Sample filename/velocity/round-robin mapping, monophonic root estimation, peak-normalized copies, zero-crossing/trim/loop helpers and playable native-project/SFZ exports with sequence roundtrips.
- Detailed API, migration and validation: `docs/SAMPLE_AUTHORING.md`.

## [0.37.0] — 2026-11-18

- Reusable mixer toolkit: Prepared stereo channel strips with smoothed gain/pan, mute/solo, groups, pre/post sends, sample-offset automation, atomic meters and reusable UI controls with a stem-rendering example.
- Detailed API, migration and validation: `docs/MIXER.md`.

## [0.36.0] — 2026-11-17

- Application command framework: Stable application commands, validated configurable keymaps, shared menu/toolbar/shortcut adapters and bounded transactional undo/redo, integrated into the clip editor.
- Detailed API, migration and validation: `docs/COMMANDS.md`.

## [0.35.0] — 2026-11-17

- Audio and MIDI clip toolkit: Validated audio/MIDI clip editing, looped frame/beat playback, seek recovery, latency alignment and reusable arrangement/piano-roll views, with a native editor and WAV rendering demo.
- Detailed API, migration and validation: `docs/CLIPS.md`.

## [0.34.0] — 2026-11-17

- Waveform thumbnail cache: Streaming multiresolution waveform peaks, fingerprinted atomic disk caches with corruption repair/eviction, latest-request background loading/cancellation, bounded PCM detail reads and a cached native waveform view.
- Detailed API, migration and validation: `docs/WAVEFORM_CACHE.md`.

## [0.33.0] — 2026-11-17

- Themeable audio UI toolkit: Serializable audio UI themes, normalized knobs/faders with fine and stepped interaction, modulation rings, typed toggles/enums, prepared SVG and filmstrip assets, and a native theme gallery with framebuffer snapshots.
- Detailed API, migration and validation: `docs/UI_TOOLKIT.md`.

## [0.32.0] — 2026-11-17

- Typed parameter SDK: Typed float/integer/bool/choice/Rust-enum declarations, shared normalization/text/step metadata and editor controls, macro-generated parameter sets and revision-checked snapshots, with a real exported typed effect.
- Detailed API, migration and validation: `docs/TYPED_PARAMETERS.md`.

## [0.31.0] — 2026-11-17

- Platform validation and release tooling: Complete schema2 staged-release inventories, ELF/PE/Mach-O architecture and checksum auditing, manifest-specific target-machine acceptance, and a read-only cross-platform native export evidence workflow.
- Detailed API, migration and validation: `docs/PLATFORM_RELEASES.md`.

## [0.30.0] — 2026-11-17

- Desktop editor integration: Owned CLAP HWND/AppKit parents with ordered GUI teardown, deferred Windows close/resize/DPI handling and desktop focus; fixes native VST3 flag ABI casts and validates Windows/macOS host code by cross-target Clippy.
- Detailed API, migration and validation: `docs/DESKTOP_EDITORS.md`.

## [0.29.0] — 2026-11-17

- Plug-in crash isolation: Opt-in VST3 child-process hosting with bounded response watchdog, explicit checkpoint/parameter recovery, permanently revoked stale callbacks and per-owner recovery quarantine.
- Detailed API, migration and validation: `docs/PLUGIN_ISOLATION.md`.

## [0.28.0] — 2026-11-17

- Multichannel hosting and delay compensation: Explicit mono/stereo/multichannel bus DAGs with per-channel feed-forward latency compensation, native VST3 sidechain/auxiliary activation and a bounded multiple-bus worker bridge.
- Detailed API, migration and validation: `docs/HOST_CHANNELS.md`.

## [0.27.0] — 2026-11-17

- VST3 musical integration: Prepared tempo/meter/loop/punch context, expressive note identities, bounded SysEx/program ingress, timed native output and callback-offset normalized automation for buffered VST3 instruments and mixed sessions.
- Detailed API, migration and validation: `docs/VST3_MUSICAL.md`.

## [0.26.0] — 2026-11-17

- Continuous offline rendering: Continuous scoped VST3/CLAP offline processing, tail-inclusive rendering, latency trimming, cancellation and transactional aligned WAV stem export.
- Detailed API, migration and validation: `docs/HOST_RENDERING.md`.

## [0.25.0] — 2026-11-17

- Unified plug-in graphs and sessions: Versioned mixed native/CLAP/VST3 graph sessions with explicit module trust, compensated stereo routing, opaque-state capture, aligned bypass and preserved missing nodes.
- Detailed API, migration and validation: `docs/MIXED_SESSIONS.md`.

## [0.24.0] — 2026-11-17

- Direct VST3 processing: Prepared native f32 stereo VST3 audio callbacks with zero worker bridge delay, bounded normalized controls and owner-thread lease revocation; buffered hosting remains available.
- Detailed API, migration and validation: `docs/VST3_DIRECT.md`.

## [0.23.0] — 2026-11-17

- VST3 hosting: Common CLAP/VST3 product controls, trusted factory scanning, native VST3 state and timed parameters, a bounded stereo audio/MIDI worker bridge, native editors and a workbench VST3 panel.
- Detailed guide: `docs/VST3_HOSTING.md`; separate release notes and validation evidence.

## [0.22.0] — 2026-11-17

- Expanded hosted editors: Native parent attachment, owned X11 editor windows, resize and scale negotiation, stable reopening, persisted editor preferences and bounded CLAP timer/POSIX FD host services.
- Detailed guide: `docs/HOSTED_EDITORS.md`; separate release notes and validation evidence.

## Unreleased

No changes yet.

## 0.21.0 — 2026-11-17

Analytic tempo steps/ramps, PPQ and meter/bar positions, loop/punch metadata, persisted project timelines, mapped automation and native CLAP musical transport.

See [guide](docs/MUSICAL_TRANSPORT.md) and [release notes](docs/releases/v0.21.0.md).

## 0.20.0 — 2026-11-17

Bounded full MIDI 1 wire packets and SysEx, explicit driver-clock mapping, allocation-free sample queues, scheduled native/virtual output workers and persisted CC learning.

See [guide](docs/MIDI_IO.md) and [release notes](docs/releases/v0.20.0.md).

## 0.19.0 — 2026-11-17

A common optional libsndfile-backed WAV/AIFF/FLAC API with encoding, seekable decoding, metadata, bounded asset loading and project/bundle integration; portable WAV APIs remain available.

See [guide](docs/AUDIO_FORMATS.md) and [release notes](docs/releases/v0.19.0.md).

## 0.18.0 — 2026-11-17

Named multichannel layouts, exact negotiation, explicit auxiliary/sidechain channel patching and scoped native multi-port CLAP processing with advertised configuration selection.

See [guide](docs/FLEXIBLE_BUSES.md) and [release notes](docs/releases/v0.18.0.md).

## 0.17.0 — 2026-11-17

Versioned native rack sessions with opaque state/checksums, parameter fallback, latency-aligned bypass, floating editor preferences and preserved missing plug-in slots.

See [guide](docs/HOST_SESSIONS.md) and [release notes](docs/releases/v0.17.0.md).

## 0.16.0 — 2026-11-17

Direct trusted CLAP callbacks with zero bridge delay, safe owner revocation, configurable buffered lookahead, deadline counters and offline impulse alignment.

See [guide](docs/LOW_LATENCY.md) and [release notes](docs/releases/v0.16.0.md).

## 0.15.0 — 2026-11-17

Structured compatibility reports, child-process CLAP probes, repeated exported ABI reopening, workflow/audit coverage, source ZIP hash verification, target-machine acceptance records and stricter performance regression input checks.

See [guide](docs/COMPATIBILITY_SUITE.md) and [release notes](docs/releases/v0.15.0.md).

## 0.14.0 — 2026-11-17

A main-thread-owned serial CLAP rack with a bounded stereo callback bridge, live parameter controls, output-event observation, floating-editor lifecycle and workbench device integration.

See [guide](docs/LIVE_CLAP_RACK.md) and [release notes](docs/releases/v0.14.0.md).

## 0.13.0 — 2026-11-17

Editable beat lanes with four curve types, snapping, looping, Touch/Latch recording, prepared playback and project persistence.

See [guide](docs/AUTOMATION.md) and [release notes](docs/releases/v0.13.0.md).

## 0.12.0 — 2026-11-17

Added shared input/output selection and channel maps, stereo duplex processing/MIDI delivery, muted-by-default monitoring, managed recording and coordinated close/reconnect with retained take errors.

See [guide](docs/UNIFIED_AUDIO.md) and [release notes](docs/releases/v0.12.0.md).

## 0.11.0 — 2026-11-17

Added velocity crossfades, sustain-aware release samples, energy-onset slicing, a strict scoped SFZ subset importer and worker-backed sample-map integration.

See [feature guide](docs/ADVANCED_SAMPLER.md) and [release notes](docs/releases/v0.11.0.md).

## 0.10.0 — 2026-11-17

Added searchable preset catalogs, tags, favourites, banks, background-rendered previews and audible audition, missing-asset checks, scoped relocation and portable preset export.

See [feature guide](docs/PRESET_BROWSER.md) and [release notes](docs/releases/v0.10.0.md).

## 0.9.0 — 2026-11-17

See [release notes](docs/releases/v0.9.0.md), [device management](docs/DEVICE_MANAGER.md) and [custom DSP](docs/REGISTRY.md).

- Added managed output selection/preferences, configuration negotiation, reconnect/fallback policy and reusable settings UI.
- Added custom DSP registrations, parameter/editor factories, versioned state migration and registry-aware project loading/restore.
- Added workbench Audio tab, registered processor menu, Tape Colour crate/preset and SDK walkthrough.
- Kept existing product IDs and built-in project schema compatible; RackNode Rust literals gain an optional custom field.

## 0.8.0 — 2026-11-17

See [release notes](docs/releases/v0.8.0.md) and [workflow guide](docs/PROJECT_WORKFLOWS.md).

- Added bounded single-file project bundles with checksums and no-replace publication.
- Added immutable recovery snapshots, retention/fallback and background opt-in autosave.
- Added bounded asynchronous resource loading with latest-request/cancellation semantics.
- Integrated project workflow controls and a portable sampler/recovery CLI walkthrough.

## 0.7.0 — 2026-11-17

See [release notes](docs/releases/v0.7.0.md) and [migration](docs/MIGRATION_070.md).

- Added per-voice modulation and Poly Motion; graphical/live routing with warmup,
  fixed latency, note seeding and deferred retirement; sample-map regions/fades/slices.
- Added bounded MIDI performance/sequence/SMF tools, trusted scoped CLAP hosting,
  developer stress/state/heap diagnostics and an independent host-contract fixture.
- Added native panels, six presets, four CLAP products and SDK/audio walkthroughs.
- Fixed identified zero-velocity release and rejected runnable graphs without outputs.
- Recovered the workspace from the saved 0.6 archive and reran verification.

## 0.6.0 — 2026-11-16

See [release notes](docs/releases/v0.6.0.md) and [migration](docs/MIGRATION_060.md).

- Added global tempo/envelope/macro/MIDI modulation with separate base values.
- Added multiband dynamics/saturation, dynamic EQ, fixed-pool granular synthesis,
  freeze/robot/pitch/frequency effects and offline phase-vocoder stretch.
- Added diagonal stereo/non-uniform convolution, configurable latency and bounded
  IR crossfades with control-side engine retirement.
- Added native DSP Workbench, four project presets, asset/export/undo workflows,
  live/preview analysis, two CLAP products and rendered audio examples.
- Added allocation/behavior/ABI regressions, native virtual-display capture and
  reproducible current-vs-0.5 CPU fixtures; optimized settled control processing.

## 0.5.0 — 2026-11-16

See the [release notes](docs/releases/v0.5.0.md) and [DSP guide](docs/DSP_ADVANCED.md).

- Added advanced filters, two/three-way LR4 crossovers, RMS/lookahead/transient
  dynamics, FFT/STFT tools and spectral gating, wavetable/PWM/sync/unison,
  FDN reverberation, creative modulation/delays and analysis/stereo utilities.
- Added sixteen automatable effect kinds and schema-2 project nodes, a wavetable
  project source, DSP Lab CLAP/standalone examples, audio/stems and measurements.
- Added behavioral, spectral, latency, state and allocation regressions and
  independent exported DSP Lab smoke checks. Preserved earlier release evidence.

## 0.4.0 — 2026-11-16

See the [release notes](docs/releases/v0.4.0.md) for migration and limitations.

- Added schema-2 project documents, asset/sample-map/rack/routing reconstruction,
  bounded undo/redo, safe asset resolution and opt-in full plug-in state.
- Added ten live-effect kinds with named parameters and sample-based smoothing.
- Added windowed-sinc quality modes, offline file resampling and sampler integration.
- Added seekable/looping transport and paged disk-backed voices with fixed caches,
  prefetch, generation-safe seeks, PCM budgets and underrun/error telemetry.
- Added expressive synthesis, explicit MIDI-1 MPE, channel/poly pressure, CC74,
  CLAP note identities and per-note tuning/pressure/timbre/pan/volume routing.
- Added Project Rack CLAP and hardware-free ecosystem examples, ABI/state/spectral/
  allocation tests, module guides and release notes. Maintained earlier release
  notes and original validation/benchmark evidence.

## 0.3.0 — 2026-11-15

Added a scoped CLAP denormal guard, processing optimizations and reproducible v0.2/v0.3 benchmarks; host-parented egui editors with bounded begin/value/end host gestures and explicit backpressure; real CLAP sidechain ports; an independent exported-ABI compatibility/heap/GUI interaction host; parametric EQ, compressor, peak limiter, gate, chorus, reverb, partitioned FFT convolution and oversampled drive; multisample banks, velocity layers, round robin, sustain/pitch bend and crossfaded stealing; bounded background forward WAV streaming; a four-product reference plug-in bundle and a standalone drum app sharing its factory; pinned static VST3/AUv2 wrapper integration; app/effect/sampler templates, CLI checks, package/installer recipes, license collection and expanded documentation.

The shared UI stack moves to egui/eframe 0.36.1. Audio preparation supports finite rates up to 1,536,000 Hz, including the VST3 validator’s extreme-rate case; actual devices still negotiate their own supported rates. Embedded Linux UI uses a pinned baseview backend and an opaque 2D GL framebuffer configuration. Existing gain/synth product and parameter IDs are retained.

Linux verification is recorded in docs/VALIDATION.md. AUv2/macOS/Windows compilation/runtime, real DAW project tests, physical loopback/unplug/replug, signing/notarization and Windows/macOS installer execution remain pending. Multisampler PCM is preloaded; the separate stream transport is forward-only and not a per-voice disk sampler. No publishing or installation is performed automatically.

## 0.2.0 — 2026-11-15

Added native duplex input and channel maps; validated sample-rate/buffer requests; saved session settings and control-thread recovery; monitored/bypassed live effects; dry/processed background stereo PCM16 recording with loss accounting and duration preservation; exact parameter-event scheduling and offline automation; separate main/sidechain bus contracts and a sidechain compressor; graph latency compensation; suspend/resume; a real CLAP adapter with gain/synth exports, state, parameters, ports and lifecycle; a CLAP project template and bundle helper; reusable knobs, XY, waveform, keyboard, envelope/EQ controls and meter ballistics; Live FX and UI gallery examples; expanded integration/allocation/recording tests and upstream CLAP validation.

Native hardware testing and interactive DAW validation remain pending. No embedded CLAP GUI, VST3/AU, CLAP sidechain port, native installers or JUCE feature-parity claim.

## 0.1.0 — 2026-11-15

Initial foundation: core processing, DSP, synth/sampler, serial chains and DAG routing, output host, MIDI, egui controls, WAV I/O, JSON presets, format-independent plug-in lifecycle and app generator. Included desktop synth and offline render examples.
