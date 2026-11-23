//! Paged disk sampling and seekable WAV transport. File handles/decoding live on
//! one worker. Requests and fixed stereo pages cross bounded SPSC queues.
use soniclaw_core::*;
use soniclaw_dsp::resample::ResampleQuality;
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
const PAGE: usize = 1024;
const CACHES: usize = 4;
#[derive(Clone)]
struct Page {
    slot: usize,
    generation: u64,
    asset: usize,
    start: u64,
    data: [f32; PAGE * 2],
}
impl Default for Page {
    fn default() -> Self {
        Self {
            slot: 0,
            generation: 0,
            asset: usize::MAX,
            start: u64::MAX,
            data: [0.0; PAGE * 2],
        }
    }
}
#[derive(Clone, Copy)]
struct Request {
    slot: usize,
    generation: u64,
    asset: usize,
    start: u64,
}
#[derive(Clone, Debug)]
pub struct DiskAsset {
    pub path: PathBuf,
    pub root_note: u8,
}
#[derive(Clone, Debug)]
pub struct DiskZone {
    pub asset: usize,
    pub keys: [u8; 2],
    pub velocities: [u8; 2],
    pub group: u8,
    pub gain: f32,
    pub one_shot: bool,
    pub loop_frames: Option<(u64, u64)>,
}
impl DiskZone {
    pub fn single(asset: usize, key: u8) -> Self {
        Self {
            asset,
            keys: [key, key],
            velocities: [1, 127],
            group: 0,
            gain: 1.0,
            one_shot: true,
            loop_frames: None,
        }
    }
}
struct Metadata {
    frames: u64,
    rate: u32,
    head: Page,
    root: u8,
}
#[derive(Default)]
pub struct DiskStats {
    pub underrun_frames: AtomicU64,
    pub io_errors: AtomicU64,
    pub request_overflows: AtomicU64,
    pub pages_read: AtomicU64,
    pub stale_pages: AtomicU64,
    published: AtomicU64,
    consumed: AtomicU64,
    stop: AtomicBool,
}
/// Keep/drop on the control thread; dropping joins the worker. Stop rendering
/// before teardown. Ready pages still need a process call to enter voice caches.
pub struct DiskHandle {
    worker: Option<JoinHandle<()>>,
    pub stats: Arc<DiskStats>,
    pub pcm_cache_bytes: usize,
}
impl DiskHandle {
    pub fn wait_ready(&self, min_pages: usize, timeout: Duration) -> bool {
        let t = Instant::now();
        while self
            .stats
            .published
            .load(Ordering::Acquire)
            .saturating_sub(self.stats.consumed.load(Ordering::Acquire))
            < min_pages as u64
        {
            if t.elapsed() >= timeout
                || self.stats.io_errors.load(Ordering::Relaxed) > 0
                || self.stats.stop.load(Ordering::Acquire)
            {
                return false;
            }
            thread::sleep(Duration::from_millis(1));
        }
        true
    }
    pub fn stop(&mut self) {
        self.stats.stop.store(true, Ordering::Release);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
impl Drop for DiskHandle {
    fn drop(&mut self) {
        self.stop();
    }
}
struct Voice {
    active: bool,
    asset: usize,
    zone: usize,
    position: f64,
    increment: f64,
    channel: u8,
    note: u8,
    id: i32,
    velocity: f32,
    level: f32,
    held: bool,
    released: bool,
    age: u64,
    generation: u64,
    pages: [Page; CACHES],
    cursor: usize,
    pending: [Option<u64>; CACHES],
    expression: VoiceExpression,
    filter: [f32; 2],
    looped: bool,
}
impl Default for Voice {
    fn default() -> Self {
        Self {
            active: false,
            asset: 0,
            zone: 0,
            position: 0.0,
            increment: 1.0,
            channel: 0,
            note: 0,
            id: -1,
            velocity: 0.0,
            level: 0.0,
            held: false,
            released: false,
            age: 0,
            generation: 0,
            pages: std::array::from_fn(|_| Page::default()),
            cursor: 0,
            pending: [None; CACHES],
            expression: VoiceExpression::default(),
            filter: [0.0; 2],
            looped: false,
        }
    }
}
/// Fixed maximum voice count and PCM budget; metadata, sample heads, all caches
/// and queues allocate in open_disk_sampler, never during note-on or rendering.
pub struct DiskSampler<const N: usize = 32> {
    metadata: Vec<Metadata>,
    zones: Vec<DiskZone>,
    rules: Vec<crate::ZoneRule>,
    switches: [Option<u8>; 16],
    known_switches: [bool; 128],
    default_switch: Option<u8>,
    voices: Vec<Voice>,
    requests: rtrb::Producer<Request>,
    responses: rtrb::Consumer<Page>,
    pub stats: Arc<DiskStats>,
    channels: ExpressionChannels,
    sustain: [bool; 16],
    rr: [u64; 256],
    age: u64,
    sr: f64,
    release: f32,
    attack_ms: f32,
    release_ms: f32,
    quality: ResampleQuality,
}
type Reader = hound::WavReader<BufReader<File>>;
fn decode(reader: &mut Reader, start: u64, page: &mut Page) -> Result<(), hound::Error> {
    reader.seek(start as u32)?;
    let spec = reader.spec();
    let available = (reader.duration() as u64)
        .saturating_sub(start)
        .min(PAGE as u64) as usize;
    page.data.fill(0.0);
    if spec.sample_format == hound::SampleFormat::Float {
        let mut samples = reader.samples::<f32>();
        for f in 0..available {
            let a = samples
                .next()
                .ok_or(hound::Error::FormatError("truncated WAV"))??;
            let b = if spec.channels == 2 {
                samples
                    .next()
                    .ok_or(hound::Error::FormatError("truncated WAV"))??
            } else {
                a
            };
            if !a.is_finite() || !b.is_finite() {
                return Err(hound::Error::FormatError("non-finite WAV"));
            }
            page.data[f * 2] = a;
            page.data[f * 2 + 1] = b;
        }
    } else {
        let scale = 2.0_f64.powi(spec.bits_per_sample as i32 - 1);
        let mut samples = reader.samples::<i32>();
        for f in 0..available {
            let a = (samples
                .next()
                .ok_or(hound::Error::FormatError("truncated WAV"))?? as f64
                / scale) as f32;
            let b = if spec.channels == 2 {
                (samples
                    .next()
                    .ok_or(hound::Error::FormatError("truncated WAV"))?? as f64
                    / scale) as f32
            } else {
                a
            };
            page.data[f * 2] = a;
            page.data[f * 2 + 1] = b;
        }
    }
    Ok(())
}
pub fn open_disk_sampler<const N: usize>(
    assets: Vec<DiskAsset>,
    zones: Vec<DiskZone>,
    max_pcm_bytes: usize,
) -> Result<(DiskHandle, DiskSampler<N>), Box<dyn std::error::Error>> {
    if N == 0
        || N > 64
        || assets.is_empty()
        || assets.len() > 128
        || zones.is_empty()
        || zones.len() > 1024
    {
        return Err("disk sampler limits exceeded".into());
    }
    // Four pages per voice, two response pages per voice, immutable head per
    // asset, plus the worker/local page. This budget excludes decoder metadata.
    let bytes = (N * (CACHES + 2) + assets.len() + 2) * std::mem::size_of::<Page>();
    if bytes > max_pcm_bytes {
        return Err("disk PCM cache budget exceeded".into());
    }
    let mut metadata = Vec::with_capacity(assets.len());
    let mut readers = Vec::with_capacity(assets.len());
    for asset in &assets {
        let mut reader = hound::WavReader::open(&asset.path)?;
        let spec = reader.spec();
        if asset.root_note > 127
            || !(1..=2).contains(&spec.channels)
            || spec.sample_rate == 0
            || spec.sample_rate > 1_536_000
            || reader.duration() == 0
            || spec.bits_per_sample == 0
            || spec.bits_per_sample > 32
        {
            return Err("unsupported disk WAV".into());
        }
        let mut head = Page {
            asset: metadata.len(),
            start: 0,
            ..Page::default()
        };
        decode(&mut reader, 0, &mut head)?;
        metadata.push(Metadata {
            frames: reader.duration() as u64,
            rate: spec.sample_rate,
            head,
            root: asset.root_note,
        });
        readers.push(reader);
    }
    for z in &zones {
        let Some(m) = metadata.get(z.asset) else {
            return Err("unknown disk asset".into());
        };
        if z.keys[0] > z.keys[1]
            || z.keys[1] > 127
            || z.velocities[0] == 0
            || z.velocities[0] > z.velocities[1]
            || z.velocities[1] > 127
            || !z.gain.is_finite()
            || !(0.0..=8.0).contains(&z.gain)
            || z.loop_frames
                .is_some_and(|(a, b)| a >= b || b > m.frames || z.one_shot)
        {
            return Err("invalid disk zone".into());
        }
    }
    let (requests, mut receive) = rtrb::RingBuffer::<Request>::new(N * CACHES);
    let (mut send, responses) = rtrb::RingBuffer::new(N * 2);
    let stats = Arc::new(DiskStats::default());
    let worker_stats = stats.clone();
    let worker = thread::Builder::new()
        .name("soniclaw-disk-pages".into())
        .spawn(move || {
            while !worker_stats.stop.load(Ordering::Acquire) {
                let Ok(request) = receive.pop() else {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                };
                let mut page = Page {
                    slot: request.slot,
                    generation: request.generation,
                    asset: request.asset,
                    start: request.start,
                    ..Page::default()
                };
                if decode(&mut readers[request.asset], request.start, &mut page).is_err() {
                    page.data.fill(0.0);
                    worker_stats.io_errors.fetch_add(1, Ordering::Relaxed);
                }
                worker_stats.pages_read.fetch_add(1, Ordering::Relaxed);
                loop {
                    if worker_stats.stop.load(Ordering::Acquire) {
                        return;
                    }
                    match send.push(page) {
                        Ok(()) => {
                            worker_stats.published.fetch_add(1, Ordering::Release);
                            break;
                        }
                        Err(rtrb::PushError::Full(p)) => {
                            page = p;
                            thread::sleep(Duration::from_millis(1));
                        }
                    }
                }
            }
        })?;
    Ok((
        DiskHandle {
            worker: Some(worker),
            stats: stats.clone(),
            pcm_cache_bytes: bytes,
        },
        DiskSampler {
            metadata,
            zones,
            rules: Vec::new(),
            switches: [None; 16],
            known_switches: [false; 128],
            default_switch: None,
            voices: (0..N).map(|_| Voice::default()).collect(),
            requests,
            responses,
            stats,
            channels: ExpressionChannels::default(),
            sustain: [false; 16],
            rr: [0; 256],
            age: 0,
            sr: 48000.0,
            release: 1.0 / 2400.0,
            attack_ms: 5.0,
            release_ms: 50.0,
            quality: ResampleQuality::SincFast,
        },
    ))
}
impl<const N: usize> DiskSampler<N> {
    /// Control-thread configuration. Existing voices retain their attack and release settings.
    pub fn set_articulations(
        &mut self,
        rules: Vec<crate::ZoneRule>,
        default_switch: Option<u8>,
    ) -> Result<(), Error> {
        if self.active_voices() != 0 || rules.len() != self.zones.len() {
            return Err(Error::InvalidParameter);
        }
        let mut known = [false; 128];
        for r in &rules {
            if let Some(key) = r.keyswitch {
                let Some(x) = known.get_mut(key as usize) else {
                    return Err(Error::InvalidParameter);
                };
                *x = true;
            }
            if r.release_ms.is_some_and(|ms| !(1. ..=30000.).contains(&ms)) {
                return Err(Error::InvalidParameter);
            }
        }
        if default_switch.is_some_and(|k| !known.get(k as usize).copied().unwrap_or(false)) {
            return Err(Error::InvalidParameter);
        }
        self.rules = rules;
        self.switches = [default_switch; 16];
        self.default_switch = default_switch;
        self.known_switches = known;
        Ok(())
    }
    pub fn articulation(&self, channel: usize) -> Option<u8> {
        self.switches.get(channel).copied().flatten()
    }
    fn zone_enabled(&self, zone: usize, channel: u8) -> bool {
        self.rules
            .get(zone)
            .and_then(|r| r.keyswitch)
            .is_none_or(|key| Some(key) == self.switches[channel as usize])
    }
    pub fn set_envelope(&mut self, attack_ms: f32, release_ms: f32) -> Result<(), Error> {
        if !attack_ms.is_finite()
            || !(0.0..=10000.0).contains(&attack_ms)
            || !release_ms.is_finite()
            || !(1.0..=30000.0).contains(&release_ms)
        {
            return Err(Error::InvalidParameter);
        }
        self.attack_ms = attack_ms;
        self.release_ms = release_ms;
        Ok(())
    }
    pub fn set_quality(&mut self, quality: ResampleQuality) {
        self.quality = quality;
    }
    pub fn set_mpe_zone(&mut self, zone: Option<MpeZone>) -> Result<(), Error> {
        self.channels.zone = zone.map(MpeZone::validate).transpose()?;
        Ok(())
    }
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }
    fn drain(&mut self) {
        for _ in 0..N * 2 {
            let Ok(p) = self.responses.pop() else {
                break;
            };
            self.stats.consumed.fetch_add(1, Ordering::Release);
            let v = &mut self.voices[p.slot];
            if v.generation != p.generation || v.asset != p.asset {
                self.stats.stale_pages.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            for pending in &mut v.pending {
                if *pending == Some(p.start) {
                    *pending = None;
                }
            }
            v.pages[v.cursor] = p;
            v.cursor = (v.cursor + 1) % CACHES;
        }
    }
    fn request(
        voice: &mut Voice,
        slot: usize,
        start: u64,
        requests: &mut rtrb::Producer<Request>,
        stats: &DiskStats,
        metadata: &[Metadata],
    ) {
        if start >= metadata[voice.asset].frames
            || start == 0
            || voice
                .pages
                .iter()
                .any(|p| p.asset == voice.asset && p.start == start)
            || voice.pending.contains(&Some(start))
        {
            return;
        }
        let Some(pending) = voice.pending.iter_mut().find(|p| p.is_none()) else {
            return;
        };
        match requests.push(Request {
            slot,
            generation: voice.generation,
            asset: voice.asset,
            start,
        }) {
            Ok(()) => *pending = Some(start),
            Err(_) => {
                stats.request_overflows.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    fn prime(&mut self, slot: usize) {
        let v = &mut self.voices[slot];
        let page = (v.position.max(0.0) as u64 / PAGE as u64) * PAGE as u64;
        for start in [page, page.saturating_sub(PAGE as u64), page + PAGE as u64] {
            Self::request(
                v,
                slot,
                start,
                &mut self.requests,
                &self.stats,
                &self.metadata,
            );
        }
        if let Some((a, b)) = self.zones[v.zone].loop_frames {
            if b.saturating_sub(v.position as u64) < PAGE as u64 {
                Self::request(
                    v,
                    slot,
                    a / PAGE as u64 * PAGE as u64,
                    &mut self.requests,
                    &self.stats,
                    &self.metadata,
                );
            }
        }
    }
    fn start(&mut self, zone: usize, channel: u8, note: u8, velocity: f32, id: i32) {
        let slot = self
            .voices
            .iter()
            .position(|v| !v.active)
            .unwrap_or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| v.age)
                    .map(|x| x.0)
                    .unwrap_or(0)
            });
        let v = &mut self.voices[slot];
        v.generation = v.generation.wrapping_add(1);
        v.active = true;
        v.asset = self.zones[zone].asset;
        v.zone = zone;
        v.channel = channel;
        v.note = note;
        v.id = id;
        v.position = 0.0;
        v.increment = self.metadata[v.asset].rate as f64 / self.sr
            * 2.0_f64.powf((note as f64 - self.metadata[v.asset].root as f64) / 12.0);
        v.velocity = velocity * self.zones[zone].gain;
        v.level = if self.attack_ms == 0.0 { 1.0 } else { 0.0 };
        v.held = true;
        v.released = false;
        v.pending.fill(None);
        for p in &mut v.pages {
            p.asset = usize::MAX;
        }
        v.expression = VoiceExpression::default();
        v.filter = [0.0; 2];
        v.looped = false;
        self.age = self.age.wrapping_add(1);
        v.age = self.age;
        self.prime(slot);
    }
    fn release(&mut self, target: NoteTarget, choke: bool) {
        if !target.valid() {
            return;
        }
        for v in &mut self.voices {
            if v.active && target.matches(v.id, v.channel, v.note) {
                if choke {
                    v.active = false;
                } else if !self.zones[v.zone].one_shot {
                    v.held = false;
                    if !self.sustain[v.channel as usize] {
                        v.released = true;
                    }
                }
            }
        }
    }
    fn message(&mut self, m: MidiMessage) {
        if self.channels.update(m) {
            return;
        }
        let (m, id) = if let MidiMessage::NoteOnId {
            channel,
            note,
            velocity,
            note_id,
        } = m
        {
            if !velocity.is_finite() || !(0.0..=1.0).contains(&velocity) || note_id < -1 {
                return;
            }
            (
                MidiMessage::NoteOn {
                    channel,
                    note,
                    velocity: (velocity * 127.0).round() as u8,
                },
                note_id,
            )
        } else {
            (m, -1)
        };
        match m {
            MidiMessage::NoteOn {
                channel,
                note,
                velocity,
            } if channel < 16 && note < 128 && velocity > 0 => {
                if self.known_switches[note as usize] {
                    self.switches[channel as usize] = Some(note);
                    return;
                }
                let mut counts = [0usize; 256];
                for (i, z) in self.zones.iter().enumerate() {
                    if self.zone_enabled(i, channel)
                        && (z.keys[0]..=z.keys[1]).contains(&note)
                        && (z.velocities[0]..=z.velocities[1]).contains(&velocity)
                    {
                        counts[z.group as usize] += 1;
                    }
                }
                let mut ordinal = [0usize; 256];
                for i in 0..self.zones.len() {
                    let z = &self.zones[i];
                    if !self.zone_enabled(i, channel)
                        || !(z.keys[0]..=z.keys[1]).contains(&note)
                        || !(z.velocities[0]..=z.velocities[1]).contains(&velocity)
                    {
                        continue;
                    }
                    let g = z.group as usize;
                    let selected = ordinal[g] == self.rr[g] as usize % counts[g];
                    ordinal[g] += 1;
                    if selected {
                        self.start(i, channel, note, velocity as f32 / 127.0, id);
                    }
                }
                for (g, c) in counts.iter().enumerate() {
                    if *c > 0 {
                        self.rr[g] = self.rr[g].wrapping_add(1);
                    }
                }
            }
            MidiMessage::NoteOff { channel, note, .. }
            | MidiMessage::NoteOn {
                channel,
                note,
                velocity: 0,
            } => self.release(
                NoteTarget {
                    note_id: -1,
                    channel: channel as i16,
                    key: note as i16,
                },
                false,
            ),
            MidiMessage::NoteOffId { target } => self.release(target, false),
            MidiMessage::NoteChoke { target } => self.release(target, true),
            MidiMessage::NoteExpression {
                target,
                expression,
                value,
            } => {
                if target.valid() {
                    for v in &mut self.voices {
                        if target.matches(v.id, v.channel, v.note) {
                            v.expression.set(expression, value);
                        }
                    }
                }
            }
            MidiMessage::PolyPressure {
                channel,
                note,
                value,
            } if value <= 127 => {
                for v in &mut self.voices {
                    if v.channel == channel && v.note == note {
                        v.expression.pressure = value as f32 / 127.0;
                    }
                }
            }
            MidiMessage::ControlChange {
                channel,
                controller: 64,
                value,
            } if channel < 16 => {
                self.sustain[channel as usize] = value >= 64;
                if value < 64 {
                    for v in &mut self.voices {
                        if v.channel == channel && !v.held {
                            v.released = true;
                        }
                    }
                }
            }
            MidiMessage::ControlChange {
                channel,
                controller: 120,
                ..
            } => self.release(
                NoteTarget {
                    note_id: -1,
                    channel: channel as i16,
                    key: -1,
                },
                true,
            ),
            MidiMessage::ControlChange {
                channel,
                controller: 123,
                ..
            } => self.release(
                NoteTarget {
                    note_id: -1,
                    channel: channel as i16,
                    key: -1,
                },
                false,
            ),
            MidiMessage::AllNotesOff => self.reset(),
            _ => {}
        }
    }
    fn seek_slot(&mut self, slot: usize, frame: u64) -> Result<(), Error> {
        let v = &mut self.voices[slot];
        if frame > self.metadata[v.asset].frames {
            return Err(Error::InvalidParameter);
        }
        v.position = frame as f64;
        v.looped = self.zones[v.zone]
            .loop_frames
            .is_some_and(|(a, b)| frame >= a && frame < b);
        v.generation = v.generation.wrapping_add(1);
        v.pending.fill(None);
        for p in &mut v.pages {
            p.asset = usize::MAX;
        }
        self.prime(slot);
        Ok(())
    }
}
impl<const N: usize> AudioProcessor for DiskSampler<N> {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        self.sr = s.sample_rate;
        self.release = 1.0 / (s.sample_rate as f32 * self.release_ms * 0.001).max(1.0);
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        self.drain();
        for slot in 0..N {
            if self.voices[slot].active {
                self.prime(slot);
            }
        }
        let ch = b.channels();
        let mut event = 0;
        for (f, out) in b.samples_mut().chunks_exact_mut(ch).enumerate() {
            while event < ctx.events.len() && ctx.events[event].frame == f {
                self.message(ctx.events[event].message);
                event += 1;
            }
            let mut sum = [0.0; 2];
            for slot in 0..N {
                if !self.voices[slot].active {
                    continue;
                }
                self.prime(slot);
                let v = &mut self.voices[slot];
                let meta = &self.metadata[v.asset];
                let zone = &self.zones[v.zone];
                if let Some((a, z)) = zone.loop_frames {
                    if !v.released && v.position >= z as f64 {
                        v.looped = true;
                        v.position = a as f64 + (v.position - a as f64) % (z - a) as f64;
                    }
                }
                if v.position >= meta.frames as f64 {
                    v.active = false;
                    continue;
                }
                let (pitch, pressure, timbre) = self.channels.values(v.channel);
                let increment = (v.increment
                    * 2.0_f64.powf((pitch + v.expression.tuning) as f64 / 12.0))
                .clamp(1.0 / 32.0, 32.0);
                let mut missing = false;
                let mut signal = [0.0; 2];
                for (c, dest) in signal.iter_mut().enumerate() {
                    let mut x = self.quality.interpolate(v.position, increment, |i| {
                        let i = if let Some((a, z)) = zone.loop_frames {
                            if !v.released && (i >= z as isize || (v.looped && i < a as isize)) {
                                a as isize + (i - a as isize).rem_euclid((z - a) as isize)
                            } else {
                                i
                            }
                        } else {
                            i
                        };
                        if i < 0 || i as u64 >= meta.frames {
                            return 0.0;
                        }
                        let start = (i as u64 / PAGE as u64) * PAGE as u64;
                        let index = (i as u64 - start) as usize * 2 + c;
                        if start == 0 {
                            return meta.head.data[index];
                        }
                        if let Some(page) = v
                            .pages
                            .iter()
                            .find(|p| p.asset == v.asset && p.start == start)
                        {
                            page.data[index]
                        } else {
                            missing = true;
                            Self::request(
                                v,
                                slot,
                                start,
                                &mut self.requests,
                                &self.stats,
                                &self.metadata,
                            );
                            0.0
                        }
                    });
                    let brightness = (timbre + v.expression.timbre - 0.5).clamp(0.0, 1.0);
                    if (brightness - 0.5).abs() > 0.001 {
                        let cutoff =
                            (200.0 * 100.0_f32.powf(brightness)).min(self.sr as f32 * 0.45);
                        let pole = (-std::f32::consts::TAU * cutoff / self.sr as f32).exp();
                        v.filter[c] = (1.0 - pole) * x + pole * v.filter[c];
                        x = v.filter[c];
                    } else {
                        v.filter[c] = x;
                    }
                    *dest = x
                        * v.velocity
                        * v.level
                        * v.expression.volume
                        * (1.0 + pressure.max(v.expression.pressure) * 0.5)
                        * if c == 0 {
                            (1.0 - v.expression.pan) * 2.0
                        } else {
                            v.expression.pan * 2.0
                        };
                }
                if missing {
                    self.stats.underrun_frames.fetch_add(1, Ordering::Relaxed);
                } else {
                    sum[0] += signal[0];
                    sum[1] += signal[1];
                }
                v.position += increment;
                v.level = if v.released {
                    let release = self
                        .rules
                        .get(v.zone)
                        .and_then(|r| r.release_ms)
                        .map_or(self.release, |ms| {
                            1. / (self.sr as f32 * ms * 0.001).max(1.)
                        });
                    (v.level - release).max(0.0)
                } else {
                    (v.level + 1.0 / (self.sr as f32 * self.attack_ms * 0.001).max(1.0)).min(1.0)
                };
                if v.released && v.level == 0.0 {
                    v.active = false;
                }
            }
            out.fill((sum[0] + sum[1]) * 0.5);
            if ch >= 2 {
                out[0] = sum[0];
                out[1] = sum[1];
            }
        }
    }
    fn reset(&mut self) {
        self.switches = [self.default_switch; 16];
        for v in &mut self.voices {
            v.active = false;
            v.generation = v.generation.wrapping_add(1);
            v.pending.fill(None);
        }
        self.channels.reset();
        self.sustain.fill(false);
        self.rr.fill(0);
    }
}
/// Audio-owner transport commands are nonblocking. Seek advances the cache
/// generation; late pages cannot restore audio from the previous position.
pub struct SeekablePlayer {
    sampler: DiskSampler<1>,
    playing: bool,
}
pub fn open_seekable_wav(
    path: impl AsRef<Path>,
    max_pcm_bytes: usize,
) -> Result<(DiskHandle, SeekablePlayer), Box<dyn std::error::Error>> {
    let (handle, mut sampler) = open_disk_sampler::<1>(
        vec![DiskAsset {
            path: path.as_ref().into(),
            root_note: 60,
        }],
        vec![DiskZone::single(0, 60)],
        max_pcm_bytes,
    )?;
    sampler.set_quality(ResampleQuality::Linear);
    Ok((
        handle,
        SeekablePlayer {
            sampler,
            playing: true,
        },
    ))
}
impl SeekablePlayer {
    /// Retry bounded prefetch requests after a seek or response-queue drain. No disk I/O.
    pub fn prefetch(&mut self) {
        self.sampler.prime(0);
        let page = (self.sampler.voices[0].position.max(0.) as u64 / 1024) * 1024;
        DiskSampler::<1>::request(
            &mut self.sampler.voices[0],
            0,
            page + 2048,
            &mut self.sampler.requests,
            &self.sampler.stats,
            &self.sampler.metadata,
        );
    }

