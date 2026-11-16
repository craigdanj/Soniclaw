//! Native output and duplex adapters. Device configuration is negotiated before prepare.
//! F32/I16/U16 streams, channel mapping, drift bridge and recording taps.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use soniclaw_core::*;
use soniclaw_engine::Engine;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    Arc,
};
mod duplex;
pub use duplex::*;

pub type DeviceError = Box<dyn std::error::Error>;
const EVENTS_PER_BLOCK: usize = 128;

#[derive(Default)]
pub struct Telemetry {
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
pub struct EventSender {
    producer: rtrb::Producer<MidiMessage>,
    telemetry: Arc<Telemetry>,
}
impl EventSender {
    /// False means queue full. Requests a global all-notes-off to prevent stuck
    /// notes; the failed event is not retried. One producer per queue.
    pub fn send(&mut self, message: MidiMessage) -> bool {
        if self.producer.push(message).is_ok() {
            true
        } else {
            self.telemetry
                .dropped_events
                .fetch_add(1, Ordering::Relaxed);
            self.telemetry.panic.store(true, Ordering::Release);
            false
        }
    }
    pub fn all_notes_off(&self) {
        self.telemetry.panic.store(true, Ordering::Release);
    }
}
pub struct EventReceiver {
    consumer: rtrb::Consumer<MidiMessage>,
    telemetry: Arc<Telemetry>,
}
pub fn event_channel(capacity: usize, telemetry: Arc<Telemetry>) -> (EventSender, EventReceiver) {
    let (producer, consumer) = rtrb::RingBuffer::new(capacity.max(1));
    (
        EventSender {
            producer,
            telemetry: telemetry.clone(),
        },
        EventReceiver {
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
        engine.prepare(spec)?;
        let config: cpal::StreamConfig = supported.clone().into();
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => {
                build::<f32>(&device, &config, engine, receivers, telemetry.clone(), spec)?
            }
            cpal::SampleFormat::I16 => {
                build::<i16>(&device, &config, engine, receivers, telemetry.clone(), spec)?
            }
            cpal::SampleFormat::U16 => {
                build::<u16>(&device, &config, engine, receivers, telemetry.clone(), spec)?
            }
            format => return Err(format!("unsupported output sample format: {format:?}").into()),
        };
        stream.play()?;
        Ok(Self {
            _stream: stream,
            spec,
            device_name,
            telemetry,
        })
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut engine: Engine,
    mut receivers: Vec<EventReceiver>,
    telemetry: Arc<Telemetry>,
    spec: ProcessSpec,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let mut scratch = vec![0.0; spec.max_block_frames * spec.channels];
    let mut events = [Event {
        frame: 0,
        message: MidiMessage::AllNotesOff,
    }; EVENTS_PER_BLOCK];
    let stream_telemetry = telemetry.clone();
    device.build_output_stream(
        config,
        move |output: &mut [T], _: &cpal::OutputCallbackInfo| {
            let mut peak = 0.0_f32;
            for chunk in output.chunks_mut(scratch.len()) {
                let mut count = 0;
                // Bounded drain: reserve one slot for panic and divide fairly.
                let budget = (EVENTS_PER_BLOCK - 1) / receivers.len().max(1);
                for receiver in &mut receivers {
                    for _ in 0..budget {
                        match receiver.consumer.pop() {
                            Ok(message) => {
                                events[count] = Event { frame: 0, message };
                                count += 1;
                            }
                            Err(_) => break,
                        }
                    }
                }
                if telemetry.panic.swap(false, Ordering::AcqRel) {
                    // Panic after all dequeued events. Also discard a bounded queue
                    // snapshot so stale note-ons do not immediately restart voices.
                    for receiver in &mut receivers {
                        let queued = receiver.consumer.slots();
                        for _ in 0..queued {
                            let _ = receiver.consumer.pop();
                        }
                    }
                    events[count] = Event {
                        frame: 0,
                        message: MidiMessage::AllNotesOff,
                    };
                    count += 1;
                }
                let audio = &mut scratch[..chunk.len()];
                audio.fill(0.0);
                if engine.process(audio, &events[..count]).is_err() {
                    telemetry.process_errors.fetch_add(1, Ordering::Relaxed);
                }
                for (destination, &sample) in chunk.iter_mut().zip(audio.iter()) {
                    let value = if sample.is_finite() { sample } else { 0.0 };
                    peak = peak.max(value.abs());
                    *destination = T::from_sample(value.clamp(-1.0, 1.0));
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
    fn queue_overflow_requests_panic_without_overwriting() {
        let telemetry = Arc::new(Telemetry::default());
        let (mut tx, mut rx) = event_channel(1, telemetry.clone());
        assert!(tx.send(MidiMessage::AllNotesOff));
        assert!(!tx.send(MidiMessage::AllNotesOff));
        assert_eq!(telemetry.dropped_events.load(Ordering::Relaxed), 1);
        assert!(telemetry.panic.load(Ordering::Acquire));
        assert_eq!(rx.consumer.pop().unwrap(), MidiMessage::AllNotesOff);
    }
}
