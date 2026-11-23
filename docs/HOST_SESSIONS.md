# Complete hosted plug-in sessions (0.17)

`soniclaw::host::session` saves a hosted rack's topology, native opaque plug-in state, writable parameter snapshots, bypass and floating-editor preferences. This is separate from internal Soniclaw ProjectState. It replaces the limitations of a path-only host list for users who need sounds to reopen correctly.

## Workbench walkthrough

1. In Host, scan trusted products, add them to a rack and Start. Edit parameters or the plug-in's native editor.
2. **Bypass selected** keeps native DSP processing, MIDI and tails running, but routes a continuously maintained dry delay equal to that node's declared latency. Bypass changes are discrete; no crossfade is promised.
3. **Save full session (stops audio)** closes the device stream, stops/revokes processing, captures native state on the GUI thread and writes `host-session.json` in the project root. A plug-in that cannot save its advertised state produces an error; the previous file remains intact.
4. **Load full session** parses/validates without loading native code. **Start live rack** explicitly loads the configured trusted paths and restores state before activation. Current Direct/buffer controls determine processing mode; that choice remains a workbench runtime preference.
5. Missing paths/product IDs become visible placeholder slots. Available effects retain their order around those slots; a missing effect is transparent in the serial audio path. Its opaque state remains in the document for later repair. If every node is missing, Start fails with a clear empty-rack error; the session remains editable/savable through the API.
6. Saving a partially resolved session merges available native captures back into their original stable slots and preserves missing payloads. A loaded session's topology is edited through the Session API; Clear rack exits it so the workbench can build a new list. Legacy Save/Load rack list remains available as a path-only import/export and deliberately does not carry native state.

Full session Save is control-thread/blocking work. Do not trigger it inside an audio callback. It stops audio by design; it does not take a concurrent native state snapshot. Load does not automatically execute downloaded plug-ins. Repair missing paths using the Session API or by editing a copy of the JSON, retaining its slot IDs/state/checksums.

## Executable example

```sh
cargo build -p soniclaw-sdk-tools -p soniclaw-host-fixture
cargo run -p soniclaw-sdk-tools -- --session-demo \
  target/debug/libsoniclaw_host_fixture.so /tmp/new-host-session-demo
cargo test -p soniclaw-host
```

The independent fixture demo records two non-default gains, bypasses the first, captures opaque state, exercises parameter-only fallback for the second, adds a missing slot, saves/reloads and verifies identical output. Native lifecycle/thread checks are enforced. It writes session JSON and a report. Substitute platform library filenames as appropriate; this delivery does not claim runtime acceptance on unexecuted platforms. See [verification](validation/v017/REPORT.md).

## Data model and API

`Session { schema: 1, name, slots }` contains 1–8 ordered `Slot`s. Each slot has a unique stable `id`, `RackEntry { path, plugin_id }`, saved `plugin_version`, `bypass`, `EditorSettings`, a writable physical-value parameter map, optional `state_hex` and paired SHA256. `EditorSettings` contains `open`, optional pixel size and scale. Native bytes are hex encoded to avoid JSON byte-array expansion. The hash detects accidental state corruption; it is not a signature or a trust decision.

```rust,ignore
use soniclaw::host::session::{Session, MissingPolicy};
// owner and entries describe the same running rack on its creation thread.
let document = Session::capture(&mut owner, &entries, "My instrument rack")?;
document.write(std::path::Path::new("rack-session.json"))?;
let document = Session::read(std::path::Path::new("rack-session.json"))?;
// SAFETY: the user explicitly trusts every configured native library.
let mut resolved = unsafe { document.resolve(MissingPolicy::Placeholder)? };
let instances = resolved.take_instances();
let (mut owner, callback) = soniclaw::host::live::LiveRack::start_direct(instances, spec)?;
resolved.configure(&mut owner)?; // restore bypass before the first callback
let editor_warnings = owner.restore_editors(&document, &resolved.slot_indices);
```

`capture` requires entries in exact runtime-node order and stops the owner before native state I/O. It deactivates nodes, refreshes parameter values and invokes their advertised CLAP state save extension. A node without that extension stores a parameter-only snapshot. A node whose advertised save fails makes capture fail. The stopped owner can be dropped afterward; reconstruct to play again. Capture returns a new document only after complete validation; it does not mutate the caller's previous snapshot.

