# Third-party components

Soniclaw first-party code is MIT OR Apache-2.0. Dependencies retain their licenses; Cargo.lock pins Rust dependency versions. No crate has been published by this work.

| Component | Purpose | License/source |
|---|---|---|
| egui / eframe | Shared audio controls and native apps | MIT OR Apache-2.0; https://github.com/emilk/egui |
| egui-baseview 0.7.2 / baseview | Host-parented editor | MIT OR Apache-2.0; https://codeberg.org/RustAudio/egui-baseview and https://github.com/RustAudio/baseview |
| rustfft 6.4.1 | Partitioned convolution | MIT OR Apache-2.0; https://github.com/ejmahler/RustFFT |
| CPAL / midir | Native audio / MIDI | Apache-2.0 / MIT; RustAudio projects |
| rtrb | Bounded real-time queues | MIT OR Apache-2.0; https://github.com/mgeier/rtrb |
| hound | WAV decoding/encoding | Apache-2.0; https://github.com/ruuda/hound |
| serde / serde_json | State and session serialization | MIT OR Apache-2.0; https://github.com/serde-rs |
| clap-sys / CLAP SDK | ABI declarations / independent host headers | MIT OR Apache-2.0 / MIT; https://github.com/micahrj/clap-sys and https://github.com/free-audio/clap |
| clap-wrapper | VST3/AUv2 format translation | MIT; https://github.com/free-audio/clap-wrapper |
| Steinberg VST3 SDK v3.8.0_build_66 | VST3 wrapper and validator | MIT at this pin; https://github.com/steinbergmedia/vst3sdk |
| Apple AudioUnitSDK 1.1.0 | AUv2 build recipe | Apache-2.0; https://github.com/apple/AudioUnitSDK |

SDK and wrapper sources are downloaded explicitly by the format build script and are not vendored into the framework source archive. Their upstream license files apply when distributing binaries. The package helper can include a collected license directory alongside the first-party licenses. VST naming/trademark requirements are distinct from source-code licensing.

The reference drum PCM is synthesized deterministically by first-party code. It does not contain third-party sample recordings. The validator executables are development tools, not bundled plug-ins.
