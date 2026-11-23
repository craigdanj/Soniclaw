# Soniclaw integration changes

Based on vst3-host 0.11.1 (MIT), copyright 2026 Helge Sverre. Original LICENSE and upstream source retained.

`src/internal/plugin_impl.rs`, `service_host_requests`: query latency while active; skip stop/deactivate/reactivate when a latency-only notification reports an unchanged value. When a transition is necessary, refresh latency/tail after reactivation. This prevents repeated notifications and instrument resets with CLAP-wrapper products which report zero latency while inactive.

The Soniclaw native VST3 regression checks bounded restart notifications, sustained multi-block MIDI audio, timed gain, opaque state and callback ownership. Upstream device/process-isolation default features stay disabled. No upstream affiliation is implied.

## v0.24 prepared direct integration

Adds an unsafe native processor lease to the backend, a main-thread controller-only parameter mirror, and a restart-flag drain which does not transition the component. Soniclaw owns the separate fixed-storage COM input queues and guarantees lease revocation before backend/module destruction. These entry points must not be used to run concurrent native processing paths.

## v0.27 musical context

Adds a native musical-context update for bar/meter/PPQ/cycle/recording flags without lifecycle transitions, and exposes the processor-only automation queue to the integration worker. Native protocol fixture and ordinary VST3 regression results are recorded under `docs/validation/v027`. Isolated backends explicitly reject the new musical context until supported.

## v0.30 cross-target validation

Explicitly cast native ProcessContext flags to u32: generated Windows bindings use signed enum constants while Linux/macOS use unsigned constants. Windows MSVC and macOS ARM64 host cross-compilation and Clippy are recorded under `docs/validation/v030`; these are not runtime GUI acceptance claims.

## v0.88 Studio isolated musical context

The matching helper protocol now forwards full MusicalTransport (PPQ, bar, meter, cycle, play and recording flags) rather than rejecting Studio musical processing. The helper and parent must be rebuilt together. Existing typed event transport remains unchanged. The owned native Studio isolation fixture exercises this path; see docs/validation/v0_88_0. Historical v0.27 isolation limitation is superseded by this addition.
