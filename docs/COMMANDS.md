# Application commands (0.36)

## Using commands

`soniclaw::commands::Registry` stores up to 1024 stable IDs, labels, categories and runtime enabled/checked state. Register once, refresh state from the application, then dispatch the returned ID. The same registry feeds `ui::command_button`, `command_menu` (also usable inside an egui response context menu) and `command_shortcuts`. The application composes menus and toolbars; no OS global shortcut registration occurs.

`cargo run -p soniclaw-clip-studio -- NEW_DIRECTORY --gui` demonstrates the Edit menu, toolbar and Ctrl+Z/Ctrl+Y undo/redo for actual clip and piano-roll changes. Rendering and save remain explicit operations. The demo records each changed UI frame; applications should use history groups to coalesce a drag into one operation.

## API and transactions

`History<T: Clone>::new(capacity)` accepts 1–1024 snapshots. `edit(&mut document, closure)` rolls back the entire clone if validation fails and records the previous snapshot on success. `Registry::invoke` skips missing/disabled commands and applies this transaction. Use `begin_group`, several edits, then `finish_group`; `cancel_group` restores the original document. Nested groups fail; undo/redo are unavailable until a group ends. A new edit clears the redo branch. `record(previous,label)` integrates editors that already implement transactional validation. Do not record an operation that changed nothing.

```rust
use soniclaw::commands::History;
let mut document = vec![1];
let mut history = History::new(64).unwrap();
history.edit(&mut document, |d| { d.push(2); Ok::<_, ()>(()) }).unwrap();
assert!(history.undo(&mut document));
assert_eq!(document, vec![1]);
```

## Shortcuts and persistence

Schema 1 `Keymap` maps registered stable IDs to exact key/modifier combinations. Loading is transactional, bounded to 256 KiB, rejects unknown IDs, unsupported schema, duplicate bindings and malformed key names. Core key strings are portable tokens; the UI adapter matches egui key names. Applications should validate platform-supported keys before presenting configuration. `control` is Ctrl and `command` is macOS Command; choose platform defaults explicitly. Text editing suppresses shortcut routing, repeats are ignored and a matched event is consumed. Modal dialogs should disable application dispatch. Native OS menu accelerators are not installed automatically.

Save `keymap_json`, reload only after registering commands. Keymaps are independent of project JSON; history contains process-local snapshots and is not serialized. There is no change to existing project schemas. Renamed action IDs require application migration before load. Runtime enabled/check state is not persisted.

## Ownership, limits, errors and validation

All registry, UI and history operations run on the control/UI thread; cloning, allocation and destruction are expected. Rebuild/publish prepared audio after document changes using the engine's safe boundary, never from a callback. Snapshot memory is application-owned: the entry count is bounded, the size of generic T is not. Use lightweight immutable assets or a product-specific byte budget for large documents. Clear history after replacing a project; cancel an active group before normal navigation. Edits that panic are outside the fallible rollback contract.

Tests cover failed-edit rollback, grouped cancellation/undo/redo, bounded eviction, redo branching, exact keymap roundtrips/conflict rejection and disabled dispatch. The clip editor is a runnable integration. This is an application command framework, not a scripting language, cross-process transaction journal or automatic host automation system.
