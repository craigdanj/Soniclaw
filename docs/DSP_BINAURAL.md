# Binaural spatial rendering

Use `soniclaw::dsp::binaural::{HrtfSet, Direction, Binaural}` to render a mono
source to stereo headphones. Open an AES69 SOFA head-related impulse-response
dataset with the desired processing rate. The reader interpolates between local
measurements and includes the file's per-ear time offsets in the FIRs. Direction
uses front as azimuth zero, positive azimuth toward the listener's left, and
positive elevation up. Distance selects a dataset position; it does not add a
separate distance attenuation model.

```sh
cargo run -p soniclaw-dsp-suite -- binaural output/headphones
cargo run -p soniclaw-dsp-suite -- binaural output/custom path/to/hrtf.sofa
```

The example uses Gardner and Martin's 1994 MIT KEMAR normal-pinna dataset and
moves a noise source from left to right. Listen on headphones. See the
[dataset notice](../examples/dsp-suite/assets/KEMAR_DATASET.md) for attribution.
The reader uses the `sofar` crate and resamples datasets when necessary.

Call `Binaural::prepare` to get a control owner and audio owner. On the control
thread, call `set_direction` to interpolate new FIRs and enqueue a prepared
replacement, and collect retired kernels there. The audio owner calls
`process_mono` with a mono slice and matching stereo output. It crossfades
between its engines without allocating or locking. A full queue is recoverable:
coalesce direction changes and retry the latest position. Rebuild the pair for
a rate or format change. The convolver adds no algorithmic buffering in this
mode. Physical delays in the supplied HRIRs remain audible.

The importer accepts supported two-receiver FIR datasets through 32 MiB,
up to 32,768 measured positions, up to 8,192 taps at the target rate,
8–192 kHz and up to 40 ms of reported time offset. This is one mono source
per renderer. HRTF suitability depends on the listener. Direction changes
are crossfaded at control rate. This API leaves head tracking and combining
multiple sources to the application. Persist the licensed dataset reference
and source directions. Previous convolution state and project files are valid.

[Executed checks](validation/v0_127_0/REPORT.md).
