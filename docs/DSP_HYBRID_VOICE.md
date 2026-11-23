# Hybrid polyphonic instrument

Create a HybridSynth from a schema-1 Patch, a reviewed band-limited WavetableBank and a mono Sample. The oscillators and sample region players are prepared together for 1–64 voices. Each voice mixes a root-key-pitched one-shot with a morphable wavetable, one shared ADSR, a shared cutoff recipe with independent filter state, and a per-note sine LFO for morph. AudioProcessor accepts MIDI 1 note-on, note-off and all-notes-off at exact frame offsets. The oldest active slot is selected if all voices are busy. A 32-frame blend from the previous sample value suppresses onset discontinuities. Stack overlapping same-key notes only if a shared release is acceptable.

Prepare on the control thread at 8–192 kHz, mono or dual-mono stereo. Processing, note dispatch, reset and stealing use prepared slots. Retire asset owners away from the audio thread. Output sums voices directly, so budget gain or add a limiter. Note pitches that exceed the region player's supported rate disable the sample layer for that note. Invalid block shapes or ordered event validation errors clear the output without advancing. Reset silence flushes all notes.

Serialize Patch with the app's asset identifiers. Restore both assets and reprepare. Earlier synth and sampler schemas are unchanged. This version layers wavetable and sampled attacks. Granular layering, individual-note expression, a full modulation matrix and a native GUI are separate additions. The offline reference example plays through the real MIDI/render API, runs 32 voices, exercises stealing and release, checks silence around note onset, round-trips the patch and records CPU time and allocations.

Run `cargo run -p soniclaw-dsp-suite --locked --offline -- hybrid-voice NEW_DIRECTORY`. [Executed checks](validation/v0_109_0/REPORT.md).


For live audio run `cargo run -p soniclaw-hybrid-instrument --locked --offline` on a machine with a mono or stereo output device. Enter `on 60 100`, `off 60`, `panic` or `quit`. These commands use the native event queue. The release checks compile this example and exercise its offline DSP counterpart. Live device playback requires a local audio device.
