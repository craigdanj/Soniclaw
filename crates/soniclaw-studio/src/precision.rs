//! A prepared f64 render path for stereo sessions with resident audio clips.
use super::{AudioClip, Descriptor, Document};
use soniclaw_core::{AudioBuffer64, AudioProcessor64, Error, ProcessContext, ProcessSpec};
use soniclaw_graph::Graph64;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone)]
pub struct Asset64 {
    channels: usize,
    sample_rate: u32,
    samples: Arc<[f64]>,
}
impl Asset64 {
    pub fn new(sample_rate: u32, channels: usize, samples: Arc<[f64]>) -> Result<Self, Error> {
        if sample_rate == 0
            || !(1..=2).contains(&channels)
            || !samples.len().is_multiple_of(channels)
            || samples.iter().any(|sample| !sample.is_finite())
        {
            return Err(Error::InvalidBuffer);
        }
        Ok(Self {
            sample_rate,
            channels,
            samples,
        })
    }
}

struct ClipSource(Vec<(AudioClip, Asset64)>);
impl AudioProcessor64 for ClipSource {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        if spec.channels == 2 {
            Ok(())
        } else {
            Err(Error::InvalidSpec)
        }
    }
    fn process(&mut self, audio: &mut AudioBuffer64<'_>, context: &ProcessContext<'_>) {
        if !context.transport.playing {
            return;
        }
        let start = context.transport.sample_position;
        let end = start.saturating_add(audio.frames() as u64);
        let output = audio.samples_mut().as_chunks_mut::<2>().0;
        for (clip, asset) in &self.0 {
            let a = start.max(clip.start_frame);
            let b = end.min(clip.start_frame + clip.length_frames);
            for sample in a..b {
                let local = sample - clip.start_frame;
                let phase = clip.source_phase() + local;
                let position = if clip.looping {
                    phase % clip.source_length
                } else {
                    phase
                };
                if position >= clip.source_length {
                    continue;
                }
                let gain = f64::from(clip.region_gain(local));
                let source = (clip.source_offset + position) as usize * asset.channels;
                let target = &mut output[(sample - start) as usize];
                target[0] += asset.samples[source] * gain;
                target[1] += asset.samples[source + asset.channels - 1] * gain;
            }
        }
    }
    fn reset(&mut self) {}
}

struct StereoGain([f64; 2]);
impl AudioProcessor64 for StereoGain {
    fn prepare(&mut self, _: ProcessSpec) -> Result<(), Error> {
        Ok(())
    }
    fn process(&mut self, audio: &mut AudioBuffer64<'_>, _: &ProcessContext<'_>) {
        for [l, r] in audio.samples_mut().as_chunks_mut::<2>().0 {
            *l *= self.0[0];
            *r *= self.0[1];
        }
    }
    fn reset(&mut self) {}
}
struct Insert(Box<dyn AudioProcessor64>);
impl AudioProcessor64 for Insert {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        self.0.prepare(spec)
    }
    fn process(&mut self, audio: &mut AudioBuffer64<'_>, context: &ProcessContext<'_>) {
        self.0.process(audio, context);
    }
    fn reset(&mut self) {
        self.0.reset();
    }
    fn latency_frames(&self) -> usize {
        self.0.latency_frames()
    }
}

