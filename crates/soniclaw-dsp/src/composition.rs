//! Prepared adapters for composing the existing interleaved processor contract.
use soniclaw_core::{
    AudioBuffer, AudioProcessor, Error, ProcessContext, ProcessSpec, SmoothedValue,
};
use std::sync::Arc;

fn validate_block(spec: Option<ProcessSpec>, buffer: &AudioBuffer<'_>) -> Result<(), Error> {
    let spec = spec.ok_or(Error::NotPrepared)?;
    if buffer.channels() != spec.channels {
        return Err(Error::InvalidBuffer);
    }
    if buffer.frames() > spec.max_block_frames {
        return Err(Error::BlockTooLarge);
    }
    Ok(())
}

/// Serial, in-place processing. Every stage receives the same block-relative context.
#[derive(Default)]
pub struct ProcessorChain {
    stages: Vec<Box<dyn AudioProcessor>>,
    spec: Option<ProcessSpec>,
    latency: usize,
}

impl ProcessorChain {
    /// Adding a stage requires another prepare before processing.
    pub fn push(&mut self, processor: impl AudioProcessor + 'static) {
        self.stages.push(Box::new(processor));
        self.spec = None;
        self.latency = 0;
    }

    pub fn process_checked(
        &mut self,
        buffer: &mut AudioBuffer<'_>,
        context: &ProcessContext<'_>,
    ) -> Result<(), Error> {
        validate_block(self.spec, buffer)?;
        for stage in &mut self.stages {
            stage.process(buffer, context);
        }
        Ok(())
    }

    /// Read from one interleaved buffer and write to another with matching shape.
    pub fn process_separate(
        &mut self,
        input: &[f32],
        output: &mut AudioBuffer<'_>,
        context: &ProcessContext<'_>,
    ) -> Result<(), Error> {
        validate_block(self.spec, output)?;
        if input.len() != output.samples().len() {
            return Err(Error::InvalidBuffer);
        }
        output.samples_mut().copy_from_slice(input);
        self.process_checked(output, context)
    }
}

impl AudioProcessor for ProcessorChain {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        let spec = spec.validate()?;
        self.spec = None;
        self.latency = 0;
        let mut latency = 0usize;
        for stage in &mut self.stages {
            stage.prepare(spec)?;
            latency = latency
                .checked_add(stage.latency_frames())
                .ok_or(Error::InvalidSpec)?;
        }
        self.spec = Some(spec);
        self.latency = latency;
        Ok(())
    }

    fn process(&mut self, buffer: &mut AudioBuffer<'_>, context: &ProcessContext<'_>) {
        if self.process_checked(buffer, context).is_err() {
            buffer.clear();
        }
    }

    fn reset(&mut self) {
        for stage in &mut self.stages {
            stage.reset();
        }
    }

    fn latency_frames(&self) -> usize {
        self.latency
    }
}

/// A factory receives the same immutable coefficients for each independent mono processor.
pub struct Duplicated<P, C, F> {
    coefficients: Arc<C>,
    factory: F,
    channels: Vec<P>,
    scratch: Vec<f32>,
    spec: Option<ProcessSpec>,
    latency: usize,
}

impl<P, C, F> Duplicated<P, C, F>
where
    P: AudioProcessor,
    C: Send + Sync,
    F: FnMut(Arc<C>) -> Result<P, Error> + Send,
{
    pub fn new(coefficients: Arc<C>, factory: F) -> Self {
        Self {
            coefficients,
            factory,
            channels: Vec::new(),
            scratch: Vec::new(),
            spec: None,
            latency: 0,
        }
    }
}

