//! Final-stage quantization for signed 16- and 24-bit PCM.
use serde::{Deserialize, Serialize};
use soniclaw_core::Error;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Depth {
    Pcm16,
    Pcm24,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Shaping {
    Flat,
    FirstOrder,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub depth: Depth,
    pub shaping: Shaping,
    pub seed: u64,
}
#[derive(Clone, Copy)]
struct Channel {
    rng: u64,
    error: f64,
}
impl Channel {
    fn uniform(&mut self) -> f64 {
        self.rng = self.rng.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        ((z ^ (z >> 31)) >> 11) as f64 * (1. / 9007199254740992.)
    }
}
pub struct Quantizer {
    settings: Settings,
    channels: Vec<Channel>,
}
impl Quantizer {
    pub fn new(settings: Settings, channels: usize) -> Result<Self, Error> {
        if !(1..=64).contains(&channels) {
            return Err(Error::InvalidParameter);
        }
        let mut q = Self {
            settings,
            channels: vec![Channel { rng: 0, error: 0. }; channels],
        };
        q.reset();
        Ok(q)
    }
    pub fn reset(&mut self) {
        for (i, c) in self.channels.iter_mut().enumerate() {
            *c = Channel {
                rng: self
                    .settings
                    .seed
                    .wrapping_add((i as u64).wrapping_mul(0xd1342543de82ef95)),
                error: 0.,
            };
        }
    }
    pub fn bits(&self) -> u16 {
        match self.settings.depth {
            Depth::Pcm16 => 16,
            Depth::Pcm24 => 24,
        }
    }
    pub fn process(&mut self, input: &[f32], output: &mut [i32]) -> Result<(), Error> {
        let channels = self.channels.len();
        if input.len() != output.len()
            || !input.len().is_multiple_of(channels)
            || input.iter().any(|x| !x.is_finite())
        {
            return Err(Error::InvalidBuffer);
        }
        let scale = f64::from(1i32 << (self.bits() - 1));
        let feedback = match self.settings.shaping {
            Shaping::Flat => 0.,
            Shaping::FirstOrder => 0.85,
        };
        for (x, y) in input
            .chunks_exact(channels)
            .zip(output.chunks_exact_mut(channels))
        {
            for ((&sample, out), c) in x.iter().zip(y).zip(&mut self.channels) {
                let wanted =
                    (f64::from(sample) * scale).clamp(-scale, scale - 1.) - feedback * c.error;
                let tpdf = c.uniform() - c.uniform();
                let rounded = (wanted + tpdf).round();
                let bounded = rounded.clamp(-scale, scale - 1.);
                c.error = if rounded == bounded {
                    bounded - wanted
                } else {
                    0.
                };
                *out = bounded as i32;
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deterministic_split_and_silence_statistics() {
        let s = Settings {
            depth: Depth::Pcm16,
            shaping: Shaping::Flat,
            seed: 42,
        };
        let mut q = Quantizer::new(s, 2).unwrap();
        let mut a = vec![0; 32768];
        let mut b = a.clone();
        let x = vec![0.; a.len()];
        q.process(&x, &mut a).unwrap();
        q.reset();
        for (i, o) in x.chunks(66).zip(b.chunks_mut(66)) {
            q.process(i, o).unwrap();
        }
        assert_eq!(a, b);
        assert!(a.iter().all(|x| (-1..=1).contains(x)));
        let mean = f64::from(a.iter().sum::<i32>()) / a.len() as f64;
        let noise = a.iter().filter(|&&x| x != 0).count() as f64 / a.len() as f64;
        assert!(mean.abs() < 0.02 && (0.23..0.27).contains(&noise));
        assert!(q.process(&[f32::NAN, 0.], &mut [0; 2]).is_err());
    }
}
