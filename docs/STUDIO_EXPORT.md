# Studio export centre — 0.65.0

## Workflow

Stop the reference studio, disable arrangement looping and live input/MIDI input on every track, expand Export centre and enter a **new** destination folder. Select full mix, pre-mixer track stems, or both; enter start, length and tail in project-rate frames. Choose WAV/AIFF (float32, PCM16 or PCM24) or FLAC (PCM16/24). Select no normalization, sample-peak normalization, or integrated LUFS normalization at 48 kHz; configure the normalization sample-peak ceiling. Integer output can use deterministic TPDF dither. Export snapshots the stopped project and uses the shared bounded job queue. Progress is stage-local and can reset between rendering and encoding. Cancel requests cooperative cancellation; a completed committed directory is retained even if a cancellation races with completion.

The full mix includes automation, mixer pan/gain/groups and auxiliary returns. Track stems use the existing freeze renderer: track clips/instrument/inserts and their automation, before mixer pan/gain/aux returns. They are not additive mix stems. Original tracks and source files are untouched. Native plug-in descriptors need an owner-aware factory through the API; the GUI export worker uses Builtins and reports unsupported native descriptors rather than silently bypassing them.

## API and example

`studio::export::{Settings, Format, Depth, Normalize, render, Report, FileReport}`. Validate a schema-1 Settings against the project; `to_json/from_json` supports bounded 16 KiB settings persistence. `render(project, settings, new_directory, factory, jobs_context)` is offline/control-worker-only. It returns and writes an `export.json` report only after rendering, analysis, encoding and source verification. The example schedules the real renderer and decodes each output with the installed codec backend:

```sh
cargo run -p soniclaw-studio-export -- NEW_DIRECTORY
```

It checks WAV float32, AIFF PCM16 and FLAC PCM24 mix/stem files, exact 50,400-frame ranges/tails, a decoded normalized peak within quantization tolerance, settings round-trip and cancellation without a committed manifest. It requires the dynamically loaded libsndfile backend, as documented in FORMATS.md. No codec implementation was newly written here.

## Render and ownership contract

The mix uses `Document::compile_offline` and its error-returning `OfflineSession::render_block`. Disk-page waits remain off callback. Rendering warms from project frame zero, preserving instrument/automation/effect history before the selected range; processor latency is flushed and trimmed separately from the requested tail. This can make late timeline ranges expensive. Loops are rejected for a finite linear export. Each pre-mixer stem is independently rendered and latency aligned by the existing freeze path; shared mixer/aux state is excluded by definition.

Working PCM streams into temporary float WAVs inside the newly owned directory. Analysis and encoding stream in 8,192-sample blocks; they do not load the song into one Vec. PCM assets already in the project remain shared and retained by the job snapshot. The scheduler reserves a 64 MiB working estimate; captured assets and processor factories require separate honest budgets. Native factory creation, state, processing and destruction must stay on their permitted owner thread. No method in this module belongs in a device callback.

The source fingerprint covers document, in-memory sample bits and disk file contents before/after export. Changes during the job fail and clean this job's new directory. Fingerprinting is I/O and is cancellable between disk chunks. It is not a sandbox against hostile concurrent filesystem manipulation.

## Analysis, normalization and dither

Source sample peaks are measured in rendered floating PCM. The existing 48 kHz K-weighted gated meter supplies integrated LUFS; short/silent output has no value, and non-48-kHz exports report null LUFS. LUFS normalization rejects unavailable measurements. No normalization preserves gain; integer encoding clips at its representable range. Peak/LUFS normalization uses one constant gain per output, limited by the sample-peak ceiling. A ceiling-limited output can miss its requested LUFS target. No limiter or intersample true-peak guarantee is implied.

Reports contain source peak/LUFS, applied gain, pre-codec output sample peak, estimated post-gain LUFS and whether the ceiling limited gain. The estimated LUFS is source measurement plus gain, before quantization; it is not a separate codec-decoded measurement. Dither is deterministic independent uniform noise difference at one destination LSB; no noise shaping. Integer samples clamp after dither. Float+dither and FLAC+float combinations fail validation.

## Persistence, failures and boundaries

Settings/reports use schema 1. Reference UI choices/job queue are session-local; reports live in the output directory, outside the portable project. No project schema change. Existing original sources and historical exports are never overwritten. Successful working files are removed before manifest publication. Errors/cancellation remove the newly created job directory; callers must treat it as exclusively job-owned while rendering. Export.json is the completion marker; there is no filesystem-directory fsync/power-loss durability guarantee.

1..32 output files, nonzero length, checked start/length/tail, maximum four-hour end and RIFF frame bounds; stereo at the project rate. Duplicate/unknown stems, invalid schema/nonfinite gains, live input, loops, codec-unavailable/unsupported choices, changed sources, disk errors, render underruns, unavailable LUFS and cancellation are explicit errors. No sample-rate conversion, batch project queue, MP3/AAC, cue-sheet or external DAW round-trip acceptance is included. Validation reports list actual checks; physical audio and native external plug-in export acceptance remain pending.

## Later addition

Starting in 0.68.0, optional destination-rate conversion runs before analysis/encoding. See [Export conversion](EXPORT_CONVERSION.md); the original 0.65.0 scope above describes project-rate-only behavior.
