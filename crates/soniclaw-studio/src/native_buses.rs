//! Typed native audio-bus requests, dependency edges and prepared sample-clock exchanges.
use crate::*;
use soniclaw_bus::flex::Layout;
use std::sync::atomic::{AtomicU32, AtomicUsize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub layout: Layout,
    #[serde(default)]
    pub sidechains: BTreeMap<usize, String>,
}
impl Request {
    pub fn from_descriptor(d: &Descriptor) -> Result<Option<Self>, Error> {
        if !matches!(d.kind.as_str(), "vst3" | "clap") {
            return Ok(None);
        }
        d.state
            .get("buses")
            .map(|v| serde_json::from_value(v.clone()).map_err(|_| Error::InvalidRouting))
            .transpose()
    }
    pub fn store(&self, d: &mut Descriptor) -> Result<(), Error> {
        self.validate()?;
        d.state["buses"] = serde_json::to_value(self).map_err(|_| Error::InvalidRouting)?;
        Ok(())
    }
    pub fn validate(&self) -> Result<(), Error> {
        self.layout.validate()?;
        if self.layout.outputs[0].channels > 2
            || self.layout.inputs.first().is_some_and(|b| b.channels > 2)
            || self.sidechains.iter().any(|(i, id)| {
                *i == 0
                    || *i >= self.layout.inputs.len()
                    || self.layout.inputs[*i].channels > 2
                    || !valid_id(id)
            })
        {
            return Err(Error::InvalidRouting);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Return {
    pub processor: String,
    pub bus: usize,
    pub channels: Vec<usize>,
}
impl Return {
    pub fn descriptor(&self, id: &str) -> Result<Descriptor, Error> {
        if !valid_id(id) || !valid_id(&self.processor) || !(1..=2).contains(&self.channels.len()) {
            return Err(Error::InvalidRouting);
        }
        Ok(Descriptor {
            id: id.into(),
            kind: "native.return".into(),
            state: serde_json::to_value(self).map_err(|_| Error::InvalidRouting)?,
        })
    }
}
/// Sequential single-writer/readers exchange. Tags prevent stale audio across arbitrary callback partitions.
pub struct Frames {
    channels: usize,
    maximum: usize,
    data: Vec<AtomicU32>,
    tags: Vec<AtomicU64>,
    pub latency: AtomicUsize,
}
impl Frames {
    pub fn new(channels: usize, maximum: usize) -> Result<Self, Error> {
        if !(1..=64).contains(&channels) || !(1..=8192).contains(&maximum) {
            return Err(Error::InvalidSpec);
        }
        Ok(Self {
            channels,
            maximum,
            data: (0..channels * maximum).map(|_| AtomicU32::new(0)).collect(),
            tags: (0..maximum).map(|_| AtomicU64::new(u64::MAX)).collect(),
            latency: AtomicUsize::new(0),
        })
    }
    pub fn channels(&self) -> usize {
        self.channels
    }
    pub fn invalidate(&self) {
        for t in &self.tags {
            t.store(u64::MAX, Ordering::Relaxed);
        }
    }
    pub fn write(&self, sample: u64, data: &[f32]) -> Result<(), Error> {
        if !data.len().is_multiple_of(self.channels)
            || data.len() / self.channels > self.maximum
            || sample
                .checked_add((data.len() / self.channels) as u64)
                .is_none()
        {
            return Err(Error::InvalidBuffer);
        }
        for (f, frame) in data.chunks(self.channels).enumerate() {
            let tag = sample + f as u64;
            let slot = tag as usize % self.maximum;
            for (c, v) in frame.iter().enumerate() {
                self.data[slot * self.channels + c].store(v.to_bits(), Ordering::Relaxed);
            }
            self.tags[slot].store(tag, Ordering::Release);
        }
        Ok(())
    }
    pub fn sample(&self, sample: u64, channel: usize) -> f32 {
        let slot = sample as usize % self.maximum;
        if channel >= self.channels || self.tags[slot].load(Ordering::Acquire) != sample {
            0.
        } else {
            f32::from_bits(self.data[slot * self.channels + channel].load(Ordering::Relaxed))
        }
    }
}
#[derive(Default)]
pub struct Registry {
    pub tracks: BTreeMap<String, Arc<Frames>>,
    pub outputs: BTreeMap<String, Vec<Arc<Frames>>>,
}
pub fn dependencies(d: &Document) -> Result<Vec<(usize, usize)>, Error> {
    let mut native = BTreeMap::new();
    let mut edges = std::collections::BTreeSet::new();
    for (i, t) in d.tracks.iter().enumerate() {
        for desc in t.instrument.iter().chain(&t.inserts) {
            if let Some(r) = Request::from_descriptor(desc)? {
                r.validate()?;
                if !matches!(desc.kind.as_str(), "vst3" | "clap") {
                    return Err(Error::InvalidRouting);
                }
                for source in r.sidechains.values() {
                    let from = d
                        .tracks
                        .iter()
                        .position(|t| &t.id == source)
                        .ok_or(Error::InvalidRouting)?;
                    if from == i {
                        return Err(Error::InvalidRouting);
                    }
                    edges.insert((from, i));
                }
                native.insert(desc.id.clone(), (i, r));
            }
        }
    }
    for desc in d
        .auxiliaries
        .iter()
        .flat_map(|a| &a.inserts)
        .chain(d.mix_routing.descriptors())
    {
        if matches!(desc.kind.as_str(), "vst3" | "clap") && desc.state.get("buses").is_some() {
            return Err(Error::InvalidRouting);
        }
    }
    for (i, t) in d.tracks.iter().enumerate() {
        for desc in t
            .instrument
            .iter()
            .chain(&t.inserts)
            .filter(|d| d.kind == "native.return")
        {
            if t.instrument.as_ref().is_none_or(|x| x.id != desc.id)
                || !t.audio.is_empty()
                || !t.streamed.is_empty()
                || !t.midi.is_empty()
                || t.input_gain != 0.
                || t.midi_input
            {
                return Err(Error::InvalidRouting);
            }
            let r: Return =
                serde_json::from_value(desc.state.clone()).map_err(|_| Error::InvalidRouting)?;
            let (from, request) = native.get(&r.processor).ok_or(Error::InvalidRouting)?;
            let bus = request
                .layout
                .outputs
                .get(r.bus)
                .ok_or(Error::InvalidRouting)?;
            if *from == i
                || !(1..=2).contains(&r.channels.len())
                || r.channels.iter().any(|c| *c >= bus.channels)
            {
                return Err(Error::InvalidRouting);
            }
            edges.insert((*from, i));
        }
    }
    Ok(edges.into_iter().collect())
}
impl Registry {
    pub fn prepare(&mut self, d: &Document) -> Result<(), Error> {
        dependencies(d)?;
        let mut next = Self::default();
        let mut samples = 0usize;
        for desc in d
            .tracks
            .iter()
            .flat_map(|t| t.instrument.iter().chain(&t.inserts))
        {
            if let Some(r) = Request::from_descriptor(desc)? {
                let mut outputs = Vec::new();
                for b in &r.layout.outputs {
                    samples += b.channels * d.max_block_frames;
                    outputs.push(Arc::new(Frames::new(b.channels, d.max_block_frames)?));
                }
                next.outputs.insert(desc.id.clone(), outputs);
                for id in r.sidechains.values() {
                    if !next.tracks.contains_key(id) {
                        samples += 2 * d.max_block_frames;
                        next.tracks
                            .insert(id.clone(), Arc::new(Frames::new(2, d.max_block_frames)?));
                    }
                }
                if samples > 16 * 1024 * 1024 {
                    return Err(Error::InvalidSpec);
                }
            }
        }
        *self = next;
        Ok(())
    }
    pub fn return_processor(&self, d: &Descriptor) -> Result<Built, Error> {
        let r: Return =
            serde_json::from_value(d.state.clone()).map_err(|_| Error::InvalidRouting)?;
        let bank = self
            .outputs
            .get(&r.processor)
            .and_then(|v| v.get(r.bus))
            .ok_or(Error::InvalidRouting)?
            .clone();
        if !(1..=2).contains(&r.channels.len()) || r.channels.iter().any(|c| *c >= bank.channels) {
            return Err(Error::InvalidRouting);
        }
        Ok(Built {
            processor: Box::new(ReturnAudio {
                bank,
                channels: r.channels,
            }),
            parameters: vec![],
        })
    }
}
struct ReturnAudio {
    bank: Arc<Frames>,
    channels: Vec<usize>,
}
impl AudioProcessor for ReturnAudio {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        if s.channels != 2 {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
    fn reset(&mut self) {}
    fn latency_frames(&self) -> usize {
        self.bank.latency.load(Ordering::Acquire)
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        for (f, pair) in b
            .samples_mut()
            .as_chunks_mut::<2>()
            .0
            .iter_mut()
            .enumerate()
        {
            for (c, value) in pair.iter_mut().enumerate() {
                *value = self.bank.sample(
                    ctx.transport.sample_position + f as u64,
                    self.channels[c.min(self.channels.len() - 1)],
                );
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frames_handle_partition_and_stale_tags() {
        let b = Frames::new(2, 8).unwrap();
        b.write(12, &[0.1, 0.2, 0.3, 0.4]).unwrap();
        b.write(14, &[0.5, 0.6]).unwrap();
        assert_eq!(b.sample(12, 1), 0.2);
        assert_eq!(b.sample(14, 0), 0.5);
        assert_eq!(b.sample(4, 0), 0.);
        b.invalidate();
        assert_eq!(b.sample(12, 1), 0.);
    }
}
