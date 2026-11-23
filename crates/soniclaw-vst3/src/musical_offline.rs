//! Blocking continuous offline musical processing. Never use on a device callback.
use super::*;
use soniclaw_transport::Clock;
pub struct OfflineProcessor {
    plugin: std::sync::Weak<Mutex<vst3_host::Plugin>>,
    revoked: Arc<AtomicBool>,
    thread: std::thread::ThreadId,
    buffers: vst3_host::AudioBuffers,
    clock: Clock,
    voices: musical::Voices,
    end: Option<u64>,
    spec: ProcessSpec,
    latency: usize,
    parameters: Vec<u32>,
    errors: Arc<AtomicU64>,
    output: Vec<musical::TimedOutput>,
    capture_events: bool,
    recording: bool,
    panic_on_loop: bool,
}
impl Instance {
    /// Starts processing once for an entire offline render. Owner and processor must remain on this thread.
    pub fn start_offline_musical(&mut self, config: musical::Config) -> Result<OfflineProcessor> {
        config.validate()?;
        if self.worker.is_some() || self.direct.is_some() || self.offline.is_some() {
            return Err("native processing is already active".into());
        }
        let clock = Clock::new(&config.timeline, self.spec.sample_rate)?;
        let parameters = self
            .parameters()?
            .into_iter()
            .filter(|p| !p.is_read_only)
            .map(|p| p.id)
            .collect();
        let latency = {
            let mut p = self.lock()?;
            flush_stopped(&mut p, self.spec.sample_rate)?;
            let layout = p.audio_bus_layout().map_err(err)?;
            if layout.outputs.len() != 1
                || layout.outputs[0].channel_count != 2
                || layout.inputs.len() > 1
                || layout.inputs.first().is_some_and(|b| b.channel_count != 2)
            {
                return Err("offline Studio host requires stereo buses".into());
            }
            p.set_process_mode(vst3_host::ProcessMode::Offline)
                .map_err(err)?;
            p.start_processing().map_err(err)?;
            p.latency_samples() as usize
        };
        let revoked = Arc::new(AtomicBool::new(false));
        self.offline = Some(revoked.clone());
        let errors = Arc::new(AtomicU64::new(0));
        self.offline_errors = errors.clone();
        Ok(OfflineProcessor {
            plugin: Arc::downgrade(&self.plugin),
            revoked,
            thread: std::thread::current().id(),
            buffers: vst3_host::AudioBuffers::new(
                2,
                2,
                self.spec.max_block_frames,
                self.spec.sample_rate,
            ),
            clock,
            voices: Default::default(),
            end: None,
            spec: self.spec,
            latency,
            parameters,
            errors,
            output: Vec::with_capacity(1024),
            capture_events: false,
            recording: config.recording,
            panic_on_loop: config.timeline.panic_on_loop,
        })
    }
    pub fn offline_errors(&self) -> u64 {
        self.offline_errors.load(Ordering::Acquire)
    }
}
impl OfflineProcessor {
    /// Optional bounded event capture; audio exports discard native output MIDI by default.
    pub fn capture_output_events(&mut self, enable: bool) {
        self.capture_events = enable;
        self.output.clear();
    }
    pub fn take_output_events(&mut self) -> Vec<musical::TimedOutput> {
        std::mem::replace(&mut self.output, Vec::with_capacity(1024))
    }

