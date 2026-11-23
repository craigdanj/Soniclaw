//! Prepared serial chains, sample-timed automation and offline rendering.
//! Insert soniclaw-graph for compensated acyclic routing; edits happen off-thread.
use soniclaw_core::*;
use std::sync::Arc;

pub const MAX_BLOCK_EVENTS: usize = 1024;

// Keep the timing and validation contracts identical for both sample types.
macro_rules! serial_engine {
    ($engine:ident, $render:ident, $sample:ty, $buffer:ident, $processor:ident) => {
        #[derive(Default)]
        pub struct $engine {
            processors: Vec<Box<dyn $processor>>,
            spec: Option<ProcessSpec>,
            transport: Transport,
            parameters: Vec<(u32, Arc<Parameter>)>,
            event_scratch: Vec<Event>,
            suspended: bool,
        }
        impl $engine {
            pub fn suspend(&mut self) {
                self.suspended = true;
                for processor in &mut self.processors {
                    processor.reset();
                }
            }
            pub fn resume(&mut self) {
                self.suspended = false;
            }
            /// Adding a processor invalidates preparation. Call prepare before processing.
            pub fn push(&mut self, processor: impl $processor + 'static) {
                self.processors.push(Box::new(processor));
                self.spec = None;
            }
            pub fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
                self.spec = None;
                spec.validate()?;
                self.event_scratch = vec![
                    Event {
                        frame: 0,
                        message: MidiMessage::AllNotesOff
                    };
                    MAX_BLOCK_EVENTS
                ];
                for p in &mut self.processors {
                    p.prepare(spec)?;
                }
                self.spec = Some(spec);
                self.transport.sample_position = 0;
                Ok(())
            }
            pub fn spec(&self) -> Option<ProcessSpec> {
                self.spec
            }
            pub fn transport(&self) -> Transport {
                self.transport
            }
            pub fn bind_parameter(
                &mut self,
                id: u32,
                parameter: Arc<Parameter>,
            ) -> Result<(), Error> {
                if self.parameters.iter().any(|(existing, _)| *existing == id) {
                    return Err(Error::InvalidParameter);
                }
                self.parameters.push((id, parameter));
                Ok(())
            }
            pub fn set_transport(&mut self, transport: Transport) -> Result<(), Error> {
                if !transport.tempo.is_finite() || transport.tempo <= 0.0 {
                    return Err(Error::InvalidParameter);
                }
                self.transport = transport;
                Ok(())
            }
            pub fn set_tempo(&mut self, bpm: f64) -> Result<(), Error> {
                if !bpm.is_finite() || bpm <= 0.0 {
                    return Err(Error::InvalidParameter);
                }
                self.transport.tempo = bpm;
                Ok(())
            }
            pub fn latency_frames(&self) -> usize {
                self.processors
                    .iter()
                    .fold(0usize, |sum, p| sum.saturating_add(p.latency_frames()))
            }
            pub fn reset(&mut self) {
                for p in &mut self.processors {
                    p.reset();
                }
                self.transport.sample_position = 0;
            }
            /// On validation failure, clear the output before returning. Input may contain
            /// audio (effects), or silence (synth). Events must be sorted by frame.
            pub fn process(&mut self, data: &mut [$sample], events: &[Event]) -> Result<(), Error> {
                let result = self.validate(data, events);
                if let Err(error) = result {
                    data.fill(0.0);
                    return Err(error);
                }
                if self.suspended {
                    data.fill(0.0);
                    return Ok(());
                }
                let spec = self.spec.ok_or(Error::NotPrepared)?;
                let mut buffer = $buffer::new(data, spec.channels)?;
                let ctx = ProcessContext {
                    events,
                    transport: self.transport,
                };
                for p in &mut self.processors {
                    p.process(&mut buffer, &ctx);
                }
                self.transport.sample_position = self
                    .transport
                    .sample_position
                    .saturating_add(buffer.frames() as u64);
                Ok(())
            }
            /// Split at exact parameter-event offsets. Each same-frame group is applied
            /// completely before any processor sees the next sample. No allocations.
            pub fn process_automated(
                &mut self,
                data: &mut [$sample],
                events: &[Event],
                automation: &[AutomationEvent],
            ) -> Result<(), Error> {
                if automation.is_empty() {
                    return self.process(data, events);
                }
                let check = (|| {
                    self.validate(data, events)?;
                    let spec = self.spec.ok_or(Error::NotPrepared)?;
                    if events.len() > MAX_BLOCK_EVENTS || automation.len() > MAX_BLOCK_EVENTS {
                        return Err(Error::InvalidEvents);
                    }
                    let mut previous = 0;
                    for e in automation {
                        if e.frame < previous
                            || e.frame >= data.len() / spec.channels
                            || !e.value.is_finite()
                        {
                            return Err(Error::InvalidEvents);
                        }
                        let p = self
                            .parameters
                            .iter()
                            .find(|(id, _)| *id == e.parameter)
                            .ok_or(Error::InvalidParameter)?;
                        if !(p.1.info().min..=p.1.info().max).contains(&e.value) {
                            return Err(Error::InvalidParameter);
                        }
                        previous = e.frame;
                    }
                    Ok(())
                })();
                if let Err(error) = check {
                    data.fill(0.0);
                    return Err(error);
                }
                if self.suspended {
                    data.fill(0.0);
                    return Ok(());
                }
                let spec = self.spec.ok_or(Error::NotPrepared)?;
                let frames = data.len() / spec.channels;
                let mut start = 0;
                let mut ai = 0;
                let mut mi = 0;
                while start < frames {
                    while ai < automation.len() && automation[ai].frame == start {
                        let event = automation[ai];
                        if let Some((_, p)) = self
                            .parameters
                            .iter()
                            .find(|(id, _)| *id == event.parameter)
                        {
                            let _ = p.set(event.value);
                        }
                        ai += 1;
                    }
                    let end = automation.get(ai).map_or(frames, |e| e.frame);
                    let mut count = 0;
                    while mi < events.len() && events[mi].frame < end {
                        self.event_scratch[count] = Event {
                            frame: events[mi].frame - start,
                            message: events[mi].message,
                        };
                        count += 1;
                        mi += 1;
                    }
                    let mut buffer = $buffer::new(
                        &mut data[start * spec.channels..end * spec.channels],
                        spec.channels,
                    )?;
                    let ctx = ProcessContext {
                        events: &self.event_scratch[..count],
                        transport: self.transport,
                    };
                    for processor in &mut self.processors {
                        processor.process(&mut buffer, &ctx);
                    }
                    self.transport.sample_position = self
                        .transport
                        .sample_position
                        .saturating_add((end - start) as u64);
                    start = end;
                }
                Ok(())
            }
            fn validate(&self, data: &[$sample], events: &[Event]) -> Result<(), Error> {
                let spec = self.spec.ok_or(Error::NotPrepared)?;
                if !data.len().is_multiple_of(spec.channels) {
                    return Err(Error::InvalidBuffer);
                }
                let frames = data.len() / spec.channels;
                if frames > spec.max_block_frames {
                    return Err(Error::BlockTooLarge);
                }
                validate_events(events, frames)
            }
        }

        /// Offline convenience. Allocates the complete result, never use in a callback.
        /// Event positions are relative to the start of the render (not current transport).
        /// Engine must already be prepared. Its current DSP state is preserved.
        pub fn $render(
            engine: &mut $engine,
            frames: usize,
            events: &[Event],
        ) -> Result<Vec<$sample>, Error> {
            let spec = engine.spec().ok_or(Error::NotPrepared)?;
            validate_events(events, frames)?;
            let samples = frames
                .checked_mul(spec.channels)
                .ok_or(Error::InvalidBuffer)?;
            let mut output = vec![0.0; samples];
            let mut local_events = Vec::with_capacity(events.len());
            let mut event_index = 0;
            for (block_index, block) in output
                .chunks_mut(spec.max_block_frames * spec.channels)
                .enumerate()
            {
                let start = block_index * spec.max_block_frames;
                let end = start + block.len() / spec.channels;
                local_events.clear();
                while event_index < events.len() && events[event_index].frame < end {
                    local_events.push(Event {
                        frame: events[event_index].frame - start,
                        message: events[event_index].message,
                    });
                    event_index += 1;
                }
                engine.process(block, &local_events)?;
            }
            Ok(output)
        }
    };
}

