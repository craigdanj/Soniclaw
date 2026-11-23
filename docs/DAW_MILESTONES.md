# DAW implementation milestones

This series implements the three requested software workflows in order. A completed source/API/UI workflow is separate from target-machine acceptance. Physical audio/MIDI, arbitrary native plug-ins and Windows/macOS runtime acceptance stay pending until actually executed.

| Milestone | Releases | Delivery goal |
|---|---|---|
| Usable recording DAW | 0.72–0.76 | Create/edit tracks in the UI, save in place, independently record hardware channels, calibrate timing and manage audio/MIDI takes. |
| Complete music-production workflow | 0.77–0.80 | Replay/edit expressive MIDI, arrange any selected clip, manage tempo/markers, use stock tools/media and align grouped audio. |
| Complete mixing workflow | 0.81–0.85 | Submix/sidechain routing, independent cues/outputs, studio VST3 management, native automation and complete native mix/stem export. |

Each release has a detailed guide, runnable example and executed checks. The final integration example must create, record, edit, save/recover, mix and export without editing project JSON. Native device/plug-in acceptance needs real target systems and is never inferred from virtual-input tests. See `daw-milestones.json` for delivery/acceptance state.

## Implemented through 0.85.0

All three software workflow series are implemented with their documented limits. The final [integrated example](../examples/daw-delivery/src/main.rs) creates a blank project, records and places two virtual audio tracks, records/replays controller-bearing MIDI, recovers pending MIDI, adds a stock instrument, configures submix/sidechain/cue routing and a trusted native master, saves two portable generations, removes original take folders, reopens, and exports a mix plus five routed stems. It uses public APIs without hand-editing project files.

The Linux owned-fixture run produced six 6144-frame files and an exact expected bus/return-to-master relationship. Separate checks cover native note-identity/expression continuity, owner revocation, source-specific latency trimming, cancellation cleanup and callback partition invariance. [Full workspace verification](validation/v0_85_0/REPORT.md) and [machine-readable integration results](validation/v0_85_0/integrated.json) record the evidence.

Target-system acceptance remains separate: physical audio/MIDI and calibration, native editor interaction, third-party products, Windows/macOS runtime, installers and sustained large-session stress have not been established by these virtual and owned-fixture checks. This milestone record does not claim commercial DAW completeness or JUCE parity.
