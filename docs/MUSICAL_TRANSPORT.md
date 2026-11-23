# Shared musical transport and tempo maps (0.21)

`soniclaw::transport` / `soniclaw-transport` provides a shared musical clock for native applications: PPQ (quarter-note beat) position, tempo steps/ramps, meter/bar position, sample-accurate loop boundaries and punch-window metadata. Internal project automation can now use a persisted tempo map rather than assuming current tempo multiplied by absolute sample time.

## Workbench workflow

1. Open **Transport**. The draft initially contains 120 BPM and 4/4 at beat zero.
2. Add tempo points, edit beats/BPM and choose Step or Ramp. Ramp means BPM changes linearly **over beats** until the next point. A final point holds its BPM.
3. Add meter points at bar boundaries, and edit numerator/denominator. Denominators must be powers of two. Invalid drafts are shown and cannot replace the applied project.
4. Enable/edit Loop and Punch ranges in quarter-note beats. Loop wraps project musical time. Punch is an eligibility window exposed by the shared Clock; the existing workbench WAV recorder is not automatically armed/gated by this checkbox.
5. **Apply musical timeline** validates/rebuilds through prepared graph publication. The persisted project map becomes authoritative for its processing tempo; the ordinary top-bar tempo value no longer defines that project's tempo. Add automation lanes to hear values follow the map. The automation draft recorder cursor also uses the mapped beat, while retaining GUI telemetry timing granularity.
6. **Release notes on loop** emits AllNotesOff before new same-frame MIDI at wrap. It avoids indefinitely held notes but does not replay a MIDI clip or automatically retrigger an audition chord. Disable it only when the application supplies its own note continuity policy.
7. Save/bundle/autosave to preserve the applied timeline. Reload timeline discards the draft; Disable project tempo map returns to ordinary incoming host/device transport. Reload drafts after changing presets/Undo, as with the other structural editors.

This is an opt-in internal project timeline. It does not import a DAW's full tempo map or claim host PPQ synchronization automatically. Applications can use the Clock directly for sequencers, synced delays, metronomes and scheduling. Tempo-synced durations should be calculated from two beat-to-sample positions if the interval crosses a ramp, rather than using one BPM for its whole length.

## Runnable demonstration

```sh
cargo build -p soniclaw-sdk-tools -p soniclaw-host-fixture
cargo run -p soniclaw-sdk-tools -- --transport-demo \
  target/debug/libsoniclaw_host_fixture.so /tmp/new-transport-demo
cargo test -p soniclaw-transport
```

The demo builds a 60→120 BPM ramp, changes 4/4 to 7/8 at a bar boundary, loops a region, persists the project and renders six seconds of gain automation with 257-frame blocks. It independently computes the expected final mapped parameter and compares it with the processed project. It also calls the native CLAP musical transport API through the independent fixture. It writes project JSON, WAV and timing evidence. Tests check analytic inverses, meter validation, loop/punch sample boundaries, allocation-free clock use and block-partition-invariant loop processing. See [verification](validation/v021/REPORT.md).

## Document and timing API

`Timeline { schema: 1, tempo, meters, loop_range, punch_range, panic_on_loop }` is serde data. TempoPoint has beat/BPM/Transition; MeterPoint has beat/numerator/denominator. Both lists start at beat zero and are strictly ordered. Beats are quarter notes, including in 7/8: one 7/8 bar is 3.5 quarter notes. Bar indices in Position are **zero-based**; `beat_in_bar` is also a quarter-note quantity. A UI displaying denominator-relative beat numbers should divide by `4 / denominator` and add one.

```rust
use soniclaw::transport::{Timeline, TempoPoint, Transition, TempoMap, Clock};
let timeline = Timeline {
    tempo: vec![
        TempoPoint { beat: 0.0, bpm: 60.0, transition: Transition::Linear },
        TempoPoint { beat: 4.0, bpm: 120.0, transition: Transition::Step },
    ],
    ..Default::default()
};
let map = TempoMap::compile(&timeline)?;
let sample = map.sample_at(4.0, 48_000.0)?;
let beat = map.beat_at_sample(sample, 48_000.0)?;
let mut clock = Clock::new(&timeline, 48_000.0)?;
clock.set_playing(true);
clock.seek(sample)?;
let frame = clock.frame()?;
let legacy_context = frame.core_transport();
clock.advance(256)?;
# Ok::<(), soniclaw::core::Error>(())
```

TempoMap compiles immutable segment/meter caches off-thread. `seconds_at`, `beat_at_seconds`, `sample_at`, `beat_at_sample` and `position` are allocation-free queries. Sample conversion rounds to the nearest sample. Loops/punch boundaries therefore have at most half-sample quantization relative to ideal beat time, and a range must span at least one sample at the selected rate.

