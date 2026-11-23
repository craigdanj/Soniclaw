//! FFT-built harmonic mipmaps, morphing, phase-difference PWM, sync and unison.
//! Mipmaps band-limit static waves; fast morph/PWM and hard sync can still alias.
use super::spectral::{Complex32, FftProcessor};
use super::*;
use std::sync::Arc;
pub struct WavetableBank {
    tables: Vec<Vec<Vec<f32>>>,
    harmonics: Vec<usize>,
    size: usize,
}
impl WavetableBank {
    pub fn new(frames: Vec<Vec<f32>>) -> Result<Self, Error> {
        let size = frames.first().map_or(0, Vec::len);
        if frames.is_empty()
            || frames.len() > 16
            || !size.is_power_of_two()
            || !(64..=4096).contains(&size)
            || frames
                .iter()
                .any(|f| f.len() != size || f.iter().any(|x| !x.is_finite()))
        {
            return Err(Error::InvalidBuffer);
        }
        let mut harmonics = vec![];
        let mut h = size / 2 - 1;
        while h >= 1 {
            harmonics.push(h);
            h /= 2;
        }
        let mut fft = FftProcessor::new(size)?;
        let mut tables = vec![];
        for frame in frames {
            let mean = frame.iter().sum::<f32>() / size as f32;
            let mut spectrum: Vec<_> = frame
                .iter()
                .map(|x| Complex32::new(*x - mean, 0.0))
                .collect();
            fft.forward(&mut spectrum)?;
            let mut levels = vec![];
            for &h in &harmonics {
                let mut bins = spectrum.clone();
                for (i, bin) in bins.iter_mut().enumerate() {
                    if i.min(size - i) > h {
                        *bin = Complex32::default();
                    }
                }
                fft.inverse(&mut bins)?;
                levels.push(bins.into_iter().map(|x| x.re).collect());
            }
            tables.push(levels);
        }
        Ok(Self {
            tables,
            harmonics,
            size,
        })
    }
    pub fn basic(size: usize) -> Result<Self, Error> {
        if !size.is_power_of_two() || !(64..=4096).contains(&size) {
            return Err(Error::InvalidSpec);
        }
        let sine = (0..size)
            .map(|i| (std::f32::consts::TAU * i as f32 / size as f32).sin())
            .collect();
        let saw = (0..size)
            .map(|i| 2.0 * i as f32 / size as f32 - 1.0)
            .collect();
        let square = (0..size)
            .map(|i| if i < size / 2 { 1.0 } else { -1.0 })
            .collect();
        Self::new(vec![sine, saw, square])
    }
    pub fn frame_count(&self) -> usize {
        self.tables.len()
    }
    pub(crate) fn read(&self, phase: f64, frequency: f64, sr: f64, morph: f32) -> f32 {
        let maximum = (sr * 0.45 / frequency.abs().max(0.01)).floor().max(1.0) as usize;
        let level = self
            .harmonics
            .iter()
            .position(|h| *h <= maximum)
            .unwrap_or(self.harmonics.len() - 1);
        let t = morph.clamp(0.0, 1.0) * (self.tables.len() - 1) as f32;
        let a = t.floor() as usize;
        let b = (a + 1).min(self.tables.len() - 1);
        let position = phase.rem_euclid(1.0) * self.size as f64;
        let i = position as usize;
        let j = (i + 1) % self.size;
        let fraction = position.fract() as f32;
        let read = |f: usize| {
            let table = &self.tables[f][level];
            table[i] + (table[j] - table[i]) * fraction
        };
        let x = read(a);
        x + (read(b) - x) * t.fract()
    }
}
#[derive(Clone, Copy, Default)]
struct Phase {
    master: f64,
    slave: f64,
}
impl Phase {
    fn tick(&mut self, bank: &WavetableBank, sr: f64, hz: f64, controls: [f32; 4]) -> f32 {
        let [morph, pwm, width, sync] = controls;
        let frequency = (hz * sync as f64).clamp(0.01, sr * 0.45);
        let a = bank.read(self.slave, frequency, sr, morph);
        let pulse = (a - bank.read(self.slave + width as f64, frequency, sr, morph)) * 0.5;
        let y = a * (1.0 - pwm) + pulse * pwm;
        self.master += hz / sr;
        self.slave = (self.slave + frequency / sr).rem_euclid(1.0);
        if self.master >= 1.0 {
            self.master = self.master.rem_euclid(1.0);
            if sync > 1.0001 {
                self.slave = 0.0;
            }
        }
        y
    }
}
pub struct WavetableOscillator {
    bank: Arc<WavetableBank>,
    phase: Phase,
    sr: f64,
}
impl WavetableOscillator {
    pub fn new(bank: Arc<WavetableBank>, sr: f64) -> Result<Self, Error> {
        if !sr.is_finite() || !(8000.0..=192000.0).contains(&sr) {
            return Err(Error::InvalidSpec);
        }
        Ok(Self {
            bank,
            phase: Phase::default(),
            sr,
        })
    }
    pub fn next_sample(&mut self, hz: f64, morph: f32, pwm: f32, width: f32, sync: f32) -> f32 {
        if !hz.is_finite()
            || hz <= 0.0
            || hz > self.sr * 0.45
            || [morph, pwm, width, sync].iter().any(|x| !x.is_finite())
        {
            return 0.0;
        }
        self.phase.tick(
            &self.bank,
            self.sr,
            hz,
            [
                morph.clamp(0.0, 1.0),
                pwm.clamp(0.0, 1.0),
                width.clamp(0.05, 0.95),
                sync.clamp(1.0, 8.0),
            ],
        )
    }
    pub fn reset(&mut self) {
        self.phase = Phase::default();
    }
}
#[derive(Clone, Copy, Default)]
struct Voice {
    phase: [Phase; 8],
    envelope: crate::Adsr,
    note: u8,
    channel: u8,
    id: i32,
    velocity: f32,
    age: u64,
    held: bool,
    expression: VoiceExpression,
}
pub struct WavetableSynth<const N: usize = 16> {
    bank: Arc<WavetableBank>,
    voices: [Voice; N],
    parameters: Vec<Arc<Parameter>>,
    smooth: Vec<SmoothedValue>,
    channels: ExpressionChannels,
    sustain: [bool; 16],
    sr: f64,
    age: u64,
    unison: usize,
    pub envelope: crate::AdsrSettings,
}
impl<const N: usize> WavetableSynth<N> {
    pub fn new(bank: Arc<WavetableBank>, prefix: &str, unison: usize) -> Result<Self, Error> {
        if prefix.is_empty() || !(1..=8).contains(&unison) {
            return Err(Error::InvalidParameter);
        }
        let controls = [
            ("morph", 0.0, 1.0, 0.5),
            ("pwm", 0.0, 1.0, 0.0),
            ("width", 0.05, 0.95, 0.5),
            ("detune", 0.0, 50.0, 8.0),
            ("sync", 1.0, 8.0, 1.0),
            ("gain", 0.0, 1.0, 0.2),
        ];
        let parameters = controls
            .iter()
            .map(|&(key, min, max, default)| {
                Ok(Arc::new(Parameter::new(ParameterInfo {
                    id: &format!("{prefix}.{key}"),
                    name: key,
                    min,
                    max,
                    default,
                    unit: "",
                })?))
            })
            .collect::<Result<_, Error>>()?;
        Ok(Self {
            bank,
            voices: [Voice::default(); N],
            parameters,
            smooth: vec![],
            channels: ExpressionChannels::default(),
            sustain: [false; 16],
            sr: 48000.0,
            age: 0,
            unison,
            envelope: crate::AdsrSettings::default(),
        })
    }
    pub fn reuse_parameters(&mut self, existing: &[Arc<Parameter>]) -> Result<(), Error> {
        for p in &mut self.parameters {
            if let Some(old) = existing.iter().find(|q| q.info().id == p.info().id) {
                if old.info().min != p.info().min || old.info().max != p.info().max {
                    return Err(Error::InvalidParameter);
                }
                *p = old.clone();
            } else if !existing.is_empty() {
                return Err(Error::InvalidParameter);
            }
        }
        Ok(())
    }
    pub fn parameters(&self) -> &[Arc<Parameter>] {
        &self.parameters
    }
    pub fn set_mpe_zone(&mut self, zone: Option<MpeZone>) -> Result<(), Error> {
        self.channels.zone = zone.map(MpeZone::validate).transpose()?;
        Ok(())
    }
    fn start(&mut self, channel: u8, note: u8, velocity: f32, id: i32) {
        if N == 0
            || channel > 15
            || note > 127
            || !velocity.is_finite()
            || !(0.0..=1.0).contains(&velocity)
        {
            return;
        }
        let index = self
            .voices
            .iter()
            .position(|v| !v.envelope.is_active())
            .unwrap_or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| v.age)
                    .map(|x| x.0)
                    .unwrap_or(0)
            });
        self.age = self.age.wrapping_add(1);
        let v = &mut self.voices[index];
        *v = Voice {
            channel,
            note,
            velocity,
            id,
            age: self.age,
            held: true,
            ..Voice::default()
        };
        for (i, p) in v.phase.iter_mut().enumerate() {
            p.slave = i as f64 / self.unison as f64;
            p.master = p.slave;
        }
        let _ = v.envelope.configure(self.envelope);
        v.envelope.note_on();
    }
    fn release(&mut self, target: NoteTarget, choke: bool) {
        if !target.valid() {
            return;
        }
        for v in &mut self.voices {
            if target.matches(v.id, v.channel, v.note) {
                v.held = false;
                if choke {
                    v.envelope.reset();
                } else if !self.sustain[v.channel as usize] {
                    v.envelope.note_off(self.sr as f32);
                }
            }
        }
    }
    fn message(&mut self, m: MidiMessage) {
        if self.channels.update(m) {
            return;
        }
        match m {
            MidiMessage::NoteOn {
                channel,
                note,
                velocity,
            } if velocity > 0 => self.start(channel, note, velocity as f32 / 127.0, -1),
            MidiMessage::NoteOnId {
                channel,
                note,
                velocity,
                note_id,
            } => self.start(channel, note, velocity, note_id),
            MidiMessage::NoteOff { channel, note, .. }
            | MidiMessage::NoteOn {
                channel,
                note,
                velocity: 0,
            } => self.release(
                NoteTarget {
                    note_id: -1,
                    channel: channel as i16,
                    key: note as i16,
                },
                false,
            ),
            MidiMessage::NoteOffId { target } => self.release(target, false),
            MidiMessage::NoteChoke { target } => self.release(target, true),
            MidiMessage::NoteExpression {
                target,
                expression,
                value,
            } => {
                for v in &mut self.voices {
                    if target.valid() && target.matches(v.id, v.channel, v.note) {
                        v.expression.set(expression, value);
                    }
                }
            }
            MidiMessage::PolyPressure {
                channel,
                note,
                value,
            } => {
                for v in &mut self.voices {
                    if v.channel == channel && v.note == note && value < 128 {
                        v.expression.pressure = value as f32 / 127.0;
                    }
                }
            }
            MidiMessage::ControlChange {
                channel,
                controller: 64,
                value,
            } if channel < 16 => {
                self.sustain[channel as usize] = value >= 64;
                if value < 64 {
                    for v in &mut self.voices {
                        if v.channel == channel && !v.held {
                            v.envelope.note_off(self.sr as f32);
                        }
                    }
                }
            }
            MidiMessage::ControlChange {
                channel,
                controller: 120,
                ..
            } => self.release(
                NoteTarget {
                    note_id: -1,
                    channel: channel as i16,
                    key: -1,
                },
                true,
            ),
            MidiMessage::ControlChange {
                channel,
                controller: 123,
                ..
            } => self.release(
                NoteTarget {
                    note_id: -1,
                    channel: channel as i16,
                    key: -1,
                },
                false,
            ),
            MidiMessage::AllNotesOff => self.reset(),
            _ => {}
        }
    }
}
impl<const N: usize> AudioProcessor for WavetableSynth<N> {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        spec(s)?;
        crate::Adsr::default().configure(self.envelope)?;
        self.sr = s.sample_rate;
        self.smooth = self
            .parameters
            .iter()
            .map(|p| SmoothedValue::new(p.modulated(), (self.sr * 0.01).ceil() as usize))
            .collect();
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        if self.smooth.len() != 6 {
            b.clear();
            return;
        }
        for (s, p) in self.smooth.iter_mut().zip(&self.parameters) {
            s.set_target(p.modulated());
        }
        let ch = b.channels();
        let mut event = 0;
        for (i, frame) in b.samples_mut().chunks_exact_mut(ch).enumerate() {
            while event < ctx.events.len() && ctx.events[event].frame == i {
                self.message(ctx.events[event].message);
                event += 1;
            }
            let mut control = [0.0; 6];
            for (v, s) in control.iter_mut().zip(&mut self.smooth) {
                *v = s.next_value();
            }
            let mut output = [0.0; 2];
            for v in &mut self.voices {
                if !v.envelope.is_active() {
                    continue;
                }
                let (pitch, pressure, timbre) = self.channels.values(v.channel);
                let base = crate::midi_hz(v.note) as f64
                    * 2.0_f64.powf((pitch + v.expression.tuning) as f64 / 12.0);
                let gain = v.envelope.next_sample(self.sr as f32)
                    * v.velocity
                    * control[5]
                    * v.expression.volume
                    * (1.0 + 0.5 * pressure.max(v.expression.pressure))
                    / self.unison as f32;
                for u in 0..self.unison {
                    let spread = if self.unison == 1 {
                        0.0
                    } else {
                        u as f32 / (self.unison - 1) as f32 * 2.0 - 1.0
                    };
                    let hz = (base * 2.0_f64.powf((spread * control[3]) as f64 / 1200.0))
                        .min(self.sr * 0.45);
                    let morph =
                        (control[0] + (v.expression.timbre + timbre - 1.0) * 0.5).clamp(0.0, 1.0);
                    let y = v.phase[u].tick(
                        &self.bank,
                        self.sr,
                        hz,
                        [morph, control[1], control[2], control[4]],
                    ) * gain;
                    let pan = (v.expression.pan + spread * 0.25).clamp(0.0, 1.0);
                    output[0] += y * (1.0 - pan) * 2.0;
                    output[1] += y * pan * 2.0;
                }
            }
            frame.fill((output[0] + output[1]) * 0.5);
            if ch >= 2 {
                frame[0] = output[0];
                frame[1] = output[1];
            }
        }
    }
    fn reset(&mut self) {
        self.voices.fill(Voice::default());
        self.channels.reset();
        self.sustain.fill(false);
        for (s, p) in self.smooth.iter_mut().zip(&self.parameters) {
            s.reset(p.modulated());
        }
        self.age = 0;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mipmaps_remove_out_of_band_harmonics() {
        let bank = Arc::new(WavetableBank::basic(2048).unwrap());
        let mut osc = WavetableOscillator::new(bank, 48000.0).unwrap();
        let mut fft = FftProcessor::new(4096).unwrap();
        let frequency = 48000.0 * 700.0 / 4096.0;
        let mut bins: Vec<_> = (0..4096)
            .map(|_| Complex32::new(osc.next_sample(frequency, 0.5, 0.0, 0.5, 1.0), 0.0))
            .collect();
        fft.forward(&mut bins).unwrap();
        let fundamental = bins[700].norm();
        let alias = bins[2100].norm();
        assert!(fundamental > 100.0);
        assert!(alias < fundamental * 0.001, "{alias} / {fundamental}");
        assert_eq!(osc.next_sample(f64::NAN, 0.0, 0.0, 0.5, 1.0), 0.0);
    }
}
