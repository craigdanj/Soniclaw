//! Continuous native VST3 offline lease. Preparation/activation/destruction stay on the owner thread.
use super::*;
use std::sync::atomic::AtomicBool;
pub struct OfflineAudio {
    plugin: Arc<Mutex<vst3_host::Plugin>>,
    buffers: vst3_host::AudioBuffers,
    spec: ProcessSpec,
    parameters: Vec<Parameter>,
    failure: Option<HostError>,
    cancel: Arc<AtomicBool>,
    latency: usize,
}
impl OfflineAudio {
    pub fn process_block(
        &mut self,
        data: &mut [f32],
        events: &[Event],
        changes: &[(usize, u32, f64)],
        transport: TransportPosition,
        bpm: f64,
    ) -> Result<()> {
        if self.cancel.load(Ordering::Acquire) {
            return Err("offline render cancelled or reconfiguration requested".into());
        }
        let frames = data.len() / 2;
        if !data.len().is_multiple_of(2)
            || frames == 0
            || frames > self.spec.max_block_frames
            || validate_events(events, frames).is_err()
            || events.len() > 1024
            || changes.len() > 1024
            || !bpm.is_finite()
            || !(1.0..=1000.0).contains(&bpm)
            || !transport.quarter_notes.is_finite()
        {
            return Err("invalid offline block/events/transport".into());
        }
        if changes.windows(2).any(|w| w[0].0 > w[1].0)
            || changes.iter().any(|&(off, id, v)| {
                off >= frames
                    || !v.is_finite()
                    || !(0.0..=1.0).contains(&v)
                    || !self
                        .parameters
                        .iter()
                        .any(|p| p.id == id && !p.is_read_only)
            })
        {
            return Err("invalid offline parameter changes".into());
        }
        for channel in &mut self.buffers.inputs {
            channel.resize(frames, 0.);
        }
        for channel in &mut self.buffers.outputs {
            channel.resize(frames, 0.);
            channel.fill(0.);
        }
        for (i, f) in data.as_chunks::<2>().0.iter().enumerate() {
            self.buffers.inputs[0][i] = f[0];
            self.buffers.inputs[1][i] = f[1];
        }
        let mut p = self
            .plugin
            .lock()
            .map_err(|_| err("offline native lock poisoned"))?;
        p.set_transport_position(transport).map_err(err)?;
        p.set_tempo(bpm).map_err(err)?;
        p.set_playing(true).map_err(err)?;
        for e in events {
            if e.message == MidiMessage::AllNotesOff {
                p.midi_panic().map_err(err)?;
            } else {
                let m = convert_midi(e.message).ok_or("unsupported offline MIDI expression")?;
                p.send_midi_event_at(m, e.frame as i32).map_err(err)?;
            }
        }
        for &(offset, id, value) in changes {
            p.set_parameter_at(id, value, offset as i32).map_err(err)?;
        }
        p.process_audio(&mut self.buffers).map_err(err)?;
        for (i, f) in data.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            f[0] = self.buffers.outputs[0][i];
            f[1] = self.buffers.outputs[1][i];
        }
        Ok(())
    }
}
impl AudioProcessor for OfflineAudio {
    fn prepare(&mut self, s: ProcessSpec) -> std::result::Result<(), Error> {
        s.validate()?;
        if s.channels != 2
            || s.sample_rate != self.spec.sample_rate
            || s.max_block_frames > self.spec.max_block_frames
        {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        if b.channels() != 2 {
            b.clear();
            self.failure = Some(err("offline VST3 requires stereo"));
            return;
        }
        let transport = TransportPosition {
            samples: ctx.transport.sample_position.min(i64::MAX as u64) as i64,
            quarter_notes: ctx.transport.sample_position as f64 / self.spec.sample_rate
                * ctx.transport.tempo
                / 60.,
        };
        if let Err(e) = self.process_block(
            b.samples_mut(),
            ctx.events,
            &[],
            transport,
            ctx.transport.tempo,
        ) {
            b.clear();
            self.failure = Some(e);
        }
    }
    fn reset(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
    fn latency_frames(&self) -> usize {
        self.latency
    }
}
impl Instance {
    /// One continuous native stream, processed on a scoped audio worker with main-thread lifecycle/services.
    /// The callback may allocate (offline). Errors are checked even if the caller ignores a process failure.
    pub fn with_offline<R: Send, F>(&mut self, callback: F) -> Result<R>
    where
        F: FnOnce(&mut OfflineAudio) -> Result<R> + Send,
    {
        if self.worker.is_some() || self.direct.is_some() || self.offline.is_some() {
            return Err("stop live processing before offline rendering".into());
        }
        self.close_editor();
        let (parameters, latency) = {
            let mut p = self.lock()?;
            p.set_process_mode(vst3_host::ProcessMode::Offline)
                .map_err(err)?;
            p.service_host_requests().map_err(err)?;
            p.start_processing().map_err(err)?;
            p.service_host_requests().map_err(err)?;
            (
                p.get_parameters().map_err(err)?,
                p.latency_samples() as usize,
            )
        };
        let plugin = self.plugin.clone();
        let spec = self.spec;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let result = std::thread::scope(|scope| {
            let handle = scope.spawn(move || {
                let mut audio = OfflineAudio {
                    plugin,
                    buffers: vst3_host::AudioBuffers::new(
                        2,
                        2,
                        spec.max_block_frames,
                        spec.sample_rate,
                    ),
                    spec,
                    parameters,
                    failure: None,
                    cancel: worker_cancel,
                    latency,
                };
                let result = callback(&mut audio);
                if let Some(error) = audio.failure {
                    Err(error)
                } else {
                    result
                }
            });
            while !handle.is_finished() {
                if let Ok(mut p) = self.lock() {
                    p.service_run_loop();
                    let flags = p.direct_restart_flags();
                    if flags.io_changed() || flags.reload_component() || flags.latency_changed() {
                        cancel.store(true, Ordering::Release);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            handle.join().map_err(|_| err("offline worker panicked"))?
        });
        let stop = {
            let mut p = self.lock()?;
            let stop = p.stop_processing().map_err(err);
            let mode = p
                .set_process_mode(vst3_host::ProcessMode::Realtime)
                .map_err(err);
            stop.and(mode)
        };
        result.and_then(|value| {
            stop?;
            Ok(value)
        })
    }
}
