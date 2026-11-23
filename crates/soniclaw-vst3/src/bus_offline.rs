//! Continuous, owner-thread-only offline multiple-bus processing.
use super::*;
use soniclaw_bus::{
    flex::Layout,
    multigraph::{MultiProcessor, MultiSpec},
};
use soniclaw_transport::Clock;
pub struct Processor {
    plugin: std::sync::Weak<Mutex<vst3_host::Plugin>>,
    revoked: Arc<AtomicBool>,
    thread: std::thread::ThreadId,
    layout: Layout,
    audio: vst3_host::BusAudioBuffers,
    spec: ProcessSpec,
    clock: Clock,
    voices: musical::Voices,
    end: Option<u64>,
    latency: usize,
    errors: Arc<AtomicU64>,
}
impl buses::BusInstance {
    pub fn start_offline(&mut self, config: musical::Config) -> Result<Processor> {
        let expected = self.layout().clone();
        let owner = self.owner();
        if owner.worker.is_some() || owner.direct.is_some() || owner.offline.is_some() {
            return Err("stop native stream before offline bus rendering".into());
        }
        let spec = owner.spec;
        let clock = Clock::new(&config.timeline, spec.sample_rate)?;
        let native = owner.lock()?.audio_bus_layout().map_err(err)?;
        if native.inputs.len() != expected.inputs.len()
            || native.outputs.len() != expected.outputs.len()
            || native
                .inputs
                .iter()
                .zip(&expected.inputs)
                .any(|(a, b)| a.channel_count != b.channels)
            || native
                .outputs
                .iter()
                .zip(&expected.outputs)
                .any(|(a, b)| a.channel_count != b.channels)
        {
            return Err("native bus layout changed during state restoration".into());
        }
        owner.layout = native.clone();
        let latency = {
            let mut p = owner.lock()?;
            flush_stopped(&mut p, spec.sample_rate)?;
            p.set_process_mode(vst3_host::ProcessMode::Offline)
                .map_err(err)?;
            p.start_processing().map_err(err)?;
            p.latency_samples() as usize
        };
        let flag = Arc::new(AtomicBool::new(false));
        let errors = Arc::new(AtomicU64::new(0));
        owner.offline = Some(flag.clone());
        owner.offline_errors = errors.clone();
        let plugin = Arc::downgrade(&owner.plugin);
        let layout = self.layout().clone();
        Ok(Processor {
            plugin,
            revoked: flag,
            thread: std::thread::current().id(),
            layout,
            audio: vst3_host::BusAudioBuffers::new(
                &native,
                spec.max_block_frames,
                spec.sample_rate,
            ),
            spec,
            clock,
            voices: Default::default(),
            end: None,
            latency,
            errors,
        })
    }
}
impl Processor {
    pub fn process_automated(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        ctx: &ProcessContext<'_>,
        points: &[musical::ParameterPoint],
    ) -> std::result::Result<(), Error> {
        let r = self.render(inputs, outputs, ctx, points);
        if r.is_err() {
            outputs.iter_mut().for_each(|o| o.fill(0.));
            self.errors.fetch_add(1, Ordering::Release);
        }
        r.map_err(|_| Error::InvalidBuffer)
    }
    fn render(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        ctx: &ProcessContext<'_>,
        points: &[musical::ParameterPoint],
    ) -> Result<()> {
        if self.thread != std::thread::current().id() || self.revoked.load(Ordering::Acquire) {
            return Err("offline native bus owner/thread revoked".into());
        }
        let n = self
            .layout
            .validate_buffers(inputs, outputs, self.spec.max_block_frames)?;
        validate_events(ctx.events, n)?;
        if ctx.events.len() > 1024
            || points.len() > 1024
            || points.windows(2).any(|p| p[0].frame > p[1].frame)
            || points
                .iter()
                .any(|p| p.frame >= n || !p.value.is_finite() || !(0.0..=1.0).contains(&p.value))
        {
            return Err("invalid bus native events".into());
        }
        let plugin = self.plugin.upgrade().ok_or("offline bus owner dropped")?;
        let mut p = plugin
            .lock()
            .map_err(|_| err("offline bus lock poisoned"))?;
        if self.end != Some(ctx.transport.sample_position) {
            p.midi_panic().map_err(err)?;
            self.voices = Default::default();
            self.clock.seek(ctx.transport.sample_position)?;
        }
        self.clock.set_playing(ctx.transport.playing);
        let mut start = 0;
        while start < n {
            let f = self.clock.frame()?;
            if f.looped {
                p.midi_panic().map_err(err)?;
                self.voices = Default::default();
            }
            self.clock.advance(1)?;
            let mut count = 1;
            while start + count < n {
                let next = self.clock.frame()?;
                if !musical::same_span(f, next)
                    || next.sample != f.sample + if f.playing { count as u64 } else { 0 }
                {
                    break;
                }
                self.clock.advance(1)?;
                count += 1;
            }
            p.set_musical_transport(musical::context(f, false))
                .map_err(err)?;
            self.audio.block_size = count;
            for ((bus, source), shape) in self
                .audio
                .inputs
                .iter_mut()
                .zip(inputs)
                .zip(&self.layout.inputs)
            {
                for (c, channel) in bus.channels.iter_mut().enumerate() {
                    channel.resize(count, 0.);
                    for (i, v) in channel.iter_mut().enumerate() {
                        *v = source[(start + i) * shape.channels + c];
                    }
                }
            }
            for bus in &mut self.audio.outputs {
                for channel in &mut bus.channels {
                    channel.resize(count, 0.);
                    channel.fill(0.);
                }
            }
            for e in ctx
                .events
                .iter()
                .filter(|e| (start..start + count).contains(&e.frame))
            {
                self.voices
                    .send(&mut p, e.message, (e.frame - start) as i32)?;
            }
            for point in points
                .iter()
                .filter(|e| (start..start + count).contains(&e.frame))
            {
                p.queue_processor_parameter_at(point.id, point.value, (point.frame - start) as i32)
                    .map_err(err)?;
            }
            p.process_bus_audio(&mut self.audio).map_err(err)?;
            let _ = p.take_output_events();
            for ((bus, out), shape) in self
                .audio
                .outputs
                .iter()
                .zip(outputs.iter_mut())
                .zip(&self.layout.outputs)
            {
                for (c, channel) in bus.channels.iter().enumerate() {
                    for (i, v) in channel.iter().enumerate() {
                        out[(start + i) * shape.channels + c] = *v;
                    }
                }
            }
            start += count;
        }
        self.end =
            Some(ctx.transport.sample_position + if ctx.transport.playing { n as u64 } else { 0 });
        Ok(())
    }
}
impl Drop for Processor {
    fn drop(&mut self) {
        self.revoked.store(true, Ordering::Release);
    }
}
impl MultiProcessor for Processor {
    fn prepare(&mut self, s: MultiSpec) -> std::result::Result<(), Error> {
        s.validate()?;
        if s.layout != self.layout
            || s.sample_rate != self.spec.sample_rate
            || s.max_block_frames > self.spec.max_block_frames
        {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
    fn reset(&mut self) {
        self.end = None;
    }
    fn latency_frames(&self) -> usize {
        self.latency
    }
    fn process(
        &mut self,
        i: &[&[f32]],
        o: &mut [&mut [f32]],
        ctx: &ProcessContext<'_>,
    ) -> std::result::Result<(), Error> {
        self.process_automated(i, o, ctx, &[])
    }
}
