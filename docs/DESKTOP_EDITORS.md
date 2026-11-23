# Owned desktop plug-in editors

CLAP hosted editors can now open in framework-owned parents on Windows, macOS
and Linux. Applications no longer need to create and pass raw Win32/Cocoa
handles for this convenience path. Editor creation, parenting, resize, scale,
focus and destruction remain on the GUI/main thread.

## Workflow and public API

Use an existing stopped/prepared `host::live::LiveRack` created on the main
thread. In the application's GUI loop:

```rust,no_run
# fn example(rack:&mut soniclaw_host::live::LiveRack)->Result<(),soniclaw_host::HostError>{
rack.show_embedded_editor(0,"My instrument")?;
rack.focus_editor(0)?;
rack.resize_editor(0,[800,600])?; // may be refused by a fixed-size native editor
// Tick from the application event loop; dispatches services and queued native GUI events.
rack.poll();
for event in rack.drain_editor_events(){println!("{event:?}");}
rack.close_editor(0);
# Ok(()) }
```

`show_embedded_editor` now uses an owned HWND on Windows and a retained NSWindow
with an NSView container on macOS; Linux keeps its X11 implementation.
`focus_editor` is available for all three. `editor_size`, `resize_editor`,
`scale_editor`, `close_editor` and event draining keep their existing API.
External embedding still uses unsafe `attach_editor(node,ParentWindow)` and
requires the caller to keep the selected native parent alive through GUI
teardown. Floating editor support is unchanged.

Workbench's existing “embedded editor” action and session reopen behavior use
the same convenience API; no separate parent-window implementation is needed
in applications using `LiveRack`. The existing VST3 `Instance::open_editor`
continues to use the backend's native owner and platform event service.

## Native lifecycle and DPI behavior

Windows WndProc records close, resize and DPI requests; it never invokes the
plug-in. WM_CLOSE is deferred so native `hide`/`destroy` completes before the
HWND is destroyed. A poll drains up to128 queued native messages for that
parent and children, then applies coalesced requests. A DPI change discards a
stale simultaneous WM_SIZE, negotiates `set_scale` and resizes the parent to the
accepted native dimensions. A refusal produces `EditorEvent::Error` and restores
the last accepted editor dimensions. The application selects its process DPI
awareness policy; the library does not change global process awareness.

AppKit construction requires `MainThreadMarker`. The host retains both NSWindow
and its container NSView, sets `releasedWhenClosed=false`, and releases them
after native GUI destruction. A user close is distinguished from minimization
or application hiding. Focus raises the editor and selects its responder.
Cocoa dimensions are logical points; AppKit handles Retina backing scale.
Backing-scale changes are not fed back as another CLAP logical-size multiplier.
Explicit `scale_editor` requests can be refused by a Cocoa plug-in. The host
application must keep its AppKit event loop running; this API does not create a
second NSApplication loop.

X11 retains the existing Xlib connection, WM_DELETE_WINDOW handling, focus and
configure-size negotiation. This owned parent is X11; a native Wayland-only
embedding protocol is not introduced. All parent windows stay main-thread-only
through their containing rack/instance owner. Closing or dropping the rack
calls native GUI destruction before releasing an owned parent, then follows the
existing worker shutdown rules before native module unloading.

## Bounds, persistence and recovery

Accepted editor size remains64–8192 per dimension and requested scale0.5–4.0.
Window titles cannot contain NUL and are limited to1024 bytes on new Windows/
Cocoa parents. Unsupported native GUI APIs, creation failures, rejected resize
or scale return errors/events; audio may continue. Close/reopen after a GUI
failure and reconstruct the owner after a native structural restart. Native
plug-in code remains trusted; in-process GUI crashes are not contained by the
separate v0.29 headless isolation owner.

Existing session `EditorSettings` stores open/embedded/size/scale preferences.
Schema1 does not change. Main-thread session capture/reconstruction preserves
these values; native HWND/NSView/X11 handles are never serialized. Window screen
position, monitor selection and OS focus state are not persisted. Reopening
negotiates with the actual editor; do not assume every plug-in supports every
saved size or scale.

This release also corrects VST3 native flag casts for generated Windows ABI
bindings in the earlier direct/musical paths. The source fix is included here;
prior Linux-only validation is preserved in historical release reports.

## Reproduce validation

```sh
cargo test -p soniclaw-host -p soniclaw-vst3 --all-features --locked
cargo clippy -p soniclaw-host --all-targets --all-features --target x86_64-pc-windows-msvc --locked -- -D warnings
cargo clippy -p soniclaw-host --all-targets --all-features --target aarch64-apple-darwin --locked -- -D warnings
cargo build -p soniclaw-reference -p soniclaw-host-fixture -p soniclaw-sdk-tools --features soniclaw-sdk-tools/vst3-tools
# In a Linux X11 session, each output path must be new:
target/debug/soniclaw-sdk-tools --editor-demo target/debug/libsoniclaw_host_fixture.so new-fixture-report
target/debug/soniclaw-sdk-tools --editor-demo target/debug/libsoniclaw_reference.so new-reference-report
```

The fixture validates ABI/thread/lifecycle and timer/fd services but mocks its
native child GUI. The reference run embeds a real baseview/egui editor, resizes,
scales, focuses, closes and reopens, checking stable dimensions. Its captured
pixels and structured report are retained with release evidence. Cross-target
Clippy/check verifies Rust bindings and conditional code, not linking or native
Windows/macOS execution. Windows per-monitor DPI moves, native keyboard focus,
macOS hide/minimize/Retina/close and external DAW acceptance remain pending on
those actual platforms. See the validation report for exactly executed checks.
