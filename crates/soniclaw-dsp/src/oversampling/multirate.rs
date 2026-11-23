//! Prepared cascaded half-band FIR conversion. All allocation belongs to construction/prepare.
use serde::{Deserialize, Serialize};
use soniclaw_core::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Factor {
    Two,
    Four,
    Eight,
}
impl Factor {
    pub fn multiplier(self) -> usize {
        match self {
            Self::Two => 2,
            Self::Four => 4,
            Self::Eight => 8,
        }
    }
    fn stages(self) -> usize {
        self.multiplier().trailing_zeros() as usize
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    Balanced,
    High,
}
impl Quality {
    fn taps(self) -> usize {
        match self {
            Self::Balanced => 65,
            Self::High => 129,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema: u32,
    pub factor: Factor,
    pub quality: Quality,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            schema: 1,
            factor: Factor::Two,
            quality: Quality::Balanced,
        }
    }
}
struct Fir {
    data: Vec<f64>,
    cursor: usize,
}
impl Fir {
    fn new(n: usize) -> Self {
        Self {
            data: vec![0.; n],
            cursor: 0,
        }
    }
    fn tick(&mut self, x: f32, h: &[(usize, f64)]) -> f32 {
        self.data[self.cursor] = x as f64;
        let mut y = 0.;
        for &(i, c) in h {
            y += c * self.data[(self.cursor + self.data.len() - i) % self.data.len()];
        }
        self.cursor = (self.cursor + 1) % self.data.len();
        y as f32
    }
    fn reset(&mut self) {
        self.data.fill(0.);
        self.cursor = 0;
    }
}
struct Stage {
    up: Vec<Fir>,
    down: Vec<Fir>,
}
pub struct Oversampler {
    config: Config,
    spec: ProcessSpec,
    high: ProcessSpec,
    taps: Vec<(usize, f64)>,
    stages: Vec<Stage>,
    work: Vec<f32>,
    scratch: Vec<f32>,
}
impl Oversampler {
    pub fn new(config: Config, spec: ProcessSpec) -> Result<Self, Error> {
        Self::with_stages(config, spec, config.factor.stages())
    }

