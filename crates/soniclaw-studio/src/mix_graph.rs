//! Prepared stereo submix forests, master processing and explicit zero-latency track sidechains.
use crate::*;
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "id")]
pub enum Destination {
    #[default]
    Main,
    Bus(String),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bus {
    pub id: String,
    pub name: String,
    pub output: Destination,
    pub gain: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub inserts: Vec<Descriptor>,
}
impl Bus {
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            output: Destination::Main,
            gain: 1.,
            pan: 0.,
            mute: false,
            solo: false,
            inserts: vec![],
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Master {
    pub gain: f32,
    pub pan: f32,
    pub mute: bool,
    pub inserts: Vec<Descriptor>,
}
impl Default for Master {
    fn default() -> Self {
        Self {
            gain: 1.,
            pan: 0.,
            mute: false,
            inserts: vec![],
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sidechain {
    pub source: String,
    pub target: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    #[serde(default)]
    pub buses: Vec<Bus>,
    #[serde(default)]
    pub track_outputs: BTreeMap<String, Destination>,
    #[serde(default)]
    pub sidechains: Vec<Sidechain>,
    #[serde(default)]
    pub master: Master,
}
impl Model {
    pub fn is_empty(&self) -> bool {
        self.buses.is_empty()
            && self.track_outputs.is_empty()
            && self.sidechains.is_empty()
            && self.master.inserts.is_empty()
            && self.master.gain == 1.
            && self.master.pan == 0.
            && !self.master.mute
    }
    fn destination(&self, d: &Destination) -> Result<Option<usize>, Error> {
        match d {
            Destination::Main => Ok(None),
            Destination::Bus(id) => self
                .buses
                .iter()
                .position(|b| &b.id == id)
                .map(Some)
                .ok_or(Error::InvalidRouting),
        }
    }
    fn bus_order(&self) -> Result<Vec<usize>, Error> {
        let mut indegree = vec![0; self.buses.len()];
        for b in &self.buses {
            if let Some(i) = self.destination(&b.output)? {
                indegree[i] += 1;
            }
        }
        let mut order = vec![];
        while let Some(i) = (0..indegree.len()).find(|i| indegree[*i] == 0 && !order.contains(i)) {
            order.push(i);
            if let Some(to) = self.destination(&self.buses[i].output)? {
                indegree[to] -= 1;
            }
        }
        if order.len() != self.buses.len() {
            return Err(Error::InvalidRouting);
        }
        Ok(order)
    }
    pub fn validate(&self, d: &Document) -> Result<(), Error> {
        if self.buses.len() > 8
            || self.sidechains.len() > 32
            || self.master.inserts.len() > 16
            || !strip(self.master.gain, self.master.pan)
        {
            return Err(Error::InvalidRouting);
        }
        let mut ids = BTreeSet::new();
        for b in &self.buses {
            if !valid_id(&b.id)
                || !ids.insert(&b.id)
                || b.name.trim().is_empty()
                || b.name.len() > 256
                || d.tracks.iter().any(|t| t.id == b.id)
                || !strip(b.gain, b.pan)
                || b.inserts.len() > 16
                || b.inserts.iter().any(|p| p.kind == "stock.sidechain")
            {
                return Err(Error::InvalidRouting);
            }
            self.destination(&b.output)?;
        }
        for (id, to) in &self.track_outputs {
            if !d.tracks.iter().any(|t| &t.id == id) {
                return Err(Error::InvalidRouting);
            }
            self.destination(to)?;
        }
        if self
            .master
            .inserts
            .iter()
            .any(|p| p.kind == "stock.sidechain")
        {
            return Err(Error::InvalidRouting);
        }
        self.bus_order()?;
        self.sidechain_order(d)?;
        Ok(())
    }
    pub(crate) fn sidechain_order(&self, d: &Document) -> Result<Vec<usize>, Error> {
        let mut edges = vec![vec![]; d.tracks.len()];
        let mut indegree = vec![0; d.tracks.len()];
        let mut targets = BTreeSet::new();
        for sc in &self.sidechains {
            if !targets.insert(&sc.target) {
                return Err(Error::InvalidRouting);
            }
            let source = d
                .tracks
                .iter()
                .position(|t| t.id == sc.source)
                .ok_or(Error::InvalidRouting)?;
            let target = d
                .tracks
                .iter()
                .position(|t| {
                    t.inserts
                        .iter()
                        .any(|p| p.id == sc.target && p.kind == "stock.sidechain")
                })
                .ok_or(Error::InvalidRouting)?;
            if target == source {
                return Err(Error::InvalidRouting);
            }
            edges[source].push(target);
            indegree[target] += 1;
        }
        for (source, target) in native_buses::dependencies(d)? {
            if !edges[source].contains(&target) {
                edges[source].push(target);
                indegree[target] += 1;
            }
        }
        for p in d
            .tracks
            .iter()
            .flat_map(|t| &t.inserts)
            .filter(|p| p.kind == "stock.sidechain")
        {
            if !targets.contains(&p.id) {
                return Err(Error::InvalidRouting);
            }
        }
        let mut order = vec![];
        while let Some(i) = (0..d.tracks.len()).find(|i| indegree[*i] == 0 && !order.contains(i)) {
            order.push(i);
            for &to in &edges[i] {
                indegree[to] -= 1;
            }
        }
        if order.len() != d.tracks.len() {
            return Err(Error::InvalidRouting);
        }
        Ok(order)
    }
    pub fn descriptors(&self) -> impl Iterator<Item = &Descriptor> {
        self.buses
            .iter()
            .flat_map(|b| &b.inserts)
            .chain(&self.master.inserts)
    }
}
fn strip(gain: f32, pan: f32) -> bool {
    gain.is_finite() && (0. ..=4.).contains(&gain) && pan.is_finite() && (-1. ..=1.).contains(&pan)
}
pub fn sidechain_descriptor(id: String) -> Result<Descriptor, Error> {
    if !valid_id(&id) {
        return Err(Error::InvalidParameter);
    }
    Ok(Descriptor {
        id,
        kind: "stock.sidechain".into(),
        state: serde_json::json!({"threshold":-24.,"ratio":6.,"attack_ms":5.,"release_ms":100.,"makeup_db":0.}),
    })
}
pub(crate) fn sidechain(d: &Descriptor, bus: Arc<ChannelBus>) -> Result<Built, Error> {
    let object = d.state.as_object().ok_or(Error::InvalidParameter)?;
    let specs = [
        ("threshold", -80., 0., -24.),
        ("ratio", 1., 40., 6.),
        ("attack_ms", 0., 1000., 5.),
        ("release_ms", 1., 10000., 100.),
        ("makeup_db", -24., 24., 0.),
    ];
    if object
        .keys()
        .any(|key| !specs.iter().any(|(s, _, _, _)| s == key))
    {
        return Err(Error::InvalidParameter);
    }
    let mut parameters = vec![];
    for (key, min, max, default) in specs {
        let id = format!("{}.{}", d.id, key);
        let p = Arc::new(Parameter::new(ParameterInfo {
            id: &id,
            name: key,
            min,
            max,
            default,
            unit: "",
        })?);
        if let Some(v) = object.get(key) {
            p.set(v.as_f64().ok_or(Error::InvalidParameter)? as f32)?;
        }
        parameters.push(p);
    }
    Ok(Built {
        processor: Box::new(Detector {
            bus,
            parameters: parameters.clone(),
            rate: 48000.,
            reduction: 0.,
        }),
        parameters,
    })
}
struct Detector {
    bus: Arc<ChannelBus>,
    parameters: Vec<Arc<Parameter>>,
    rate: f32,
    reduction: f32,
}
impl AudioProcessor for Detector {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        if s.channels != 2 || s.max_block_frames > self.bus.maximum_frames() {
            return Err(Error::InvalidSpec);
        }
        self.rate = s.sample_rate as f32;
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let p = &self.parameters;
        let threshold = p[0].get();
        let ratio = p[1].get();
        let attack = if p[2].get() == 0. {
            0.
        } else {
            (-1. / (self.rate * p[2].get() * 0.001)).exp()
        };
        let release = (-1. / (self.rate * p[3].get() * 0.001)).exp();
        let makeup = p[4].get();
        for (i, f) in b
            .samples_mut()
            .as_chunks_mut::<2>()
            .0
            .iter_mut()
            .enumerate()
        {
            let peak = self.bus.sample(i, 0).abs().max(self.bus.sample(i, 1).abs());
            let level = 20. * peak.max(1e-10).log10();
            let target = (level - threshold).max(0.) * (1. - 1. / ratio);
            let k = if target > self.reduction {
                attack
            } else {
                release
            };
            self.reduction = k * self.reduction + (1. - k) * target;
            let gain = 10f32.powf((makeup - self.reduction) / 20.);
            f[0] *= gain;
            f[1] *= gain;
        }
    }
    fn reset(&mut self) {
        self.reduction = 0.;
    }
}
/// Peak-hold exchange; consume on the control/UI thread. This is not an RMS/LUFS meter.
pub struct Meter {
    pub id: String,
    peak: [std::sync::atomic::AtomicU32; 2],
}
impl Meter {
    pub(crate) fn new(id: String) -> Self {
        Self {
            id,
            peak: std::array::from_fn(|_| std::sync::atomic::AtomicU32::new(0)),
        }
    }
    pub(crate) fn publish(&self, peak: [f32; 2]) {
        for (p, v) in self.peak.iter().zip(peak) {
            p.fetch_max(v.to_bits(), Ordering::Relaxed);
        }
    }

    pub fn take_peaks(&self) -> [f32; 2] {
        std::array::from_fn(|i| f32::from_bits(self.peak[i].swap(0, Ordering::Relaxed)))
    }
}
struct Edge {
    source: usize,
    delay: Delay,
    scratch: Vec<f32>,
}
struct Node {
    meter: Arc<Meter>,
    slot: usize,
    edges: Vec<Edge>,
    effects: Vec<Box<dyn AudioProcessor>>,
    parameters: Vec<Arc<Parameter>>,
    gain: f32,
    pan: f32,
    gate: f32,
}
pub(crate) struct Runtime {
    bus_latencies: Vec<usize>,
    pub meters: Vec<Arc<Meter>>,
    nodes: Vec<Node>,
    buffers: Vec<Vec<f32>>,
    track_dest: Vec<Option<usize>>,
    bus_dest: Vec<Option<usize>>,
    bus_solo: Vec<Arc<Parameter>>,
    pub latency: usize,
    tracks: usize,
    buses: usize,
    coefficient: f32,
}
fn parameter(id: &str, key: &str, min: f32, max: f32, value: f32) -> Result<Arc<Parameter>, Error> {
    Ok(Arc::new(Parameter::new(ParameterInfo {
        id: &format!("{id}.{key}"),
        name: key,
        min,
        max,
        default: value,
        unit: "",
    })?))
}
impl Runtime {
    pub fn bus_latency(&self, index: usize) -> usize {
        self.bus_latencies[index]
    }
    pub fn bus_pcm(&self, index: usize, samples: usize) -> &[f32] {
        &self.buffers[self.tracks + index][..samples]
    }

    pub fn new(
        d: &Document,
        f: &mut impl Factory,
        parameters: &mut Vec<Arc<Parameter>>,
        track_latency: usize,
        aux_latency: usize,
        delay_samples: &mut usize,
    ) -> Result<Self, Error> {
        let model = d.mix_routing.clone();
        let tracks = d.tracks.len();
        let buses = model.buses.len();
        let aux = d.auxiliaries.len();
        let master = tracks + buses + aux;
        let spec = d.spec();
        let track_dest = d
            .tracks
            .iter()
            .map(|t| {
                model.destination(model.track_outputs.get(&t.id).unwrap_or(&Destination::Main))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let bus_dest = model
            .buses
            .iter()
            .map(|b| model.destination(&b.output))
            .collect::<Result<Vec<_>, _>>()?;
        let mut incoming = vec![vec![]; buses + 1];
        for (i, to) in track_dest.iter().enumerate() {
            incoming[to.unwrap_or(buses)].push(i);
        }
        for (i, to) in bus_dest.iter().enumerate() {
            incoming[to.unwrap_or(buses)].push(tracks + i);
        }
        for i in 0..aux {
            incoming[buses].push(tracks + buses + i);
        }
        let mut latency = vec![track_latency; master + 1];
        for value in &mut latency[tracks + buses..master] {
            *value += aux_latency;
        }
        let mut nodes = vec![];
        let mut meters = vec![];
        let mut order = model.bus_order()?;
        order.push(buses);
        let mut solo_params = vec![None; buses];
        for i in order {
            let (id, gain, pan, mute, solo, descriptors) = if i == buses {
                (
                    "master".into(),
                    model.master.gain,
                    model.master.pan,
                    model.master.mute,
                    false,
                    &model.master.inserts,
                )
            } else {
                let b = &model.buses[i];
                (
                    format!("bus.{}", b.id),
                    b.gain,
                    b.pan,
                    b.mute,
                    b.solo,
                    &b.inserts,
                )
            };
            let slot = if i == buses { master } else { tracks + i };
            let input_latency = incoming[i]
                .iter()
                .map(|s| latency[*s])
                .max()
                .unwrap_or(track_latency);
            let mut effects = vec![];
            let mut own = 0usize;
            for desc in descriptors {
                let p = build(desc, f, spec, parameters)?;
                own = own
                    .checked_add(p.latency_frames())
                    .ok_or(Error::InvalidSpec)?;
                effects.push(p);
            }
            latency[slot] = input_latency
                .checked_add(own)
                .filter(|v| *v <= 262144)
                .ok_or(Error::InvalidSpec)?;
            let mut controls = vec![
                parameter(&id, "gain", 0., 4., gain)?,
                parameter(&id, "pan", -1., 1., pan)?,
                parameter(&id, "mute", 0., 1., if mute { 1. } else { 0. })?,
            ];
            if i < buses {
                let p = parameter(&id, "solo", 0., 1., if solo { 1. } else { 0. })?;
                solo_params[i] = Some(p.clone());
                controls.push(p);
            }
            parameters.extend(controls.iter().cloned());
            let mut edges = vec![];
            for &source in &incoming[i] {
                edges.push(Edge {
                    source,
                    delay: Delay::new(input_latency - latency[source], delay_samples)?,
                    scratch: vec![0.; spec.max_block_frames * 2],
                });
            }
            let meter = Arc::new(Meter {
                id: id.clone(),
                peak: std::array::from_fn(|_| std::sync::atomic::AtomicU32::new(0)),
            });
            meters.push(meter.clone());
            nodes.push(Node {
                meter,
                slot,
                edges,
                effects,
                parameters: controls,
                gain,
                pan,
                gate: if mute { 0. } else { 1. },
            });
        }
        Ok(Self {
            latency: latency[master],
            bus_latencies: latency[tracks..tracks + buses].to_vec(),
            meters,
            nodes,
            buffers: vec![vec![0.; spec.max_block_frames * 2]; master + 1],
            track_dest,
            bus_dest,
            bus_solo: solo_params.into_iter().map(Option::unwrap).collect(),
            tracks,
            buses,
            coefficient: 1. - (-1. / (spec.sample_rate as f32 * 0.005)).exp(),
        })
    }
    /// Bus solo includes its descendant tracks; the ordinary mixer still owns track solo/mute and sends.
    pub fn audible_tracks(&self) -> [bool; 32] {
        let mut result = [true; 32];
        if self.bus_solo.iter().any(|p| p.get() >= 0.5) {
            for (i, out) in self.track_dest.iter().enumerate() {
                let mut at = *out;
                let mut audible = false;
                while let Some(b) = at {
                    audible |= self.bus_solo[b].get() >= 0.5;
                    at = self.bus_dest[b];
                }
                result[i] = audible;
            }
        }
        result
    }
    fn allowed_buses(&self) -> [bool; 8] {
        let mut allowed = [true; 8];
        if self.bus_solo.iter().any(|p| p.get() >= 0.5) {
            allowed.fill(false);
            for (b, allowed) in allowed.iter_mut().enumerate().take(self.buses) {
                let mut at = Some(b);
                while let Some(i) = at {
                    if self.bus_solo[i].get() >= 0.5 {
                        *allowed = true;
                    }
                    at = self.bus_dest[i];
                }
            }
            for (i, p) in self.bus_solo.iter().enumerate() {
                if p.get() >= 0.5 {
                    let mut at = Some(i);
                    while let Some(i) = at {
                        allowed[i] = true;
                        at = self.bus_dest[i];
                    }
                }
            }
        }
        allowed
    }
    pub fn initialize_controls(&mut self) {
        let allowed = self.allowed_buses();
        for n in &mut self.nodes {
            n.gain = n.parameters[0].get();
            n.pan = n.parameters[1].get();
            n.gate = if n.parameters[2].get() >= 0.5
                || (n.slot >= self.tracks
                    && n.slot < self.tracks + self.buses
                    && !allowed[n.slot - self.tracks])
            {
                0.
            } else {
                1.
            };
        }
    }
    pub fn process(
        &mut self,
        post: &[Vec<f32>],
        aux: &[Vec<f32>],
        out: &mut [f32],
        ctx: &ProcessContext<'_>,
    ) -> Result<(), Error> {
        let allowed = self.allowed_buses();
        let len = out.len();
        for (i, p) in post.iter().enumerate() {
            self.buffers[i][..len].copy_from_slice(&p[..len]);
        }
        for (i, p) in aux.iter().enumerate() {
            self.buffers[self.tracks + self.buses + i][..len].copy_from_slice(&p[..len]);
        }
        for n in &mut self.nodes {
            self.buffers[n.slot][..len].fill(0.);
            for e in &mut n.edges {
                e.scratch[..len].copy_from_slice(&self.buffers[e.source][..len]);
                e.delay.process(&mut e.scratch[..len]);
                for (a, b) in self.buffers[n.slot][..len]
                    .iter_mut()
                    .zip(&e.scratch[..len])
                {
                    *a += *b;
                }
            }
            for p in &mut n.effects {
                p.process(
                    &mut AudioBuffer::new(&mut self.buffers[n.slot][..len], 2)?,
                    ctx,
                );
            }
            let mut gate = if n.parameters[2].get() >= 0.5 { 0. } else { 1. };
            if n.slot >= self.tracks
                && n.slot < self.tracks + self.buses
                && !allowed[n.slot - self.tracks]
            {
                gate = 0.;
            }
            let mut peak = [0f32; 2];
            for f in self.buffers[n.slot][..len].as_chunks_mut::<2>().0 {
                n.gain += (n.parameters[0].get() - n.gain) * self.coefficient;
                n.pan += (n.parameters[1].get() - n.pan) * self.coefficient;
                n.gate += (gate - n.gate) * self.coefficient;
                f[0] *= n.gain * n.gate * (1. - n.pan.max(0.));
                f[1] *= n.gain * n.gate * (1. + n.pan.min(0.));
                peak[0] = peak[0].max(f[0].abs());
                peak[1] = peak[1].max(f[1].abs());
            }
            for (p, v) in n.meter.peak.iter().zip(peak) {
                p.fetch_max(v.to_bits(), Ordering::Relaxed);
            }
        }
        out.copy_from_slice(&self.buffers.last().ok_or(Error::InvalidRouting)?[..len]);
        Ok(())
    }
    pub fn reset(&mut self) {
        for n in &mut self.nodes {
            for e in &mut n.edges {
                e.delay.reset();
            }
            for p in &mut n.effects {
                p.reset();
            }
        }
        for b in &mut self.buffers {
            b.fill(0.);
        }
    }
}
/// A stopped control-side routing edit, guarded against replacing newer project edits.
#[derive(Clone)]
pub struct Draft {
    baseline: project::Project,
    pub document: Document,
}
impl Draft {
    pub fn new(p: &project::Project) -> Result<Self, project::ProjectError> {
        p.validate()?;
        let mut document = p.document.clone();
        let apply = |prefix: &str,
                     gain: &mut f32,
                     pan: &mut f32,
                     mute: &mut bool,
                     solo: Option<&mut bool>| {
            if let Some(v) = document.parameter_values.get(&format!("{prefix}.gain")) {
                *gain = *v * 4.;
            }
            if let Some(v) = document.parameter_values.get(&format!("{prefix}.pan")) {
                *pan = *v * 2. - 1.;
            }
            if let Some(v) = document.parameter_values.get(&format!("{prefix}.mute")) {
                *mute = *v >= 0.5;
            }
            if let Some(solo) = solo {
                if let Some(v) = document.parameter_values.get(&format!("{prefix}.solo")) {
                    *solo = *v >= 0.5;
                }
            }
        };
        for b in &mut document.mix_routing.buses {
            apply(
                &format!("bus.{}", b.id),
                &mut b.gain,
                &mut b.pan,
                &mut b.mute,
                Some(&mut b.solo),
            );
        }
        let m = &mut document.mix_routing.master;
        apply("master", &mut m.gain, &mut m.pan, &mut m.mute, None);
        crate::cues::materialize(&mut document);
        Ok(Self {
            baseline: p.clone(),
            document,
        })
    }
    /// Verify/prepare before adopting. Retire/publish the returned engine on its normal owner.
    pub fn apply(
        self,
        p: &mut project::Project,
        factory: &mut impl Factory,
    ) -> Result<PreparedSession, project::ProjectError> {
        if !history::equivalent(&self.baseline, p)? {
            return Err(
                "project changed while routing draft was open; reload the routing draft".into(),
            );
        }
        let mut next = p.clone();
        let old = &p.document;
        next.document = self.document;
        let descriptors = |d: &Document| {
            d.tracks
                .iter()
                .flat_map(|t| t.instrument.iter().chain(&t.inserts))
                .chain(d.auxiliaries.iter().flat_map(|a| &a.inserts))
                .chain(d.mix_routing.descriptors())
                .map(|d| d.id.clone())
                .collect::<BTreeSet<_>>()
        };
        let old_ids = descriptors(old);
        let new_ids = descriptors(&next.document);
        let mut removed: Vec<String> = old_ids
            .difference(&new_ids)
            .map(|id| format!("{id}."))
            .collect();
        for b in &old.mix_routing.buses {
            if !next.document.mix_routing.buses.iter().any(|n| n.id == b.id) {
                removed.push(format!("bus.{}.", b.id));
            }
        }
        for c in &old.cues {
            if let Some(new) = next.document.cues.iter().find(|n| n.id == c.id) {
                for send in &c.sends {
                    if !new.sends.iter().any(|n| n.id == send.id) {
                        removed.push(format!("cue.{}.send.{}.", c.id, send.id));
                    }
                }
            } else {
                removed.push(format!("cue.{}.", c.id));
            }
        }
        next.document
            .parameter_values
            .retain(|id, _| !removed.iter().any(|s| id.starts_with(s)));
        next.document
            .automation
            .lanes
            .retain(|l| !removed.iter().any(|s| l.parameter.starts_with(s)));
        let mut controls = vec![
            "master.gain".to_string(),
            "master.pan".into(),
            "master.mute".into(),
        ];
        for b in &next.document.mix_routing.buses {
            for key in ["gain", "pan", "mute", "solo"] {
                controls.push(format!("bus.{}.{key}", b.id));
            }
        }
        for c in &next.document.cues {
            let prefix = format!("cue.{}", c.id);
            for key in ["gain", "pan", "mute"] {
                controls.push(format!("{prefix}.{key}"));
            }
            for send in &c.sends {
                for key in ["gain", "pan"] {
                    controls.push(format!("{prefix}.send.{}.{key}", send.id));
                }
            }
        }
        for id in controls {
            next.document.parameter_values.remove(&id);
        }
        next.validate()?;
        let prepared = next
            .document
            .compile_streamed(&next.assets, &next.disk, factory)?;
        *p = next;
        Ok(prepared)
    }
}
