# Timestamped input and recording calibration — 0.75.0

## User workflow

Reference Studio physical MIDI connections retain backend microsecond timestamps, map them to the device acquisition clock and deliver intra-block event offsets. One maximum internal audio block of lookahead is deliberately scheduled so arrivals can retain their relative positions when the next callback drains them. Completed MIDI note clips/SMF compensate that lookahead in addition to the explicit MIDI recording offset. The computer keyboard keeps the existing immediate block-boundary behavior. Late events are counted and clamped to frame zero; loss/queue overflow requests all-notes-off and clears stale queued events. Timestamping preserves scheduling intent; driver/OS timing must still be measured on the target machine.

Open **Recording calibration** to enable positive/negative audio and MIDI frame offsets. Positive means the recorded performance arrived late and should move earlier. Raw WAV/MIDI data is retained; placement/conversion applies the correction. Calibration includes project sample rate and label; a changed rate invalidates it. It is specific to the interface, buffer and routing configuration; remeasure after changing those. Do not add an already included bridge/hardware delay twice.

For electrical loopback measurement, prepare an isolated single-impulse mono float32 reference WAV and its recorded return at project rate. Enter both paths and choose **Measure impulse loopback files**. The bounded worker finds the strongest return in a one-second window, checks isolation and peak/noise ratio, and stores the audio offset. This is an impulse utility; music/sweeps, ambiguous reflections or missing/noisy returns are rejected. It does not automatically route or emit a loud calibration signal. Manual offset editing remains available. Measure with appropriate low interface levels and speaker monitoring disconnected from the loop.

```sh
cargo run -p soniclaw-daw-timing -- NEW_DIRECTORY
```

The runnable example measures an inverted synthetic impulse delayed 137 frames, records virtual MIDI, verifies 64-frame compensation without changing raw timestamps, checks zero-boundary trimming and reopens the calibration. No physical measurement is claimed.

## API and callback ownership

`device::EventSender::{send_at,send_timed}` supplement legacy `send`. `send_at` uses absolute device-acquisition samples, independent of arrangement seek/loop. Per-producer scheduled timestamps must be nondecreasing. The bounded queue retains the first future event, drains a fair per-source budget and stable-sorts the fixed event block without allocation. Different producers may interleave; equal-time producer order is stable. Late events increment `Telemetry::late_events`. Overflow/out-of-order scheduling increments dropped events and requests panic. Panic discards due and queued stale note-ons before delivering a single all-notes-off. `Telemetry::input_clock` offers a bounded sequence-checked wall/audio anchor; unavailable/stale anchors fall back to legacy immediate send. Rebuild event sources for a new stream/reconnect clock.

`midi::native::connect_input_timed` exposes the driver's microseconds. `midi::schedule::TimestampMapper` explicitly anchors them; the GUI anchors at first timestamp to the current lookahead clock. Driver clock drift or callback jitter can still affect long performances. No hardware MIDI timestamp accuracy is inferred. `studio::timing::{Calibration,measure_impulse,measure_wav,placement}` is the public calibration/control-side API. File reads, impulse analysis, project cloning and saving are worker/control work. Audio exchange, event queues and bounded sorting stay callback-safe. Plug-in processing latency remains separate from physical input compensation.

## Persistence and placement

Project schema 3 layout gains optional calibration; absence preserves previous behavior/legacy identity derivation. Recording Schedule stores the arming-time calibration snapshot, preventing later UI changes from changing that take's placement. Positive audio compensation extends punch capture latency tail and shifts the dry region earlier. When its corrected start would be negative, placement starts at zero and trims only the required source prefix. Insufficient media after compensation rejects placement atomically while retaining raw takes. Raw source file positions remain available for slip edits. Source processing latency is independently removed from rendered source offsets. Compensation is stored per take through its portable schedule sidecar.

MIDI Settings gains `input_offset_frames`, default zero, stored with raw takes; conversion adjusts notes and SMF controls/bend/pressure. Fully out-of-range notes are omitted; notes crossing the recording start are clipped. Hanging notes close at the corrected endpoint. Timeline/quantization remain explicit and raw events are unchanged. Strict previous readers may reject added fields. Existing PCM/take formats remain float32 and project schema remains 3.

## Bounds, errors and evidence

Manual calibration permits +/- one second at 8–192 kHz; MIDI take conversion permits +/- two seconds including delivery lookahead. Reference arrays have at most 192000 samples, return arrays at most 384000, delay search at most 192000; WAVs must be mono float32/project-rate. Peaks must be finite and isolated; a weak return or peak/noise ratio below eight is refused. This estimates integer-sample delay from an isolated impulse, without sub-sample estimation, frequency-response measurement or confidence intervals. Device scheduling samples are bounded by 2^48; queues/event budgets retain existing limits. Arbitrary driver clock resets should end the take and reconnect explicitly.

The desktop Stop/punch completion path also snapshots controls before applying finalized dry regions; a regression test verifies the placed clip survives the final synchronization.

Tests cover exact intra-block offsets across 64/127-frame partitions, future retention, late/out-of-order input, panic clearing, polarity-independent impulse measurement, missing-return refusal, rate mismatch and placement at zero. The example checks persisted calibration and corrected MIDI note times. See executed validation logs. Physical interfaces, actual MIDI timestamp jitter and Windows/macOS runtime remain pending; this release is a software timing implementation, not a latency benchmark.
