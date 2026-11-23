use super::*;
use cpal::{Sample, SampleFormat};
use soniclaw_record::RecordTap;

#[derive(Clone, Debug)]
pub struct SupportedConfig {
    pub channels: u16,
    pub min_rate: u32,
    pub max_rate: u32,
    pub buffer_range: Option<(u32, u32)>,
    pub format: String,
}
#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub name: String,
    pub configs: Vec<SupportedConfig>,
}
#[derive(Clone, Debug, Default)]
pub struct DeviceCatalog {
    pub inputs: Vec<DeviceInfo>,
    pub outputs: Vec<DeviceInfo>,
}
impl DeviceCatalog {
    pub fn enumerate() -> Result<Self, DeviceError> {
        fn info(device: cpal::Device, input: bool) -> Result<DeviceInfo, DeviceError> {
            let ranges: Vec<_> = if input {
                device.supported_input_configs()?.collect()
            } else {
                device.supported_output_configs()?.collect()
            };
            Ok(DeviceInfo {
                name: device.name()?,
                configs: ranges
                    .into_iter()
                    .map(|r| SupportedConfig {
                        channels: r.channels(),
                        min_rate: r.min_sample_rate().0,
                        max_rate: r.max_sample_rate().0,
                        buffer_range: match r.buffer_size() {
                            cpal::SupportedBufferSize::Range { min, max } => Some((*min, *max)),
                            _ => None,
                        },
                        format: format!("{:?}", r.sample_format()),
                    })
                    .collect(),
            })
        }
        let host = cpal::default_host();
        Ok(Self {
            inputs: host
                .input_devices()?
                .map(|d| info(d, true))
                .collect::<Result<_, _>>()?,
            outputs: host
                .output_devices()?
                .map(|d| info(d, false))
                .collect::<Result<_, _>>()?,
        })
    }
}
#[derive(Clone, Debug)]
pub struct DuplexOptions {
    pub output_bus: Option<Arc<ChannelBus>>,
    pub cue_outputs: Vec<[u16; 2]>,
    pub input_bus: Option<Arc<ChannelBus>>,
    pub host: Option<String>,
    pub input_occurrence: Option<u16>,
    pub output_occurrence: Option<u16>,
    pub output_hardware_channels: Option<u16>,
    /// None means input disabled. Output None selects the current OS default.
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub input_channels: [u16; 2],
    pub output_channels: [u16; 2],
    pub sample_rate: u32,
    pub buffer_frames: Option<u32>,
    pub max_internal_frames: usize,
}
impl Default for DuplexOptions {
    fn default() -> Self {
        Self {
            output_bus: None,
            cue_outputs: Vec::new(),
            input_bus: None,
            host: None,
            input_occurrence: None,
            output_occurrence: None,
            output_hardware_channels: None,
            input_device: None,
            output_device: None,
            input_channels: [0, 0],
            output_channels: [0, 1],
            sample_rate: 48000,
            buffer_frames: None,
            max_internal_frames: 256,
        }
    }
}
#[derive(Default)]
pub struct SessionControls {
    pub monitor: AtomicBool,
    pub bypass: AtomicBool,
}

