# Shared convolution update queue

Use `soniclaw_dsp::convolution_update_queue` when several cabinet, room or other IR processors need to load new responses without each owning a worker thread. The existing `convolution_pair` API remains available for callers that prepare replacements synchronously on a control thread.

1. Create a `ConvolutionUpdateQueue` with 1–1024 pending commands. The queue starts one worker. Use `create` for each initial IR, conversion mode, latency, fade length and process spec.
2. Keep each `SharedConvolution` on the control side. Give the paired `ReloadableConvolution` to the render graph. The render-side processor uses the existing SPSC queues.
3. Call `try_replace(&handle, impulse)`. Its ticket resolves when the worker has prepared and enqueued a replacement. The audio callback starts the crossfade when it can accept that replacement. Poll with `try_result` or wait from a control thread.
4. Call `try_collect` after the render graph has run through the fade and entered a later block. It reports how many retired kernels it reclaimed. A replacement job also collects earlier retired kernels automatically.

```sh
cargo run -p soniclaw-dsp-suite --locked -- convolution-update-queue /tmp/soniclaw-convolution-queue
```

The example submits two IR replacements to one worker, renders the independent fades and checks zero callback allocations and kernel retirement.

## Ownership, limits and recovery

Initialize, submit, wait, collect and drop on control threads. Plan building and queued retirement run on the worker. After stopping audio, drop audio processors and control handles away from the render thread. Dropping the queue drains accepted work and joins its worker. Tickets may safely be dropped if notification is unnecessary.

The worker-facing capacity counts pending jobs, plus one running job. `SubmitError::Full` returns the original impulse. Retain it, coalesce newer requests if appropriate and retry when ready. `WrongQueue` means the handle came from a different queue. `Closed` means the worker can no longer accept submissions. A replacement result of `InvalidBuffer` can mean the processor's two-slot SPSC replacement queue is full. Keep or reload the source IR and retry after the processor renders and retires an older kernel. The convolution engine retains its existing fade and format bounds.

Persist the selected IR identity and processing settings in application state. Recreate queue, processors and tickets when restoring. Worker scheduling introduces variable update latency and one long design can temporarily delay another target. Existing plug-in and state formats are unchanged.

[Release validation](validation/v0_139_0/REPORT.md)
