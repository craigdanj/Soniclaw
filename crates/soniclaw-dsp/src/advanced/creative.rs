use super::*;
use std::f32::consts::{PI, TAU};
pub struct Phaser {
    states: Vec<[f32; 6]>,
    feedback_state: Vec<f32>,
    sr: f32,
    phase: f32,
    rate: f32,
    depth: f32,
    feedback: f32,
    mix: f32,
}
impl Default for Phaser {
    fn default() -> Self {
        Self {
            states: vec![],
            feedback_state: vec![],
            sr: 48000.0,
            phase: 0.0,
            rate: 0.3,
            depth: 0.7,
            feedback: 0.2,
            mix: 0.5,
        }
    }
}
impl Phaser {
    pub fn set(&mut self, rate: f32, depth: f32, feedback: f32, mix: f32) -> Result<(), Error> {
        if !valid(
            &[rate, depth, feedback, mix],
            &[(0.05, 5.0), (0.0, 1.0), (-0.8, 0.8), (0.0, 1.0)],
        ) {
            return Err(Error::InvalidParameter);
        }
        self.rate = rate;
        self.depth = depth;
        self.feedback = feedback;
        self.mix = mix;
        Ok(())
    }
}
impl LiveDsp for Phaser {
    fn update(&mut self, v: &[f32]) {
        let _ = self.set(v[0], v[1], v[2], v[3]);
    }
}
impl AudioProcessor for Phaser {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        spec(s)?;
        self.sr = s.sample_rate as f32;
        self.states = vec![[0.0; 6]; s.channels];
        self.feedback_state = vec![0.0; s.channels];
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let ch = b.channels();
        if ch != self.states.len() {
            b.clear();
            return;
        }
        for f in b.samples_mut().chunks_exact_mut(ch) {
            for (c, x) in f.iter_mut().enumerate() {
                let mut y = *x + self.feedback_state[c] * self.feedback;
                for (stage, z) in self.states[c].iter_mut().enumerate() {
                    let lfo = (self.phase + c as f32 * 0.25).sin();
                    let hz = (180.0
                        * 2.0_f32.powf(stage as f32 * 0.45 + self.depth * (lfo + 1.0) * 2.5))
                    .min(self.sr * 0.4);
                    let g = (PI * hz / self.sr).tan();
                    let a = (1.0 - g) / (1.0 + g);
                    let next = -a * y + *z;
                    *z = clean(y + a * next);
                    y = next;
                }
                self.feedback_state[c] = clean(y);
                *x = *x * (1.0 - self.mix) + y * self.mix;
            }
            self.phase = (self.phase + TAU * self.rate / self.sr).rem_euclid(TAU);
        }
    }
    fn reset(&mut self) {
        self.states.fill([0.0; 6]);
        self.feedback_state.fill(0.0);
        self.phase = 0.0;
    }
}
fn valid(values: &[f32], ranges: &[(f32, f32)]) -> bool {
    values
        .iter()
        .zip(ranges)
        .all(|(v, (a, b))| v.is_finite() && (*a..=*b).contains(v))
}
pub struct Flanger {
    ring: Vec<f32>,
    frames: usize,
    cursor: usize,
    channels: usize,
    sr: f32,
    phase: f32,
    rate: f32,
    depth_ms: f32,
    feedback: f32,
    mix: f32,
}
impl Default for Flanger {
    fn default() -> Self {
        Self {
            ring: vec![],
            frames: 0,
            cursor: 0,
            channels: 0,
            sr: 48000.0,
            phase: 0.0,
            rate: 0.25,
            depth_ms: 3.0,
            feedback: 0.3,
            mix: 0.5,
        }
    }
}
impl Flanger {
    pub fn set(&mut self, rate: f32, depth_ms: f32, feedback: f32, mix: f32) -> Result<(), Error> {
        if !valid(
            &[rate, depth_ms, feedback, mix],
            &[(0.05, 5.0), (0.0, 5.0), (-0.9, 0.9), (0.0, 1.0)],
        ) {
            return Err(Error::InvalidParameter);
        }
        self.rate = rate;
        self.depth_ms = depth_ms;
        self.feedback = feedback;
        self.mix = mix;
        Ok(())
    }
}
impl LiveDsp for Flanger {
    fn update(&mut self, v: &[f32]) {
        let _ = self.set(v[0], v[1], v[2], v[3]);
    }
}
impl AudioProcessor for Flanger {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        spec(s)?;
        self.sr = s.sample_rate as f32;
        self.channels = s.channels;
        self.frames = (self.sr * 0.02).ceil() as usize + 2;
        self.ring = vec![0.0; self.frames * s.channels];
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        if b.channels() != self.channels || self.frames == 0 {
            b.clear();
            return;
        }
        for f in b.samples_mut().chunks_exact_mut(self.channels) {
            for (c, x) in f.iter_mut().enumerate() {
                let distance = (0.25
                    + self.depth_ms * (0.5 + 0.5 * (self.phase + c as f32 * 0.4).sin()))
                    * self.sr
                    * 0.001;
                let pos = (self.cursor as f32 - distance).rem_euclid(self.frames as f32);
                let a = pos as usize % self.frames;
                let frac = pos.fract();
                let wet = self.ring[a * self.channels + c] * (1.0 - frac)
                    + self.ring[((a + 1) % self.frames) * self.channels + c] * frac;
                self.ring[self.cursor * self.channels + c] = clean(*x + wet * self.feedback);
                *x = *x * (1.0 - self.mix) + wet * self.mix;
            }
            self.cursor = (self.cursor + 1) % self.frames;
            self.phase = (self.phase + TAU * self.rate / self.sr).rem_euclid(TAU);
        }
    }
    fn reset(&mut self) {
        self.ring.fill(0.0);
        self.cursor = 0;
        self.phase = 0.0;
    }
}
/// Tremolo or independent stereo autopan. Beat sync is optional; rate controls
/// remain Hz when sync is None. Constant-power pan can increase channel peaks.
pub struct AmplitudeModulator {
    pan: bool,
    channels: usize,
    sr: f32,
    phase: f32,
    rate: f32,
    depth: f32,
    beats: Option<f32>,
}
impl AmplitudeModulator {
    pub fn tremolo() -> Self {
        Self {
            pan: false,
            channels: 0,
            sr: 48000.0,
            phase: 0.0,
            rate: 4.0,
            depth: 0.5,
            beats: None,
        }
    }
    pub fn autopan() -> Self {
        Self {
            pan: true,
            ..Self::tremolo()
        }
    }
    pub fn set(&mut self, rate: f32, depth: f32) -> Result<(), Error> {
        if !valid(&[rate, depth], &[(0.05, 20.0), (0.0, 1.0)]) {
            return Err(Error::InvalidParameter);
        }
        self.rate = rate;
        self.depth = depth;
        Ok(())
    }
    pub fn set_tempo_sync(&mut self, beats: Option<f32>) -> Result<(), Error> {
        if beats.is_some_and(|b| !b.is_finite() || !(0.0625..=16.0).contains(&b)) {
            return Err(Error::InvalidParameter);
        }
        self.beats = beats;
        Ok(())
    }
}
impl LiveDsp for AmplitudeModulator {
    fn update(&mut self, v: &[f32]) {
        let _ = self.set(v[0], v[1]);
    }
}
impl AudioProcessor for AmplitudeModulator {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        spec(s)?;
        self.channels = s.channels;
        self.sr = s.sample_rate as f32;
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        if b.channels() != self.channels {
            b.clear();
            return;
        }
        let rate = self.beats.map_or(self.rate, |beat| {
            (ctx.transport.tempo as f32 / 60.0 / beat).clamp(0.01, 40.0)
        });
        for f in b.samples_mut().chunks_exact_mut(self.channels) {
            let lfo = self.phase.sin();
            if self.pan && self.channels >= 2 {
                let pan = 0.5 + 0.5 * self.depth * lfo;
                f[0] *= (pan * PI * 0.5).cos() * 2.0_f32.sqrt();
                f[1] *= (pan * PI * 0.5).sin() * 2.0_f32.sqrt();
            } else {
                let gain = 1.0 - self.depth * 0.5 + self.depth * 0.5 * lfo;
                for x in f {
                    *x *= gain;
                }
            }
            self.phase = (self.phase + TAU * rate / self.sr).rem_euclid(TAU);
        }
    }
    fn reset(&mut self) {
        self.phase = 0.0;
    }
}
#[derive(Clone, Copy, Debug)]
pub struct DelayTap {
    pub beats: f32,
    pub gain: f32,
    pub pan: f32,
}
pub struct MultiTapDelay {
    ring: Vec<f32>,
    frames: usize,
    cursor: usize,
    channels: usize,
    sr: f32,
    taps: Vec<DelayTap>,
    beats: f32,
    feedback: f32,
    mix: f32,
    ping_pong: bool,
}
impl Default for MultiTapDelay {
    fn default() -> Self {
        Self {
            ring: vec![],
            frames: 0,
            cursor: 0,
            channels: 0,
            sr: 48000.0,
            taps: vec![
                DelayTap {
                    beats: 1.0,
                    gain: 0.5,
                    pan: 0.0,
                },
                DelayTap {
                    beats: 2.0,
                    gain: 0.25,
                    pan: 1.0,
                },
                DelayTap {
                    beats: 3.0,
                    gain: 0.125,
                    pan: 0.0,
                },
            ],
            beats: 0.5,
            feedback: 0.3,
            mix: 0.3,
            ping_pong: true,
        }
    }
}
impl MultiTapDelay {
    pub fn set_taps(&mut self, taps: Vec<DelayTap>, ping_pong: bool) -> Result<(), Error> {
        if taps.is_empty()
            || taps.len() > 8
            || taps.iter().any(|t| {
                !valid(
                    &[t.beats, t.gain, t.pan],
                    &[(0.0625, 8.0), (-1.0, 1.0), (0.0, 1.0)],
                )
            })
        {
            return Err(Error::InvalidParameter);
        }
        self.taps = taps;
        self.ping_pong = ping_pong;
        Ok(())
    }
    pub fn set(&mut self, beats: f32, feedback: f32, mix: f32) -> Result<(), Error> {
        if !valid(
            &[beats, feedback, mix],
            &[(0.0625, 2.0), (0.0, 0.9), (0.0, 1.0)],
        ) {
            return Err(Error::InvalidParameter);
        }
        self.beats = beats;
        self.feedback = feedback;
        self.mix = mix;
        Ok(())
    }
    fn read(&self, distance: f32, c: usize) -> f32 {
        let pos = (self.cursor as f32 - distance.clamp(1.0, self.frames as f32 - 2.0))
            .rem_euclid(self.frames as f32);
        let a = pos as usize % self.frames;
        let frac = pos.fract();
        self.ring[a * self.channels + c] * (1.0 - frac)
            + self.ring[((a + 1) % self.frames) * self.channels + c] * frac
    }
}
impl LiveDsp for MultiTapDelay {
    fn update(&mut self, v: &[f32]) {
        let _ = self.set(v[0], v[1], v[2]);
    }
}
impl AudioProcessor for MultiTapDelay {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        spec(s)?;
        self.sr = s.sample_rate as f32;
        self.channels = s.channels;
        self.frames = (self.sr * 4.0).ceil() as usize + 2;
        self.ring = vec![0.0; self.frames * self.channels];
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        if b.channels() != self.channels || self.frames == 0 {
            b.clear();
            return;
        }
        let tempo = if ctx.transport.tempo.is_finite() {
            ctx.transport.tempo.clamp(20.0, 400.0) as f32
        } else {
            120.0
        };
        let beat = self.sr * 60.0 / tempo * self.beats;
        for frame in b.samples_mut().chunks_exact_mut(self.channels) {
            let mut wet = [0.0; 64];
            let mut back = [0.0; 64];
            for c in 0..self.channels {
                back[c] = self.read(beat * self.taps[0].beats, c);
                for tap in &self.taps {
                    let pan = if self.channels >= 2 {
                        if c == 0 {
                            (1.0 - tap.pan) * 2.0
                        } else if c == 1 {
                            tap.pan * 2.0
                        } else {
                            1.0
                        }
                    } else {
                        1.0
                    };
                    wet[c] += self.read(beat * tap.beats, c) * tap.gain * pan;
                }
            }
            for (c, x) in frame.iter_mut().enumerate() {
                let source = if self.ping_pong && self.channels >= 2 && c < 2 {
                    back[1 - c]
                } else {
                    back[c]
                };
                self.ring[self.cursor * self.channels + c] = clean(*x + source * self.feedback);
                *x = *x * (1.0 - self.mix) + wet[c] * self.mix;
            }
            self.cursor = (self.cursor + 1) % self.frames;
        }
    }
    fn reset(&mut self) {
        self.ring.fill(0.0);
        self.cursor = 0;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tempo_delay_impulse_and_tremolo_bounds() {
        let sp = ProcessSpec {
            sample_rate: 8000.0,
            max_block_frames: 16000,
            channels: 1,
        };
        let mut d = MultiTapDelay::default();
        d.set_taps(
            vec![DelayTap {
                beats: 1.0,
                gain: 1.0,
                pan: 0.5,
            }],
            false,
        )
        .unwrap();
        d.set(0.5, 0.0, 1.0).unwrap();
        d.prepare(sp).unwrap();
        let mut out = vec![0.0; 8000];
        out[0] = 1.0;
        d.process(&mut AudioBuffer::new(&mut out, 1).unwrap(), &context());
        assert_eq!(out[2000], 1.0);
        assert_eq!(out.iter().filter(|x| **x != 0.0).count(), 1);
        let mut trem = AmplitudeModulator::tremolo();
        trem.set(5.0, 1.0).unwrap();
        trem.prepare(sp).unwrap();
        let mut out = vec![1.0; 8000];
        trem.process(&mut AudioBuffer::new(&mut out, 1).unwrap(), &context());
        assert!(out.iter().all(|x| *x >= 0.0 && *x <= 1.0));
        assert!(out.iter().any(|x| *x < 0.001));
    }
}