    pub fn process_automated(
        &mut self,
        b: &mut AudioBuffer<'_>,
        ctx: &ProcessContext<'_>,
        points: &[musical::ParameterPoint],
    ) -> std::result::Result<(), Error> {
        let result = self.render(b, ctx, points);
        if result.is_err() {
            b.clear();
            self.errors.fetch_add(1, Ordering::Release);
        }
        result.map_err(|_| Error::InvalidEvents)
    }
    fn render(
        &mut self,
        b: &mut AudioBuffer<'_>,
        ctx: &ProcessContext<'_>,
        points: &[musical::ParameterPoint],
    ) -> Result<()> {
        if self.thread != std::thread::current().id()
            || self.revoked.load(Ordering::Acquire)
            || b.channels() != 2
            || b.frames() > self.spec.max_block_frames
            || ctx.events.len() > 1024
            || validate_events(ctx.events, b.frames()).is_err()
            || points.len() > 1024
            || points.windows(2).any(|p| p[0].frame > p[1].frame)
            || points.iter().any(|p| {
                p.frame >= b.frames()
                    || !p.value.is_finite()
                    || !(0.0..=1.0).contains(&p.value)
                    || !self.parameters.contains(&p.id)
            })
        {
            return Err("invalid offline block, owner thread, events or parameters".into());
        }
        let plugin = self
            .plugin
            .upgrade()
            .ok_or("offline native owner retired")?;
        let mut p = plugin.lock().map_err(|_| err("poisoned offline lock"))?;
        p.service_run_loop();
        let flags = p.service_host_requests().map_err(err)?;
        if flags.io_changed() || flags.reload_component() || flags.latency_changed() {
            return Err("offline native reconfiguration requested; restart render".into());
        }
        if self.end != Some(ctx.transport.sample_position) {
            p.midi_panic().map_err(err)?;
            self.voices = Default::default();
            self.clock.seek(ctx.transport.sample_position)?;
        }
        self.clock.set_playing(ctx.transport.playing);
        let mut start = 0;
        while start < b.frames() {
            let frame = self.clock.frame()?;
            if frame.looped && self.panic_on_loop {
                p.midi_panic().map_err(err)?;
                self.voices = Default::default();
            }
            self.clock.advance(1)?;
            let mut count = 1;
            while start + count < b.frames() {
                let f = self.clock.frame()?;
                if !musical::same_span(frame, f)
                    || f.sample != frame.sample + if frame.playing { count as u64 } else { 0 }
                {
                    break;
                }
                self.clock.advance(1)?;
                count += 1;
            }
            p.set_musical_transport(musical::context(frame, self.recording))
                .map_err(err)?;
            self.buffers.block_size = count;
            for c in 0..2 {
                self.buffers.inputs[c].resize(count, 0.);
                self.buffers.outputs[c].resize(count, 0.);
                self.buffers.outputs[c].fill(0.);
                for (value, frame) in self.buffers.inputs[c].iter_mut().zip(
                    b.samples()[start * 2..(start + count) * 2]
                        .as_chunks::<2>()
                        .0
                        .iter(),
                ) {
                    *value = frame[c];
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
                .filter(|p| (start..start + count).contains(&p.frame))
            {
                p.queue_processor_parameter_at(point.id, point.value, (point.frame - start) as i32)
                    .map_err(err)?;
            }
            p.process_audio(&mut self.buffers).map_err(err)?;
            for e in p.take_output_events() {
                if self.capture_events {
                    if self.output.len() >= 1024 {
                        return Err(
                            "offline native output event capacity exceeded; drain between blocks"
                                .into(),
                        );
                    }
                    self.output.push(musical::TimedOutput {
                        sample: ctx.transport.sample_position
                            + start as u64
                            + e.sample_offset.max(0) as u64,
                        event: e,
                    });
                }
            }
            for (i, frame) in b.samples_mut()[start * 2..(start + count) * 2]
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .enumerate()
            {
                frame[0] = self.buffers.outputs[0][i];
                frame[1] = self.buffers.outputs[1][i];
            }
            start += count;
        }
        self.end = Some(
            ctx.transport.sample_position
                + if ctx.transport.playing {
                    b.frames() as u64
                } else {
                    0
                },
        );
        Ok(())
    }
}
impl Drop for OfflineProcessor {
    fn drop(&mut self) {
        self.revoked.store(true, Ordering::Release);
    }
}
impl AudioProcessor for OfflineProcessor {
    fn prepare(&mut self, s: ProcessSpec) -> std::result::Result<(), Error> {
        if s.sample_rate != self.spec.sample_rate
            || s.channels != 2
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
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        let _ = self.process_automated(b, ctx, &[]);
    }
}
