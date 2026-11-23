# Shared background jobs — 0.64.0

## Workflow and integration

Reference Studio now owns one scheduler for waveform analysis, warp-cache rendering and built-in track freezing. Its header shows active, queued and retained jobs and reserved MiB. The existing warp/freeze Cancel buttons request cancellation; accepted work does not run in the callback. Opening/restoring another project invalidates old results; warp draft changes cancel the accepted preview and advance its generation. Apply still verifies actual project/source/timeline fingerprints. Completed results are explicitly applied on the stopped/control owner.

Native plug-ins requiring main-thread owners retain their existing synchronous freeze path. Project autosave retains its serialized per-store recovery writer. These ownership-specific paths are not secretly moved onto worker threads. The scheduler is the shared facility for compatible Send tasks; other clients can opt in without replacing existing single-job APIs.

## API and example

`studio::jobs::{Scheduler, Limits, Priority, Context, Ticket<T>, Stats, Error}`. Construct one Scheduler with validated limits. `submit(key, epoch, priority, reserved_bytes, closure)` accepts an owned `Send + 'static` closure and typed result, reserving capacity before queueing. Matching keys cancel an older accepted task. A rejected submission leaves the previous job untouched. Three priorities are Interactive, Normal and Maintenance; within each, FIFO. An already active job is never preempted. Continuous interactive submissions can delay maintenance; no deadline or starvation-free scheduler is claimed.

The closure receives Context: inspect `cancelled`, share `cancel_flag` with existing cooperative workers and publish bounded progress using `progress(done,total)`. A Ticket supplies progress, cancel and `poll(current_epoch)`. Poll consumes a completed result once; an epoch mismatch returns Stale. Explicit cancellation is also checked before and after the closure, preventing cancelled results from being accepted even if work completed. Closure errors and panics become Failed; a panic hook may still log the panic. Error, cancellation and stale results release their reservation when polled/dropped.

```sh
cargo run -p soniclaw-studio-jobs -- NEW_DIRECTORY
```

The example demonstrates typed output, progress, over-budget rejection, stale-epoch rejection and cooperative cancellation. Unit tests additionally verify queued priority/FIFO, replacement, retained capacity and shutdown cancellation.

## Ownership and budgets

Default: two workers, sixteen retained jobs, 256 MiB reservations. Allowed: 1..4 workers, 1..64 jobs and 1 MiB..1 GiB reservation budget. Keys are 1..128 bytes; each task must reserve at least one byte. The cap includes queued tasks, active closures and handles retaining unconsumed results. Ticket Drop cancels; active work retains its reservation until it returns. Scheduler Drop cancels active/queued tasks, publishes Cancelled for queued handles, wakes workers and joins them. A noncooperative closure can delay shutdown; forced thread termination is not provided.

Reservations are caller-declared estimates of captured data, working sets and result memory, not an allocator-enforced RSS limit. Underestimating memory violates the caller contract. Reference waveform jobs reserve 72 MiB for their bounded pyramid, warp/freeze 64 MiB as working estimates; shared project PCM may be retained beyond those estimates. Bound captured projects separately if strict application memory control is required.

All scheduler methods use control/worker mutexes and may allocate. None belongs on an audio callback. Closures/results must be worker-safe; do not move native UI owners into a Send wrapper. Workers own their mutable DSP instances. Ticket consumers own publication, document history and validation. Freeze Monitor now supports `with_cancel` to share its progress with the scheduler token.

## Persistence, errors and limits

Queues/tickets/progress are transient and are never serialized. Durable outputs use the existing destination-specific temp/commit rules. Cancellation or stale publication does not delete an already committed output; retention is explicit. A task may finish useful file work just before cancellation wins. There is no cross-process scheduler, resumable job journal, automatic retry or deadline contract. Existing documents and project schema remain compatible.

Full/Invalid rejects admission without running the closure. Failed/Cancelled/Stale are surfaced in studio status; the current project remains unchanged. UI result application still performs the underlying artifact validation. Hardware, native window and external DAW acceptance are not implied by the scheduler tests. See this release's verification report for executed checks.

## Retention correction in 0.70.0

Replacement-key cleanup now follows reservation lifetime rather than a consumed ticket's surviving cancellation handle. Keeping many already-polled tickets no longer grows the scheduler's key map without bound. A regression retains 128 consumed tickets and verifies bounded keys; it failed before the correction and passes afterwards.
