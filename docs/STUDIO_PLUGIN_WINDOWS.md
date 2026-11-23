# Native plug-in windows in Reference Studio — 0.67.0

## Workflow

Start Reference Studio with a project and explicit canonical module allowlist:

```sh
cargo run -p soniclaw-desktop-studio --features gui -- --gui PROJECT /absolute/trusted/synth.clap
```

Add the CLAP path/product ID using Native CLAP insert, then Play. The main thread owns each LiveRack; the device owns its direct processor. Each loaded instance exposes Host window/Floating mode, Open editor, Focus, Close editor, scale and resize controls alongside generic parameter sliders. Host window embeds the plug-in view into an owned platform window; Floating asks the plug-in to own its window. Unsupported GUI modes/scale/resize/focus produce explicit status errors or warnings. Selecting a mode does not recreate a currently open view until Open is invoked.

The existing host poll services native events each UI frame. Close notifications update reopen intent; resize notifications update saved size. Stop captures size, closes editors before native state capture, stops processing and drops owners on the main thread. It preserves reopen intent: subsequent Play reconstructs the same allowlisted instance and requests its window again. Window preferences alone never load a library. Auxiliary native inserts now participate in stop-time checkpoint capture as track inserts already did.

## API and examples

`soniclaw_host::editor_workspace::{Workspace, State, Mode}` supplies validated preferences for already loaded LiveRack instances. `Workspace::for_instance(descriptor_id, RackEntry)` returns identity-matched state, resetting preferences when module/product changes. `State::open(rack,node,title,entry)` checks identity, opens the requested mode and applies saved scale/size. Creation errors fail; optional scale/resize refusal leaves an open usable view and returns warnings. LiveRack's existing `focus_editor`, `resize_editor`, `scale_editor`, `close_editor`, `editor_size`, `poll` and `drain_editor_events` remain the actual platform lifecycle implementation.

```sh
cargo run -p soniclaw-studio-plugin-windows -- NEW_DIRECTORY
```

This **headless preferences example** round-trips an instance workspace, resets mismatched product identity and rejects invalid scales without loading modules. Use the studio command above for the interactive native-window example with your trusted plug-in. The automated example is not a native-window acceptance test.

## Thread/lifetime and persistence

Keep LiveRack, native library/instance owners, window creation, parenting, resize/scale/focus, event polling and destruction on their existing GUI/main thread. The direct rack processor remains the audio thread's owner of processing access. Do not send these UI owners through the background scheduler. Close the view before destroying its owner. No unsafe external parent handle is manufactured by this studio integration; owned parenting uses the host's existing platform implementation.

Save new folder writes `plugin-windows.json` beside `editor-view.json`, after stopping/checkpointing. Open validates the optional workspace before replacing the project. Initial CLI project open also reads it. Schema 1 records module/product identity, mode, requested scale, size and reopen intent by descriptor ID. It does not record window position, monitor, z-order or OS handle. Existing projects without the sidecar get defaults. Core Project save/recovery copies the core project, not this optional reference-app view sidecar; recovery resets view preferences. Explicit startup trust is still required after reopening.

## Bounds, errors and limits

At most 256 workspace entries; IDs 1..128 bytes; product IDs 1..256; module paths 1..4096 platform string units; scale 0.5..4; saved dimensions 64..8192. Input/output JSON is limited to 256 KiB and rejects unknown fields/future schema/nonfinite or out-of-range values. Identity changes reset reopen intent; missing GUI extensions, unsupported modes and unavailable native display services remain recoverable UI errors. A refused resize/scale is reported, not presented as accepted.

The reference studio's native factory currently supports CLAP. This does not add VST3/AU editor instantiation to the studio, even though other framework hosting APIs exist. Linux owned windows use X11; a Wayland session needs compatible XWayland/X11 plug-in support. Existing Cocoa/Win32 host implementations compile under their targets but were not run here. Focus support depends on ownership/platform. Plug-in timer/fd/service limitations remain those of the host.

This release ran Linux all-feature workspace Clippy/docs, host/desktop tests and the preferences example. **Actual native windows, physical audio and third-party plug-in interoperability were not run**. No screenshot, DAW or broad platform acceptance is claimed. See the verification report for exact commands and evidence.