/// Shared, bounded frame bridge with prefill and small adaptive rate correction.
/// Linear interpolation corrects normal device-clock drift, not differing rates.
struct InputReader {
    queue: rtrb::Consumer<[f32; 64]>,
    target: usize,
    ready: bool,
    a: [f32; 64],
    b: [f32; 64],
    fraction: f64,
}
impl InputReader {
    fn new(queue: rtrb::Consumer<[f32; 64]>, target: usize) -> Self {
        Self {
            queue,
            target,
            ready: false,
            a: [0.0; 64],
            b: [0.0; 64],
            fraction: 0.0,
        }
    }
    fn frame(&mut self, stats: &Telemetry) -> [f32; 64] {
        if !self.ready {
            if self.queue.slots() < self.target.max(2) {
                return [0.0; 64];
            }
            self.a = self.queue.pop().unwrap_or([0.0; 64]);
            self.b = self.queue.pop().unwrap_or([0.0; 64]);
            self.ready = true;
            self.fraction = 0.0;
        }
        let error = (self.queue.slots() as f64 - self.target as f64) / self.target.max(1) as f64;
        let step = 1.0 + (error * 0.001).clamp(-0.002, 0.002);
        let output =
            std::array::from_fn(|i| self.a[i] + (self.b[i] - self.a[i]) * self.fraction as f32);
        self.fraction += step;
        while self.fraction >= 1.0 {
            self.a = self.b;
            match self.queue.pop() {
                Ok(frame) => self.b = frame,
                Err(_) => {
                    self.ready = false;
                    stats.input_underruns.fetch_add(1, Ordering::Relaxed);
                    return [0.0; 64];
                }
            }
            self.fraction -= 1.0;
        }
        output
    }
}
pub struct DuplexHost {
    _input: Option<cpal::Stream>,
    _output: cpal::Stream,
    pub spec: ProcessSpec,
    pub input_name: Option<String>,
    pub output_name: String,
    pub telemetry: Arc<Telemetry>,
    pub controls: Arc<SessionControls>,
    /// Additional software prefill; excludes hardware and OS latency.
    pub bridge_prefill_frames: usize,
}
fn choose_device(
    host: &cpal::Host,
    name: Option<&str>,
    occurrence: Option<u16>,
    input: bool,
) -> Result<cpal::Device, DeviceError> {
    if let Some(name) = name {
        let devices = if input {
            host.input_devices()?
        } else {
            host.output_devices()?
        };
        let mut matches = devices.filter_map(|d| d.name().ok().filter(|n| n == name).map(|_| d));
        if let Some(n) = occurrence {
            return matches
                .nth(n as usize)
                .ok_or_else(|| "selected device occurrence unavailable".into());
        }
        let device = matches
            .next()
            .ok_or_else(|| -> DeviceError { format!("device unavailable: {name}").into() })?;
        if matches.next().is_some() {
            return Err("device name is ambiguous; choose a uniquely named device".into());
        }
        Ok(device)
    } else {
        (if input {
            host.default_input_device()
        } else {
            host.default_output_device()
        })
        .ok_or_else(|| "no default audio device".into())
    }
}
fn config(
    device: &cpal::Device,
    input: bool,
    map: [u16; 2],
    options: &DuplexOptions,
) -> Result<(cpal::StreamConfig, SampleFormat), DeviceError> {
    let ranges: Vec<_> = if input {
        device.supported_input_configs()?.collect()
    } else {
        device.supported_output_configs()?.collect()
    };
    let needed = if input {
        map[0].max(map[1])
    } else {
        map.iter()
            .chain(options.cue_outputs.iter().flatten())
            .copied()
            .max()
            .unwrap_or(1)
    }
    .checked_add(1)
    .ok_or("channel index overflow")?;
    let mut candidates: Vec<_> = ranges
        .into_iter()
        .filter(|r| {
            r.channels()
                >= needed.max(if input {
                    options
                        .input_bus
                        .as_ref()
                        .map_or(0, |b| b.channels() as u16)
                } else {
                    0
                })
                && (input
                    || options
                        .output_hardware_channels
                        .is_none_or(|n| r.channels() == n))
                && r.min_sample_rate().0 <= options.sample_rate
                && r.max_sample_rate().0 >= options.sample_rate
        })
        .filter(|r| {
            matches!(
                r.sample_format(),
                SampleFormat::F32 | SampleFormat::I16 | SampleFormat::U16
            )
        })
        .filter(|r| match (options.buffer_frames, r.buffer_size()) {
            (Some(size), cpal::SupportedBufferSize::Range { min, max }) => {
                size >= *min && size <= *max
            }
            _ => true,
        })
        .collect();
    candidates.sort_by_key(|r| {
        (
            r.channels(),
            if r.sample_format() == SampleFormat::F32 {
                0
            } else {
                1
            },
        )
    });
    let supported = (*candidates
        .first()
        .ok_or("requested rate/channel map/buffer is not supported by this device")?)
    .with_sample_rate(cpal::SampleRate(options.sample_rate));
    let mut config: cpal::StreamConfig = supported.clone().into();
    config.buffer_size = options
        .buffer_frames
        .map_or(cpal::BufferSize::Default, cpal::BufferSize::Fixed);
    Ok((config, supported.sample_format()))
}
impl DuplexHost {
    pub fn start(
        engine: Engine,
        options: &DuplexOptions,
        telemetry: Arc<Telemetry>,
        controls: Arc<SessionControls>,
        tap: Option<RecordTap>,
    ) -> Result<Self, DeviceError> {
        Self::start_with_events(engine, options, telemetry, controls, tap, vec![])
    }
    pub fn start_with_events(
        mut engine: Engine,
        options: &DuplexOptions,
        telemetry: Arc<Telemetry>,
        controls: Arc<SessionControls>,
        tap: Option<RecordTap>,
        receivers: Vec<EventReceiver>,
    ) -> Result<Self, DeviceError> {
        if receivers.len() > 8
            || receivers
                .iter()
                .any(|r| !Arc::ptr_eq(&r.telemetry, &telemetry))
        {
            return Err("invalid duplex event channels".into());
        }
        let host = super::manager::native_host(options.host.as_deref())?;
        let spec = ProcessSpec {
            sample_rate: options.sample_rate as f64,
            channels: 2,
            max_block_frames: options.max_internal_frames,
        };
        spec.validate()?;
        if options.input_channels.iter().any(|c| *c >= 64) {
            return Err("input channel index exceeds 63".into());
        }
        if options.output_channels[0] == options.output_channels[1] {
            return Err("output channel destinations must be distinct".into());
        }
        if options.buffer_frames.is_some_and(|n| n == 0 || n > 65536) {
            return Err("invalid requested buffer size".into());
        }
        let output = choose_device(
            &host,
            options.output_device.as_deref(),
            options.output_occurrence,
            false,
        )?;
        crate::OutputMap::new(64, options.output_channels, &options.cue_outputs)?;
        let (out_config, out_format) = config(&output, false, options.output_channels, options)?;
        let mapping = crate::OutputMap::new(
            out_config.channels as usize,
            options.output_channels,
            &options.cue_outputs,
        )?;
        if options.output_bus.as_ref().is_some_and(|b| {
            b.channels() != mapping.logical_channels() || b.maximum_frames() < spec.max_block_frames
        }) || (!options.cue_outputs.is_empty() && options.output_bus.is_none())
        {
            return Err("cue output map does not match the prepared output bank".into());
        }

        if options.input_bus.as_ref().is_some_and(|b| {
            b.maximum_frames() < spec.max_block_frames || options.input_device.is_none()
        }) {
            return Err("invalid multichannel input bus/device".into());
        }
        engine.prepare(spec)?;
        let prefill = options.buffer_frames.unwrap_or(512).clamp(64, 8192) as usize * 2;
        let (input, reader, input_name) = if let Some(name) = options.input_device.as_deref() {
            let input = choose_device(&host, Some(name), options.input_occurrence, true)?;
            let (configuration, format) = config(&input, true, options.input_channels, options)?;
            let (producer, consumer) = rtrb::RingBuffer::new(prefill * 8);
            let stream = match format {
                SampleFormat::F32 => {
                    capture::<f32>(&input, &configuration, producer, telemetry.clone())?
                }
                SampleFormat::I16 => {
                    capture::<i16>(&input, &configuration, producer, telemetry.clone())?
                }
                SampleFormat::U16 => {
                    capture::<u16>(&input, &configuration, producer, telemetry.clone())?
                }
                _ => return Err("unsupported input format".into()),
            };
            (
                Some(stream),
                Some(InputReader::new(consumer, prefill)),
                Some(input.name()?),
            )
        } else {
            (None, None, None)
        };
        let stream = match out_format {
            SampleFormat::F32 => play::<f32>(
                &output,
                &out_config,
                mapping,
                options.output_bus.clone(),
                engine,
                reader,
                telemetry.clone(),
                controls.clone(),
                tap,
                spec,
                receivers,
                options.input_bus.clone(),
                options.input_channels,
            )?,
            SampleFormat::I16 => play::<i16>(
                &output,
                &out_config,
                mapping,
                options.output_bus.clone(),
                engine,
                reader,
                telemetry.clone(),
                controls.clone(),
                tap,
                spec,
                receivers,
                options.input_bus.clone(),
                options.input_channels,
            )?,
            SampleFormat::U16 => play::<u16>(
                &output,
                &out_config,
                mapping,
                options.output_bus.clone(),
                engine,
                reader,
                telemetry.clone(),
                controls.clone(),
                tap,
                spec,
                receivers,
                options.input_bus.clone(),
                options.input_channels,
            )?,
            _ => return Err("unsupported output format".into()),
        };
        if let Some(input) = &input {
            input.play()?;
        }
        stream.play()?;
        Ok(Self {
            _input: input,
            _output: stream,
            spec,
            input_name,
            output_name: output.name()?,
            telemetry,
            controls,
            bridge_prefill_frames: if options.input_device.is_some() {
                prefill
            } else {
                0
            },
        })
    }
}
fn capture<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut producer: rtrb::Producer<[f32; 64]>,
    stats: Arc<Telemetry>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let channels = config.channels as usize;
    let errors = stats.clone();
    device.build_input_stream(
        config,
        move |input: &[T], _: &cpal::InputCallbackInfo| {
            let mut peak = 0.0_f32;
            for frame in input.chunks_exact(channels) {
                let mut pair = [0.; 64];
                for (d, s) in pair.iter_mut().zip(frame) {
                    *d = f32::from_sample(*s);
                }
                for v in &mut pair {
                    if !v.is_finite() {
                        *v = 0.0;
                    }
                    peak = peak.max(v.abs());
                }
                if producer.push(pair).is_err() {
                    stats.input_overflows.fetch_add(1, Ordering::Relaxed);
                }
            }
            stats
                .input_peak
                .fetch_max(peak.to_bits(), Ordering::Relaxed);
        },
        move |_| {
            errors.stream_errors.fetch_add(1, Ordering::Relaxed);
        },
        None,
    )
}
#[allow(clippy::too_many_arguments)]
fn play<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    map: crate::OutputMap,
    output_bus: Option<Arc<ChannelBus>>,
    mut engine: Engine,
    mut reader: Option<InputReader>,
    stats: Arc<Telemetry>,
    controls: Arc<SessionControls>,
    mut tap: Option<RecordTap>,
    spec: ProcessSpec,
    mut receivers: Vec<EventReceiver>,
    input_bus: Option<Arc<ChannelBus>>,
    input_map: [u16; 2],
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let hardware_channels = config.channels as usize;
    let errors = stats.clone();
    let mut dry = vec![0.0; spec.max_block_frames * 2];
    let mut wet = dry.clone();
    let bus_channels = input_bus.as_ref().map_or(0, |b| b.channels());
    let mut wide = vec![0.; spec.max_block_frames * bus_channels];
    let mut last_input_errors = 0;
    let mut acquisition = 0u64;
    let mut events = [Event {
        frame: 0,
        message: MidiMessage::AllNotesOff,
    }; EVENTS_PER_BLOCK];
    device.build_output_stream(
        config,
        move |output: &mut [T], _: &cpal::OutputCallbackInfo| {
            let mut peak = 0.0_f32;
            for block in output.chunks_mut(spec.max_block_frames * hardware_channels) {
                let frames = block.len() / hardware_channels;
                let count = frames * 2;
                let mut ready = true;
                for (f, pair) in dry[..count].as_chunks_mut::<2>().0.iter_mut().enumerate() {
                    let raw = reader.as_mut().map_or([0.; 64], |r| r.frame(&stats));
                    ready &= reader.as_ref().is_some_and(|r| r.ready);
                    *pair = [raw[input_map[0] as usize], raw[input_map[1] as usize]];
                    if bus_channels > 0 {
                        wide[f * bus_channels..(f + 1) * bus_channels]
                            .copy_from_slice(&raw[..bus_channels]);
                    }
                }
                if let Some(bus) = &input_bus {
                    let _ = bus.write(&wide[..frames * bus_channels]);
                    bus.set_ready(ready);
                    let errors = stats.input_underruns.load(Ordering::Relaxed)
                        + stats.input_overflows.load(Ordering::Relaxed);
                    if errors != last_input_errors {
                        bus.mark_discontinuity();
                    }
                    last_input_errors = errors;
                }
                wet[..count].copy_from_slice(&dry[..count]);
                stats.input_clock.anchor(
                    acquisition,
                    spec.sample_rate as u32,
                    spec.max_block_frames,
                );
                let event_count =
                    collect_events_window(&mut receivers, &stats, &mut events, acquisition, frames);
                acquisition = acquisition.saturating_add(frames as u64);
                if let Some(bus) = &output_bus {
                    bus.begin();
                }
                if engine
                    .process(&mut wet[..count], &events[..event_count])
                    .is_err()
                {
                    stats.process_errors.fetch_add(1, Ordering::Relaxed);
                }
                if controls.bypass.load(Ordering::Relaxed) {
                    wet[..count].copy_from_slice(&dry[..count]);
                }
                if let Some(tap) = &mut tap {
                    tap.capture_pair(&dry[..count], &wet[..count]);
                }
                let monitor = controls.monitor.load(Ordering::Relaxed);
                let mut routed = [0f32; 64];
                for (f, (destination, source)) in block
                    .chunks_exact_mut(hardware_channels)
                    .zip(wet[..count].as_chunks::<2>().0.iter())
                    .enumerate()
                {
                    let frame = &mut routed[..hardware_channels];
                    match map.frame(*source, output_bus.as_deref(), f, frame, monitor) {
                        Ok(p) => peak = peak.max(p),
                        Err(_) => {
                            frame.fill(0.);
                            stats.process_errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    for (out, sample) in destination.iter_mut().zip(frame) {
                        *out = T::from_sample(*sample);
                    }
                }
            }
            stats.peak.fetch_max(peak.to_bits(), Ordering::Relaxed);
        },
        move |_| {
            errors.stream_errors.fetch_add(1, Ordering::Relaxed);
        },
        None,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bridge_prefill_and_underrun_recover_without_panicking() {
        let (mut tx, rx) = rtrb::RingBuffer::new(64);
        let stats = Telemetry::default();
        let mut reader = InputReader::new(rx, 8);
        assert_eq!(reader.frame(&stats), [0.0; 64]);
        for _ in 0..32 {
            let mut frame = [0.; 64];
            frame[..2].copy_from_slice(&[0.5, -0.5]);
            tx.push(frame).unwrap();
        }
        for _ in 0..10 {
            let frame = reader.frame(&stats);
            assert_eq!(&frame[..2], &[0.5, -0.5]);
        }
        for _ in 0..100 {
            reader.frame(&stats);
        }
        assert!(stats.input_underruns.load(Ordering::Relaxed) > 0);
        for _ in 0..32 {
            tx.push([0.25; 64]).unwrap();
        }
        assert_eq!(reader.frame(&stats), [0.25; 64]);
    }
}
#[cfg(test)]
mod drift_tests {
    use super::*;
    #[test]
    fn adaptive_bridge_tracks_a_slightly_faster_input_clock() {
        let (mut tx, rx) = rtrb::RingBuffer::new(128);
        let stats = Telemetry::default();
        let mut reader = InputReader::new(rx, 16);
        for _ in 0..32 {
            tx.push([0.25; 64]).unwrap();
        }
        for n in 0..100000 {
            tx.push([0.25; 64]).unwrap();
            if n % 1000 == 0 {
                tx.push([0.25; 64]).unwrap();
            }
            assert_eq!(reader.frame(&stats), [0.25; 64]);
        }
        assert_eq!(stats.input_underruns.load(Ordering::Relaxed), 0);
        assert!(reader.queue.slots() < 64);
    }
}
