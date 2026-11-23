# Component-level circuit modelling

Use `soniclaw::dsp::wave_digital::{Resistor, Capacitor, Circuit, DiodeClipper}`.
The example circuit has a series resistor from an ideal voltage source to a
node. A capacitor and an antiparallel pair of identical diodes shunt that node
to ground. The node voltage is the output. Set the resistor in ohms, capacitor
in farads, diode saturation current in amps, and effective thermal voltage
in volts. `set_drive` applies a linear input scale.

The resistor is a matched wave port. A trapezoidally discretized capacitor
uses the port resistance 1/(2 × capacitance × sample rate) and returns its
prior incident wave. A bounded scalar solve applies the nonlinear diode load
between the two linear wave ports. Use the one-port primitives to assemble
other prepared wave-digital topologies in an application.

```sh
cargo run -p soniclaw-dsp-suite -- wave-digital output/wdf
cd output/wdf
ngspice -b diode-reference.cir
```

The demo writes a clipped sine WAV, a voltage trace CSV, and an original
SPICE deck. The accompanying validation records the comparison when the
reference simulator is available. The model shares only the mathematical
topology with the deck. It has independent capacitor state per channel and
reports zero samples of algorithmic buffering. There is no general-purpose
netlist importer. Use the existing oversampler for alias control when the
input drive or frequency warrants it.

Prepare off-thread and mutate validated drive on the audio owner. The
callback uses a bounded solver and channel state with no heap work. Rate and
channel bounds follow ProcessSpec and the capacitor's 8–1536 kHz range.
Component values and drive are checked before use. Invalid block shape
clears output, and nonfinite input becomes zero. Reset discharges the
capacitors. Persist Circuit plus drive, then reconstruct runtime state.
Existing amps and plug-in identities require no migration.

[Executed checks](validation/v0_124_0/REPORT.md).
