//! Scoped multi-port f32 CLAP processing. Port/configuration edits are main-thread stopped work.
use super::*;
use clap_sys::ext::audio_ports_config::*;
#[derive(Clone, Debug, Serialize)]
pub struct Port {
    pub id: u32,
    pub name: String,
    pub channels: usize,
    pub main: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Configuration {
    pub id: u32,
    pub name: String,
    pub inputs: u32,
    pub outputs: u32,
}
impl ClapInstance {
    /// Common named layout; mark known auxiliary inputs as Sidechain in application policy.
    pub fn bus_layout(&self) -> soniclaw_bus::flex::Layout {
        use soniclaw_bus::flex::*;
        let ports = |p: &[Port]| {
            p.iter()
                .map(|p| Bus {
                    id: p.id.to_string(),
                    name: p.name.clone(),
                    channels: p.channels,
                    role: if p.main { Role::Main } else { Role::Auxiliary },
                })
                .collect()
        };
        Layout {
            inputs: ports(&self.input_ports),
            outputs: ports(&self.output_ports),
        }
    }

    pub fn audio_ports(&self) -> (&[Port], &[Port]) {
        (&self.input_ports, &self.output_ports)
    }
    pub fn audio_configurations(&self) -> Result<Vec<Configuration>, HostError> {
        if self.active {
            return Err("deactivate before configuration query".into());
        }
        unsafe {
            let ext = (*self.plugin).get_extension.expect("checked")(
                self.plugin,
                CLAP_EXT_AUDIO_PORTS_CONFIG.as_ptr(),
            )
            .cast::<clap_plugin_audio_ports_config>();
            if ext.is_null() {
                return Ok(vec![]);
            }
            let count = (*ext).count.ok_or("missing config count")?(self.plugin);
            if count > 256 {
                return Err("configuration limit exceeded".into());
            }
            let mut out = vec![];
            for i in 0..count {
                let mut info = std::mem::zeroed();
                if !(*ext).get.ok_or("missing config get")?(self.plugin, i, &mut info) {
                    return Err("configuration query failed".into());
                }
                let name = String::from_utf8_lossy(
                    &info
                        .name
                        .iter()
                        .take_while(|c| **c != 0)
                        .map(|c| *c as u8)
                        .collect::<Vec<_>>(),
                )
                .into_owned();
                out.push(Configuration {
                    id: info.id,
                    name,
                    inputs: info.input_port_count,
                    outputs: info.output_port_count,
                });
            }
            Ok(out)
        }
    }
    pub fn select_audio_configuration(&mut self, id: u32) -> Result<(), HostError> {
        if self.active {
            return Err("deactivate before selecting audio configuration".into());
        }
        if !self.audio_configurations()?.iter().any(|c| c.id == id) {
            return Err("unknown audio configuration".into());
        }
        unsafe {
            let ext = (*self.plugin).get_extension.expect("checked")(
                self.plugin,
                CLAP_EXT_AUDIO_PORTS_CONFIG.as_ptr(),
            )
            .cast::<clap_plugin_audio_ports_config>();
            if !(*ext).select.ok_or("missing config select")?(self.plugin, id) {
                return Err("configuration refused".into());
            }
        }
        self.inspect()
    }
    /// Activate, lend all audio ports to a separate scoped audio thread, join and deactivate.
    pub fn with_bus_audio<R: Send, F: for<'a> FnOnce(&mut BusAudio<'a>) -> R + Send>(
        &mut self,
        rate: f64,
        max_frames: usize,
        f: F,
    ) -> Result<R, HostError> {
        self.with_bus_audio_serviced(rate, max_frames, std::sync::Arc::new(AtomicU32::new(0)), f)
    }
    /// Keep CLAP main-thread callbacks/timers/FDs alive throughout a persistent audio scope.
    /// Observed host request bits accumulate in `requests` for the control owner.
    pub fn with_bus_audio_serviced<R: Send, F: for<'a> FnOnce(&mut BusAudio<'a>) -> R + Send>(
        &mut self,
        rate: f64,
        max_frames: usize,
        requests: std::sync::Arc<AtomicU32>,
        f: F,
    ) -> Result<R, HostError> {
        if self.active {
            return Err("deactivate before bus lease".into());
        }
        self.inspect()?;
        let spec = ProcessSpec {
            sample_rate: rate,
            max_block_frames: max_frames,
            channels: self.output_channels,
        };
        spec.validate()?;
        if max_frames > 8192 {
            return Err("bus block limit 8192".into());
        }
        let storage = Storage::new(&self.input_ports, &self.output_ports, max_frames);
        unsafe {
            if !(*self.plugin).activate.expect("checked")(self.plugin, rate, 1, max_frames as u32) {
                return Err("bus activation refused".into());
            }
        }
        self.active = true;
        let latency = unsafe {
            (*self.plugin).get_extension.expect("checked")(self.plugin, CLAP_EXT_LATENCY.as_ptr())
                .cast::<clap_plugin_latency>()
        };
        self.latency = if latency.is_null() {
            0
        } else {
            unsafe { (*latency).get.ok_or("missing bus latency getter")?(self.plugin) as usize }
        };
        if self.latency > 262144 {
            self.deactivate();
            return Err("native latency exceeds limit".into());
        }
        let mut audio = BusAudio {
            latency: self.latency,
            plugin: self.plugin,
            _context: &self.context,
            parameters: &self.parameters,
            inputs: &self.input_ports,
            outputs: &self.output_ports,
            spec,
            storage,
            processing: false,
            steady: 0,
            note_dialect: self.note_dialect,
        };
        let context = &self.context;
        let plugin = self.plugin;
        let result = std::thread::scope(|scope| {
            let worker = scope.spawn(move || {
                AUDIO.set(true);
                if !unsafe { (*audio.plugin).start_processing.expect("checked")(audio.plugin) } {
                    return Err("bus start failed");
                }
                audio.processing = true;
                let result = f(&mut audio);
                audio.stop();
                AUDIO.set(false);
                Ok(result)
            });
            while !worker.is_finished() {
                let flags = context.flags.swap(0, Ordering::AcqRel);
                requests.fetch_or(flags, Ordering::Release);
                if flags & 2 != 0 {
                    unsafe {
                        if let Some(callback) = (*plugin).on_main_thread {
                            callback(plugin);
                        }
                    }
                }
                super::services::pump_parts(plugin, context);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            worker.join()
        });
        self.deactivate();
        match result {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(e)) => Err(e.into()),
            Err(_) => Err("bus audio scope panicked".into()),
        }
    }
}
struct Planar {
    channels: Vec<Vec<f32>>,
    pointers: Vec<*mut f32>,
}
impl Planar {
    fn new(ch: usize, max: usize) -> Self {
        let mut channels = vec![vec![0.0; max]; ch];
        let pointers = channels.iter_mut().map(|c| c.as_mut_ptr()).collect();
        Self { channels, pointers }
    }
    fn native(&mut self) -> clap_audio_buffer {
        clap_audio_buffer {
            data32: self.pointers.as_mut_ptr(),
            data64: ptr::null_mut(),
            channel_count: self.channels.len() as u32,
            latency: 0,
            constant_mask: 0,
        }
    }
}
struct Storage {
    input: Vec<Planar>,
    output: Vec<Planar>,
    native_in: Vec<clap_audio_buffer>,
    native_out: Vec<clap_audio_buffer>,
    events: Vec<NativeEvent>,
    pointers: Vec<*const clap_event_header>,
    out: Vec<OutputEvent>,
    dropped: u64,
}
impl Storage {
    fn new(input: &[Port], output: &[Port], max: usize) -> Self {
        let mut input: Vec<_> = input.iter().map(|p| Planar::new(p.channels, max)).collect();
        let mut output: Vec<_> = output
            .iter()
            .map(|p| Planar::new(p.channels, max))
            .collect();
        Self {
            native_in: input.iter_mut().map(|p| p.native()).collect(),
            native_out: output.iter_mut().map(|p| p.native()).collect(),
            input,
            output,
            events: Vec::with_capacity(LIMIT),
            pointers: Vec::with_capacity(LIMIT),
            out: Vec::with_capacity(LIMIT),
            dropped: 0,
        }
    }
}
/// Raw pointers target this scope's owned stable Vec allocations; no lifetime escapes.
pub struct BusAudio<'a> {
    pub(crate) latency: usize,
    plugin: *const clap_plugin,
    pub(crate) _context: &'a HostContext,
    parameters: &'a [ParameterInfo],
    inputs: &'a [Port],
    outputs: &'a [Port],
    spec: ProcessSpec,
    storage: Storage,
    processing: bool,
    steady: i64,
    note_dialect: u32,
}
// SAFETY: exclusive instance lease moves only into a joined scope, with owned planar storage.
unsafe impl Send for BusAudio<'_> {}
impl BusAudio<'_> {
    fn stop(&mut self) {
        if self.processing {
            unsafe {
                (*self.plugin).stop_processing.expect("checked")(self.plugin);
            }
            self.processing = false;
        }
    }
    pub fn output_events(&self) -> &[OutputEvent] {
        &self.storage.out
    }
    pub fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: &[Event],
        parameters: &[ParameterEvent],
        transport: Transport,
    ) -> Result<(), Error> {
        let result = self.process_inner(inputs, outputs, events, parameters, transport);
        if result.is_err() {
            for b in outputs {
                b.fill(0.0);
            }
        }
        result
    }
    fn process_inner(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: &[Event],
        parameters: &[ParameterEvent],
        transport: Transport,
    ) -> Result<(), Error> {
        if inputs.len() != self.inputs.len() || outputs.len() != self.outputs.len() {
            return Err(Error::InvalidBuffer);
        }
        let frames = outputs[0].len() / self.outputs[0].channels;
        if frames == 0
            || frames > self.spec.max_block_frames
            || inputs
                .iter()
                .zip(self.inputs)
                .any(|(b, p)| b.len() != frames * p.channels)
            || outputs
                .iter()
                .zip(self.outputs)
                .any(|(b, p)| b.len() != frames * p.channels)
        {
            return Err(Error::InvalidBuffer);
        }
        validate_events(events, frames)?;
        if events
            .iter()
            .map(|e| {
                if matches!(e.message, MidiMessage::AllNotesOff)
                    && self.note_dialect != CLAP_NOTE_DIALECT_CLAP
                {
                    16
                } else {
                    1
                }
            })
            .sum::<usize>()
            + parameters.len()
            > LIMIT
            || (!events.is_empty() && self.note_dialect == 0)
            || !transport.tempo.is_finite()
            || !(20.0..=400.0).contains(&transport.tempo)
        {
            return Err(Error::InvalidEvents);
        }
        let mut previous = 0;
        for e in parameters {
            let p = self
                .parameters
                .iter()
                .find(|p| p.id == e.id)
                .ok_or(Error::InvalidParameter)?;
            if e.frame < previous
                || e.frame >= frames
                || !e.value.is_finite()
                || !(p.min..=p.max).contains(&e.value)
                || p.flags & CLAP_PARAM_IS_READONLY != 0
            {
                return Err(Error::InvalidParameter);
            }
            previous = e.frame;
        }
        let r = &mut self.storage;
        r.events.clear();
        r.pointers.clear();
        r.out.clear();
        for event in events {
            if matches!(event.message, MidiMessage::AllNotesOff)
                && self.note_dialect != CLAP_NOTE_DIALECT_CLAP
            {
                for channel in 0..16 {
                    r.events.push(native(
                        &Event {
                            frame: event.frame,
                            message: MidiMessage::ControlChange {
                                channel,
                                controller: 123,
                                value: 0,
                            },
                        },
                        self.note_dialect,
                    )?);
                }
            } else {
                r.events.push(native(event, self.note_dialect)?);
            }
        }
        for e in parameters {
            r.events.push(NativeEvent {
                param: clap_event_param_value {
                    header: header::<clap_event_param_value>(CLAP_EVENT_PARAM_VALUE, e.frame),
                    param_id: e.id,
                    cookie: ptr::null_mut(),
                    note_id: -1,
                    port_index: -1,
                    channel: -1,
                    key: -1,
                    value: e.value,
                },
            });
        }
        for event in &r.events {
            r.pointers.push((event as *const NativeEvent).cast());
        }
        for i in 1..r.pointers.len() {
            let mut j = i;
            while j > 0 && unsafe { (*r.pointers[j - 1]).time > (*r.pointers[j]).time } {
                r.pointers.swap(j - 1, j);
                j -= 1;
            }
        }
        for ((input, port), data) in inputs.iter().zip(self.inputs).zip(&mut r.input) {
            for (frame, samples) in input.chunks_exact(port.channels).enumerate() {
                for (channel, x) in samples.iter().enumerate() {
                    data.channels[channel][frame] = *x;
                }
            }
        }
        for port in &mut r.output {
            for channel in &mut port.channels {
                channel[..frames].fill(0.0);
            }
        }
        for b in &mut r.native_out {
            b.constant_mask = 0;
        }
        let ie = clap_input_events {
            ctx: (&r.pointers as *const Vec<*const clap_event_header>)
                .cast_mut()
                .cast(),
            size: Some(event_size),
            get: Some(event_get),
        };
        let mut context = OutputContext {
            events: &mut r.out,
            frames,
            dropped: &mut r.dropped,
        };
        let oe = clap_output_events {
            ctx: (&mut context as *mut OutputContext<'_>).cast(),
            try_push: Some(event_push),
        };
        let seconds = transport.sample_position as f64 / self.spec.sample_rate;
        let mut t: clap_event_transport = unsafe { std::mem::zeroed() };
        t.header = header::<clap_event_transport>(CLAP_EVENT_TRANSPORT, 0);
        t.flags = CLAP_TRANSPORT_HAS_TEMPO
            | CLAP_TRANSPORT_HAS_SECONDS_TIMELINE
            | CLAP_TRANSPORT_HAS_BEATS_TIMELINE
            | if transport.playing {
                CLAP_TRANSPORT_IS_PLAYING
            } else {
                0
            };
        t.tempo = transport.tempo;
        t.song_pos_beats = (seconds * transport.tempo / 60.0 * CLAP_BEATTIME_FACTOR as f64)
            .min(i64::MAX as f64) as i64;
        t.song_pos_seconds = (seconds * CLAP_SECTIME_FACTOR as f64).min(i64::MAX as f64) as i64;
        t.tsig_num = 4;
        t.tsig_denom = 4;
        let process = clap_process {
            steady_time: self.steady,
            frames_count: frames as u32,
            transport: &t,
            audio_inputs: if r.native_in.is_empty() {
                ptr::null()
            } else {
                r.native_in.as_ptr()
            },
            audio_outputs: r.native_out.as_mut_ptr(),
            audio_inputs_count: r.native_in.len() as u32,
            audio_outputs_count: r.native_out.len() as u32,
            in_events: &ie,
            out_events: &oe,
        };
        let status = unsafe { (*self.plugin).process.expect("checked")(self.plugin, &process) };
        if ![
            CLAP_PROCESS_CONTINUE,
            CLAP_PROCESS_CONTINUE_IF_NOT_QUIET,
            CLAP_PROCESS_TAIL,
            CLAP_PROCESS_SLEEP,
        ]
        .contains(&status)
        {
            return Err(Error::InvalidBuffer);
        }
        for ((output, port), data) in outputs.iter_mut().zip(self.outputs).zip(&r.output) {
            for (frame, samples) in output.chunks_exact_mut(port.channels).enumerate() {
                for (channel, x) in samples.iter_mut().enumerate() {
                    *x = data.channels[channel][frame];
                    if !x.is_finite() {
                        return Err(Error::InvalidBuffer);
                    }
                }
            }
        }
        self.steady = self.steady.saturating_add(frames as i64);
        Ok(())
    }
}
impl Drop for BusAudio<'_> {
    fn drop(&mut self) {
        self.stop();
    }
}
