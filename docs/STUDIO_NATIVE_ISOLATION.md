# Studio native crash isolation — 0.88.0

Studio can host a stereo VST3 instance in an independently approved helper process. Native module loading, parameter/state calls and processing execute in the child. A crashed or timed-out instance can fall back to silence or latency-preserving dry audio while the remaining graph continues. This is a crash boundary, not an operating-system security sandbox.

## Studio workflow

1. Build `cargo build -p soniclaw-isolation-helper --offline`. The executable is `target/debug/vst3-host-helper` (`.exe` on Windows; target-platform runtime acceptance is still required).
2. Select an existing stereo VST3 instance in Studio's native plug-in manager. Open **Native crash isolation**, enter the helper's absolute path, approve the executable/dependencies and choose **Enable / reset budget**. This approval is session-only. Loaded projects cannot approve helpers themselves.
3. Select delayed dry fallback for an effect if wanted; instruments always fall silent after failure. Press Play. The panel shows each running helper's PID and diagnostic.
4. On failure, the graph retains its declared latency and the rest of the session can continue. Stop snapshots the last good checkpoint plus latest host control values and marks the instance quarantined. The diagnostic explains that recovery state was used. Save normally.
5. **Recover instance** consumes one persisted attempt. Play constructs a fresh helper from the checkpoint. No automatic restart loop runs. The default UI budget is three attempts; after exhaustion, explicitly enabling/resetting the policy grants a new budget. The plug-in manager's bypass remains available.

After reopening, enter the helper path and use **Approve helper for this session** before playback. This approval does not reset the saved recovery attempts. Disabling isolation removes the saved policy; approval of the native module is still required for in-process hosting. Disconnect multi-bus routes before enabling this stereo adapter. Host-exposed parameter faders and automation remain available; remote native editor windows are unavailable.

## Reusable framework API

Enable `soniclaw-host` features `studio,isolation`. `NativeFactory` now requires `helpers: &studio::isolation_adapter::TrustedHelpers`. Existing constructors should supply `&Default::default()` to deny helper execution. Construct the unsafe `TrustedHelpers::new(&[absolute_paths])` capability on the control thread only after explicit executable/dependency trust. Module approval is separately enforced by `formats::TrustedLoader`.

`isolation_adapter::Policy` serializes at `Descriptor.state.isolation`:

```json
{"helper":"/absolute/path/vst3-host-helper","timeout_ms":500,"recovery_budget":3,"dry_on_fault":true}
```

Timeouts accept 50–10000 ms, budgets 1–8. The saved path is configuration, not authorization. `Policy::request_recovery(&mut descriptor)` validates and increments `state.isolation_attempts` and clears quarantine before a new graph is built. Applications should commit that document before native execution. `Owner::isolated_pid()`, `Owner::fault` and `Owner::snapshot()` provide control-thread diagnostics and recovery state. On a failed snapshot, surface the fault and persist `state.quarantined=true`, as Studio does.

`render_export_with_helpers` takes the same independent capability on an offline worker; existing `render_export` defaults to no helper approval. Native export processing errors reject the result and remove a newly created output directory rather than accepting a fallback mix.

The matching helper protocol forwards complete musical context, including meter, bar/PPQ, cycle and recording flags; rebuild helper and parent together.

The lower-level `soniclaw_vst3::isolation::Isolated` gains `start_offline_musical`, `offline_errors`, `info`, `latency_frames`, `save_state` and `restore_state`; its existing checkpoint/recover API remains available. Plain state access does not install an in-owner recovery checkpoint: use `checkpoint` or `restore_checkpoint` before relying on that lower-level owner's `recover`. Studio graph reconstruction instead uses the retained project blob. `soniclaw_vst3::Processor::faulted()` reads processing error counters without locks. The existing CLAP child-hosting API remains in `soniclaw_host::isolation`; this Studio integration currently supports stereo VST3 only. Simultaneous Studio multi-bus isolation and isolated editor windows are not implemented.

## Ownership and timing

Create capabilities and native owners on a single control/main thread; retire audio graphs before dropping owners. Live processing uses a prepared asynchronous bridge: 1024 frames plus reported native latency. The callback performs bounded copies, parameter updates and atomic reads, with a preallocated dry delay. It performs no child RPC, allocation or native state calls. Fault detection occurs at callback/poll boundaries; initial missing/failed packets can be silent. A fallback effect keeps the same delay as the failed native path. Instruments output silence even when dry fallback was selected.

Offline processing uses continuous blocking RPC on the owning worker and no bridge latency. Its native operations have bounded response deadlines; process creation and platform loading can take longer than an individual operation deadline. Never put offline processing on a device callback. Owner destruction terminates the helper. Old graphs must be retired before recovery; low-level revocation remains enforced by the existing isolated owner.

## Persistence, bounds and recovery

Before starting, the owner retains a verified last-good `native_state::Blob`. If the helper fails, snapshot uses that opaque state (when supported) plus latest host parameter scalars. Unsaved internal edits, DSP histories and held voices cannot be recovered. `Owner.fault` explicitly identifies fallback. Products without state support must use `state_mode:"parameters"`; ordinary snapshot errors are not silently converted into success.

Descriptor fields are optional, so old project schemas require no migration. Product IDs, parameter IDs and normalized automation identities are unchanged. Existing native-state byte/count/hash limits apply. Helper executable paths are canonicalized and checked against a session capability; the capability accepts at most 256 paths and is control-thread-bound. Use a matching helper build from this source release. Recovery counters survive project save/reopen; deliberate policy reset or history operations can reset them. These counters prevent application restart loops, rather than enforce a security boundary.

## Runnable example and validation

Run only against the owned HostProtocolFixture, which exposes a deliberate crash switch:

```sh
cargo run -p soniclaw-studio-isolation --offline -- NEW_FOLDER ABS_PROTOCOL_FIXTURE ABS_HELPER
```

The example checks healthy audio, callback allocation counts, child abort and timeout, delayed dry and silent effect/instrument fallbacks, opaque and parameter-only checkpoint recovery, fresh helper PIDs, native tempo/play flags, complete Studio graph audio, unapproved-helper denial, project persistence and failed offline-export removal. The owned fixture now implements small opaque state streams for these recovery checks. It deliberately faults native code only inside the child. Executed results and release commands are retained under `docs/validation/v0_88_0/`; do not treat the example's existence as evidence of execution.

Physical audio/MIDI, native graphical window interaction, broad third-party compatibility, and Windows/macOS runtime acceptance remain pending. Existing in-process VST3 bus routing remains available from 0.87.0.
