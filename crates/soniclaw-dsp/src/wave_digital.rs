//! Wave-digital one-port components and an original antiparallel diode circuit.
use serde::{Deserialize, Serialize};
use soniclaw_core::{AudioBuffer, AudioProcessor, Error, ProcessContext, ProcessSpec};

#[derive(Clone, Copy, Debug)]
pub struct Resistor {
    port_ohms: f64,
}

impl Resistor {
    pub fn new(ohms: f64) -> Result<Self, Error> {
        if !(1. ..=10_000_000.).contains(&ohms) {
            return Err(Error::InvalidParameter);
        }
        Ok(Self { port_ohms: ohms })
    }

    pub fn port_ohms(self) -> f64 {
        self.port_ohms
    }

    /// A resistor matched to its wave port absorbs the incident wave.
    pub fn reflected(self) -> f64 {
        0.
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Capacitor {
    port_ohms: f64,
    previous_incident: f64,
}

impl Capacitor {
    pub fn new(farads: f64, rate: f64) -> Result<Self, Error> {
        if !(1e-12..=1e-2).contains(&farads) || !(8_000. ..=1_536_000.).contains(&rate) {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            port_ohms: 1. / (2. * farads * rate),
            previous_incident: 0.,
        })
    }

    pub fn port_ohms(self) -> f64 {
        self.port_ohms
    }
    pub fn reflected(self) -> f64 {
        self.previous_incident
    }

    /// Store the incident voltage wave for reflection on the next sample.
    pub fn accept(&mut self, incident: f64) {
        self.previous_incident = if incident.is_finite() { incident } else { 0. };
    }

    pub fn reset(&mut self) {
        self.previous_incident = 0.;
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Circuit {
    pub series_ohms: f64,
    pub shunt_farads: f64,
    pub saturation_current: f64,
    /// Thermal voltage multiplied by the diode ideality factor.
    pub thermal_volts: f64,
}
impl Default for Circuit {
    fn default() -> Self {
        Self {
            series_ohms: 2_200.,
            shunt_farads: 22e-9,
            saturation_current: 2.52e-9,
            thermal_volts: 0.025_864_926 * 1.752,
        }
    }
}
impl Circuit {
    fn validate(self) -> Result<Self, Error> {
        Resistor::new(self.series_ohms)?;
        Capacitor::new(self.shunt_farads, 48_000.)?;
        if !(1e-13..=1e-3).contains(&self.saturation_current)
            || !(0.005..=0.2).contains(&self.thermal_volts)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(self)
    }
}

/// Solve the odd Shockley pair at the junction of the adapted linear ports.
fn diode_voltage(incident: f64, diode_scale: f64, thermal: f64) -> f64 {
    let target = incident.abs();
    let mut lo = 0.;
    let mut hi = target.min(thermal * (target / diode_scale).asinh());
    let mut x = hi;
    for _ in 0..24 {
        let exponent = x / thermal;
        let error = x + diode_scale * exponent.sinh() - target;
        if error.abs() <= 1e-11 * (1. + target) {
            break;
        }
        if error > 0. {
            hi = x;
        } else {
            lo = x;
        }
        let candidate = x - error / (1. + diode_scale / thermal * exponent.cosh());
        x = if (lo..hi).contains(&candidate) {
            candidate
        } else {
            0.5 * (lo + hi)
        };
    }
    x.copysign(incident)
}

struct State {
    spec: ProcessSpec,
    capacitors: Vec<Capacitor>,
    source_fraction: f64,
    diode_scale: f64,
}

pub struct DiodeClipper {
    circuit: Circuit,
    drive: f64,
    state: Option<State>,
}
impl DiodeClipper {
    pub fn new(circuit: Circuit) -> Result<Self, Error> {
        Ok(Self {
            circuit: circuit.validate()?,
            drive: 1.,
            state: None,
        })
    }

    pub fn set_drive(&mut self, drive: f64) -> Result<(), Error> {
        if !(0. ..=32.).contains(&drive) {
            return Err(Error::InvalidParameter);
        }
        self.drive = drive;
        Ok(())
    }
}

impl AudioProcessor for DiodeClipper {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        let spec = spec.validate()?;
        let cap = Capacitor::new(self.circuit.shunt_farads, spec.sample_rate)?;
        let g_r = self.circuit.series_ohms.recip();
        let g_c = cap.port_ohms().recip();
        self.state = Some(State {
            spec,
            capacitors: vec![cap; spec.channels],
            source_fraction: g_r / (g_r + g_c),
            diode_scale: 2. * self.circuit.saturation_current / (g_r + g_c),
        });
        Ok(())
    }

    fn process(&mut self, buffer: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let Some(state) = &mut self.state else {
            buffer.clear();
            return;
        };
        if buffer.channels() != state.spec.channels || buffer.frames() > state.spec.max_block_frames
        {
            buffer.clear();
            return;
        }
        for frame in buffer.samples_mut().chunks_exact_mut(state.spec.channels) {
            for (sample, cap) in frame.iter_mut().zip(&mut state.capacitors) {
                let source = if sample.is_finite() {
                    f64::from(*sample).clamp(-8., 8.) * self.drive
                } else {
                    0.
                };
                let reflected = cap.reflected();
                let incident = reflected + state.source_fraction * (source - reflected);
                let voltage =
                    diode_voltage(incident, state.diode_scale, self.circuit.thermal_volts);
                cap.accept(2. * voltage - reflected);
                *sample = voltage as f32;
            }
        }
    }

    fn reset(&mut self) {
        if let Some(state) = &mut self.state {
            state.capacitors.iter_mut().for_each(Capacitor::reset);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junction_satisfies_kirchhoff_for_signed_inputs() {
        for voltage in [-100., -5., -1., -0.02, 0., 0.02, 1., 5., 100.] {
            let solved = diode_voltage(voltage, 1e-5, 0.041);
            let residual = solved + 1e-5 * (solved / 0.041).sinh() - voltage;
            assert!(residual.abs() < 1e-8, "{voltage} {residual}");
            assert!(solved.abs() <= f64::abs(voltage));
        }
    }

    #[test]
    fn prepared_circuit_is_zero_latency_and_channel_independent() {
        let mut clipper = DiodeClipper::new(Circuit::default()).unwrap();
        let mut stereo = [0.; 2048];
        let spec = ProcessSpec {
            sample_rate: 48_000.,
            max_block_frames: 1024,
            channels: 2,
        };
        clipper.prepare(spec).unwrap();
        for frame in stereo.as_chunks_mut::<2>().0 {
            frame[0] = 2.;
        }
        let context = ProcessContext {
            events: &[],
            transport: Default::default(),
        };
        assert_eq!(
            allocation_counter::measure(
                || clipper.process(&mut AudioBuffer::new(&mut stereo, 2).unwrap(), &context,)
            )
            .count_total,
            0
        );
        assert!(stereo.iter().skip(1).step_by(2).all(|&v| v == 0.));
        assert!((0.45..0.65).contains(&stereo[2046]));
        clipper.reset();
        assert_eq!(clipper.latency_frames(), 0);
    }
}
