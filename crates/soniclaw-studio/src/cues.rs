//! Prepared independent stereo monitor mixes and a bounded multichannel publication bank.
use crate::*;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "tap", content = "value")]
pub enum Source {
    TrackPre(String),
    TrackPost(String),
    Bus(String),
    Aux(usize),
    Main,
    Input(Vec<u16>),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Send {
    pub id: String,
    pub source: Source,
    pub gain: f32,
    pub pan: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cue {
    #[serde(default)]
    pub metronome: bool,
    pub id: String,
    pub name: String,
    pub outputs: [u16; 2],
    pub gain: f32,
    pub pan: f32,
    pub mute: bool,
    pub align_sources: bool,
    pub sends: Vec<Send>,
}
impl Cue {
    pub fn new(id: String, name: String, outputs: [u16; 2]) -> Self {
        Self {
            metronome: true,
            id,
            name,
            outputs,
            gain: 1.,
            pan: 0.,
            mute: false,
            align_sources: true,
            sends: vec![],
        }
    }
}
fn strip(gain: f32, pan: f32) -> bool {
    gain.is_finite() && (0. ..=4.).contains(&gain) && pan.is_finite() && (-1. ..=1.).contains(&pan)
}
pub fn validate(d: &Document) -> Result<(), Error> {
    if d.cues.len() > 4 {
        return Err(Error::InvalidRouting);
    }
    let mut ids = BTreeSet::new();
    let mut ports = BTreeSet::new();
    for c in &d.cues {
        if !valid_id(&c.id)
            || !ids.insert(&c.id)
            || c.name.trim().is_empty()
            || c.name.len() > 256
            || !strip(c.gain, c.pan)
            || c.sends.len() > 32
            || c.outputs.iter().any(|p| *p >= 64 || !ports.insert(*p))
        {
            return Err(Error::InvalidRouting);
        }
        let mut sends = BTreeSet::new();
        for s in &c.sends {
            if !valid_id(&s.id) || !sends.insert(&s.id) || !strip(s.gain, s.pan) {
                return Err(Error::InvalidRouting);
            }
            match &s.source {
                Source::TrackPre(id) | Source::TrackPost(id) => {
                    if !d.tracks.iter().any(|t| &t.id == id) {
                        return Err(Error::InvalidRouting);
                    }
                }
                Source::Bus(id) => {
                    if !d.mix_routing.buses.iter().any(|b| &b.id == id) {
                        return Err(Error::InvalidRouting);
                    }
                }
                Source::Aux(i) => {
                    if *i >= d.auxiliaries.len() {
                        return Err(Error::InvalidRouting);
                    }
                }
                Source::Input(ch) => {
                    if !(1..=2).contains(&ch.len()) || ch.iter().any(|c| *c >= 64) {
                        return Err(Error::InvalidRouting);
                    }
                }
                Source::Main => {}
            }
        }
    }
    Ok(())
}
pub fn input_channels(d: &Document) -> impl Iterator<Item = u16> + '_ {
    d.cues
        .iter()
        .flat_map(|c| &c.sends)
        .flat_map(|s| {
            if let Source::Input(ch) = &s.source {
                ch.as_slice()
            } else {
                &[]
            }
        })
        .copied()
}
fn control(
    prefix: &str,
    key: &str,
    min: f32,
    max: f32,
    value: f32,
) -> Result<Arc<Parameter>, Error> {
    Ok(Arc::new(Parameter::new(ParameterInfo {
        id: &format!("{prefix}.{key}"),
        name: key,
        min,
        max,
        default: value,
        unit: "",
    })?))
}
enum Resolved {
    Pre(usize),
    Post(usize),
    Bus(usize),
    Aux(usize),
    Main,
    Input(Vec<u16>),
}
struct Edge {
    source: Resolved,
    delay: Delay,
    data: Vec<f32>,
    parameters: Vec<Arc<Parameter>>,
    gain: f32,
    pan: f32,
}
struct Node {
    metronome: bool,
    edges: Vec<Edge>,
    parameters: Vec<Arc<Parameter>>,
    gain: f32,
    pan: f32,
    gate: f32,
    meter: Arc<mix_graph::Meter>,
}
pub(crate) struct Runtime {
    pub click_bus: Arc<ChannelBus>,
    click_data: Vec<f32>,
    nodes: Vec<Node>,
    input: Option<Arc<ChannelBus>>,
    pub output: Arc<ChannelBus>,
    pub latencies: Vec<(String, usize)>,
    wide: Vec<f32>,
    coefficient: f32,
}
impl Runtime {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        d: &Document,
        tracks: &[TrackRuntime],
        graph: Option<&mix_graph::Runtime>,
        track_latency: usize,
        aux_latency: usize,
        input: Option<Arc<ChannelBus>>,
        parameters: &mut Vec<Arc<Parameter>>,
        delay_samples: &mut usize,
        meters: &mut Vec<Arc<mix_graph::Meter>>,
    ) -> Result<Self, Error> {
        validate(d)?;
        let mut nodes = vec![];
        let mut latencies = vec![];
        let spec = d.spec();
        for c in &d.cues {
            let mut sources = vec![];
            let mut largest = 0;
            for s in &c.sends {
                let (source, latency) = match &s.source {
                    Source::TrackPre(id) => {
                        let i = d
                            .tracks
                            .iter()
                            .position(|t| &t.id == id)
                            .ok_or(Error::InvalidRouting)?;
                        (Resolved::Pre(i), tracks[i].latency)
                    }
                    Source::TrackPost(id) => {
                        let i = d
                            .tracks
                            .iter()
                            .position(|t| &t.id == id)
                            .ok_or(Error::InvalidRouting)?;
                        (Resolved::Post(i), track_latency)
                    }
                    Source::Bus(id) => {
                        let i = d
                            .mix_routing
                            .buses
                            .iter()
                            .position(|b| &b.id == id)
                            .ok_or(Error::InvalidRouting)?;
                        (
                            Resolved::Bus(i),
                            graph.ok_or(Error::InvalidRouting)?.bus_latency(i),
                        )
                    }
                    Source::Aux(i) => (Resolved::Aux(*i), track_latency + aux_latency),
                    Source::Main => (
                        Resolved::Main,
                        graph.map_or(track_latency + aux_latency, |g| g.latency),
                    ),
                    Source::Input(ch) => {
                        if input
                            .as_ref()
                            .is_some_and(|b| ch.iter().any(|c| *c as usize >= b.channels()))
                        {
                            return Err(Error::InvalidRouting);
                        }
                        (Resolved::Input(ch.clone()), 0)
                    }
                };
                largest = largest.max(latency);
                sources.push((s, source, latency));
            }
            let mut edges = vec![];
            for (s, source, latency) in sources {
                let prefix = format!("cue.{}.send.{}", c.id, s.id);
                let controls = vec![
                    control(&prefix, "gain", 0., 4., s.gain)?,
                    control(&prefix, "pan", -1., 1., s.pan)?,
                ];
                parameters.extend(controls.iter().cloned());
                edges.push(Edge {
                    source,
                    delay: Delay::new(
                        if c.align_sources {
                            largest - latency
                        } else {
                            0
                        },
                        delay_samples,
                    )?,
                    data: vec![0.; spec.max_block_frames * 2],
                    parameters: controls,
                    gain: s.gain,
                    pan: s.pan,
                });
            }
            let prefix = format!("cue.{}", c.id);
            let controls = vec![
                control(&prefix, "gain", 0., 4., c.gain)?,
                control(&prefix, "pan", -1., 1., c.pan)?,
                control(&prefix, "mute", 0., 1., if c.mute { 1. } else { 0. })?,
            ];
            parameters.extend(controls.iter().cloned());
            let meter = Arc::new(mix_graph::Meter::new(prefix));
            meters.push(meter.clone());
            nodes.push(Node {
                metronome: c.metronome,
                edges,
                parameters: controls,
                gain: c.gain,
                pan: c.pan,
                gate: if c.mute { 0. } else { 1. },
                meter,
            });
            latencies.push((c.id.clone(), largest));
        }
        let channels = 2 + nodes.len() * 2;
        Ok(Self {
            click_bus: Arc::new(ChannelBus::new(nodes.len() * 2, spec.max_block_frames)?),
            click_data: vec![0.; nodes.len() * 2 * spec.max_block_frames],
            nodes,
            input,
            output: Arc::new(ChannelBus::new(channels, spec.max_block_frames)?),
            latencies,
            wide: vec![0.; channels * spec.max_block_frames],
            coefficient: 1. - (-1. / (spec.sample_rate as f32 * 0.005)).exp(),
        })
    }
    pub fn initialize_controls(&mut self) {
        for n in &mut self.nodes {
            n.gain = n.parameters[0].get();
            n.pan = n.parameters[1].get();
            n.gate = if n.parameters[2].get() >= 0.5 { 0. } else { 1. };
            for e in &mut n.edges {
                e.gain = e.parameters[0].get();
                e.pan = e.parameters[1].get();
            }
        }
    }
    pub fn process(
        &mut self,
        tracks: &[TrackRuntime],
        post: &[Vec<f32>],
        graph: Option<&mix_graph::Runtime>,
        aux: &[Vec<f32>],
        main: &[f32],
        offset: usize,
    ) -> Result<(), Error> {
        let frames = main.len() / 2;
        let channels = self.output.channels();
        let click_channels = self.click_bus.channels();
        self.wide[..frames * channels].fill(0.);
        for (f, pair) in main.as_chunks::<2>().0.iter().enumerate() {
            self.wide[f * channels..f * channels + 2].copy_from_slice(pair);
        }
        for (i, n) in self.nodes.iter_mut().enumerate() {
            for e in &mut n.edges {
                let data = &mut e.data[..main.len()];
                match &e.source {
                    Resolved::Pre(t) => data.copy_from_slice(&tracks[*t].monitor_pcm[..main.len()]),
                    Resolved::Post(t) => data.copy_from_slice(&post[*t][..main.len()]),
                    Resolved::Bus(b) => data.copy_from_slice(
                        graph.ok_or(Error::InvalidRouting)?.bus_pcm(*b, main.len()),
                    ),
                    Resolved::Aux(a) => data.copy_from_slice(&aux[*a][..main.len()]),
                    Resolved::Main => data.copy_from_slice(main),
                    Resolved::Input(ch) => {
                        data.fill(0.);
                        if let Some(input) = &self.input {
                            if input.ready() {
                                for (f, pair) in data.as_chunks_mut::<2>().0.iter_mut().enumerate()
                                {
                                    pair[0] = input.sample(offset + f, ch[0] as usize);
                                    pair[1] = input.sample(offset + f, ch[ch.len() - 1] as usize);
                                }
                            }
                        }
                    }
                }
                e.delay.process(data);
                for (f, pair) in data.as_chunks::<2>().0.iter().enumerate() {
                    e.gain += (e.parameters[0].get() - e.gain) * self.coefficient;
                    e.pan += (e.parameters[1].get() - e.pan) * self.coefficient;
                    self.wide[f * channels + 2 + i * 2] += pair[0] * e.gain * (1. - e.pan.max(0.));
                    self.wide[f * channels + 3 + i * 2] += pair[1] * e.gain * (1. + e.pan.min(0.));
                }
            }
            let mut peak = [0f32; 2];
            for f in 0..frames {
                n.gain += (n.parameters[0].get() - n.gain) * self.coefficient;
                n.pan += (n.parameters[1].get() - n.pan) * self.coefficient;
                n.gate += ((if n.parameters[2].get() >= 0.5 { 0. } else { 1. }) - n.gate)
                    * self.coefficient;
                let at = f * channels + 2 + i * 2;
                let left = n.gain * n.gate * (1. - n.pan.max(0.));
                let right = n.gain * n.gate * (1. + n.pan.min(0.));
                self.wide[at] *= left;
                self.wide[at + 1] *= right;
                self.click_data[f * click_channels + i * 2] = if n.metronome { left } else { 0. };
                self.click_data[f * click_channels + i * 2 + 1] =
                    if n.metronome { right } else { 0. };
                peak[0] = peak[0].max(self.wide[at].abs());
                peak[1] = peak[1].max(self.wide[at + 1].abs());
            }
            n.meter.publish(peak);
        }
        self.click_bus
            .write_at(offset, &self.click_data[..frames * click_channels])?;
        self.output
            .write_at(offset, &self.wide[..frames * channels])
    }
    pub fn reset(&mut self) {
        for n in &mut self.nodes {
            for e in &mut n.edges {
                e.delay.reset();
            }
        }
    }
}
/// Copy saved normalized controls into an editable typed cue model.
pub(crate) fn materialize(d: &mut Document) {
    let values = &d.parameter_values;
    for c in &mut d.cues {
        let prefix = format!("cue.{}", c.id);
        if let Some(v) = values.get(&format!("{prefix}.gain")) {
            c.gain = *v * 4.;
        }
        if let Some(v) = values.get(&format!("{prefix}.pan")) {
            c.pan = *v * 2. - 1.;
        }
        if let Some(v) = values.get(&format!("{prefix}.mute")) {
            c.mute = *v >= 0.5;
        }
        for s in &mut c.sends {
            let prefix = format!("{prefix}.send.{}", s.id);
            if let Some(v) = values.get(&format!("{prefix}.gain")) {
                s.gain = *v * 4.;
            }
            if let Some(v) = values.get(&format!("{prefix}.pan")) {
                s.pan = *v * 2. - 1.;
            }
        }
    }
}
