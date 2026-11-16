//! Small composable DSP units and processors, independent of devices and UI.
use soniclaw_core::*;
use std::{f32::consts::TAU, sync::Arc};
mod convolution;
mod effects;
mod oversampling;
mod sampler;
pub use convolution::Convolution;
pub use effects::*;
pub use oversampling::OversampledDrive;
pub use sampler::{Sample, Sampler};

pub fn midi_hz(note: u8) -> f32 {
    440.0 * 2.0_f32.powf((note as f32 - 69.0) / 12.0)
}
pub fn db_to_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Waveform {
    #[default]
    Sine,
    Saw,
    Square,
}
#[derive(Clone, Copy)]
pub struct Oscillator {
    phase: f32,
}
impl Default for Oscillator {
    fn default() -> Self {
        Self { phase: 0.0 }
    }
}
fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}
impl Oscillator {
    pub fn next_sample(&mut self, hz: f32, sample_rate: f32, wave: Waveform) -> f32 {
        if !hz.is_finite() || !sample_rate.is_finite() || sample_rate <= 0.0 {
            return 0.0;
        }
        let dt = (hz / sample_rate).clamp(0.0, 0.49);
        let t = self.phase;
        let sample = match wave {
            Waveform::Sine => (TAU * t).sin(),
            Waveform::Saw => 2.0 * t - 1.0 - if dt > 0.0 { poly_blep(t, dt) } else { 0.0 },
            Waveform::Square => {
                let raw = if t < 0.5 { 1.0 } else { -1.0 };
                if dt > 0.0 {
                    raw + poly_blep(t, dt) - poly_blep((t + 0.5) % 1.0, dt)
                } else {
                    raw
                }
            }
        };
        self.phase = (t + dt).fract();
        sample
    }
    pub fn reset(&mut self) {
        self.phase = 0.0;
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AdsrSettings {
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}
impl Default for AdsrSettings {
    fn default() -> Self {
        Self {
            attack: 0.008,
            decay: 0.14,
            sustain: 0.65,
            release: 0.25,
        }
    }
}
#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}
#[derive(Clone, Copy)]
pub struct Adsr {
    settings: AdsrSettings,
    stage: Stage,
    level: f32,
    release_step: f32,
}
impl Default for Adsr {
    fn default() -> Self {
        Self {
            settings: AdsrSettings::default(),
            stage: Stage::Idle,
            level: 0.0,
            release_step: 0.0,
        }
    }
}
impl Adsr {
    pub fn configure(&mut self, settings: AdsrSettings) -> Result<(), Error> {
        if [settings.attack, settings.decay, settings.release]
            .iter()
            .any(|x| !x.is_finite() || *x < 0.0)
            || !settings.sustain.is_finite()
            || !(0.0..=1.0).contains(&settings.sustain)
        {
            return Err(Error::InvalidParameter);
        }
        self.settings = settings;
        Ok(())
    }
    pub fn note_on(&mut self) {
        self.stage = Stage::Attack;
    }
    pub fn note_off(&mut self, sample_rate: f32) {
        self.release_step = self.level / (self.settings.release * sample_rate).max(1.0);
        self.stage = Stage::Release;
    }
    pub fn is_active(&self) -> bool {
        self.stage != Stage::Idle
    }
    pub fn reset(&mut self) {
        self.stage = Stage::Idle;
        self.level = 0.0;
    }
    pub fn next_sample(&mut self, sr: f32) -> f32 {
        match self.stage {
            Stage::Idle => return 0.0,
            Stage::Attack => {
                self.level += 1.0 / (self.settings.attack * sr).max(1.0);
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.level -= (1.0 - self.settings.sustain) / (self.settings.decay * sr).max(1.0);
                if self.level <= self.settings.sustain {
                    self.level = self.settings.sustain;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => self.level = self.settings.sustain,
            Stage::Release => {
                self.level -= self.release_step;
                if self.level <= 0.000001 {
                    self.reset();
                }
            }
        }
        self.level
    }
}

pub struct Gain {
    pub gain: Arc<Parameter>,
    smooth: SmoothedValue,
}
impl Gain {
    pub fn new(gain: Arc<Parameter>) -> Self {
        let value = gain.get();
        Self {
            gain,
            smooth: SmoothedValue::new(value, 0),
        }
    }
}
impl AudioProcessor for Gain {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        spec.validate()?;
        self.smooth = SmoothedValue::new(self.gain.get(), (spec.sample_rate * 0.01) as usize);
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        self.smooth.set_target(self.gain.get());
        if let Some(gain) = self.smooth.settled_value() {
            if gain == 0.0 {
                b.clear();
            } else if gain != 1.0 {
                for sample in b.samples_mut() {
                    *sample *= gain;
                }
            }
            return;
        }
        let channels = b.channels();
        for frame in b.samples_mut().chunks_exact_mut(channels) {
            let gain = self.smooth.next_value();
            for sample in frame {
                *sample *= gain;
            }
        }
    }
    fn reset(&mut self) {
        self.smooth.reset(self.gain.get());
    }
}

/// RBJ low-pass biquad. One state pair per channel, allocated in prepare.
pub struct LowPass {
    pub cutoff: Arc<Parameter>,
    pub q: f32,
    states: Vec<[f32; 2]>,
    sr: f32,
    cached: (f32, f32),
    coefficients: [f32; 5],
}
impl LowPass {
    pub fn new(cutoff: Arc<Parameter>) -> Self {
        Self {
            cutoff,
            q: std::f32::consts::FRAC_1_SQRT_2,
            states: Vec::new(),
            sr: 48000.0,
            cached: (f32::NAN, f32::NAN),
            coefficients: [0.0; 5],
        }
    }
}
impl AudioProcessor for LowPass {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        spec.validate()?;
        self.sr = spec.sample_rate as f32;
        if !self.sr.is_finite() || !self.q.is_finite() || self.q <= 0.0 {
            return Err(Error::InvalidParameter);
        }
        self.states = vec![[0.0; 2]; spec.channels];
        self.cached = (f32::NAN, f32::NAN);
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let cutoff = self.cutoff.get().clamp(0.01, self.sr * 0.49);
        // Public q may change while stopped. Reject invalid values without poisoning state.
        if !self.q.is_finite() || self.q <= 0.0 {
            b.clear();
            return;
        }
        if self.cached != (cutoff, self.q) {
            let w = TAU * cutoff / self.sr;
            let cos = w.cos();
            let alpha = w.sin() / (2.0 * self.q);
            let a0 = 1.0 + alpha;
            let b0 = (1.0 - cos) * 0.5 / a0;
            self.coefficients = [b0, 2.0 * b0, b0, -2.0 * cos / a0, (1.0 - alpha) / a0];
            self.cached = (cutoff, self.q);
        }
        let [b0, b1, b2, a1, a2] = self.coefficients;
        let channels = b.channels();
        for frame in b.samples_mut().chunks_exact_mut(channels) {
            for (x, state) in frame.iter_mut().zip(&mut self.states) {
                let y = b0 * *x + state[0];
                state[0] = b1 * *x - a1 * y + state[1];
                state[1] = b2 * *x - a2 * y;
                if state[0].abs() < 1e-20 {
                    state[0] = 0.0;
                }
                if state[1].abs() < 1e-20 {
                    state[1] = 0.0;
                }
                *x = y;
            }
        }
    }
    fn reset(&mut self) {
        self.states.fill([0.0; 2]);
    }
}

pub struct Delay {
    seconds: f32,
    feedback: f32,
    mix: f32,
    data: Vec<f32>,
    cursor: usize,
}
impl Delay {
    pub fn new(seconds: f32, feedback: f32, mix: f32) -> Result<Self, Error> {
        if !seconds.is_finite()
            || !(0.001..=5.0).contains(&seconds)
            || !feedback.is_finite()
            || !(0.0..=0.95).contains(&feedback)
            || !mix.is_finite()
            || !(0.0..=1.0).contains(&mix)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            seconds,
            feedback,
            mix,
            data: Vec::new(),
            cursor: 0,
        })
    }
}
impl AudioProcessor for Delay {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        spec.validate()?;
        let frames = (self.seconds as f64 * spec.sample_rate).round().max(1.0) as usize;
        let size = frames
            .checked_mul(spec.channels)
            .ok_or(Error::InvalidSpec)?;
        self.data = vec![0.0; size];
        self.cursor = 0;
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        for x in b.samples_mut() {
            let wet = self.data[self.cursor];
            self.data[self.cursor] = *x + wet * self.feedback;
            *x = *x * (1.0 - self.mix) + wet * self.mix;
            self.cursor = (self.cursor + 1) % self.data.len();
        }
    }
    fn reset(&mut self) {
        self.data.fill(0.0);
        self.cursor = 0;
    }
}