serial_engine!(Engine, render, f32, AudioBuffer, AudioProcessor);
serial_engine!(Engine64, render64, f64, AudioBuffer64, AudioProcessor64);

impl AudioProcessor64 for Engine64 {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        Engine64::prepare(self, spec)
    }
    fn process(&mut self, buffer: &mut AudioBuffer64<'_>, context: &ProcessContext<'_>) {
        if self
            .spec()
            .is_none_or(|spec| spec.channels != buffer.channels())
        {
            buffer.clear();
            return;
        }
        if self.set_transport(context.transport).is_err() {
            buffer.clear();
            return;
        }
        let _ = Engine64::process(self, buffer.samples_mut(), context.events);
    }
    fn reset(&mut self) {
        Engine64::reset(self);
    }
    fn latency_frames(&self) -> usize {
        Engine64::latency_frames(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Impulse;
    impl AudioProcessor for Impulse {
        fn prepare(&mut self, _: ProcessSpec) -> Result<(), Error> {
            Ok(())
        }
        fn process(&mut self, b: &mut AudioBuffer<'_>, c: &ProcessContext<'_>) {
            b.clear();
            let channels = b.channels();
            for e in c.events {
                b.samples_mut()[e.frame * channels] = 1.0;
            }
        }
        fn reset(&mut self) {}
    }
    #[test]
    fn offline_cross_block_events_and_tail() {
        let mut e = Engine::default();
        e.push(Impulse);
        e.prepare(ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 4,
            channels: 2,
        })
        .unwrap();
        let events: Vec<_> = [0, 3, 4, 8]
            .map(|frame| Event {
                frame,
                message: MidiMessage::AllNotesOff,
            })
            .into();
        let out = render(&mut e, 9, &events).unwrap();
        assert_eq!(out.len(), 18);
        for frame in [0, 3, 4, 8] {
            assert_eq!(out[frame * 2], 1.0);
        }
        assert_eq!(out.iter().sum::<f32>(), 4.0);
        assert_eq!(e.transport().sample_position, 9);
    }
    #[test]
    fn invalid_blocks_are_silent_and_do_not_advance() {
        let mut e = Engine::default();
        let mut data = [1.0; 8];
        assert_eq!(e.process(&mut data, &[]), Err(Error::NotPrepared));
        assert_eq!(data, [0.0; 8]);
        e.prepare(ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 2,
            channels: 2,
        })
        .unwrap();
        assert_eq!(e.process(&mut data, &[]), Err(Error::BlockTooLarge));
        assert_eq!(e.transport().sample_position, 0);
    }
}