/// Prepared clip playback, inserts, stereo strip controls and compensated mix.
/// The caller owns transport and supplies zero or live stereo input per block.
pub struct PreparedSession64 {
    graph: Graph64,
    spec: ProcessSpec,
    expected_position: Option<u64>,
}
impl PreparedSession64 {
    /// Compile the supported document subset. An unknown insert or unavailable
    /// asset has to fail here, before DSP is installed or any audio is rendered.
    pub fn new(
        doc: &Document,
        assets: &BTreeMap<String, Asset64>,
        mut insert: impl FnMut(&Descriptor) -> Result<Box<dyn AudioProcessor64>, Error>,
    ) -> Result<Self, Error> {
        doc.validate()?;
        if !doc.cues.is_empty()
            || !doc.mix_routing.is_empty()
            || !doc.auxiliaries.is_empty()
            || !doc.automation.lanes.is_empty()
            || doc.automation.loop_range.is_some()
            || !doc.parameter_values.is_empty()
            || doc.timeline.loop_range.is_some()
            || doc.timeline.punch_range.is_some()
            || doc.tracks.iter().any(|t| {
                t.midi_input
                    || !t.midi.is_empty()
                    || t.instrument.is_some()
                    || !t.streamed.is_empty()
            })
        {
            return Err(Error::InvalidRouting);
        }
        let mut graph = Graph64::default();
        let solos = doc.mixer.strips.iter().any(|t| t.solo);
        for (track, strip) in doc.tracks.iter().zip(&doc.mixer.strips) {
            let mut clips = Vec::new();
            for clip in track.audio.iter().filter(|c| c.enabled) {
                let asset = assets.get(&clip.asset).ok_or(Error::InvalidBuffer)?;
                if asset.sample_rate != doc.sample_rate
                    || clip
                        .source_offset
                        .checked_add(clip.source_length)
                        .is_none_or(|end| end > (asset.samples.len() / asset.channels) as u64)
                {
                    return Err(Error::InvalidBuffer);
                }
                clips.push((clip.clone(), asset.clone()));
            }
            let mut last = graph.add(ClipSource(clips));
            graph.set_input(last, f64::from(track.input_gain))?;
            for descriptor in &track.inserts {
                let next = graph.add(Insert(insert(descriptor)?));
                graph.connect(last, next, 1.)?;
                last = next;
            }
            let group = strip.group.map(|i| &doc.mixer.groups[i]);
            let gain = if strip.mute || (solos && !strip.solo) || group.is_some_and(|g| g.mute) {
                0.
            } else {
                f64::from(strip.gain) * group.map_or(1., |g| f64::from(g.gain))
            };
            let pan = (f64::from(strip.pan) + 1.) * std::f64::consts::FRAC_PI_4;
            let bus = graph.add(StereoGain([pan.cos() * gain, pan.sin() * gain]));
            graph.connect(last, bus, 1.)?;
            graph.set_output(bus, 1.)?;
        }
        let spec = doc.spec();
        graph.prepare(spec)?;
        Ok(Self {
            graph,
            spec,
            expected_position: None,
        })
    }

    pub fn render_block(
        &mut self,
        audio: &mut AudioBuffer64<'_>,
        context: &ProcessContext<'_>,
    ) -> Result<(), Error> {
        if audio.channels() != 2
            || audio.frames() > self.spec.max_block_frames
            || context.transport.sample_position > 1 << 48
        {
            audio.clear();
            return Err(Error::InvalidBuffer);
        }
        let start = context.transport.sample_position;
        if self.expected_position != Some(start) {
            self.graph.reset();
        }
        self.graph.process(audio, context);
        if audio.samples().iter().any(|sample| !sample.is_finite()) {
            audio.clear();
            self.reset();
            return Err(Error::InvalidBuffer);
        }
        self.expected_position = Some(
            start
                + if context.transport.playing {
                    audio.frames() as u64
                } else {
                    0
                },
        );
        Ok(())
    }
    pub fn reset(&mut self) {
        self.graph.reset();
        self.expected_position = None;
    }
    pub fn latency_frames(&self) -> usize {
        self.graph.latency_frames()
    }
}
impl AudioProcessor64 for PreparedSession64 {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        spec.validate()?;
        if spec.channels != self.spec.channels
            || spec.sample_rate != self.spec.sample_rate
            || spec.max_block_frames > self.spec.max_block_frames
        {
            return Err(Error::InvalidSpec);
        }
        self.reset();
        Ok(())
    }
    fn process(&mut self, audio: &mut AudioBuffer64<'_>, context: &ProcessContext<'_>) {
        let _ = self.render_block(audio, context);
    }
    fn reset(&mut self) {
        Self::reset(self);
    }
    fn latency_frames(&self) -> usize {
        Self::latency_frames(self)
    }
}