#[derive(Clone, Copy, Default)]
struct Voice {
    oscillator: Oscillator,
    envelope: Adsr,
    note: u8,
    channel: u8,
    velocity: f32,
    age: u64,
}
/// Fixed voice storage. Repeated note-ons retrigger; note-off releases matching
/// channel + note. Stealing selects the oldest voice and can click in v0.1.
pub struct PolySynth<const N: usize = 16> {
    voices: [Voice; N],
    sr: f32,
    clock: u64,
    pub waveform: Waveform,
    pub envelope: AdsrSettings,
}
impl<const N: usize> Default for PolySynth<N> {
    fn default() -> Self {
        Self {
            voices: [Voice::default(); N],
            sr: 48000.0,
            clock: 0,
            waveform: Waveform::Saw,
            envelope: AdsrSettings::default(),
        }
    }
}
impl<const N: usize> PolySynth<N> {
    fn event(&mut self, message: MidiMessage) {
        match message {
            MidiMessage::NoteOn {
                channel,
                note,
                velocity,
            } if velocity > 0 => {
                if N == 0 {
                    return;
                }
                let index = self
                    .voices
                    .iter()
                    .position(|v| v.envelope.is_active() && v.note == note && v.channel == channel)
                    .or_else(|| self.voices.iter().position(|v| !v.envelope.is_active()))
                    .unwrap_or_else(|| {
                        self.voices
                            .iter()
                            .enumerate()
                            .min_by_key(|(_, v)| v.age)
                            .map(|(i, _)| i)
                            .unwrap_or(0)
                    });
                self.clock = self.clock.wrapping_add(1);
                let v = &mut self.voices[index];
                *v = Voice {
                    note,
                    channel,
                    velocity: velocity as f32 / 127.0,
                    age: self.clock,
                    ..Voice::default()
                };
                let _ = v.envelope.configure(self.envelope);
                v.envelope.note_on();
            }
            MidiMessage::NoteOff { channel, note, .. }
            | MidiMessage::NoteOn {
                channel,
                note,
                velocity: 0,
            } => {
                for v in &mut self.voices {
                    if v.note == note && v.channel == channel {
                        v.envelope.note_off(self.sr);
                    }
                }
            }
            MidiMessage::AllNotesOff => self.reset(),
            MidiMessage::ControlChange {
                channel,
                controller: 123,
                ..
            } => {
                for v in &mut self.voices {
                    if v.channel == channel {
                        v.envelope.note_off(self.sr);
                    }
                }
            }
            _ => {}
        }
    }
}
impl<const N: usize> AudioProcessor for PolySynth<N> {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        spec.validate()?;
        self.sr = spec.sample_rate as f32;
        if !self.sr.is_finite() {
            return Err(Error::InvalidSpec);
        }
        Adsr::default().configure(self.envelope)?;
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        let channels = b.channels();
        let mut event_index = 0;
        for (frame_index, frame) in b.samples_mut().chunks_exact_mut(channels).enumerate() {
            while event_index < ctx.events.len() && ctx.events[event_index].frame == frame_index {
                self.event(ctx.events[event_index].message);
                event_index += 1;
            }
            let mut value = 0.0;
            for v in &mut self.voices {
                if v.envelope.is_active() {
                    value += v
                        .oscillator
                        .next_sample(midi_hz(v.note), self.sr, self.waveform)
                        * v.envelope.next_sample(self.sr)
                        * v.velocity
                        * 0.12;
                }
            }
            frame.fill(value);
        }
    }
    fn reset(&mut self) {
        for v in &mut self.voices {
            v.envelope.reset();
            v.oscillator.reset();
        }
        self.clock = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a4_pitch_and_sine_period() {
        assert_eq!(midi_hz(69), 440.0);
        let mut o = Oscillator::default();
        let out: Vec<_> = (0..480)
            .map(|_| o.next_sample(100.0, 48000.0, Waveform::Sine))
            .collect();
        assert!((out[120] - 1.0).abs() < 0.0001);
        assert!((out[360] + 1.0).abs() < 0.0001);
    }
    #[test]
    fn envelope_release_from_current_level_terminates() {
        let mut e = Adsr::default();
        e.note_on();
        for _ in 0..50 {
            e.next_sample(1000.0);
        }
        e.note_off(1000.0);
        for _ in 0..260 {
            e.next_sample(1000.0);
        }
        assert!(!e.is_active());
        assert_eq!(e.next_sample(1000.0), 0.0);
    }
    #[test]
    fn synth_obeys_sample_offsets_and_panic() {
        let mut s = PolySynth::<4>::default();
        s.prepare(ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 64,
            channels: 2,
        })
        .unwrap();
        let events = [
            Event {
                frame: 16,
                message: MidiMessage::NoteOn {
                    channel: 0,
                    note: 60,
                    velocity: 100,
                },
            },
            Event {
                frame: 48,
                message: MidiMessage::AllNotesOff,
            },
        ];
        let mut data = [0.0; 128];
        let mut b = AudioBuffer::new(&mut data, 2).unwrap();
        s.process(
            &mut b,
            &ProcessContext {
                events: &events,
                transport: Transport::default(),
            },
        );
        assert!(data[..32].iter().all(|x| *x == 0.0));
        assert!(data[34..96].iter().any(|x| x.abs() > 0.001));
        assert!(data[96..].iter().all(|x| *x == 0.0));
    }
    #[test]
    fn delay_impulse_is_at_expected_frame() {
        let mut d = Delay::new(0.01, 0.0, 1.0).unwrap();
        d.prepare(ProcessSpec {
            sample_rate: 1000.0,
            max_block_frames: 32,
            channels: 2,
        })
        .unwrap();
        let mut data = [0.0; 64];
        data[0] = 1.0;
        data[1] = 0.5;
        d.process(
            &mut AudioBuffer::new(&mut data, 2).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Transport::default(),
            },
        );
        assert_eq!(data[20], 1.0);
        assert_eq!(data[21], 0.5);
        assert_eq!(data[0], 0.0);
    }
}
