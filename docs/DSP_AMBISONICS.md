# First-order Ambisonics

Call `ambisonics::encode` for a mono sample and source angles. Sum
source frames to mix a soundfield. The four AmbiX channels use ACN
order W, Y, Z, X with SN3D normalization. Azimuth is positive toward
the left from front and elevation is positive upward.
`Rotation::new(yaw, pitch, roll)` prepares an active Euler rotation.
Apply it to each frame, or apply the inverse listener orientation
for head tracking. The omnidirectional W channel stays unchanged.

`decode_six` yields a basic virtual speaker decode. For headphones,
prepare a `HeadphoneDecoder` from an existing SOFA `HrtfSet` at the
project sample rate. It filters six virtual directions with the
existing stereo convolution engine and sums the ears. Dataset acoustic
delays are retained. The decoder uses first-order spatial resolution;
localization varies with dataset and listener.

```sh
cargo run -p soniclaw-dsp-suite -- ambisonics soundfield-demo
```

The demo writes a four-channel WAV in AmbiX order and a stereo headphone
render. It uses Bill Gardner and Keith Martin's 1994 MIT KEMAR dataset.
See the [dataset notice](../examples/dsp-suite/assets/KEMAR_DATASET.md).
Use the recorded channel order explicitly when importing the raw WAV.

Prepare the dataset and convolver off-thread. Keep a single audio
owner per decoder. Process up to 8192 frames per call. Bad shapes or
invalid values return an error before writing. Reset convolution
history at discontinuities. Save the dataset asset and orientation
recipe with the project. Earlier per-source binaural projects remain
compatible.

[Executed checks](validation/v0_136_0/REPORT.md).
