//! Native output and duplex adapters. Device configuration is negotiated before prepare.
//! F32/I16/U16 streams, channel mapping, drift bridge and recording taps.
pub mod output_map;
pub use output_map::OutputMap;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use soniclaw_core::*;
use soniclaw_engine::{Engine, Engine64};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    Arc,
};
mod duplex;
pub mod timing;
pub use duplex::*;
pub mod manager;
pub use manager::*;
#[cfg(feature = "ui")]
pub mod settings;

pub type DeviceError = Box<dyn std::error::Error>;
const EVENTS_PER_BLOCK: usize = 128;

#[derive(Default)]
pub struct Telemetry {
    pub input_clock: timing::InputClock,
    pub late_events: AtomicU64,
    peak: AtomicU32,
    input_peak: AtomicU32,
    pub input_underruns: AtomicU64,
    pub input_overflows: AtomicU64,
    pub process_errors: AtomicU64,
    pub stream_errors: AtomicU64,
    pub dropped_events: AtomicU64,
    panic: AtomicBool,
}
impl Telemetry {
    pub fn take_input_peak(&self) -> f32 {
        f32::from_bits(self.input_peak.swap(0, Ordering::Relaxed))
    }
    /// Peak accumulated since the UI's previous read, before output saturation.
    pub fn take_peak(&self) -> f32 {
        f32::from_bits(self.peak.swap(0, Ordering::Relaxed))
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Queued {
    sample: Option<u64>,
    message: MidiMessage,
}
pub struct EventSender {
    last_timed: Option<u64>,
    producer: rtrb::Producer<Queued>,
    telemetry: Arc<Telemetry>,
}
impl EventSender {
    /// False means queue full. Requests a global all-notes-off to prevent stuck
    /// notes; the failed event is not retried. One producer per queue.
    pub fn send(&mut self, message: MidiMessage) -> bool {
        if self
            .producer
            .push(Queued {
                sample: None,
                message,
            })
            .is_ok()
        {
            true
        } else {
            self.telemetry
                .dropped_events
                .fetch_add(1, Ordering::Relaxed);
            self.telemetry.panic.store(true, Ordering::Release);
            false
        }
    }
    /// Absolute device-acquisition sample, independent of arrangement seeks/loops.
    pub fn send_at(&mut self, sample: u64, message: MidiMessage) -> bool {
        if sample > 1 << 48 || self.last_timed.is_some_and(|last| sample < last) {
            self.telemetry
                .dropped_events
                .fetch_add(1, Ordering::Relaxed);
            self.all_notes_off();
            return false;
        }
        if self
            .producer
            .push(Queued {
                sample: Some(sample),
                message,
            })
            .is_ok()
        {
            self.last_timed = Some(sample);
            true
        } else {
            self.telemetry
                .dropped_events
                .fetch_add(1, Ordering::Relaxed);
            self.all_notes_off();
            false
        }
    }
    pub fn send_timed(&mut self, message: MidiMessage) -> bool {
        match self.telemetry.input_clock.now() {
            Some((sample, _)) => self.send_at(sample, message),
            None => self.send(message),
        }
    }
    pub fn all_notes_off(&self) {
        self.telemetry.panic.store(true, Ordering::Release);
    }
}
pub struct EventReceiver {
    pending: Option<Queued>,
    consumer: rtrb::Consumer<Queued>,
    telemetry: Arc<Telemetry>,
}
pub fn event_channel(capacity: usize, telemetry: Arc<Telemetry>) -> (EventSender, EventReceiver) {
    let (producer, consumer) = rtrb::RingBuffer::new(capacity.max(1));
    (
        EventSender {
            last_timed: None,
            producer,
            telemetry: telemetry.clone(),
        },
        EventReceiver {
            pending: None,
            consumer,
            telemetry,
        },
    )
}

pub struct OutputHost {
    _stream: cpal::Stream,
    pub spec: ProcessSpec,
    pub device_name: String,
    pub telemetry: Arc<Telemetry>,
}
impl OutputHost {
    pub fn devices() -> Result<Vec<String>, DeviceError> {
        cpal::default_host()
            .output_devices()?
            .map(|d| d.name().map_err(Into::into))
            .collect()
    }
    /// None selects the default output. Some(index) selects from devices().
    /// Receivers are fixed before streaming. Their telemetry must be the supplied
    /// telemetry instance so queue overflow/panic reaches this host.
    pub fn start(
        mut engine: Engine,
        device_index: Option<usize>,
        max_block_frames: usize,
        receivers: Vec<EventReceiver>,
        telemetry: Arc<Telemetry>,
    ) -> Result<Self, DeviceError> {
        Self::start_with(
            device_index,
            max_block_frames,
            receivers,
            telemetry,
            move |device, supported, spec, receivers, telemetry| {
                engine.prepare(spec)?;
                let config = supported.clone().into();
                Ok(match supported.sample_format() {
                    cpal::SampleFormat::F32 => {
                        build::<f32>(device, &config, engine, receivers, telemetry, spec)?
                    }
                    cpal::SampleFormat::F64 => {
                        build::<f64>(device, &config, engine, receivers, telemetry, spec)?
                    }
                    cpal::SampleFormat::I16 => {
                        build::<i16>(device, &config, engine, receivers, telemetry, spec)?
                    }
                    cpal::SampleFormat::U16 => {
                        build::<u16>(device, &config, engine, receivers, telemetry, spec)?
                    }
                    format => {
                        return Err(format!("unsupported output sample format: {format:?}").into())
                    }
                })
            },
        )
    }

    /// Use an f64 processing chain. The negotiated device type determines the final conversion.
    pub fn start64(
        mut engine: Engine64,
        device_index: Option<usize>,
        max_block_frames: usize,
        receivers: Vec<EventReceiver>,
        telemetry: Arc<Telemetry>,
    ) -> Result<Self, DeviceError> {
        Self::start_with(
            device_index,
            max_block_frames,
            receivers,
            telemetry,
            move |device, supported, spec, receivers, telemetry| {
                engine.prepare(spec)?;
                let config = supported.clone().into();
                let process =
                    move |data: &mut [f64], events: &[Event]| engine.process(data, events);
                Ok(match supported.sample_format() {
                    cpal::SampleFormat::F32 => build_output::<f32, f64>(
                        device, &config, process, receivers, telemetry, spec,
                    )?,
                    cpal::SampleFormat::F64 => build_output::<f64, f64>(
                        device, &config, process, receivers, telemetry, spec,
                    )?,
                    cpal::SampleFormat::I16 => build_output::<i16, f64>(
                        device, &config, process, receivers, telemetry, spec,
                    )?,
                    cpal::SampleFormat::U16 => build_output::<u16, f64>(
                        device, &config, process, receivers, telemetry, spec,
                    )?,
                    format => {
                        return Err(format!("unsupported output sample format: {format:?}").into())
                    }
                })
            },
        )
    }

    fn start_with(
        device_index: Option<usize>,
        max_block_frames: usize,
        receivers: Vec<EventReceiver>,
        telemetry: Arc<Telemetry>,
        start: impl FnOnce(
            &cpal::Device,
            &cpal::SupportedStreamConfig,
            ProcessSpec,
            Vec<EventReceiver>,
            Arc<Telemetry>,
        ) -> Result<cpal::Stream, DeviceError>,
    ) -> Result<Self, DeviceError> {
        if receivers.len() > 8 {
            return Err("at most eight event sources are supported".into());
        }
        if receivers
            .iter()
            .any(|r| !Arc::ptr_eq(&r.telemetry, &telemetry))
        {
            return Err("all event channels must share host telemetry".into());
        }
        let host = cpal::default_host();
        let device = if let Some(index) = device_index {
            host.output_devices()?
                .nth(index)
                .ok_or("output device index out of range")?
        } else {
            host.default_output_device()
                .ok_or("no default output device")?
        };
        let device_name = device.name()?;
        let supported = device.default_output_config()?;
        let spec = ProcessSpec {
            sample_rate: supported.sample_rate().0 as f64,
            channels: supported.channels() as usize,
            max_block_frames,
        };
        let stream = start(&device, &supported, spec, receivers, telemetry.clone())?;
        stream.play()?;
        Ok(Self {
            _stream: stream,
            spec,
            device_name,
            telemetry,
        })
    }
}

#[cfg(test)]
fn collect_events(
    receivers: &mut [EventReceiver],
    telemetry: &Telemetry,
    events: &mut [Event; EVENTS_PER_BLOCK],
) -> usize {
    collect_events_window(receivers, telemetry, events, 0, 1)
}
fn collect_events_window(
    receivers: &mut [EventReceiver],
    telemetry: &Telemetry,
    events: &mut [Event; EVENTS_PER_BLOCK],
    start: u64,
    frames: usize,
) -> usize {
    let mut count = 0;
    let budget = (EVENTS_PER_BLOCK - 1) / receivers.len().max(1);
    for receiver in receivers.iter_mut() {
        for _ in 0..budget {
            let item = match receiver
                .pending
                .take()
                .or_else(|| receiver.consumer.pop().ok())
            {
                Some(q) => q,
                None => break,
            };
            let frame = if let Some(sample) = item.sample {
                if sample >= start.saturating_add(frames as u64) {
                    receiver.pending = Some(item);
                    break;
                }
                if sample < start {
                    telemetry.late_events.fetch_add(1, Ordering::Relaxed);
                }
                sample.saturating_sub(start) as usize
            } else {
                0
            };
            events[count] = Event {
                frame,
                message: item.message,
            };
            count += 1;
        }
    }
    if telemetry.panic.swap(false, Ordering::AcqRel) {
        count = 0;
        for receiver in receivers.iter_mut() {
            receiver.pending = None;
            for _ in 0..receiver.consumer.slots() {
                let _ = receiver.consumer.pop();
            }
        }
        events[count] = Event {
            frame: 0,
            message: MidiMessage::AllNotesOff,
        };
        count += 1;
    }
    // Stable insertion sort on the fixed bounded block; equal-time input order is preserved without allocation.
    for i in 1..count {
        let mut j = i;
        while j > 0 && events[j].frame < events[j - 1].frame {
            events.swap(j, j - 1);
            j -= 1;
        }
    }
    count
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut engine: Engine,
    receivers: Vec<EventReceiver>,
    telemetry: Arc<Telemetry>,
    spec: ProcessSpec,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    build_output::<T, f32>(
        device,
        config,
        move |data, events| engine.process(data, events),
        receivers,
        telemetry,
        spec,
    )
}

trait DeviceSample: Default + Copy + Send + 'static {
    fn clean(self) -> Self;
    fn device_range(self) -> Self;
    fn peak(self) -> f32;
}
macro_rules! device_sample {
    ($t:ty) => {
        impl DeviceSample for $t {
            fn clean(self) -> Self {
                if self.is_finite() {
                    self
                } else {
                    0.
                }
            }
            fn device_range(self) -> Self {
                self.clamp(-1., 1.)
            }
            fn peak(self) -> f32 {
                self.abs() as f32
            }
        }
    };
}
device_sample!(f32);
device_sample!(f64);

fn build_output<T, S>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut process: impl FnMut(&mut [S], &[Event]) -> Result<(), Error> + Send + 'static,
    mut receivers: Vec<EventReceiver>,
    telemetry: Arc<Telemetry>,
    spec: ProcessSpec,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + cpal::FromSample<S>,
    S: DeviceSample,
{
    let mut scratch = vec![S::default(); spec.max_block_frames * spec.channels];
    let mut events = [Event {
        frame: 0,
        message: MidiMessage::AllNotesOff,
    }; EVENTS_PER_BLOCK];
    let stream_telemetry = telemetry.clone();
    let mut acquisition = 0u64;
    device.build_output_stream(
        config,
        move |output: &mut [T], _: &cpal::OutputCallbackInfo| {
            let mut peak = 0.0_f32;
            for chunk in output.chunks_mut(scratch.len()) {
                telemetry.input_clock.anchor(
                    acquisition,
                    spec.sample_rate as u32,
                    spec.max_block_frames,
                );
                let frames = chunk.len() / spec.channels;
                let count = collect_events_window(
                    &mut receivers,
                    &telemetry,
                    &mut events,
                    acquisition,
                    frames,
                );
                acquisition = acquisition.saturating_add(frames as u64);
                let audio = &mut scratch[..chunk.len()];
                audio.fill(S::default());
                if process(audio, &events[..count]).is_err() {
                    telemetry.process_errors.fetch_add(1, Ordering::Relaxed);
                }
                for (destination, &sample) in chunk.iter_mut().zip(audio.iter()) {
                    let value = sample.clean();
                    peak = peak.max(value.peak().min(f32::MAX));
                    *destination = T::from_sample(value.device_range());
                }
            }
            // All finite non-negative f32 bit patterns sort like their values.
            telemetry.peak.fetch_max(peak.to_bits(), Ordering::Relaxed);
        },
        move |_| {
            stream_telemetry
                .stream_errors
                .fetch_add(1, Ordering::Relaxed);
        },
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_output_duplex_event_drain_is_bounded_and_panic_clears_backlog() {
        let t = Arc::new(Telemetry::default());
        let (mut tx, rx) = event_channel(256, t.clone());
        for _ in 0..200 {
            assert!(tx.send(MidiMessage::NoteOn {
                channel: 0,
                note: 60,
                velocity: 100
            }));
        }
        let mut receivers = vec![rx];
        let mut events = [Event {
            frame: 0,
            message: MidiMessage::AllNotesOff,
        }; EVENTS_PER_BLOCK];
        let count = collect_events(&mut receivers, &t, &mut events);
        assert_eq!(count, 127);
        tx.all_notes_off();
        let count = collect_events(&mut receivers, &t, &mut events);
        assert!(count <= 128);
        assert_eq!(events[count - 1].message, MidiMessage::AllNotesOff);
        assert!(receivers[0].consumer.pop().is_err());
    }
    #[test]
    fn queue_overflow_requests_panic_without_overwriting() {
        let telemetry = Arc::new(Telemetry::default());
        let (mut tx, mut rx) = event_channel(1, telemetry.clone());
        assert!(tx.send(MidiMessage::AllNotesOff));
        assert!(!tx.send(MidiMessage::AllNotesOff));
        assert_eq!(telemetry.dropped_events.load(Ordering::Relaxed), 1);
        assert!(telemetry.panic.load(Ordering::Acquire));
        assert_eq!(rx.consumer.pop().unwrap().message, MidiMessage::AllNotesOff);
    }
}
#[cfg(test)]
mod timestamp_tests {
    use super::*;
    fn run(block: usize) -> Vec<(u64, MidiMessage)> {
        let stats = Arc::new(Telemetry::default());
        let (mut tx, rx) = event_channel(128, stats.clone());
        for (sample, note) in [(10, 60), (63, 61), (64, 62), (127, 63), (254, 64)] {
            assert!(tx.send_at(
                sample,
                MidiMessage::NoteOn {
                    channel: 0,
                    note,
                    velocity: 100
                }
            ));
        }
        let mut receivers = vec![rx];
        let mut events = [Event {
            frame: 0,
            message: MidiMessage::AllNotesOff,
        }; EVENTS_PER_BLOCK];
        let mut result = vec![];
        for start in (0..256).step_by(block) {
            let n = block.min(256 - start);
            let count = collect_events_window(&mut receivers, &stats, &mut events, start as u64, n);
            for e in &events[..count] {
                result.push((start as u64 + e.frame as u64, e.message));
            }
        }
        result
    }
    #[test]
    fn exact_offsets_and_future_retention_are_block_partition_invariant() {
        assert_eq!(run(64), run(127));
        assert_eq!(
            run(64).iter().map(|e| e.0).collect::<Vec<_>>(),
            vec![10, 63, 64, 127, 254]
        );
    }
    #[test]
    fn late_out_of_order_and_panic_never_restart_future_notes() {
        let stats = Arc::new(Telemetry::default());
        let (mut tx, rx) = event_channel(4, stats.clone());
        let mut receivers = vec![rx];
        let mut out = [Event {
            frame: 0,
            message: MidiMessage::AllNotesOff,
        }; EVENTS_PER_BLOCK];
        assert!(tx.send_at(
            10,
            MidiMessage::NoteOn {
                channel: 0,
                note: 60,
                velocity: 100
            }
        ));
        assert_eq!(
            collect_events_window(&mut receivers, &stats, &mut out, 20, 64),
            1
        );
        assert_eq!(out[0].frame, 0);
        assert_eq!(stats.late_events.load(Ordering::Relaxed), 1);
        assert!(tx.send_at(
            100,
            MidiMessage::NoteOn {
                channel: 0,
                note: 60,
                velocity: 100
            }
        ));
        assert!(!tx.send_at(99, MidiMessage::AllNotesOff));
        assert_eq!(
            collect_events_window(&mut receivers, &stats, &mut out, 84, 64),
            1
        );
        assert_eq!(out[0].message, MidiMessage::AllNotesOff);
        assert!(receivers[0].pending.is_none());
    }
}