For a ramp `bpm(b) = bpm0 + slope × (b - b0)`, seconds are integrated analytically as `60/slope × ln(bpm(b)/bpm0)`; the inverse uses expm1. Step/near-zero slope uses constant-tempo conversion. This avoids accumulating a block-sized tempo approximation. Meter changes must occur after a whole number of bars in the preceding meter; partial-bar meter changes are deliberately rejected rather than ambiguously renumbered.

Clock owns an Arc map, sample position, play state and sample-domain loop/punch ranges. `frame()` exposes exact mapped Position, playing/punching/looped and beat/second loop ranges. `advance` moves only while playing and accepts up to 8,192 frames; arithmetic/range errors return Error. `seek` validates and, when seeking beyond an active loop end, wraps into its region. Clock can advance a whole block for metadata-only clients, or one frame for exact musical processing. If several wraps occur in one advance, looped indicates at least one; per-sample use observes each boundary.

Punching is true while playing inside the configured punch interval; with no punch interval it is true for the whole playing timeline. It is **not** a recording-armed flag. Consumers must combine it with their own record-enable state. No audio is removed from a recorded file implicitly. No MIDI clip arrangement, clock-master/slave PLL or external timecode reader is added in this release.

## Engine, automation and native-host integration

`TimelineProcessor::new(inner, timeline)` implements AudioProcessor. Preparation allocates its scratch events and prepares inner DSP. It invokes the inner processor with one-frame borrowed buffers and mapped tempo/sample position, preserving incoming MIDI offsets. It detects discontinuities in the incoming sample position, seeks accordingly, and otherwise advances its own looped position. Incoming playing controls the clock; incoming scalar tempo is overridden by the document map. Loops wrap timing without resetting effect history. Optional note panic precedes new same-frame input; it does not rewind/replay MIDI events supplied by the caller.

`ProjectState.timeline` is optional. The facade builder compiles one map, shares it with AutomatedProcessor and wraps the graph in TimelineProcessor. Lanes evaluate against exact mapped beats under ramps/seeks/loops. An automation document's own loop range can still wrap lane time independently; usually omit it when using a global project loop unless a nested lane cycle is intentional. Direct users can call `AutomatedProcessor::with_tempo_map` with the matching compiled map. `from_map` constructors validate document/cache correspondence rather than accepting an unrelated map silently.

The per-sample wrapper prioritizes exact timing through the existing scalar context API. It adds invocation/math overhead and can be expensive for large spectral graphs. Prepare replacements off-thread and measure callback deadlines on the target machine. It performs no callback heap allocation, locks or file access in Soniclaw; foreign/inner DSP retains its own contract. It is not an automatic optimized segment scheduler for every block-oriented algorithm.

`HostedAudio::process_musical(data, midi, parameters, Frame)` sends mapped PPQ, meter, bar start/number and loop bounds through the CLAP transport event. It uses the frame's block-start tempo. For a ramp, split into sufficiently small blocks—or individual samples for exact per-sample tempo—because the legacy native process call remains block-based and does not schedule arbitrary mid-block tempo events. Existing `process_block` and the scoped multi-bus API retain their scalar transport behavior; converting Frame to core Transport alone loses its extra PPQ/meter fields. Inbound CLAP host PPQ/tempo-map import into an internal project remains outside this release.

## Limits, errors and persistence

Schema 1 supports up to 1,024 tempo points and 1,024 meter points. Beat domain is finite 0–1,000,000; BPM 20–400; meter numerator 1–32 and denominator a power of two ≤64. First points are at beat zero, point beats strictly increase, and range ends exceed starts. Sample rates are 1–768,000 Hz. Unsupported values/configurations fail compilation before publication. Very long timelines beyond the beat domain must be split/re-anchored by the application. A failed preparation invalidates the processor's prepared state.

TimelineProcessor accepts at most 1,024 MIDI events, or 1,023 when reserving a note-panic slot. Invalid preparation/buffer/event shapes are silent. The Clock returns typed errors for invalid seeks, rate/ranges or arithmetic overflow. Failed native musical metadata validation returns an event error before processing; handle errors using the native host's existing policy. When changing rate, reconstruct the Clock so beat boundaries are recomputed consistently.

Project schema 2 gains optional `timeline`; absent preserves prior behavior. New readers load older projects. Older strict readers reject the new field; remove the map deliberately or export a compatible project for them. The document carries tempo/meter/ranges, not DSP history, callback anchors or active MIDI notes. Undo, portable project bundles, catalog snapshots and recovery carry it through ordinary ProjectState serialization. Updating a map requires stopped/prepared reconstruction; a public mutable document is not a live shared cache.

See [release notes](releases/v0.21.0.md), [verification](validation/v021/REPORT.md) and [desktop roadmap](DESKTOP_ROADMAP.md). Physical device/DAW clock synchronization, external MIDI clock accuracy, commercial plug-in interpretation and macOS/Windows runtime remain target-machine checks, not consequences of synthetic timing tests.
