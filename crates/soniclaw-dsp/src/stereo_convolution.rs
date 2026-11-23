//! Stereo IR convolution, configurable algorithmic latency and deferred-retirement
//! crossfades. All kernels/plans are built before entering the callback.
use crate::{
    advanced::spec,
    resample::{resample_interleaved, ResampleQuality},
    Convolution,
};
use soniclaw_core::*;
use std::sync::Arc;
#[derive(Clone, Debug)]
pub struct StereoImpulse {
    data: Vec<f32>,
    channels: usize,
    rate: f64,
}
impl StereoImpulse {
    pub fn new(data: Vec<f32>, channels: usize, rate: f64) -> Result<Self, Error> {
        if !(1..=2).contains(&channels)
            || data.is_empty()
            || !data.len().is_multiple_of(channels)
            || data.len() / channels > 1_048_576
            || data.iter().any(|x| !x.is_finite())
            || !rate.is_finite()
            || !(8000.0..=192000.0).contains(&rate)
        {
            return Err(Error::InvalidBuffer);
        }
        Ok(Self {
            data,
            channels,
            rate,
        })
    }
    pub fn frames(&self) -> usize {
        self.data.len() / self.channels
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConvolutionMode {
    ZeroLatency,
    Uniform { partition: usize },
}
struct Kernel {
    head: Vec<f32>,
    history: Vec<f32>,
    cursor: usize,
    early: Option<Convolution>,
    late: Option<Convolution>,
}
impl Kernel {
    fn new(ir: Vec<f32>, mode: ConvolutionMode, rate: f64, s: ProcessSpec) -> Result<Self, Error> {
        let single = ProcessSpec { channels: 1, ..s };
        let make = |data: Vec<f32>, partition| -> Result<Convolution, Error> {
            let mut c = Convolution::new(data, rate, partition)?;
            c.prepare(single)?;
            Ok(c)
        };
        let (head, early, late) = match mode {
            ConvolutionMode::Uniform { partition } => (vec![], Some(make(ir, partition)?), None),
            ConvolutionMode::ZeroLatency => {
                let head = ir[..ir.len().min(32)].to_vec();
                let early = if ir.len() > 32 {
                    Some(make(ir[32..ir.len().min(1024)].to_vec(), 32)?)
                } else {
                    None
                };
                let late = if ir.len() > 1024 {
                    Some(make(ir[1024..].to_vec(), 1024)?)
                } else {
                    None
                };
                (head, early, late)
            }
        };
        Ok(Self {
            history: vec![0.0; head.len()],
            head,
            cursor: 0,
            early,
            late,
        })
    }
    fn tick(&mut self, x: f32) -> f32 {
        let mut result = 0.0;
        if !self.head.is_empty() {
            self.history[self.cursor] = x;
            for (i, h) in self.head.iter().enumerate() {
                result += h * self.history[(self.cursor + self.head.len() - i) % self.head.len()];
            }
            self.cursor = (self.cursor + 1) % self.head.len();
        }
        for convolver in [&mut self.early, &mut self.late].into_iter().flatten() {
            let mut sample = [x];
            convolver.process(
                &mut AudioBuffer::new(&mut sample, 1).expect("mono"),
                &ProcessContext {
                    events: &[],
                    transport: Transport::default(),
                },
            );
            result += sample[0];
        }
        result
    }
    fn reset(&mut self) {
        self.history.fill(0.0);
        self.cursor = 0;
        for c in [&mut self.early, &mut self.late].into_iter().flatten() {
            c.reset();
        }
    }
}
pub struct StereoConvolution {
    impulse: StereoImpulse,
    mode: ConvolutionMode,
    extra: usize,
    kernels: Vec<Kernel>,
    wet_delay: Vec<f32>,
    dry_delay: Vec<f32>,
    cursor: usize,
    dry_cursor: usize,
    channels: usize,
    latency: usize,
    mix: f32,
}
impl StereoConvolution {
    pub fn new(
        impulse: StereoImpulse,
        mode: ConvolutionMode,
        extra_delay: usize,
    ) -> Result<Self, Error> {
        if extra_delay > 8192
            || matches!(mode,ConvolutionMode::Uniform{partition}if !partition.is_power_of_two()||!(32..=4096).contains(&partition))
        {
            return Err(Error::InvalidParameter);
        }
        let latency = extra_delay
            + match mode {
                ConvolutionMode::ZeroLatency => 0,
                ConvolutionMode::Uniform { partition } => partition,
            };
        Ok(Self {
            impulse,
            mode,
            extra: extra_delay,
            kernels: vec![],
            wet_delay: vec![],
            dry_delay: vec![],
            cursor: 0,
            dry_cursor: 0,
            channels: 0,
            latency,
            mix: 1.0,
        })
    }
    pub fn set_mix(&mut self, mix: f32) -> Result<(), Error> {
        if !mix.is_finite() || !(0.0..=1.0).contains(&mix) {
            return Err(Error::InvalidParameter);
        }
        self.mix = mix;
        Ok(())
    }
}
impl AudioProcessor for StereoConvolution {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        spec(s)?;
        if s.channels > 2 {
            return Err(Error::InvalidSpec);
        }
        let output_frames =
            (self.impulse.frames() as f64 * s.sample_rate / self.impulse.rate).ceil() as usize;
        if output_frames > 1_048_576 {
            return Err(Error::InvalidBuffer);
        }
        let mut ir = if (s.sample_rate - self.impulse.rate).abs() < 0.001 {
            self.impulse.data.clone()
        } else {
            resample_interleaved(
                &self.impulse.data,
                self.impulse.channels,
                self.impulse.rate,
                s.sample_rate,
                ResampleQuality::SincBest,
            )?
        };
        if (s.sample_rate - self.impulse.rate).abs() >= 0.001 {
            for x in &mut ir {
                *x *= (self.impulse.rate / s.sample_rate) as f32;
            }
        }
        let mut kernels = Vec::with_capacity(s.channels);
        for channel in 0..s.channels {
            let data = ir
                .chunks_exact(self.impulse.channels)
                .map(|f| {
                    if s.channels == 1 && self.impulse.channels == 2 {
                        (f[0] + f[1]) * 0.5
                    } else {
                        f[channel.min(self.impulse.channels - 1)]
                    }
                })
                .collect();
            kernels.push(Kernel::new(data, self.mode, s.sample_rate, s)?);
        }
        self.kernels = kernels;
        self.channels = s.channels;
        self.wet_delay = vec![0.0; self.extra * s.channels];
        self.dry_delay = vec![0.0; self.latency * s.channels];
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        if b.channels() != self.channels || self.kernels.len() != self.channels {
            b.clear();
            return;
        }
        for frame in b.samples_mut().chunks_exact_mut(self.channels) {
            for (c, x) in frame.iter_mut().enumerate() {
                let input = *x;
                let mut wet = self.kernels[c].tick(input);
                if self.extra > 0 {
                    let index = self.cursor * self.channels + c;
                    std::mem::swap(&mut wet, &mut self.wet_delay[index]);
                }
                let dry = if self.latency > 0 {
                    let index = self.dry_cursor * self.channels + c;
                    let value = self.dry_delay[index];
                    self.dry_delay[index] = input;
                    value
                } else {
                    input
                };
                *x = dry * (1.0 - self.mix) + wet * self.mix;
            }
            if self.extra > 0 {
                self.cursor = (self.cursor + 1) % self.extra;
            }
            if self.latency > 0 {
                self.dry_cursor = (self.dry_cursor + 1) % self.latency;
            }
        }
    }
    fn reset(&mut self) {
        for k in &mut self.kernels {
            k.reset();
        }
        self.wet_delay.fill(0.0);
        self.dry_delay.fill(0.0);
        self.cursor = 0;
        self.dry_cursor = 0;
    }
    fn latency_frames(&self) -> usize {
        self.latency
    }
}
/// Named wet/dry control for project/host integration. The low-level engine also
/// exposes stopped-time algorithm configuration and a direct set_mix method.
pub struct ConvolutionEffect {
    engine: StereoConvolution,
    parameters: Vec<Arc<Parameter>>,
    smooth: SmoothedValue,
}
impl ConvolutionEffect {
    pub fn new(engine: StereoConvolution, prefix: &str) -> Result<Self, Error> {
        Ok(Self {
            engine,
            parameters: vec![Arc::new(Parameter::new(ParameterInfo {
                id: &format!("{prefix}.mix"),
                name: "Mix",
                min: 0.0,
                max: 1.0,
                default: 0.5,
                unit: "",
            })?)],
            smooth: SmoothedValue::new(0.5, 480),
        })
    }
    pub fn parameters(&self) -> &[Arc<Parameter>] {
        &self.parameters
    }
    pub fn reuse_parameters(&mut self, reuse: &[Arc<Parameter>]) -> Result<(), Error> {
        let id = self.parameters[0].info().id;
        if let Some(p) = reuse.iter().find(|p| p.info().id == id) {
            if p.info().min != 0.0 || p.info().max != 1.0 {
                return Err(Error::InvalidParameter);
            }
            self.parameters[0] = p.clone();
        } else if !reuse.is_empty() {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
}
impl AudioProcessor for ConvolutionEffect {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        self.engine.prepare(s)?;
        self.smooth = SmoothedValue::new(
            self.parameters[0].modulated(),
            (s.sample_rate * 0.01).round() as usize,
        );
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        let ch = b.channels();
        self.smooth.set_target(self.parameters[0].modulated());
        for frame in b.samples_mut().chunks_exact_mut(ch) {
            let _ = self.engine.set_mix(self.smooth.next_value());
            self.engine
                .process(&mut AudioBuffer::new(frame, ch).expect("frame"), ctx);
        }
    }
    fn reset(&mut self) {
        self.engine.reset();
        self.smooth.reset(self.parameters[0].modulated());
    }
    fn latency_frames(&self) -> usize {
        self.engine.latency_frames()
    }
}
/// Control-side owner. Both creating replacements and dropping retired kernels
/// belong here. A full replacement queue returns an error without affecting audio.
pub struct ConvolutionController {
    requests: rtrb::Producer<Box<StereoConvolution>>,
    retired: rtrb::Consumer<Box<StereoConvolution>>,
    spec: ProcessSpec,
    mode: ConvolutionMode,
    extra: usize,
}
impl ConvolutionController {
    pub fn replace(&mut self, impulse: StereoImpulse) -> Result<(), Error> {
        self.collect_retired();
        let mut engine = Box::new(StereoConvolution::new(impulse, self.mode, self.extra)?);
        engine.prepare(self.spec)?;
        self.requests.push(engine).map_err(|_| Error::InvalidBuffer)
    }
    pub fn collect_retired(&mut self) -> usize {
        let mut count = 0;
        while let Ok(engine) = self.retired.pop() {
            drop(engine);
            count += 1;
        }
        count
    }
}
pub struct ReloadableConvolution {
    current: Box<StereoConvolution>,
    previous: Option<Box<StereoConvolution>>,
    requests: rtrb::Consumer<Box<StereoConvolution>>,
    retired: rtrb::Producer<Box<StereoConvolution>>,
    spec: ProcessSpec,
    fade_frames: usize,
    position: usize,
}
pub fn convolution_pair(
    impulse: StereoImpulse,
    mode: ConvolutionMode,
    extra: usize,
    fade_frames: usize,
    s: ProcessSpec,
) -> Result<(ConvolutionController, ReloadableConvolution), Error> {
    if !(1..=65536).contains(&fade_frames) {
        return Err(Error::InvalidParameter);
    }
    let mut current = Box::new(StereoConvolution::new(impulse, mode, extra)?);
    current.prepare(s)?;
    let (sender, requests) = rtrb::RingBuffer::new(2);
    let (retired, receiver) = rtrb::RingBuffer::new(2);
    Ok((
        ConvolutionController {
            requests: sender,
            retired: receiver,
            spec: s,
            mode,
            extra,
        },
        ReloadableConvolution {
            current,
            previous: None,
            requests,
            retired,
            spec: s,
            fade_frames,
            position: fade_frames,
        },
    ))
}
impl AudioProcessor for ReloadableConvolution {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        if s.channels != self.spec.channels || (s.sample_rate - self.spec.sample_rate).abs() > 0.001
        {
            return Err(Error::InvalidSpec);
        }
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        if b.channels() != self.spec.channels {
            b.clear();
            return;
        }
        if self.position >= self.fade_frames && self.previous.is_some() && self.retired.slots() > 0
        {
            let old = self.previous.take().expect("retiring engine");
            if let Err(rtrb::PushError::Full(old)) = self.retired.push(old) {
                self.previous = Some(old);
            }
        }
        if self.previous.is_none() {
            if let Ok(next) = self.requests.pop() {
                self.previous = Some(std::mem::replace(&mut self.current, next));
                self.position = 0;
            }
        }
        let ch = b.channels();
        for frame in b.samples_mut().chunks_exact_mut(ch) {
            let mut old = [0.0; 2];
            old[..ch].copy_from_slice(frame);
            self.current
                .process(&mut AudioBuffer::new(frame, ch).expect("frame"), ctx);
            if self.position < self.fade_frames {
                if let Some(previous) = &mut self.previous {
                    previous.process(
                        &mut AudioBuffer::new(&mut old[..ch], ch).expect("frame"),
                        ctx,
                    );
                    let t = (self.position + 1) as f32 / self.fade_frames as f32;
                    for (x, old) in frame.iter_mut().zip(old) {
                        *x = old * (1.0 - t) + *x * t;
                    }
                }
                self.position += 1;
            }
        }
    }
    fn reset(&mut self) {
        self.current.reset();
        if let Some(p) = &mut self.previous {
            p.reset();
        }
        self.position = self.fade_frames;
    }
    fn latency_frames(&self) -> usize {
        self.current.latency_frames()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stereo_two_tier_matches_direct_and_uniform_latency() {
        let mut ir = vec![0.0; 2400 * 2];
        ir[0] = 1.0;
        ir[1] = 0.5;
        ir[33 * 2] = 0.3;
        ir[44 * 2 + 1] = -0.2;
        ir[1050 * 2] = 0.1;
        ir[1333 * 2 + 1] = 0.15;
        let input: Vec<_> = (0..400)
            .flat_map(|i| [(i as f32 * 0.1).sin() * 0.1, (i as f32 * 0.13).cos() * 0.2])
            .collect();
        for (mode, latency) in [
            (ConvolutionMode::ZeroLatency, 7),
            (ConvolutionMode::Uniform { partition: 64 }, 71),
        ] {
            let mut c = StereoConvolution::new(
                StereoImpulse::new(ir.clone(), 2, 48000.0).unwrap(),
                mode,
                7,
            )
            .unwrap();
            c.prepare(ProcessSpec {
                sample_rate: 48000.0,
                max_block_frames: 127,
                channels: 2,
            })
            .unwrap();
            assert_eq!(c.latency_frames(), latency);
            let mut d = input.clone();
            d.resize(3200 * 2, 0.0);
            for block in d.chunks_mut(254) {
                c.process(
                    &mut AudioBuffer::new(block, 2).unwrap(),
                    &ProcessContext {
                        events: &[],
                        transport: Transport::default(),
                    },
                );
            }
            for (i, frame) in d.as_chunks::<2>().0.iter().enumerate() {
                for ch in 0..2 {
                    let expected = if i < latency {
                        0.0
                    } else {
                        let n = i - latency;
                        (0..2400)
                            .filter_map(|j| {
                                n.checked_sub(j)
                                    .and_then(|k| input.get(k * 2 + ch))
                                    .map(|x| *x * ir[j * 2 + ch])
                            })
                            .sum::<f32>()
                    };
                    assert!(
                        (frame[ch] - expected).abs() < 2e-5,
                        "{i} ch{ch}: {} vs {expected}",
                        frame[ch]
                    );
                }
            }
        }
    }
    #[test]
    fn swap_crossfades_and_retires_on_control() {
        let s = ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 128,
            channels: 1,
        };
        let (mut control, mut c) = convolution_pair(
            StereoImpulse::new(vec![1.0], 1, 48000.0).unwrap(),
            ConvolutionMode::ZeroLatency,
            0,
            64,
            s,
        )
        .unwrap();
        control
            .replace(StereoImpulse::new(vec![0.5], 1, 48000.0).unwrap())
            .unwrap();
        let mut d = [1.0; 128];
        let ctx = ProcessContext {
            events: &[],
            transport: Transport::default(),
        };
        c.process(&mut AudioBuffer::new(&mut d, 1).unwrap(), &ctx);
        assert!((d[0] - 0.9921875).abs() < 1e-6);
        assert_eq!(d[63], 0.5);
        assert_eq!(control.collect_retired(), 0);
        c.process(&mut AudioBuffer::new(&mut d, 1).unwrap(), &ctx);
        assert_eq!(control.collect_retired(), 1);
    }
}
