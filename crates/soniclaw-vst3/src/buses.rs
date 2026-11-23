//! Native VST3 multiple-bus owner and fixed-memory worker bridge.
use super::*;
use soniclaw_bus::{
    flex::{Bus, Layout, Role},
    multigraph::{MultiProcessor, MultiSpec},
};
const CHANNELS: usize = 64;
#[derive(Clone)]
struct BusPacket {
    input: [f32; CHANNELS * BLOCK],
    output: [f32; CHANNELS * BLOCK],
    sequence: u64,
    transport: soniclaw_core::Transport,
    events: [Event; 128],
    count: usize,
    parameters: [musical::ParameterPoint; 1024],
    parameter_count: usize,
}
impl Default for BusPacket {
    fn default() -> Self {
        Self {
            input: [0.; CHANNELS * BLOCK],
            output: [0.; CHANNELS * BLOCK],
            sequence: 0,
            transport: Default::default(),
            events: [Event {
                frame: 0,
                message: MidiMessage::AllNotesOff,
            }; 128],
            count: 0,
            parameters: [musical::ParameterPoint::default(); 1024],
            parameter_count: 0,
        }
    }
}
pub struct BusInstance {
    inner: Instance,
    layout: Layout,
}
impl BusInstance {
    /// # Safety
    /// Load trusted ABI-conforming native code on the GUI/main thread. This enables all advertised audio buses.
    pub unsafe fn load(path: &Path, class: Option<&str>, spec: ProcessSpec) -> Result<Self> {
        let inner = unsafe { Instance::load_internal(path, class, spec, false, None) }?;
        let make = |buses: &[vst3_host::AudioBusConfig], prefix: &str| {
            buses
                .iter()
                .enumerate()
                .map(|(i, b)| Bus {
                    id: format!("{prefix}-{i}"),
                    name: format!("{prefix} {}", i + 1),
                    channels: b.channel_count,
                    role: if i == 0 { Role::Main } else { Role::Auxiliary },
                })
                .collect()
        };
        let layout = Layout {
            inputs: make(&inner.layout.inputs, "input"),
            outputs: make(&inner.layout.outputs, "output"),
        };
        layout.validate()?;
        {
            let mut p = inner.lock()?;
            for i in 0..layout.inputs.len() {
                p.set_bus_active(
                    vst3_host::MediaType::Audio,
                    vst3_host::BusDirection::Input,
                    i as i32,
                    true,
                )
                .map_err(err)?;
            }
            for i in 0..layout.outputs.len() {
                p.set_bus_active(
                    vst3_host::MediaType::Audio,
                    vst3_host::BusDirection::Output,
                    i as i32,
                    true,
                )
                .map_err(err)?;
            }
        }
        Ok(Self { inner, layout })
    }
    pub fn layout(&self) -> &Layout {
        &self.layout
    }
    /// Application metadata only; bus indices/channels remain the native configuration.
    pub fn set_input_role(&mut self, index: usize, role: Role) -> Result<()> {
        if self.inner.worker.is_some() {
            return Err("stop before editing bus metadata".into());
        }
        self.layout
            .inputs
            .get_mut(index)
            .ok_or("unknown input bus")?
            .role = role;
        Ok(())
    }
    pub fn into_owner(self) -> Instance {
        self.inner
    }
    pub fn owner(&mut self) -> &mut Instance {
        &mut self.inner
    }
    pub fn start_buffered(&mut self) -> Result<BusProcessor> {
        if self.inner.worker.is_some()
            || self.inner.direct.is_some()
            || self.inner.spec.max_block_frames < BLOCK
        {
            return Err("stop prior run; prepare maximum >=256".into());
        }
        let (native_latency, native_layout) = {
            let mut p = self.inner.lock()?;
            p.service_host_requests().map_err(err)?;
            p.start_processing().map_err(err)?;
            p.service_host_requests().map_err(err)?;
            (p.latency_samples(), p.audio_bus_layout().map_err(err)?)
        };
        let matches = |a: &[Bus], b: &[vst3_host::AudioBusConfig]| {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|(a, b)| a.channels == b.channel_count && b.active)
        };
        if !matches(&self.layout.inputs, &native_layout.inputs)
            || !matches(&self.layout.outputs, &native_layout.outputs)
        {
            self.inner.lock()?.stop_processing().map_err(err)?;
            return Err("native bus configuration changed; recreate owner".into());
        }
        self.inner.native_latency = native_latency;
        self.inner.layout = native_layout.clone();
        self.inner.shared = Arc::new(Shared::default());
        let shared = self.inner.shared.clone();
        let plugin = self.inner.plugin.clone();
        let rate = self.inner.spec.sample_rate;
        let (tx, mut input) = rtrb::RingBuffer::<BusPacket>::new(8);
        let (mut output, rx) = rtrb::RingBuffer::<BusPacket>::new(8);
        self.inner.worker = Some(
            std::thread::Builder::new()
                .name("soniclaw-vst3-buses".into())
                .stack_size(4 * 1024 * 1024)
                .spawn(move || {
                    let mut audio = vst3_host::BusAudioBuffers::new(&native_layout, BLOCK, rate);
                    while !shared.stop.load(Ordering::Acquire) {
                        let Ok(mut packet) = input.pop() else {
                            std::thread::park_timeout(std::time::Duration::from_micros(100));
                            continue;
                        };
                        let result = (|| -> Result<()> {
                            let mut p =
                                plugin.lock().map_err(|_| err("native bus lock poisoned"))?;
                            if shared.panic.swap(false, Ordering::AcqRel) {
                                p.midi_panic().map_err(err)?;
                            }
                            p.set_tempo(packet.transport.tempo).map_err(err)?;
                            p.set_playing(packet.transport.playing).map_err(err)?;
                            p.set_transport_position(TransportPosition {
                                samples: packet.transport.sample_position.min(i64::MAX as u64)
                                    as i64,
                                quarter_notes: packet.transport.sample_position as f64 / rate
                                    * packet.transport.tempo
                                    / 60.,
                            })
                            .map_err(err)?;
                            let mut channel = 0;
                            for bus in &mut audio.inputs {
                                for c in &mut bus.channels {
                                    for (i, sample) in c.iter_mut().enumerate() {
                                        *sample = packet.input[channel * BLOCK + i];
                                    }
                                    channel += 1;
                                }
                            }
                            audio
                                .outputs
                                .iter_mut()
                                .for_each(|bus| bus.channels.iter_mut().for_each(|c| c.fill(0.)));
                            for e in &packet.events[..packet.count] {
                                if e.message == MidiMessage::AllNotesOff {
                                    p.midi_panic().map_err(err)?;
                                } else {
                                    p.send_midi_event_at(
                                        convert_midi(e.message)
                                            .ok_or("multibus bridge supports MIDI1 events")?,
                                        e.frame as i32,
                                    )
                                    .map_err(err)?;
                                }
                            }
                            for point in &packet.parameters[..packet.parameter_count] {
                                p.queue_processor_parameter_at(
                                    point.id,
                                    point.value,
                                    point.frame as i32,
                                )
                                .map_err(err)?;
                            }
                            p.process_bus_audio(&mut audio).map_err(err)?;
                            let mut channel = 0;
                            for bus in &audio.outputs {
                                for c in &bus.channels {
                                    for (i, sample) in c.iter().enumerate() {
                                        packet.output[channel * BLOCK + i] = *sample;
                                    }
                                    channel += 1;
                                }
                            }
                            Ok(())
                        })();
                        if result.is_err() {
                            packet.output.fill(0.);
                            shared.errors.fetch_add(1, Ordering::Relaxed);
                        }
                        if output.push(packet).is_err() {
                            shared.output_drops.fetch_add(1, Ordering::Relaxed);
                        }
                        shared.blocks.fetch_add(1, Ordering::Relaxed);
                    }
                })?,
        );
        Ok(BusProcessor {
            layout: self.layout.clone(),
            rate,
            max_frames: self.inner.spec.max_block_frames,
            tx,
            rx,
            input: Default::default(),
            output: None,
            queued: None,
            position: 0,
            shared: self.inner.shared.clone(),
            latency: LATENCY + native_latency as usize,
            pending: Vec::with_capacity(1024),
        })
    }
}
pub struct BusProcessor {
    layout: Layout,
    rate: f64,
    max_frames: usize,
    tx: rtrb::Producer<BusPacket>,
    rx: rtrb::Consumer<BusPacket>,
    input: BusPacket,
    output: Option<BusPacket>,
    queued: Option<BusPacket>,
    position: u64,
    shared: Arc<Shared>,
    latency: usize,
    pending: Vec<musical::ParameterPoint>,
}
impl BusProcessor {
    pub fn process_automated(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        ctx: &ProcessContext<'_>,
        points: &[musical::ParameterPoint],
    ) -> std::result::Result<(), Error> {
        let frames = self
            .layout
            .validate_buffers(inputs, outputs, self.max_frames)?;
        if points.len() > 1024
            || points.windows(2).any(|p| p[0].frame > p[1].frame)
            || points.iter().any(|p| {
                p.frame >= frames || !p.value.is_finite() || !(0.0..=1.0).contains(&p.value)
            })
        {
            return Err(Error::InvalidEvents);
        }
        self.pending.clear();
        self.pending.extend_from_slice(points);
        let result = MultiProcessor::process(self, inputs, outputs, ctx);
        self.pending.clear();
        result
    }
}
impl MultiProcessor for BusProcessor {
    fn prepare(&mut self, s: MultiSpec) -> std::result::Result<(), Error> {
        s.validate()?;
        if s.layout != self.layout
            || s.sample_rate != self.rate
            || s.max_block_frames > self.max_frames
            || self.position != 0
        {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        ctx: &ProcessContext<'_>,
    ) -> std::result::Result<(), Error> {
        let result = (|| {
            let frames = self
                .layout
                .validate_buffers(inputs, outputs, self.max_frames)?;
            validate_events(ctx.events, frames)?;
            if ctx.events.len() > 1024
                || !ctx.transport.tempo.is_finite()
                || !(1.0..=1000.0).contains(&ctx.transport.tempo)
            {
                return Err(Error::InvalidEvents);
            }
            if self.shared.stop.load(Ordering::Acquire) {
                return Err(Error::NotPrepared);
            }
            let mut event = 0;
            let mut point = 0;
            for i in 0..frames {
                let offset = self.position as usize % BLOCK;
                let seq = self.position / BLOCK as u64;
                if offset == 0 {
                    self.input.sequence = seq;
                    self.input.count = 0;
                    self.input.parameter_count = 0;
                    self.input.transport = soniclaw_core::Transport {
                        sample_position: ctx.transport.sample_position.saturating_add(i as u64),
                        ..ctx.transport
                    };
                    self.output = None;
                    if seq >= 4 {
                        for _ in 0..8 {
                            match self.queued.take().or_else(|| self.rx.pop().ok()) {
                                Some(p) if p.sequence + 4 > seq => {
                                    self.queued = Some(p);
                                    break;
                                }
                                Some(p) if p.sequence + 4 == seq => {
                                    self.output = Some(p);
                                    break;
                                }
                                Some(_) => {}
                                None => break,
                            }
                        }
                        if self.output.is_none() {
                            self.shared.underruns.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                while event < ctx.events.len() && ctx.events[event].frame == i {
                    if self.input.count >= 128 {
                        self.shared.stop.store(true, Ordering::Release);
                        self.shared.errors.fetch_add(1, Ordering::Relaxed);
                        return Err(Error::InvalidEvents);
                    }
                    self.input.events[self.input.count] = Event {
                        frame: offset,
                        message: ctx.events[event].message,
                    };
                    self.input.count += 1;
                    event += 1;
                }
                while point < self.pending.len() && self.pending[point].frame == i {
                    if self.input.parameter_count >= 1024 {
                        self.shared.errors.fetch_add(1, Ordering::Relaxed);
                        self.shared.stop.store(true, Ordering::Release);
                        return Err(Error::InvalidEvents);
                    }
                    self.input.parameters[self.input.parameter_count] = musical::ParameterPoint {
                        frame: offset,
                        ..self.pending[point]
                    };
                    self.input.parameter_count += 1;
                    point += 1;
                }
                let mut channel = 0;
                for (bus, shape) in inputs.iter().zip(&self.layout.inputs) {
                    for c in 0..shape.channels {
                        self.input.input[channel * BLOCK + offset] = bus[i * shape.channels + c];
                        channel += 1;
                    }
                }
                let mut channel = 0;
                for (bus, shape) in outputs.iter_mut().zip(&self.layout.outputs) {
                    for c in 0..shape.channels {
                        bus[i * shape.channels + c] = self
                            .output
                            .as_ref()
                            .map_or(0., |p| p.output[channel * BLOCK + offset]);
                        channel += 1;
                    }
                }
                if offset + 1 == BLOCK && self.tx.push(self.input.clone()).is_err() {
                    self.shared.input_drops.fetch_add(1, Ordering::Relaxed);
                    self.shared.stop.store(true, Ordering::Release);
                    return Err(Error::InvalidBuffer);
                }
                self.position += 1;
            }
            Ok(())
        })();
        if result.is_err() {
            for output in outputs {
                output.fill(0.);
            }
        }
        result
    }
    fn reset(&mut self) {
        self.shared.panic.store(true, Ordering::Release);
    }
    fn latency_frames(&self) -> usize {
        self.latency
    }
}