#[cfg(test)]
mod automation_tests {
    use super::*;
    struct Scalar(Arc<Parameter>);
    impl AudioProcessor for Scalar {
        fn prepare(&mut self, _: ProcessSpec) -> Result<(), Error> {
            Ok(())
        }
        fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
            for s in b.samples_mut() {
                *s *= self.0.get();
            }
        }
        fn reset(&mut self) {}
    }
    #[test]
    fn automation_is_exact_and_invalid_batches_are_atomic() {
        let parameter = Arc::new(
            Parameter::new(ParameterInfo {
                id: "gain",
                name: "Gain",
                min: 0.0,
                max: 1.0,
                default: 1.0,
                unit: "",
            })
            .unwrap(),
        );
        let mut engine = Engine::default();
        engine.push(Scalar(parameter.clone()));
        engine.bind_parameter(7, parameter.clone()).unwrap();
        engine
            .prepare(ProcessSpec {
                sample_rate: 48000.0,
                channels: 1,
                max_block_frames: 8,
            })
            .unwrap();
        let mut data = [1.0; 8];
        let events = [
            AutomationEvent {
                frame: 2,
                parameter: 7,
                value: 0.5,
            },
            AutomationEvent {
                frame: 4,
                parameter: 7,
                value: 0.75,
            },
            AutomationEvent {
                frame: 4,
                parameter: 7,
                value: 0.25,
            },
        ];
        engine.process_automated(&mut data, &[], &events).unwrap();
        assert_eq!(data, [1.0, 1.0, 0.5, 0.5, 0.25, 0.25, 0.25, 0.25]);
        assert_eq!(engine.transport().sample_position, 8);
        let bad = [
            AutomationEvent {
                frame: 0,
                parameter: 7,
                value: 0.9,
            },
            AutomationEvent {
                frame: 4,
                parameter: 99,
                value: 0.5,
            },
        ];
        assert!(engine.process_automated(&mut data, &[], &bad).is_err());
        assert_eq!(data, [0.0; 8]);
        assert_eq!(parameter.get(), 0.25);
        assert_eq!(engine.transport().sample_position, 8);
    }
    #[test]
    fn midi_offsets_survive_parameter_splits() {
        struct Mark;
        impl AudioProcessor for Mark {
            fn prepare(&mut self, _: ProcessSpec) -> Result<(), Error> {
                Ok(())
            }
            fn reset(&mut self) {}
            fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
                b.clear();
                for event in ctx.events {
                    b.samples_mut()[event.frame] =
                        ctx.transport.sample_position as f32 + event.frame as f32 + 1.0;
                }
            }
        }
        let p = Arc::new(
            Parameter::new(ParameterInfo {
                id: "p",
                name: "p",
                min: 0.0,
                max: 1.0,
                default: 0.0,
                unit: "",
            })
            .unwrap(),
        );
        let mut e = Engine::default();
        e.push(Mark);
        e.bind_parameter(1, p).unwrap();
        e.prepare(ProcessSpec {
            sample_rate: 48000.0,
            channels: 1,
            max_block_frames: 8,
        })
        .unwrap();
        let midi = [1, 2, 6].map(|frame| Event {
            frame,
            message: MidiMessage::AllNotesOff,
        });
        let auto = [
            AutomationEvent {
                frame: 2,
                parameter: 1,
                value: 0.5,
            },
            AutomationEvent {
                frame: 5,
                parameter: 1,
                value: 0.9,
            },
        ];
        let mut data = [0.0; 8];
        e.process_automated(&mut data, &midi, &auto).unwrap();
        assert_eq!(data, [0.0, 2.0, 3.0, 0.0, 0.0, 0.0, 7.0, 0.0]);
    }
}

