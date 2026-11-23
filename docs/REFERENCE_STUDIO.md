# Unified reference studio (0.52)

This release joins the prepared session engine, clip/piano-roll views, automation editor, parameter mixer controls, multitrack recording, reversible freeze and portable project resources in one application example. Its project-folder API lives in `soniclaw::studio::project` and can be used without the GUI or native device dependencies.

## Run the workflow

```sh
cargo run -p soniclaw-desktop-studio -- --demo new-studio-demo
cargo run -p soniclaw-desktop-studio --features gui -- --gui new-studio-demo/original
```

The deterministic demo records bass/lead/master stems, saves a frozen synth track with its restore manifest, reopens the entire saved folder, restores the instrument and renders reopened audio. It writes `original/`, `recorded-take/`, `freeze-cache/`, `saved/`, `restored/`, `reopened.wav` and `report.json`. A report is produced only after all assertions succeed. This workflow uses software-generated audio and exercises the actual session/capture/cache/project APIs, not a mock project serializer.

The GUI opens a project without starting playback. Play opens the default output through `DeviceManager`, explicitly requesting the project's stereo sample rate and internal maximum block. It fails visibly if that specification cannot be honored. Transport, clip playback, instrument/inserts, auxiliary effects and automation use `studio::live_pair`; the example's fixed live latency budget is 4096 frames. Knobs/faders bind shared parameters. Select a track to edit its arrangement and first MIDI clip, edit automation, then Apply edits to rebuild/restart playback. The current clip view retains its existing viewport limits until the following editor release.

Record stems captures every track plus master through the multitrack recorder; Finish take finalizes its float WAVs and manifest. Takes are listed as saved resources. Freeze renders the selected track from source frame zero for four seconds plus one second of tail, disables that track's live MIDI input and replaces its processors with a streamed stem. Unfreeze restores them. Choose a source duration suitable for the project through the reusable freeze API when building your own product. The first freeze implementation still requires memory-backed source sessions.

Save new folder stops audio, captures shared controls and native state, and exclusively creates the typed destination. Open validates a candidate before replacing the active project. Applying, saving and freezing are deliberate control-thread operations in this example and can pause the UI. They never perform filesystem work on the device callback. The next steps can put these operations behind a product-specific job service; this release does not claim a fully asynchronous editor.

## Portable project API

`Project::new(name, document, assets, disk)` validates an in-memory project. It owns the session document, immutable decoded assets, explicit disk catalog, freeze restore manifests and completed take directories. `save_new(directory)` copies resources into a new folder and publishes `project.json` last as the commit marker. Existing destinations are refused. On a normal error it removes only the directory it created; interruption can leave an incomplete folder, so use a new destination on retry. File sync is performed for the manifest, not a universal power-loss durability guarantee for every asset/directory entry.

`Project::open(directory)` validates schema and resource identities, rejects escaped asset paths, verifies asset SHA-256, decodes memory WAVs within budget, reconnects disk paths to the new folder, validates take files and relocates freeze manifests. It does not execute plug-in code. Move or copy the entire project folder to relocate resources. The original memory asset IDs and explicit disk IDs use separate catalogs; they may intentionally have the same text ID.

`clips(track_id)` returns a validated editable clip document. `replace_clips` validates the full candidate before committing that track's audio/MIDI changes. `freeze` renders/applies a new cache before committing project changes; `unfreeze` restores the saved track and removes the playback cache catalog entry. Original recordings remain unchanged. Completed takes are copied and validated with the existing take schema.

## Explicit native CLAP integration

Build with `gui` and pass trusted canonical module paths after the project folder:

```sh
cargo run -p soniclaw-desktop-studio --features gui -- \
  --gui my-project /absolute/path/trusted-effect.clap
```

Enter that path and a product ID in Native CLAP insert, then add it to the selected track and Play. The application factory recognizes `kind: "clap"` descriptors only when their canonical path belongs to this startup allowlist. Saved JSON cannot grant itself trust. Unknown kinds fail compilation. The native owner stays on the GUI/main thread; a direct CLAP audio lease runs in the session processor. UI controls send live native parameters and the main loop polls host services. Stop destroys the device callback before capturing the single-slot native session state and releasing owners. Reopen requires explicitly trusting the module again.

This example integrates trusted in-process CLAP inserts, not every hosting backend or an isolation policy UI. VST3 and isolated hosts retain their separate existing APIs. Native factory state capture includes opaque plug-in state/parameters supported by the host session implementation, not ongoing voices or DSP delay histories. External native binaries are not copied into project folders; install/reselect compatible modules separately. The software demo does not load external native plug-ins or test physical devices.

## Bounds and migration

Project-folder schema 1 is distinct from the earlier `.soniclaw` bundle format. Existing bundle/workbench APIs remain available; there is no implicit conversion. Limits: 16-MiB project JSON, 1024 memory+disk resources, 64 Mi decoded float samples, two GiB of copied audio resources, 32 freeze manifests, 256 completed takes and the underlying studio session bounds. Memory resources are mono/stereo float32 WAV at the project rate. Disk sources retain their original encoded bytes and must satisfy their consuming decoder. Duplicated frozen playback/cache resources count toward the copy limit. Do not delete the original source assets if you need to unfreeze later.

Tests cover portable PCM/document roundtrip, exclusive creation and corruption rejection while preserving the previous in-memory project. The executable validates recording/freeze/take relocation and restored synth output with zero callback allocations. [Executed release checks](validation/v052/REPORT.md) distinguish software evidence from pending native GUI, physical audio and target-machine acceptance.
