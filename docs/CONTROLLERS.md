# Hardware controller integration (0.40)

## Workflow and runnable example

`cargo run -p soniclaw-controller-demo -- NEW_DIRECTORY` creates a controller profile and executes pickup, external parameter movement, bank switching and a relative encoder against a real shared Parameter. It queues feedback through the existing scheduled MIDI output worker into a virtual Sink and records packets/values. This uses no physical MIDI device. For hardware, replace the Sink with the existing native MIDI output connection and feed parsed native MIDI input to the same prepared controller.

`soniclaw::midi::controllers::Profile` contains named banks and per-input mappings to stable parameter IDs. Compile against the actual `Arc<Parameter>` list and engine sample rate before playback. Missing or duplicate parameter identities fail preparation. `Controller::apply(absolute_sample,MidiMessage)` acts at the event's sample offset; the application remains responsible for splitting processing at that offset. Non-CC messages are ignored. The existing absolute-only `learning::LearnMap` remains compatible and unchanged.

## Relative encoders, pickup and banks

Modes include absolute CC (optional soft pickup), two's complement (0 neutral, 1–63 positive, 64–127 negative), binary offset (64 neutral), sign/magnitude (0/64 neutral) and a rising-edge toggle. Relative step is a normalized parameter increment; choose a step suitable for typed/discrete controls. Mapping uses `set_normalized`, so existing parameter codecs retain their quantization and physical unit conversion.

Soft pickup waits until the physical control reaches/crosses the parameter or enters the specified tolerance. It follows later moves after latching, and re-arms when another UI/automation writer moves the parameter away from the last applied value. `select_bank` explicitly changes active mappings and re-arms pickup/toggles/feedback. The engine does not interpret arbitrary controller bank-selection messages; applications can bind those to their command framework. Duplicate inputs or feedback addresses within one bank are rejected. Multiple banks may reuse addresses and parameter IDs.

## Feedback, echo handling and ownership

Call `feedback(sample,nonblocking_send)` from the same processing owner. It offers only changed quantized 7-bit CC values, coalescing repeated targets. The closure returns true only after a bounded queue accepts the packet. A rejected packet remains pending and is retried on a later call; do not call a hardware driver or block inside this closure. The existing `output::Sender::send` and `wire::Packet::new` supply the queue/packet adapter. Its own overflow/panic counters and shutdown contract still apply.

Successfully queued feedback records a global address/value echo guard for the configured sample window, including across bank switches. Only exact echoed values are suppressed. A zero window disables suppression; choose this for devices that don't echo or whose encoder input shares ambiguous feedback values. A guard cannot distinguish a genuine coincident identical physical message from an echo. Scheduling feedback far ahead or changing sample clocks requires care: reset clock state after a discontinuity. `reset_clock` clears guards, feedback cache and takeover state; send fresh feedback after reconnect.

Apply and feedback samples must be nondecreasing; backwards CC time returns an error before mutation. `select_bank` does not reset time. The owner mutates its controller exclusively; UI can update the shared atomic parameters, but bank/profile changes should be sent through the application's control queue. Preparation, serialization and controller/Arc destruction belong off the callback. Apply, selection and feedback do not allocate; feedback's supplied closure must honor that contract too.

## Persistence, limits, validation and scope

Profile schema 1 stores stable IDs/modes/step/banks/feedback configuration, not hardware ports, latches or active bank. JSON is capped at 1 MiB; 1–32 banks, <=128 mappings each, channels 0–15, CCs 0–127, step 0.000001–1, tolerance 0–0.25 and echo window 0–1000 ms. Compile at 8–384 kHz. Invalid profile/unknown parameter leaves the old controller untouched; prepare a replacement before publishing at a safe boundary. No existing project schema changes occur. Apps explicitly persist selected bank/port preferences if needed.

Tests cover all relative encodings, toggles, pickup/crossing, external-change rearm, bank reset, timestamp errors, JSON, missing targets, callback allocations, feedback coalescing/retry and sent-message echo suppression. The example executes the real scheduled output worker with a virtual device. Mackie/HUI protocols, motor-fader calibration, 14-bit CC/NRPN, automatic device discovery and physical hardware acceptance remain outside this release.