    pub(super) fn with_stages(
        config: Config,
        spec: ProcessSpec,
        stage_count: usize,
    ) -> Result<Self, Error> {
        spec.validate()?;
        let f = 1 << stage_count;
        let count = spec
            .max_block_frames
            .checked_mul(f)
            .and_then(|n| n.checked_mul(spec.channels))
            .ok_or(Error::InvalidSpec)?;
        if config.schema != 1 || count > 4 * 1024 * 1024 {
            return Err(Error::InvalidSpec);
        }
        let high = ProcessSpec {
            sample_rate: spec.sample_rate * f as f64,
            max_block_frames: spec.max_block_frames * f,
            channels: spec.channels,
        }
        .validate()?;
        let n = config.quality.taps();
        let mid = (n - 1) / 2;
        let mut coefficients = vec![0.; n];
        for (i, c) in coefficients.iter_mut().enumerate() {
            let x = i as f64 - mid as f64;
            let sinc = if i == mid {
                0.5
            } else {
                (std::f64::consts::PI * 0.5 * x).sin() / (std::f64::consts::PI * x)
            };
            let phase = std::f64::consts::TAU * i as f64 / (n - 1) as f64;
            *c = sinc * (0.42 - 0.5 * phase.cos() + 0.08 * (2. * phase).cos());
        }
        let sum = coefficients.iter().sum::<f64>();
        let taps = coefficients
            .into_iter()
            .enumerate()
            .filter(|(_, c)| c.abs() > 1e-15)
            .map(|(i, c)| (i, c / sum))
            .collect();
        let stages = (0..stage_count)
            .map(|_| Stage {
                up: (0..spec.channels).map(|_| Fir::new(n)).collect(),
                down: (0..spec.channels).map(|_| Fir::new(n)).collect(),
            })
            .collect();
        Ok(Self {
            config,
            spec,
            high,
            taps,
            stages,
            work: vec![0.; count],
            scratch: vec![0.; count],
        })
    }
    pub fn config(&self) -> Config {
        self.config
    }
    pub fn high_rate_spec(&self) -> ProcessSpec {
        self.high
    }
    pub fn latency_frames(&self) -> usize {
        let base = (self.config.quality.taps() - 1) / 2;
        (0..self.stages.len()).map(|i| base >> i).sum()
    }
    pub fn reset(&mut self) {
        for s in &mut self.stages {
            for f in s.up.iter_mut().chain(&mut s.down) {
                f.reset();
            }
        }
        self.work.fill(0.);
        self.scratch.fill(0.);
    }
    /// The closure runs once at the higher sample rate. It must obey the caller's real-time contract.
    /// Validation errors leave filter histories unchanged; invalid rendered samples reset histories.
    pub fn process_with(
        &mut self,
        b: &mut AudioBuffer<'_>,
        render: impl FnOnce(&mut AudioBuffer<'_>, ProcessSpec),
    ) -> Result<(), Error> {
        if b.channels() != self.spec.channels || b.frames() > self.spec.max_block_frames {
            return Err(Error::InvalidBuffer);
        }
        if b.samples().iter().any(|v| !v.is_finite()) {
            return Err(Error::InvalidBuffer);
        }
        if b.frames() == 0 {
            return Ok(());
        }
        let ch = b.channels();
        let mut frames = b.frames();
        self.work[..b.samples().len()].copy_from_slice(b.samples());
        for stage in &mut self.stages {
            for f in 0..frames {
                for c in 0..ch {
                    self.scratch[2 * f * ch + c] =
                        2. * stage.up[c].tick(self.work[f * ch + c], &self.taps);
                    self.scratch[(2 * f + 1) * ch + c] = 2. * stage.up[c].tick(0., &self.taps);
                }
            }
            frames *= 2;
            std::mem::swap(&mut self.work, &mut self.scratch);
        }
        render(
            &mut AudioBuffer::new(&mut self.work[..frames * ch], ch)?,
            self.high,
        );
        if self.work[..frames * ch].iter().any(|v| !v.is_finite()) {
            b.clear();
            self.reset();
            return Err(Error::InvalidBuffer);
        }
        for stage in self.stages.iter_mut().rev() {
            for f in 0..frames / 2 {
                for c in 0..ch {
                    self.scratch[f * ch + c] =
                        stage.down[c].tick(self.work[2 * f * ch + c], &self.taps);
                    let _ = stage.down[c].tick(self.work[(2 * f + 1) * ch + c], &self.taps);
                }
            }
            frames /= 2;
            std::mem::swap(&mut self.work, &mut self.scratch);
        }
        b.samples_mut().copy_from_slice(&self.work[..frames * ch]);
        Ok(())
    }
}
/// Run any compatible processor at 2x/4x/8x, retiming MIDI offsets and the sample clock.
/// Inner latency must be an integer number of base-rate frames.
pub struct Oversampled<P> {
    inner: P,
    config: Config,
    engine: Option<Oversampler>,
    events: Vec<Event>,
    latency: usize,
    last_error: Option<Error>,
}
impl<P: AudioProcessor> Oversampled<P> {
    pub fn new(inner: P, config: Config) -> Self {
        Self {
            inner,
            config,
            engine: None,
            events: Vec::new(),
            latency: 0,
            last_error: None,
        }
    }
    pub fn inner_mut(&mut self) -> &mut P {
        &mut self.inner
    }
    pub fn last_error(&self) -> Option<Error> {
        self.last_error
    }
    pub fn process_checked(
        &mut self,
        b: &mut AudioBuffer<'_>,
        ctx: &ProcessContext<'_>,
    ) -> Result<(), Error> {
        let e = self.engine.as_mut().ok_or(Error::NotPrepared)?;
        let f = self.config.factor.multiplier();
        if b.channels() != e.spec.channels || b.frames() > e.spec.max_block_frames {
            return Err(Error::InvalidBuffer);
        }
        if ctx.events.len() > 1024 {
            return Err(Error::InvalidEvents);
        }
        validate_events(ctx.events, b.frames())?;
        let sample = ctx
            .transport
            .sample_position
            .checked_mul(f as u64)
            .ok_or(Error::InvalidEvents)?;
        self.events.clear();
        self.events.extend(ctx.events.iter().map(|e| Event {
            frame: e.frame * f,
            message: e.message,
        }));
        let high = ProcessContext {
            events: &self.events,
            transport: Transport {
                sample_position: sample,
                ..ctx.transport
            },
        };
        let inner = &mut self.inner;
        e.process_with(b, |b, _| inner.process(b, &high))
    }
}
impl<P: AudioProcessor> AudioProcessor for Oversampled<P> {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        self.engine = None;
        self.latency = 0;
        let e = Oversampler::new(self.config, s)?;
        self.inner.prepare(e.high_rate_spec())?;
        let f = self.config.factor.multiplier();
        let l = self.inner.latency_frames();
        if !l.is_multiple_of(f) || l > 262144 * f {
            return Err(Error::InvalidSpec);
        }
        self.latency = e.latency_frames() + l / f;
        self.events = Vec::with_capacity(1024);
        self.engine = Some(e);
        self.last_error = None;
        Ok(())
    }
    fn reset(&mut self) {
        if let Some(e) = &mut self.engine {
            e.reset();
        }
        self.inner.reset();
        self.events.clear();
        self.last_error = None;
    }
    fn latency_frames(&self) -> usize {
        self.latency
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, c: &ProcessContext<'_>) {
        let result = self.process_checked(b, c);
        self.last_error = result.err();
        if self.last_error.is_some() {
            b.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec() -> ProcessSpec {
        ProcessSpec {
            sample_rate: 48000.,
            max_block_frames: 127,
            channels: 2,
        }
    }
    fn render(factor: Factor, quality: Quality, block: usize) -> Vec<f32> {
        let mut e = Oversampler::new(
            Config {
                schema: 1,
                factor,
                quality,
            },
            spec(),
        )
        .unwrap();
        let mut out = vec![0.; 1024 * 2];
        out[0] = 1.;
        for b in out.chunks_mut(block * 2) {
            e.process_with(&mut AudioBuffer::new(b, 2).unwrap(), |_, _| {})
                .unwrap();
        }
        out
    }
    #[test]
    fn all_factors_latency_dc_partition_and_channel_isolation() {
        for factor in [Factor::Two, Factor::Four, Factor::Eight] {
            for quality in [Quality::Balanced, Quality::High] {
                let a = render(factor, quality, 127);
                assert_eq!(a, render(factor, quality, 1));
                let e = Oversampler::new(
                    Config {
                        schema: 1,
                        factor,
                        quality,
                    },
                    spec(),
                )
                .unwrap();
                let peak = a
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1[0].abs().total_cmp(&b.1[0].abs()))
                    .unwrap()
                    .0;
                assert_eq!(peak, e.latency_frames());
                assert!((a.iter().step_by(2).sum::<f32>() - 1.).abs() < 2e-5);
                assert!(a.iter().skip(1).step_by(2).all(|x| *x == 0.));
            }
        }
    }
    #[test]
    fn callback_allocations_reset_and_invalid_input() {
        let mut e = Oversampler::new(
            Config {
                schema: 1,
                factor: Factor::Eight,
                quality: Quality::High,
            },
            spec(),
        )
        .unwrap();
        let mut data = [0.25; 254];
        let allocation = allocation_counter::measure(|| {
            e.process_with(&mut AudioBuffer::new(&mut data, 2).unwrap(), |b, s| {
                assert_eq!(s.sample_rate, 384000.);
                for x in b.samples_mut() {
                    *x = x.tanh();
                }
            })
            .unwrap()
        });
        assert_eq!(allocation.count_total, 0);
        e.reset();
        let mut zeros = [0.; 254];
        e.process_with(&mut AudioBuffer::new(&mut zeros, 2).unwrap(), |_, _| {})
            .unwrap();
        assert!(zeros.iter().all(|x| *x == 0.));
        zeros[0] = f32::NAN;
        assert!(e
            .process_with(&mut AudioBuffer::new(&mut zeros, 2).unwrap(), |_, _| {})
            .is_err());
        assert!(Oversampler::new(
            Config {
                schema: 2,
                ..Default::default()
            },
            spec()
        )
        .is_err());
        assert!(Oversampler::new(
            Config::default(),
            ProcessSpec {
                max_block_frames: usize::MAX,
                ..spec()
            }
        )
        .is_err());
    }
    struct Probe;
    impl AudioProcessor for Probe {
        fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
            assert_eq!(s.sample_rate, 192000.);
            Ok(())
        }
        fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
            assert_eq!(ctx.events[0].frame, 12);
            assert_eq!(ctx.transport.sample_position, 28);
            b.clear();
        }
        fn reset(&mut self) {}
    }
    #[test]
    fn wrapper_retimes_events_and_clock() {
        let mut p = Oversampled::new(
            Probe,
            Config {
                schema: 1,
                factor: Factor::Four,
                quality: Quality::Balanced,
            },
        );
        p.prepare(spec()).unwrap();
        let mut data = [0.; 254];
        p.process_checked(
            &mut AudioBuffer::new(&mut data, 2).unwrap(),
            &ProcessContext {
                events: &[Event {
                    frame: 3,
                    message: MidiMessage::AllNotesOff,
                }],
                transport: Transport {
                    sample_position: 7,
                    ..Default::default()
                },
            },
        )
        .unwrap();
        assert_eq!(p.latency_frames(), 48);
    }
}
