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
    queue: rtrb::Consumer<[f32; 2]>,
    target: usize,
    ready: bool,
    a: [f32; 2],
    b: [f32; 2],
    fraction: f64,
}
impl InputReader {
    fn new(queue: rtrb::Consumer<[f32; 2]>, target: usize) -> Self {
        Self {
            queue,
            target,
            ready: false,
            a: [0.0; 2],
            b: [0.0; 2],
            fraction: 0.0,
        }
    }
    fn frame(&mut self, stats: &Telemetry) -> [f32; 2] {
        if !self.ready {
            if self.queue.slots() < self.target.max(2) {
                return [0.0; 2];
            }
            self.a = self.queue.pop().unwrap_or([0.0; 2]);
            self.b = self.queue.pop().unwrap_or([0.0; 2]);
            self.ready = true;
            self.fraction = 0.0;
        }
        let error = (self.queue.slots() as f64 - self.target as f64) / self.target.max(1) as f64;
        let step = 1.0 + (error * 0.001).clamp(-0.002, 0.002);
        let output = [
            self.a[0] + (self.b[0] - self.a[0]) * self.fraction as f32,
            self.a[1] + (self.b[1] - self.a[1]) * self.fraction as f32,
        ];
        self.fraction += step;
        while self.fraction >= 1.0 {
            self.a = self.b;
            match self.queue.pop() {
                Ok(frame) => self.b = frame,
                Err(_) => {
                    self.ready = false;
                    stats.input_underruns.fetch_add(1, Ordering::Relaxed);
                    return [0.0; 2];
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
fn choose_device(name: Option<&str>, input: bool) -> Result<cpal::Device, DeviceError> {
    let host = cpal::default_host();
    if let Some(name) = name {
        let devices = if input {
            host.input_devices()?
        } else {
            host.output_devices()?
        };
        let mut matches = devices.filter_map(|d| d.name().ok().filter(|n| n == name).map(|_| d));
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
    let needed = map[0]
        .max(map[1])
        .checked_add(1)
        .ok_or("channel index overflow")?;
    let mut candidates: Vec<_> = ranges
        .into_iter()
        .filter(|r| {
            r.channels() >= needed
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
        mut engine: Engine,
        options: &DuplexOptions,
        telemetry: Arc<Telemetry>,
        controls: Arc<SessionControls>,
        tap: Option<RecordTap>,
    ) -> Result<Self, DeviceError> {
        let spec = ProcessSpec {
            sample_rate: options.sample_rate as f64,
            channels: 2,
            max_block_frames: options.max_internal_frames,
        };
        spec.validate()?;
        if options.output_channels[0] == options.output_channels[1] {
            return Err("output channel destinations must be distinct".into());
        }
        if options.buffer_frames.is_some_and(|n| n == 0 || n > 65536) {
            return Err("invalid requested buffer size".into());
        }
        let output = choose_device(options.output_device.as_deref(), false)?;
        let (out_config, out_format) = config(&output, false, options.output_channels, options)?;
        engine.prepare(spec)?;
        let prefill = options.buffer_frames.unwrap_or(512).clamp(64, 8192) as usize * 2;
        let (input, reader, input_name) = if let Some(name) = options.input_device.as_deref() {
            let input = choose_device(Some(name), true)?;
            let (configuration, format) = config(&input, true, options.input_channels, options)?;
            let (producer, consumer) = rtrb::RingBuffer::new(prefill * 8);
            let stream = match format {
                SampleFormat::F32 => capture::<f32>(
                    &input,
                    &configuration,
                    options.input_channels,
                    producer,
                    telemetry.clone(),
                )?,
                SampleFormat::I16 => capture::<i16>(
                    &input,
                    &configuration,
                    options.input_channels,
                    producer,
                    telemetry.clone(),
                )?,
                SampleFormat::U16 => capture::<u16>(
                    &input,
                    &configuration,
                    options.input_channels,
                    producer,
                    telemetry.clone(),
                )?,
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
                options.output_channels,
                engine,
                reader,
                telemetry.clone(),
                controls.clone(),
                tap,
                spec,
            )?,
            SampleFormat::I16 => play::<i16>(
                &output,
                &out_config,
                options.output_channels,
                engine,
                reader,
                telemetry.clone(),
                controls.clone(),
                tap,
                spec,
            )?,
            SampleFormat::U16 => play::<u16>(
                &output,
                &out_config,
                options.output_channels,
                engine,
                reader,
                telemetry.clone(),
                controls.clone(),
                tap,
                spec,
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
    map: [u16; 2],
    mut producer: rtrb::Producer<[f32; 2]>,
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
                let mut pair = [
                    f32::from_sample(frame[map[0] as usize]),
                    f32::from_sample(frame[map[1] as usize]),
                ];
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
    map: [u16; 2],
    mut engine: Engine,
    mut reader: Option<InputReader>,
    stats: Arc<Telemetry>,
    controls: Arc<SessionControls>,
    mut tap: Option<RecordTap>,
    spec: ProcessSpec,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let hardware_channels = config.channels as usize;
    let errors = stats.clone();
    let mut dry = vec![0.0; spec.max_block_frames * 2];
    let mut wet = dry.clone();
    device.build_output_stream(
        config,
        move |output: &mut [T], _: &cpal::OutputCallbackInfo| {
            let mut peak = 0.0_f32;
            for block in output.chunks_mut(spec.max_block_frames * hardware_channels) {
                let frames = block.len() / hardware_channels;
                let count = frames * 2;
                for pair in dry[..count].as_chunks_mut::<2>().0 {
                    pair.copy_from_slice(&reader.as_mut().map_or([0.0; 2], |r| r.frame(&stats)));
                }
                wet[..count].copy_from_slice(&dry[..count]);
                if !controls.bypass.load(Ordering::Relaxed)
                    && engine.process(&mut wet[..count], &[]).is_err()
                {
                    stats.process_errors.fetch_add(1, Ordering::Relaxed);
                }
                if let Some(tap) = &mut tap {
                    tap.capture_pair(&dry[..count], &wet[..count]);
                }
                let monitor = controls.monitor.load(Ordering::Relaxed);
                for (destination, source) in block
                    .chunks_exact_mut(hardware_channels)
                    .zip(wet[..count].as_chunks::<2>().0.iter())
                {
                    destination.fill(T::from_sample(0.0));
                    for channel in 0..2 {
                        let value = if source[channel].is_finite() {
                            source[channel]
                        } else {
                            0.0
                        };
                        peak = peak.max(value.abs());
                        destination[map[channel] as usize] =
                            T::from_sample(if monitor { value.clamp(-1.0, 1.0) } else { 0.0 });
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
        assert_eq!(reader.frame(&stats), [0.0; 2]);
        for _ in 0..32 {
            tx.push([0.5, -0.5]).unwrap();
        }
        for _ in 0..10 {
            assert_eq!(reader.frame(&stats), [0.5, -0.5]);
        }
        for _ in 0..100 {
            reader.frame(&stats);
        }
        assert!(stats.input_underruns.load(Ordering::Relaxed) > 0);
        for _ in 0..32 {
            tx.push([0.25; 2]).unwrap();
        }
        assert_eq!(reader.frame(&stats), [0.25; 2]);
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
            tx.push([0.25; 2]).unwrap();
        }
        for n in 0..100000 {
            tx.push([0.25; 2]).unwrap();
            if n % 1000 == 0 {
                tx.push([0.25; 2]).unwrap();
            }
            assert_eq!(reader.frame(&stats), [0.25; 2]);
        }
        assert_eq!(stats.input_underruns.load(Ordering::Relaxed), 0);
        assert!(reader.queue.slots() < 64);
    }
}
