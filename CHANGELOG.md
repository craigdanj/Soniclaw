# Changelog

Detailed [release notes](docs/releases/README.md) are maintained for every major
and minor release, including pre-1.0 versions.

## Unreleased

- Added versioned release notes for 0.1.0, 0.2.0 and 0.3.0, a release-note
  template, and a maintenance checklist for future major/minor releases.

## 2026-11-16 — Framework renamed to Soniclaw

Renamed current crates, Rust imports, Cargo CLI, environment flags, example titles, build/package tooling and documentation. Release version remains 0.3.0. Existing demo plug-in product IDs, parameter IDs, preset schemas, bundle registration IDs and AU component codes are retained for compatibility. New generated plug-ins use org.soniclaw IDs. Legacy session settings are loaded when no new-name settings file exists. Frozen benchmark sources and recorded validation output remain unchanged.

## 0.3.0 — 2026-11-15

Added a scoped CLAP denormal guard, processing optimizations and reproducible v0.2/v0.3 benchmarks; host-parented egui editors with bounded begin/value/end host gestures and explicit backpressure; real CLAP sidechain ports; an independent exported-ABI compatibility/heap/GUI interaction host; parametric EQ, compressor, peak limiter, gate, chorus, reverb, partitioned FFT convolution and oversampled drive; multisample banks, velocity layers, round robin, sustain/pitch bend and crossfaded stealing; bounded background forward WAV streaming; a four-product reference plug-in bundle and a standalone drum app sharing its factory; pinned static VST3/AUv2 wrapper integration; app/effect/sampler templates, CLI checks, package/installer recipes, license collection and expanded documentation.

The shared UI stack moves to egui/eframe 0.36.1. Audio preparation supports finite rates up to 1,536,000 Hz, including the VST3 validator’s extreme-rate case; actual devices still negotiate their own supported rates. Embedded Linux UI uses a pinned baseview backend and an opaque 2D GL framebuffer configuration. Existing gain/synth product and parameter IDs are retained.

Linux verification is recorded in docs/VALIDATION.md. AUv2/macOS/Windows compilation/runtime, real DAW project tests, physical loopback/unplug/replug, signing/notarization and Windows/macOS installer execution remain pending. Multisampler PCM is preloaded; the separate stream transport is forward-only and not a per-voice disk sampler. No publishing or installation is performed automatically.

## 0.2.0 — 2026-11-15

Added native duplex input and channel maps; validated sample-rate/buffer requests; saved session settings and control-thread recovery; monitored/bypassed live effects; dry/processed background stereo PCM16 recording with loss accounting and duration preservation; exact parameter-event scheduling and offline automation; separate main/sidechain bus contracts and a sidechain compressor; graph latency compensation; suspend/resume; a real CLAP adapter with gain/synth exports, state, parameters, ports and lifecycle; a CLAP project template and bundle helper; reusable knobs, XY, waveform, keyboard, envelope/EQ controls and meter ballistics; Live FX and UI gallery examples; expanded integration/allocation/recording tests and upstream CLAP validation.

Native hardware testing and interactive DAW validation remain pending. No embedded CLAP GUI, VST3/AU, CLAP sidechain port, native installers or JUCE feature-parity claim.

## 0.1.0 — 2026-11-15

Initial foundation: core processing, DSP, synth/sampler, serial chains and DAG routing, output host, MIDI, egui controls, WAV I/O, JSON presets, format-independent plug-in lifecycle and app generator. Included desktop synth and offline render examples.
