# Audio-rate feedback patching

Construct `feedback_patch::FeedbackPatch` with an owned set of small
`SampleNode` processors, edges and input/output node indices. Feed the
patch as a mono AudioProcessor. Each node receives the sum of its inputs
and emits one sample. The built-in Sum node passes that value through;
custom nodes can model nonlinearities, filters, oscillators or controls
using their own prepared state.

A zero-delay edge participates in a per-sample topological order.
A positive edge delay uses an explicit ring buffer. All previous
values from delayed edges are read before evaluating the current sample,
then the newly computed sources are written back. This permits delayed
cycles at arbitrary block sizes and rejects cycles containing only
zero-delay edges. Edge delay is part of the musical signal path.
Report and match any extra product-level dry/wet alignment as needed.

```sh
cargo run -p soniclaw-dsp-suite -- feedback-patch patch-demo
```

The demo renders a short resonator and a longer saturated echo. Build
patches off-thread, prepare nodes at the project's rate, and use one
mutable audio owner. Use `set_edge_gain` on that owner for per-edge gain
updates. Smooth abrupt control changes in the supplying application or
node. Reset clears all history. Set restrained feedback levels and
audition patches. Signal values are sanitized and bounded inside the
patch; this gives predictable arithmetic bounds.

The graph accepts 1–128 nodes, up to 512 edges and a total 1,048,576
delay samples. Invalid indices, weights, or instantaneous cycles fail
construction. Invalid buffer shapes clear output. If a custom node fails
preparation, repair and prepare again. Use allocation-free tick/reset
implementations. Persist your node type IDs, settings and edge records;
recreate runtime state during load. The earlier block graph keeps its
acyclic signal-routing API.

[Executed checks](validation/v0_137_0/REPORT.md).