    /// Count ready source frames without waiting or touching disk. Call after draining responses.
    pub fn ready_frames(&self, maximum: usize) -> usize {
        let v = &self.sampler.voices[0];
        let m = &self.sampler.metadata[v.asset];
        let mut ready = 0;
        for offset in 0..maximum.min(8192) {
            let mut frame = v.position as u64 + offset as u64;
            if let Some((a, b)) = self.sampler.zones[v.zone].loop_frames {
                if frame >= b {
                    frame = a + (frame - a) % (b - a);
                }
            }
            if frame >= m.frames {
                ready += 1;
                continue;
            }
            if frame < 1024
                || v.pages.iter().any(|p| {
                    p.asset == v.asset
                        && p.generation == v.generation
                        && frame >= p.start
                        && frame < p.start + 1024
                })
            {
                ready += 1;
            } else {
                break;
            }
        }
        ready
    }

    pub fn position(&self) -> u64 {
        self.sampler.voices[0].position as u64
    }
    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }
    pub fn seek(&mut self, frame: u64) -> Result<(), Error> {
        self.sampler.seek_slot(0, frame)?;
        self.sampler.voices[0].active = true;
        Ok(())
    }
    pub fn set_loop(&mut self, range: Option<(u64, u64)>) -> Result<(), Error> {
        if range.is_some_and(|(a, b)| a >= b || b > self.sampler.metadata[0].frames) {
            return Err(Error::InvalidParameter);
        }
        self.sampler.zones[0].loop_frames = range;
        self.sampler.zones[0].one_shot = range.is_none();
        Ok(())
    }
}
impl AudioProcessor for SeekablePlayer {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        if (s.sample_rate - self.sampler.metadata[0].rate as f64).abs() > 0.001 {
            return Err(Error::InvalidSpec);
        }
        self.sampler.prepare(s)?;
        self.sampler.start(0, 0, 60, 1.0, -1);
        self.sampler.voices[0].level = 1.0;
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        if self.playing {
            self.sampler.process(
                b,
                &ProcessContext {
                    events: &[],
                    transport: Transport::default(),
                },
            );
        } else {
            self.sampler.drain();
            b.clear();
        }
    }
    fn reset(&mut self) {
        let _ = self.seek(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("soniclaw-{name}-{}.wav", std::process::id()));
        let mut w = hound::WavWriter::create(
            &p,
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for i in 0..8192 {
            w.write_sample(if i < 4096 { 0.25_f32 } else { -0.5 })
                .unwrap();
        }
        w.finalize().unwrap();
        p
    }
    fn ctx() -> ProcessContext<'static> {
        ProcessContext {
            events: &[],
            transport: Transport::default(),
        }
    }
    fn settle(handle: &DiskHandle, player: &mut SeekablePlayer) {
        for _ in 0..6 {
            let _ = handle.wait_ready(1, Duration::from_millis(100));
            player.process(&mut AudioBuffer::new(&mut [], 1).unwrap(), &ctx());
        }
    }
    #[test]
    fn seek_generations_pause_and_loop() {
        let p = fixture("seek");
        let (mut h, mut s) = open_seekable_wav(&p, 1024 * 1024).unwrap();
        s.prepare(ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 128,
            channels: 1,
        })
        .unwrap();
        s.seek(5000).unwrap();
        s.seek(2200).unwrap();
        settle(&h, &mut s);
        let mut out = [0.0; 128];
        s.process(&mut AudioBuffer::new(&mut out, 1).unwrap(), &ctx());
        assert!(out.iter().all(|x| *x == 0.25));
        s.set_playing(false);
        let pos = s.position();
        s.process(&mut AudioBuffer::new(&mut out, 1).unwrap(), &ctx());
        assert_eq!(pos, s.position());
        assert!(out.iter().all(|x| *x == 0.0));
        s.set_loop(Some((2200, 2220))).unwrap();
        s.seek(2200).unwrap();
        settle(&h, &mut s);
        s.set_playing(true);
        s.process(&mut AudioBuffer::new(&mut out, 1).unwrap(), &ctx());
        assert!(out.iter().all(|x| *x == 0.25));
        assert_eq!(s.position(), 2208);
        assert!(s.set_loop(Some((100, 50))).is_err());
        assert!(s.seek(9000).is_err());
        h.stop();
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn cache_budget_and_underrun_recovery() {
        let p = fixture("budget");
        let assets = || {
            vec![DiskAsset {
                path: p.clone(),
                root_note: 60,
            }]
        };
        assert!(open_disk_sampler::<32>(assets(), vec![DiskZone::single(0, 60)], 1024).is_err());
        let (mut h, mut s) = open_seekable_wav(&p, 1024 * 1024).unwrap();
        s.prepare(ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 128,
            channels: 1,
        })
        .unwrap();
        h.stop();
        s.seek(5000).unwrap(); // Stopped worker makes the cache miss deterministic.
        let mut out = [0.0; 128];
        s.process(&mut AudioBuffer::new(&mut out, 1).unwrap(), &ctx());
        assert!(out.iter().all(|v| *v == 0.0));
        assert!(h.stats.underrun_frames.load(Ordering::Relaxed) > 0);
        let (mut h, mut s) = open_seekable_wav(&p, 1024 * 1024).unwrap();
        s.prepare(ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 128,
            channels: 1,
        })
        .unwrap();
        s.seek(5000).unwrap();
        settle(&h, &mut s);
        s.process(&mut AudioBuffer::new(&mut out, 1).unwrap(), &ctx());
        assert!(out.iter().all(|v| *v == -0.5));
        assert_eq!(h.stats.io_errors.load(Ordering::Relaxed), 0);
        h.stop();
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn disk_voices_layer_and_obey_note_ids() {
        let p = fixture("voices");
        let (mut h, mut s) = open_disk_sampler::<4>(
            vec![DiskAsset {
                path: p.clone(),
                root_note: 60,
            }],
            vec![DiskZone::single(0, 60)],
            1024 * 1024,
        )
        .unwrap();
        s.set_quality(ResampleQuality::Linear);
        s.prepare(ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 512,
            channels: 2,
        })
        .unwrap();
        let on = |id| Event {
            frame: 0,
            message: MidiMessage::NoteOnId {
                channel: 1,
                note: 60,
                velocity: 1.0,
                note_id: id,
            },
        };
        let mut out = [0.0; 1024];
        s.process(
            &mut AudioBuffer::new(&mut out, 2).unwrap(),
            &ProcessContext {
                events: &[on(1), on(2)],
                transport: Transport::default(),
            },
        );
        assert_eq!(s.active_voices(), 2);
        assert!((out[1000] - 0.5).abs() < 1e-6);
        s.process(
            &mut AudioBuffer::new(&mut out, 2).unwrap(),
            &ProcessContext {
                events: &[Event {
                    frame: 0,
                    message: MidiMessage::NoteChoke {
                        target: NoteTarget {
                            note_id: 1,
                            channel: -1,
                            key: -1,
                        },
                    },
                }],
                transport: Transport::default(),
            },
        );
        assert_eq!(s.active_voices(), 1);
        h.stop();
        std::fs::remove_file(p).unwrap();
    }
}
