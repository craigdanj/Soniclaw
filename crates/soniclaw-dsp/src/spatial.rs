//! Four-path true-stereo convolution and parallel early-IR/FDN hybrid reverberation.
use crate::{advanced::reverb::FdnReverb, Convolution};
use serde::{Deserialize, Serialize};
use soniclaw_core::*;
pub struct MatrixConvolution {
    paths: [Convolution; 4],
    scratch: [Vec<f32>; 4],
    dry: Vec<f32>,
    cursor: usize,
    partition: usize,
    mix: f32,
    spec: Option<ProcessSpec>,
}
impl MatrixConvolution {
    /// Paths are [left<-left, left<-right, right<-left, right<-right].
    pub fn new(
        impulses: [Vec<f32>; 4],
        rate: f64,
        partition: usize,
        mix: f32,
    ) -> Result<Self, Error> {
        if !mix.is_finite()
            || !(0. ..=1.).contains(&mix)
            || impulses
                .iter()
                .flatten()
                .any(|x| !x.is_finite() || x.abs() > 1000.)
            || impulses.iter().map(Vec::len).sum::<usize>() > 1_048_576
        {
            return Err(Error::InvalidParameter);
        }
        let [ll, lr, rl, rr] = impulses;
        Ok(Self {
            paths: [
                Convolution::new(ll, rate, partition)?,
                Convolution::new(lr, rate, partition)?,
                Convolution::new(rl, rate, partition)?,
                Convolution::new(rr, rate, partition)?,
            ],
            scratch: std::array::from_fn(|_| vec![]),
            dry: vec![],
            cursor: 0,
            partition,
            mix,
            spec: None,
        })
    }
    pub fn set_mix(&mut self, mix: f32) -> Result<(), Error> {
        if !mix.is_finite() || !(0. ..=1.).contains(&mix) {
            return Err(Error::InvalidParameter);
        }
        self.mix = mix;
        Ok(())
    }
}
impl AudioProcessor for MatrixConvolution {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        if s.channels != 2
            || !(8000. ..=192000.).contains(&s.sample_rate)
            || s.max_block_frames > 8192
        {
            return Err(Error::InvalidSpec);
        }
        self.spec = None;
        for p in &mut self.paths {
            p.prepare(ProcessSpec { channels: 1, ..s })?;
        }
        self.scratch = std::array::from_fn(|_| vec![0.; s.max_block_frames]);
        self.dry = vec![0.; self.partition * 2];
        self.cursor = 0;
        self.spec = Some(s);
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        let Some(s) = self.spec else {
            b.samples_mut().fill(0.);
            return;
        };
        if b.channels() != 2 || b.frames() > s.max_block_frames {
            b.samples_mut().fill(0.);
            return;
        }
        let frames = b.frames();
        for (i, f) in b.samples().as_chunks::<2>().0.iter().enumerate() {
            let clean = |x: f32| {
                if x.is_finite() {
                    x.clamp(-16., 16.)
                } else {
                    0.
                }
            };
            self.scratch[0][i] = clean(f[0]);
            self.scratch[1][i] = clean(f[1]);
            self.scratch[2][i] = clean(f[0]);
            self.scratch[3][i] = clean(f[1]);
        }
        for (p, data) in self.paths.iter_mut().zip(&mut self.scratch) {
            p.process(
                &mut AudioBuffer::new(&mut data[..frames], 1).expect("prepared"),
                ctx,
            );
        }
        for (i, f) in b
            .samples_mut()
            .as_chunks_mut::<2>()
            .0
            .iter_mut()
            .enumerate()
        {
            for (ch, x) in f.iter_mut().enumerate() {
                let dry = self.dry[self.cursor + ch];
                self.dry[self.cursor + ch] = if x.is_finite() {
                    x.clamp(-16., 16.)
                } else {
                    0.
                };
                let wet = self.scratch[ch * 2][i] + self.scratch[ch * 2 + 1][i];
                *x = dry * (1. - self.mix) + wet * self.mix;
            }
            self.cursor = (self.cursor + 2) % self.dry.len();
        }
    }
    fn reset(&mut self) {
        for p in &mut self.paths {
            p.reset();
        }
        for b in &mut self.scratch {
            b.fill(0.);
        }
        self.dry.fill(0.);
        self.cursor = 0;
    }
    fn latency_frames(&self) -> usize {
        self.partition
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HybridConfig {
    pub schema: u32,
    pub mix: f32,
    pub early_balance: f32,
    pub decay_seconds: f32,
    pub damping_hz: f32,
    pub predelay_ms: f32,
    pub diffusion: f32,
    pub modulation: f32,
}
impl Default for HybridConfig {
    fn default() -> Self {
        Self {
            schema: 1,
            mix: 0.3,
            early_balance: 0.4,
            decay_seconds: 2.,
            damping_hz: 6000.,
            predelay_ms: 15.,
            diffusion: 0.7,
            modulation: 0.3,
        }
    }
}
impl HybridConfig {
    fn validate(self) -> Result<(), Error> {
        if self.schema != 1
            || [
                (self.mix, 0., 1.),
                (self.early_balance, 0., 1.),
                (self.decay_seconds, 0.1, 15.),
                (self.damping_hz, 500., 16000.),
                (self.predelay_ms, 0., 200.),
                (self.diffusion, 0., 1.),
                (self.modulation, 0., 1.),
            ]
            .iter()
            .any(|(v, a, b)| !v.is_finite() || v < a || v > b)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
}
pub struct HybridReverb {
    config: HybridConfig,
    early: MatrixConvolution,
    late: FdnReverb,
    scratch: Vec<f32>,
    input: Vec<f32>,
    dry: Vec<f32>,
    delay: Vec<f32>,
    cursor: usize,
    spec: Option<ProcessSpec>,
}
impl HybridReverb {
    pub fn new(
        config: HybridConfig,
        impulses: [Vec<f32>; 4],
        rate: f64,
        partition: usize,
    ) -> Result<Self, Error> {
        config.validate()?;
        let mut late = FdnReverb::default();
        late.set(
            config.decay_seconds,
            1.,
            config.damping_hz,
            config.predelay_ms,
            config.diffusion,
            config.modulation,
        )?;
        Ok(Self {
            config,
            early: MatrixConvolution::new(impulses, rate, partition, 1.)?,
            late,
            scratch: vec![],
            input: vec![],
            dry: vec![],
            delay: vec![],
            cursor: 0,
            spec: None,
        })
    }
    pub fn config(&self) -> HybridConfig {
        self.config
    }
    pub fn set_config(&mut self, c: HybridConfig) -> Result<(), Error> {
        c.validate()?;
        self.late.set(
            c.decay_seconds,
            1.,
            c.damping_hz,
            c.predelay_ms,
            c.diffusion,
            c.modulation,
        )?;
        self.config = c;
        Ok(())
    }
}
impl AudioProcessor for HybridReverb {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        self.spec = None;
        self.early.prepare(s)?;
        self.late.prepare(s)?;
        self.scratch = vec![0.; s.max_block_frames * 2];
        self.input = self.scratch.clone();
        self.dry = vec![0.; self.early.latency_frames() * 2];
        self.delay = self.dry.clone();
        self.cursor = 0;
        self.spec = Some(s);
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        let Some(s) = self.spec else {
            b.samples_mut().fill(0.);
            return;
        };
        if b.channels() != 2 || b.frames() > s.max_block_frames {
            b.samples_mut().fill(0.);
            return;
        }
        let n = b.samples().len();
        for (x, y) in self.input[..n].iter_mut().zip(b.samples()) {
            *x = if y.is_finite() {
                y.clamp(-16., 16.)
            } else {
                0.
            };
        }
        b.samples_mut().copy_from_slice(&self.input[..n]);
        self.scratch[..n].copy_from_slice(&self.input[..n]);
        self.early.process(b, ctx);
        self.late.process(
            &mut AudioBuffer::new(&mut self.scratch[..n], 2).expect("prepared"),
            ctx,
        );
        let c = self.config;
        for (i, x) in b.samples_mut().iter_mut().enumerate() {
            let dry = self.dry[self.cursor];
            self.dry[self.cursor] = self.input[i];
            let late = self.delay[self.cursor];
            self.delay[self.cursor] = self.scratch[i];
            let wet = *x * c.early_balance + late * (1. - c.early_balance);
            *x = dry * (1. - c.mix) + wet * c.mix;
            self.cursor = (self.cursor + 1) % self.dry.len();
        }
    }
    fn reset(&mut self) {
        self.early.reset();
        self.late.reset();
        self.scratch.fill(0.);
        self.input.fill(0.);
        self.dry.fill(0.);
        self.delay.fill(0.);
        self.cursor = 0;
    }
    fn latency_frames(&self) -> usize {
        self.early.latency_frames()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn spec() -> ProcessSpec {
        ProcessSpec {
            sample_rate: 48000.,
            max_block_frames: 128,
            channels: 2,
        }
    }
    fn ctx() -> ProcessContext<'static> {
        ProcessContext {
            events: &[],
            transport: Transport::default(),
        }
    }
    #[test]
    fn true_stereo_routing_latency_and_partition() {
        let render = |block: usize| {
            let mut p =
                MatrixConvolution::new([vec![1.], vec![2.], vec![3.], vec![4.]], 48000., 64, 1.)
                    .unwrap();
            p.prepare(spec()).unwrap();
            let mut data = vec![0.; 512];
            data[0] = 1.;
            data[3] = 1.;
            for b in data.chunks_mut(block * 2) {
                assert_eq!(
                    allocation_counter::measure(
                        || p.process(&mut AudioBuffer::new(b, 2).unwrap(), &ctx())
                    )
                    .count_total,
                    0
                );
            }
            assert_eq!(p.latency_frames(), 64);
            assert!((data[128] - 1.).abs() < 1e-6);
            assert!((data[129] - 3.).abs() < 1e-6);
            assert!((data[130] - 2.).abs() < 1e-6);
            assert!((data[131] - 4.).abs() < 1e-6);
            data
        };
        assert_eq!(render(128), render(1));
    }
    #[test]
    fn hybrid_dry_alignment_reset_and_tail() {
        let mut p = HybridReverb::new(
            HybridConfig {
                mix: 0.,
                ..Default::default()
            },
            [vec![1.], vec![0.], vec![0.], vec![1.]],
            48000.,
            64,
        )
        .unwrap();
        p.prepare(spec()).unwrap();
        let mut data = vec![0.; 512];
        data[0] = 0.5;
        for b in data.chunks_mut(128) {
            p.process(&mut AudioBuffer::new(b, 2).unwrap(), &ctx());
        }
        assert_eq!(data.iter().filter(|x| x.abs() > 1e-5).count(), 1);
        assert!((data[128] - 0.5).abs() < 1e-6);
        p.reset();
        p.set_config(HybridConfig {
            mix: 1.,
            early_balance: 0.,
            ..Default::default()
        })
        .unwrap();
        let mut data = vec![0.; 96000];
        data[0] = 1.;
        for b in data.chunks_mut(256) {
            assert_eq!(
                allocation_counter::measure(
                    || p.process(&mut AudioBuffer::new(b, 2).unwrap(), &ctx())
                )
                .count_total,
                0
            );
        }
        assert!(data.iter().all(|x| x.is_finite()));
        assert!(data[20000..].iter().any(|x| x.abs() > 1e-7));
    }
}
