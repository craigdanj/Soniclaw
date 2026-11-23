# Hosted editors, native parents and host services

Version: 0.22.0. This extends the CLAP host in `soniclaw::host`; it does not replace the GUI export or provide universal window-toolkit integration.

## Workflow

In the workbench Host tab, load a trusted CLAP product, open an embedded or floating editor, adjust size or scale, and keep polling the host. On Linux, the host owns an X11 parent. For another application window, use `attach_editor` with your own native parent. Close the editor before destroying that parent. Polling processes plug-in requests, native close/resize events and timer/FD callbacks on the thread that created the rack.

A reproducible Linux example after building the workspace:

```sh
cargo build -p soniclaw-sdk-tools -p soniclaw-workbench -p soniclaw-host-fixture
# Requires a live X11 DISPLAY (or Xvfb), and a NEW output directory.
target/debug/soniclaw-sdk-tools --editor-demo \
  target/debug/libsoniclaw_workbench.so /tmp/soniclaw-editor-check
```

The command attaches an actual workbench editor, pumps events, resizes/scales, closes/reopens, checks that physical size stays stable, focuses the window and captures a PPM plus a JSON report. Use the fixture library instead to verify timer and readable-FD delivery with an independent mock editor. The fixture has no visual controls.

## Public API

Use `LiveRack::show_embedded_editor(node, title)` for a Linux owned window; `show_editor(node)` opens a floating editor. `resize_editor(node, [width,height])`, `scale_editor(node, scale)`, `editor_size(node)`, `focus_editor(node)` and `close_editor(node)` control the active GUI. `capture_editor_ppm` captures an owned Linux parent for diagnostics. These are main-thread control operations, never audio-callback operations.

```rust,ignore
owner.show_embedded_editor(0, "My plug-in")?;
owner.scale_editor(0, 1.25)?;
owner.resize_editor(0, [1000, 750])?;
// GUI-loop tick, on the creating thread:
owner.poll();
for event in owner.drain_editor_events() {
    println!("{event:?}");
}
owner.close_editor(0);
```

For an existing native parent:

```rust,ignore
use soniclaw::host::editors::ParentWindow;
// SAFETY: this X11 window belongs to this GUI thread and outlives the editor.
unsafe { owner.attach_editor(0, ParentWindow::X11(xid))?; }
```

`ParentWindow::Win32` and `Cocoa` accept valid platform-specific handles through the same unsafe API. Automatic parent ownership and screenshot/focus helpers are implemented for Linux X11. Win32/Cocoa handle paths are not runtime acceptance on those systems. An application supplies its own toolkit event integration and parent lifetime.

`drain_editor_events()` returns bounded `Resized`, `Closed` and `Error` observations; at most 128 are queued, with overflow recorded in event-drop statistics. External-parent applications must react to resize observations in their own toolkit. Owned parents are resized by the host. A plug-in can refuse embedding, host resizing or scaling; the method returns an error and the caller can offer supported controls or a floating editor.

## Timer and POSIX FD services

The host advertises CLAP timer support, plus POSIX FD support on Unix. Each instance permits 64 timers and 64 FD registrations. Timer periods are 1–60,000 milliseconds. Registrations are main-thread only; invalid IDs, flags, duplicates and excess capacity are rejected. File descriptors remain owned by the plug-in, which must unregister before closing them.

`LiveRack::poll()` drives these services. Timer dispatch coalesces a late tick rather than generating an unbounded backlog. FD readiness uses a zero-timeout poll and invokes the plug-in after releasing registration locks, so a callback may unregister itself. Registration existence is checked again before dispatch. `service_stats(node)` exposes current registrations and dispatch counts. There is no background UI thread or promise of hard timer precision; stalled GUI loops delay all services.

## Lifecycle and thread ownership

The rack remains bound to its creating thread. Editor creation, service pumping, size/scale negotiation and destruction all occur there. Native processing keeps its existing direct or buffered audio ownership. Timer/FD registration locks are control-thread services, with wrong-thread requests rejected before locking. Native code remains trusted and in process; a malicious or broken plug-in can crash or stall the host.

Closing destroys the plug-in GUI before dropping an owned parent, clears stale close/resize requests, and permits reopening. The exported Soniclaw CLAP GUI retains physical dimensions, converts to logical dimensions when recreating the underlying editor, and applies retained scale once. This avoids compounded scale on successive reopen operations.

## Persistence and migration

Host session schema 1 gains the additive `EditorSettings.embedded` boolean, default false for older documents. Existing floating-editor sessions remain readable. Open state, selected mode, accepted scale and size are captured, including preferences retained after closing. Restore applies scale/size through the owner API. Owned embedded restore is Linux-specific; callers on other platforms must provide an external parent or recover with a floating editor. No native pointer is serialized.

## Bounds, errors and recovery

Dimensions must be 64–8192 physical pixels per axis; scale must be finite and 0.5–4.0. API mismatches, invalid/null parents, missing extension methods and rejected negotiation return errors. Failed attachment destroys a created GUI before returning. Do not retry a rejected scale continuously every GUI tick. Drain errors, keep audio ownership intact, close the editor and offer a supported mode. The host validates supplied handles structurally; only the caller can guarantee their native validity.

## Validation and limits

The [release report](validation/v022/REPORT.md) records fresh compiler/test/documentation checks, native Xvfb rendering, stable resize/scale reopening and fixture timer/FD delivery. Xvfb/software rendering is Linux GUI evidence, not physical-device latency, DAW compatibility, display-server portability, full accessibility support or Windows/macOS acceptance. Wayland-native parenting, toolkit adapters and automated keyboard-navigation coverage remain future work.