`resolve` validates the complete document/checksums before any native load. It caches identical canonical library paths within the reconstruction, restores opaque state through CLAP before activation and reports version differences as warnings. For parameter-only state it validates every ID/value/readonly flag before invoking deactivated parameter flush in bounded chunks; the native implementation must support flush. Invalid opaque state is an error, not silently replaced with defaults. A failed native restore can have modified the temporary instance, which is then dropped; no previously running rack is published.

`MissingPolicy::Fail` stops on missing files/products. `Placeholder` retains missing-file/product records and resolves available nodes; malformed or ABI-incompatible installed libraries still fail rather than being silently accepted as missing. `ResolvedSession` exposes `slot_indices`, `missing`, `warnings` and temporary instances. `take_instances` transfers available nodes to a runtime; `configure` applies the saved bypass mask and checks runtime mapping length. Call it before publishing the callback. Output events/control indices refer to available runtime nodes; use `slot_indices` to map them back to document slots.

`merge_capture(captured, indices)` transactionally merges a capture of available nodes into a cloned document, retains original IDs and missing payloads, and rejects duplicate/out-of-range mappings. Applications can reorder/remove slots in a stopped document; validate and reconstruct afterward. Native plug-in resources, samples and licensing files referenced inside opaque state remain the plug-in's responsibility and are not bundled by this format.

## Bypass, editors and ownership

`LiveRack::set_bypass(node, bool)` is a validated atomic update. Native DSP continues receiving input and MIDI while bypassed. A separate continuously clocked dry delay preserves activation-time latency. Dry delays are allocated off-thread and bounded to 1,048,576 stereo frames per node; larger declared latencies fail rack activation. Active latency change requests still stop the rack for reconstruction. This is serial node bypass, not a parallel graph mixer. Dry and wet are switched discretely, so clicks are possible at arbitrary signal discontinuities.

The main owner and native state/editor operations remain on the creation GUI thread; processing continues on its worker or direct callback until stopped/revoked. Capturing never borrows a live audio resource concurrently. Partial session results do not escape when a native capture fails. A hung plug-in can still block stop/save/load; native code is trusted and in-process. A checksum cannot make untrusted native code safe.

Editor capture records open state, supported size and the last host-applied scale. `restore_editors` recreates requested floating editors and attempts supported scale/size calls. Refused creation/settings are reported as warnings while audio can still run. Native OS desktop position is not saved: CLAP's portable floating GUI API does not supply a general position query. Unsupported embedded editors and host timer/fd requirements retain the limitations documented in the editor guide until extended in a later release.

## Persistence, limits and recovery

Schema 1 is strict (`deny_unknown_fields`). There is no change to Soniclaw ProjectState schema 2. Old path-only rack lists keep their existing API and must be explicitly turned into a running rack/captured session to gain native state. Older readers without this session feature cannot interpret the new file.

Bounds: 1–8 slots; 256-byte name/slot IDs; unique nonempty IDs; nonempty path/product ID (product ID ≤512 bytes); at most 4,096 writable parameter values per slot; finite values; opaque state at most 4 MiB per slot; paired valid hex/SHA256; editor scale 0.5–4 and optional dimensions 64–8,192. The complete JSON file is bounded to 96 MiB before parsing. A session with no opaque state can restore only parameters represented by its snapshot, not private native resources.

`write` validates/serializes before creating a unique same-directory temporary file, flushes it to disk, then renames over the destination. Errors remove the temporary and preserve an existing destination before the rename. Directory fsync/power-loss durability is platform-dependent and not guaranteed. Keep a previous version or use application recovery checkpoints for additional protection. `read/from_json` reject oversize/corrupt/incompatible documents before native loading. Never remove a state checksum to bypass a corruption error; recover the previous session or deliberately reset that slot.

See [release notes](releases/v0.17.0.md) and [desktop roadmap](DESKTOP_ROADMAP.md). Real DAW, commercial plug-in state compatibility and physical-device/macOS/Windows acceptance remain pending where unexecuted. Fixture state round trips establish the documented contract, not compatibility with every vendor.
