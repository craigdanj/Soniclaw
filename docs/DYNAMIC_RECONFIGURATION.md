# Dynamic latency and bus reconfiguration (0.59)

Running native buffers keep their prepared shape. When a plug-in requests a restart, latency change or port rescan, the host can now stage a separate replacement, approve its exact routes and delay budget, prepare the complete graph and commit it while stopped. Ordinary fault recovery remains restricted to the original topology. The new native transaction is implemented for the isolated CLAP owner; the reusable topology/routing plan is format-neutral data for other host adapters.

## Native example

```
cargo build -p soniclaw-reconfigure-host -p soniclaw-clap-isolation-helper -p soniclaw-host-fixture
SONICLAW_FIXTURE_REQUEST_RECONFIGURE=1 ./target/debug/soniclaw-reconfigure-host ./target/debug/soniclaw-clap-isolation-helper ./target/debug/libsoniclaw_host_fixture.so NEW_DIRECTORY
```

The independent fixture starts as a stereo gain processor, requests a restart/port rescan, then advertises a second configuration with a mono sidechain and a real preallocated 16-frame stereo delay. The example preserves the latest UI gain, rejects an unknown configuration and a too-small budget, rejects a deliberately failed graph build and a stale candidate, then prepares the replacement plus a parallel dry branch. The measured impulse appears at frame 272: 256 pipeline frames plus 16 native frames. Dry and sidechain-processed wet impulses sum to the expected 1.125 at that same frame. Every audio callback is measured allocation-free. A retained old processor emits silence. `plan.json` records the approved topology/routing; `report.json` records outcomes.

## Observe and quiesce

`BufferedOwner::requires_reconfiguration()` observes accumulated restart/latency (bit 1), parameter-rescan (bit 8) and audio-port-rescan (bit 64) requests. CLAP's host audio-port rescan extension is now advertised. These flags request control-side work; they do not resize buffers or query topology inside audio processing. Other existing request bits remain available through `owner.control.requests`.

Stop the device or detach the old callback from its render source, and drop the old BufferedOwner off callback. Its graph worker joins before Stop/deactivation; a retained old Processor becomes silent. Pending notes and recording/transport transitions still require the application's usual stop/panic policy. The transaction does not make an in-place channel-width change or promise uninterrupted audio across a configuration change.

The bus lease now queries native latency after activation. The helper probes an activation before its handshake and after state restoration, and verifies layout/latency again on actual Start. A plug-in that changes topology on each activation is refused rather than published with mismatched PDC. This fixes the earlier bus helper's zero-latency handshake assumption.

## Stage, prepare, build, commit

1. Copy the original isolation Config and change only approved topology settings, such as sample rate, maximum frames or audio_configuration. Helper executable, module path and product identity must remain exactly the same explicitly trusted identity.
2. Call `current.stage_reconfiguration(next_config)`. It requires a stopped owner, creates a separate child and captures the latest healthy stopped state automatically. If the current child faulted, an existing saved checkpoint is required instead. The new child loads state, reapplies its explicitly selected audio configuration and probes actual topology. Inspect `Candidate::topology()`.
3. Supply `reconfigure::Routing` and call `Candidate::prepare(routing, pipeline_frames, latency_budget)`. Every native input channel must have a connection or appear in silent_inputs. Every native output must have a connection or appear in discarded_outputs. Unknown/duplicate channels, missing sidechains and implicit output summing reject. This creates a prepared RoutedProcessor and helper audio owner without changing current.
4. If the application has an external graph, call `PreparedChange::build(|routed, plan| ...)`. Return the complete graph containing that processor. Build prepares it with the approved external spec, recalculates its PDC and rejects total graph latency above budget. Allocation and routing happen here, off callback. The native example includes wet and dry branches in this step.
5. Inspect `PreparedChange::plan()` and call `commit(&mut current)`. The current owner must still be stopped with the same revision and previous topology. Success returns the new BufferedOwner, prepared processor/graph and plan. Only then publish that complete callback to the device/session. Keep the returned owner alive until its callback is retired.

Starting old playback, restoring/checkpointing state or successfully recovering advances the revision and invalidates prior candidates. Failed activation attempts also invalidate them because a native activation can reset internal history. A stale commit drops its separate candidate and preserves current. Configuration, state, routing, processor or graph-build errors similarly leave the stopped original available to restart. A failed state-restore attempt may have changed the current native instance while stopped; it invalidates candidates and playback rechecks topology. Recover from the saved checkpoint or stage a new approved topology before resuming.

## Routes and PDC

A Connection names a native bus ID/channel and an external interleaved channel. IDs come from the advertised layout, not display names or implicit port ordering. The preallocated RoutedProcessor clears its flat native input, copies explicit connections, processes native audio/automation and copies explicit outputs. Unconnected external outputs are cleared. Native inputs listed as silent and outputs listed as discarded are deliberate application choices. The adapter supports one source per native input and one native source per external output; explicit external graph nodes can provide more complex summing/downmixing.

`reconfigure::Topology` holds layout, native latency, rate and block limit. `Plan` schema 1 holds previous/next topology, routes, pipeline and budget. `to_json`/`from_json` validate it; serialized input is capped at 64 KiB. Plans contain no module paths or executable state and do not authorize native loading by themselves. Direct CLAP/VST3 hosts may reuse this plan and the existing common Layout validation, but must implement their own stopped native lifecycle/factory publication. Automatic VST3 restart negotiation is not added by this CLAP transaction.

Native plus pipeline latency must fit budget before a graph is created. The optional complete-graph build repeats that check against the actual graph latency. Existing graph preparation aligns parallel paths to their longest prepared latency. A new latency can require rebuilding downstream delays as well as the changed processor; call build with the complete affected graph. Physical input/output converter latency and external MIDI scheduling remain separate policies.

## Bounds, persistence and recovery

The isolated adapter keeps eight aggregate channels per direction, 1–1,024 frames, 8–192 kHz and the existing native timeout/recovery limits. Pipeline delay is at least twice the maximum block, at most 262,144 frames; native latency and the full approved budget are capped at 262,144. Every shape, endpoint and addition is validated before allocation/publication. Preparation can temporarily own two stopped/running helper resources during staging; the caller bounds the number of candidates. Native side effects or a crashed process cannot be rolled back by Rust, but the current child is never mutated by the staged replacement.

No project manifest change is required. Hosts may persist the validated plan alongside their existing native checkpoint and Config, then re-probe on reopen; a saved plan never overrides newly observed topology. The fixture retains its existing native state schema and makes configuration an explicit host setting. Parent and helper binaries must match this release's protocol. Ordinary `recover` validates topology both before and after loading state; topology-changing restores require explicit staging. Fresh preparation, joins, state capture, file/protocol operations and destruction must stay off the audio callback.

[Release evidence](validation/v059/REPORT.md) records full-workspace software checks and native fixture runs. It does not assert native device, external DAW, Windows/macOS runtime or arbitrary third-party acceptance.

## Freeze boundary regression found during final verification

The first full-workspace run caught a single zero sample at source frame 1023 in a streamed freeze. Linear interpolation was reading the next page even at an integer position, giving that unavailable zero-weight read the status of an underrun. The correction skips a zero-weight lookahead, waits for the complete prepared interpolation range and makes offline mix reject any observed underrun/I/O error before publishing a stem. A pure interpolation regression and the complete 48,000-frame waveform test verify it. `initial-test-failure.log` preserves the discovery; the final checks record the corrected run.
