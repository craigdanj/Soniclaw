## v0.9 additions

The tenth tab, Audio, configures managed output and saved preferences. A seventh Tape Colour preset and the registered processor menu demonstrate a separate DSP crate. See [device management](DEVICE_MANAGER.md) and [custom DSP registration](REGISTRY.md). Older sections retain their release-specific scope.

## Project workflow update in v0.8.0

The left panel scrolls and provides background JSON/bundle/recovery loading, single-file pack/import and optional recovery autosave. See the [workflow guide](PROJECT_WORKFLOWS.md) for exact behavior. The older folder export remains available.

# Workbench additions in v0.7

Read [SDK guide](SDK_070.md) for new panels, Poly Motion/Sampler Studio, four CLAP
products and live graph/hosting contracts. The v0.6 workflow below remains the
starting point; its serial/stop description is superseded for the new Apply actions.

# Soniclaw DSP Workbench — v0.6

The native workbench is an executable rack/project editor. It shares the same
project factory and atomic parameters as the library and CLAP products.

```sh
cargo run -p soniclaw-workbench
# Hardware-free rendering, also available without native libraries:
cargo run -p soniclaw-workbench --no-default-features -- --render /new/output/folder
# Export Multiband Workbench FX + Granular Workbench as CLAP:
cargo build --release -p soniclaw-workbench
```

Linux GUI builds need the native dependencies described in [setup](../README.md#build-and-run).
The native executable opens no audio device at startup. A worker renders a
2-second preview; **Play** opens the default output device. FX preview/live mode
uses a synthetic 220 Hz test tone, not a microphone. Instrument mode starts a
C-major chord and accepts the on-screen keyboard. Stop releases the device.
Keep output volume conservative when editing resonance/drive or removing a limiter.

## Workflows

- Choose **Motion Rack**, **Multiband Glue**, **Granular Cloud** or **Spectral Space**.
- Select a processor, edit its named controls, or reorder/remove/add rack nodes.
  Base values are saved; the live readout includes modulation during playback.
  Changes to controls are smoothed in the DSP. Undo/redo records committed edits.
- Use **Modulation** to add LFO/envelope/macro sources and routes. Apply stops
  audio and rebuilds transactionally. BPM is a runtime transport control.
- **Assets** imports a WAV into the project and binds it to a selected granular
  or convolution node. Source and IR assets have different musical meanings.
- Save JSON in the project root. Load resolves assets relative to that JSON.
  Export a portable folder to copy WAVs and rewritten relative paths together.
  Destinations must be new; no installer or system plug-in directory is modified.
- **Render preview** takes a snapshot of current base controls and project state.
  The last completed preview can be exported as 48 kHz stereo WAV. It may predate
  later knob edits; render again for an updated WAV. A running render is not canceled.

Spectrum and waveform panels display rendered output or a rolling live 1024-sample
window at the actual device rate. Live telemetry is bounded and can drop packets;
its spectrum is diagnostic when overloaded. The response curve is an explicitly
labeled linear low-pass approximation, not a measured nonlinear ladder response.
CPU displays processing wall time divided by the audio time budget, excluding
control-side reconstruction/assets. Debug builds and shared VMs are unsuitable
for performance decisions. Latency comes from the actual constructed graph.

The GUI uses serial racks; branching graphs can be authored through project APIs.
Structural edits stop/rebuild rather than swap a running graph. File import/save
runs on the UI thread; large files can pause the interface. This is a developer
workbench, without native file dialogs, arbitrary third-party plug-in hosting,
a timeline, graphical sample-map editing or an IR browser.

## Resources and plug-in exports

`SONICLAW_WORKBENCH_ROOT` can override the source example's project root. It must
contain `assets/grain-source.wav` and `assets/stereo-ir.wav`. Preserve this root
when loading the example shared library; these are external demonstration
resources, not embedded signed/installable plug-in bundles. Default builds also
provide the shared generic parameter editor when embedded CLAP GUI is supported;
the standalone rack editor is not the embedded editor.

The headless walkthrough writes four full project JSONs/audio renders, an
IR-crossfade render/report and offline pitch/time examples. It copies both WAV assets alongside
the JSONs so they can be loaded from the render folder; GUI **Export bundle**
exports an individual project with the same relative-path approach. No external samples or third-party sound licenses are needed.

See [DSP/module APIs](DSP_MODULATION.md), [migration](MIGRATION_060.md),
[release notes](releases/v0.6.0.md) and [verification](validation/v060/REPORT.md).

For repeatable native captures, `--screenshot /new/image.png --preset 0 --tab 1`
waits for the preview, captures a native window and exits (preset 0–3, tab 0–2).