/// Offline MIDI and automation positions are absolute relative to this render.
pub fn render_automated(
    engine: &mut Engine,
    frames: usize,
    midi: &[Event],
    automation: &[AutomationEvent],
) -> Result<Vec<f32>, Error> {
    let spec = engine.spec().ok_or(Error::NotPrepared)?;
    validate_events(midi, frames)?;
    let mut previous = 0;
    for event in automation {
        if event.frame < previous || event.frame >= frames || !event.value.is_finite() {
            return Err(Error::InvalidEvents);
        }
        previous = event.frame;
    }
    let mut output = vec![
        0.0;
        frames
            .checked_mul(spec.channels)
            .ok_or(Error::InvalidBuffer)?
    ];
    let mut local_midi = Vec::with_capacity(midi.len());
    let mut local_auto = Vec::with_capacity(automation.len());
    let (mut mi, mut ai) = (0, 0);
    for (index, block) in output
        .chunks_mut(spec.channels * spec.max_block_frames)
        .enumerate()
    {
        let start = index * spec.max_block_frames;
        let end = start + block.len() / spec.channels;
        local_midi.clear();
        local_auto.clear();
        while mi < midi.len() && midi[mi].frame < end {
            local_midi.push(Event {
                frame: midi[mi].frame - start,
                message: midi[mi].message,
            });
            mi += 1;
        }
        while ai < automation.len() && automation[ai].frame < end {
            local_auto.push(AutomationEvent {
                frame: automation[ai].frame - start,
                parameter: automation[ai].parameter,
                value: automation[ai].value,
            });
            ai += 1;
        }
        engine.process_automated(block, &local_midi, &local_auto)?;
    }
    Ok(output)
}

#[cfg(test)]
mod precision_tests {
    use super::*;

    struct Controlled(Arc<Parameter>);
    impl AudioProcessor64 for Controlled {
        fn prepare(&mut self, _: ProcessSpec) -> Result<(), Error> {
            Ok(())
        }
        fn process(&mut self, audio: &mut AudioBuffer64<'_>, context: &ProcessContext<'_>) {
            let gain = f64::from(self.0.get());
            for (i, sample) in audio.samples_mut().iter_mut().enumerate() {
                if context.events.iter().any(|e| e.frame == i) {
                    *sample += f64::EPSILON;
                }
                *sample *= gain;
            }
        }
        fn reset(&mut self) {}
    }

    #[test]
    fn f64_audio_retains_bits_through_automation_and_rebased_events() {
        let gain = Arc::new(
            Parameter::new(ParameterInfo {
                id: "gain",
                name: "Gain",
                min: 0.,
                max: 1.,
                default: 1.,
                unit: "",
            })
            .unwrap(),
        );
        let mut engine = Engine64::default();
        engine.push(Controlled(gain.clone()));
        engine.bind_parameter(3, gain.clone()).unwrap();
        engine
            .prepare(ProcessSpec {
                sample_rate: 48000.,
                max_block_frames: 8,
                channels: 1,
            })
            .unwrap();
        let mut block = [1.; 4];
        engine
            .process_automated(
                &mut block,
                &[Event {
                    frame: 3,
                    message: MidiMessage::AllNotesOff,
                }],
                &[AutomationEvent {
                    frame: 2,
                    parameter: 3,
                    value: 0.5,
                }],
            )
            .unwrap();
        assert_eq!(block, [1., 1., 0.5, 0.5 + f64::EPSILON * 0.5]);
        assert_eq!(engine.transport().sample_position, 4);
        assert_eq!(
            engine.process_automated(
                &mut block,
                &[],
                &[AutomationEvent {
                    frame: 1,
                    parameter: 3,
                    value: 2.
                }]
            ),
            Err(Error::InvalidParameter)
        );
        assert_eq!(block, [0.; 4]);
        assert_eq!(engine.transport().sample_position, 4);
        assert_eq!(gain.get(), 0.5);
    }
}