impl<P, C, F> AudioProcessor for Duplicated<P, C, F>
where
    P: AudioProcessor,
    C: Send + Sync,
    F: FnMut(Arc<C>) -> Result<P, Error> + Send,
{
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        let spec = spec.validate()?;
        if spec.max_block_frames > 1_048_576 {
            return Err(Error::InvalidSpec);
        }
        let mut channels = Vec::with_capacity(spec.channels);
        let mut latency = None;
        for _ in 0..spec.channels {
            let mut mono = (self.factory)(Arc::clone(&self.coefficients))?;
            mono.prepare(ProcessSpec {
                channels: 1,
                ..spec
            })?;
            let frames = mono.latency_frames();
            if latency.is_some_and(|previous| previous != frames) {
                return Err(Error::InvalidSpec);
            }
            latency = Some(frames);
            channels.push(mono);
        }
        self.channels = channels;
        self.scratch = vec![0.; spec.max_block_frames];
        self.latency = latency.unwrap_or_default();
        self.spec = Some(spec);
        Ok(())
    }

    fn process(&mut self, buffer: &mut AudioBuffer<'_>, context: &ProcessContext<'_>) {
        if validate_block(self.spec, buffer).is_err() {
            buffer.clear();
            return;
        }
        let stride = buffer.channels();
        let scratch = &mut self.scratch[..buffer.frames()];
        for (channel, mono) in self.channels.iter_mut().enumerate() {
            let data = buffer.samples_mut();
            for (to, from) in scratch
                .iter_mut()
                .zip(data.iter().skip(channel).step_by(stride))
            {
                *to = *from;
            }
            mono.process(
                &mut AudioBuffer::new(scratch, 1).expect("mono scratch"),
                context,
            );
            for (to, from) in data
                .iter_mut()
                .skip(channel)
                .step_by(stride)
                .zip(scratch.iter())
            {
                *to = *from;
            }
        }
    }

    fn reset(&mut self) {
        for channel in &mut self.channels {
            channel.reset();
        }
    }

    fn latency_frames(&self) -> usize {
        self.latency
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub enum MixingRule {
    #[default]
    Linear,
    EqualPower,
}

fn check_mix(mix: f32) -> Result<(), Error> {
    if (0. ..=1.).contains(&mix) {
        Ok(())
    } else {
        Err(Error::InvalidParameter)
    }
}

/// Integer-frame alignment of an independently rendered wet path. One call per block.
pub struct DryWetMixer {
    channels: usize,
    history: Vec<f32>,
    cursor: usize,
    rule: MixingRule,
    mix: SmoothedValue,
    target: f32,
}

impl DryWetMixer {
    pub fn new(
        channels: usize,
        wet_latency_frames: usize,
        rule: MixingRule,
        mix: f32,
        ramp_frames: usize,
    ) -> Result<Self, Error> {
        check_mix(mix)?;
        let samples = wet_latency_frames
            .checked_mul(channels)
            .filter(|count| *count <= 4_194_304 && (1..=64).contains(&channels))
            .ok_or(Error::InvalidSpec)?;
        Ok(Self {
            channels,
            history: vec![0.; samples],
            cursor: 0,
            rule,
            mix: SmoothedValue::new(mix, ramp_frames),
            target: mix,
        })
    }

    pub fn set_mix(&mut self, mix: f32) -> Result<(), Error> {
        check_mix(mix)?;
        self.target = mix;
        self.mix.set_target(mix);
        Ok(())
    }

    pub fn process(&mut self, dry: &[f32], wet: &mut AudioBuffer<'_>) -> Result<(), Error> {
        if dry.len() != wet.samples().len() || wet.channels() != self.channels {
            return Err(Error::InvalidBuffer);
        }
        for (dry, wet) in dry
            .chunks_exact(self.channels)
            .zip(wet.samples_mut().chunks_exact_mut(self.channels))
        {
            let mix = self.mix.next_value();
            let (dry_gain, wet_gain) = match (mix, self.rule) {
                (0., _) => (1., 0.),
                (1., _) => (0., 1.),
                (_, MixingRule::Linear) => (1. - mix, mix),
                (_, MixingRule::EqualPower) => {
                    let (wet, dry) = (mix * std::f32::consts::FRAC_PI_2).sin_cos();
                    (dry, wet)
                }
            };
            for (&input, output) in dry.iter().zip(wet) {
                let aligned = if self.history.is_empty() {
                    input
                } else {
                    let old = std::mem::replace(&mut self.history[self.cursor], input);
                    self.cursor = (self.cursor + 1) % self.history.len();
                    old
                };
                *output = match mix {
                    0. => aligned,
                    1. => *output,
                    _ => dry_gain * aligned + wet_gain * *output,
                };
            }
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        self.history.fill(0.);
        self.cursor = 0;
        self.mix.reset(self.target);
    }
}

/// Wrap an existing processor with a prepared dry/wet path. Mix zero acts as latency-preserving bypass.
pub struct Parallel<P> {
    inner: P,
    spec: Option<ProcessSpec>,
    dry: Vec<f32>,
    mixer: Option<DryWetMixer>,
    mix: f32,
    rule: MixingRule,
    ramp_frames: usize,
    latency: usize,
}

impl<P> Parallel<P> {
    pub fn new(inner: P, rule: MixingRule, mix: f32, ramp_frames: usize) -> Result<Self, Error> {
        check_mix(mix)?;
        Ok(Self {
            inner,
            spec: None,
            dry: Vec::new(),
            mixer: None,
            mix,
            rule,
            ramp_frames,
            latency: 0,
        })
    }

    /// For controls that do not change the processor's prepared latency.
    pub fn inner_mut(&mut self) -> &mut P {
        &mut self.inner
    }

    pub fn set_mix(&mut self, mix: f32) -> Result<(), Error> {
        check_mix(mix)?;
        if let Some(mixer) = &mut self.mixer {
            mixer.set_mix(mix)?;
        }
        self.mix = mix;
        Ok(())
    }
}

impl<P: AudioProcessor> AudioProcessor for Parallel<P> {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        let spec = spec.validate()?;
        let count = spec.max_block_frames * spec.channels;
        if count > 4_194_304 {
            return Err(Error::InvalidSpec);
        }
        self.spec = None;
        self.latency = 0;
        self.inner.prepare(spec)?;
        let latency = self.inner.latency_frames();
        self.mixer = Some(DryWetMixer::new(
            spec.channels,
            latency,
            self.rule,
            self.mix,
            self.ramp_frames,
        )?);
        self.dry = vec![0.; count];
        self.latency = latency;
        self.spec = Some(spec);
        Ok(())
    }

    fn process(&mut self, buffer: &mut AudioBuffer<'_>, context: &ProcessContext<'_>) {
        if validate_block(self.spec, buffer).is_err() || self.inner.latency_frames() != self.latency
        {
            buffer.clear();
            return;
        }
        let dry = &mut self.dry[..buffer.samples().len()];
        dry.copy_from_slice(buffer.samples());
        self.inner.process(buffer, context);
        self.mixer
            .as_mut()
            .expect("prepared mixer")
            .process(dry, buffer)
            .expect("prepared shape");
    }

    fn reset(&mut self) {
        self.inner.reset();
        if let Some(mixer) = &mut self.mixer {
            mixer.reset();
        }
    }

    fn latency_frames(&self) -> usize {
        self.latency
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dynamics::{Config, Controls, Dynamics};

    fn spec() -> ProcessSpec {
        ProcessSpec {
            sample_rate: 48_000.,
            channels: 2,
            max_block_frames: 128,
        }
    }

    fn context() -> ProcessContext<'static> {
        ProcessContext {
            events: &[],
            transport: Default::default(),
        }
    }

    #[test]
    fn aligned_lookahead_and_separate_processing() {
        let mut chain = ProcessorChain::default();
        chain.push(
            Parallel::new(
                Dynamics::new(Config {
                    lookahead_ms: 1.,
                    controls: Controls {
                        ratio: 1.,
                        ..Controls::default()
                    },
                    ..Config::default()
                })
                .unwrap(),
                MixingRule::Linear,
                0.5,
                32,
            )
            .unwrap(),
        );
        chain.prepare(spec()).unwrap();
        assert_eq!(chain.latency_frames(), 48);
        let mut input = [0.; 256];
        input[0] = 0.75;
        let mut out = [0.; 256];
        let allocations = allocation_counter::measure(|| {
            chain
                .process_separate(
                    &input,
                    &mut AudioBuffer::new(&mut out, 2).unwrap(),
                    &context(),
                )
                .unwrap();
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(out[96], 0.75);
        assert_eq!(out.iter().filter(|x| **x != 0.).count(), 1);
        chain.reset();
        chain
            .process_checked(&mut AudioBuffer::new(&mut input, 2).unwrap(), &context())
            .unwrap();
        assert_eq!(input, out);
    }

    #[test]
    fn dry_path_crossfades_without_changing_delay() {
        let mut mix = DryWetMixer::new(1, 1, MixingRule::Linear, 1., 2).unwrap();
        mix.set_mix(0.).unwrap();
        let mut wet = [2., 2., 2.];
        mix.process(&[1., 3., 6.], &mut AudioBuffer::new(&mut wet, 1).unwrap())
            .unwrap();
        assert_eq!(wet, [1., 1., 3.]);
        mix.reset();
        let mut wet = [10.];
        mix.process(&[5.], &mut AudioBuffer::new(&mut wet, 1).unwrap())
            .unwrap();
        assert_eq!(wet, [0.]);
    }

    struct Mono(Arc<f32>, f32);
    impl AudioProcessor for Mono {
        fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
            assert_eq!(spec.channels, 1);
            Ok(())
        }
        fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
            for sample in b.samples_mut() {
                self.1 = self.1 * *self.0 + *sample;
                *sample = self.1;
            }
        }
        fn reset(&mut self) {
            self.1 = 0.;
        }
    }

    #[test]
    fn duplicated_mono_uses_independent_history() {
        let mut processor =
            Duplicated::new(Arc::new(0.5), |coefficients| Ok(Mono(coefficients, 0.)));
        processor.prepare(spec()).unwrap();
        let mut data = [1., 0., 0., 0., 0., 0.];
        processor.process(&mut AudioBuffer::new(&mut data, 2).unwrap(), &context());
        assert_eq!(data, [1., 0., 0.5, 0., 0.25, 0.]);
    }
}
