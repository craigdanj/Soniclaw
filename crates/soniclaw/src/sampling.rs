//! Bounded control-thread SFZ subset import and energy-onset slicing.
use crate::{io::AudioFile, state::*};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
};
#[derive(Clone, Copy, Debug)]
pub struct TransientOptions {
    pub threshold: f32,
    pub ratio: f32,
    pub minimum_gap_ms: f32,
    pub pre_roll_ms: f32,
    pub max_slices: usize,
}
impl Default for TransientOptions {
    fn default() -> Self {
        Self {
            threshold: 0.02,
            ratio: 3.0,
            minimum_gap_ms: 40.0,
            pre_roll_ms: 1.0,
            max_slices: 64,
        }
    }
}
/// One-millisecond mean-absolute energy windows compared with a decaying baseline.
/// Returns region starts including zero; repeated nearby peaks are suppressed.
pub fn detect_transients(audio: &AudioFile, o: TransientOptions) -> Result<Vec<usize>, StateError> {
    if audio.sample_rate == 0
        || audio.channels == 0
        || audio.samples.is_empty()
        || !audio.samples.len().is_multiple_of(audio.channels as usize)
        || audio.samples.iter().any(|x| !x.is_finite())
        || !o.threshold.is_finite()
        || !(0.0..=1.0).contains(&o.threshold)
        || !o.ratio.is_finite()
        || !(1.1..=100.0).contains(&o.ratio)
        || !o.minimum_gap_ms.is_finite()
        || !(5.0..=5000.0).contains(&o.minimum_gap_ms)
        || !o.pre_roll_ms.is_finite()
        || !(0.0..=100.0).contains(&o.pre_roll_ms)
        || !(1..=128).contains(&o.max_slices)
    {
        return Err("invalid transient detection input/options".into());
    }
    let ch = audio.channels as usize;
    let window = (audio.sample_rate as usize / 1000).max(1);
    let gap = (audio.sample_rate as f32 * o.minimum_gap_ms * 0.001) as usize;
    let pre = (audio.sample_rate as f32 * o.pre_roll_ms * 0.001) as usize;
    let mut markers = vec![0];
    let mut baseline = 0.0_f32;
    let mut armed = true;
    for (i, block) in audio.samples.chunks(window * ch).enumerate() {
        let energy = block.iter().map(|x| x.abs()).sum::<f32>() / block.len() as f32;
        let frame = (i * window).saturating_sub(pre);
        if armed
            && energy >= o.threshold
            && energy > baseline.max(0.000001) * o.ratio
            && frame.saturating_sub(*markers.last().unwrap()) >= gap
            && markers.len() < o.max_slices
        {
            markers.push(frame);
            armed = false;
        }
        if energy < baseline * 0.7 || energy < o.threshold * 0.5 {
            armed = true;
        }
        baseline = baseline * 0.8 + energy * 0.2;
    }
    Ok(markers)
}
pub fn sliced_zones(
    asset: &str,
    markers: &[usize],
    frames: usize,
    first_key: u8,
) -> Result<Vec<SampleZone>, StateError> {
    if markers.is_empty()
        || markers[0] != 0
        || markers.len() > 128 - first_key as usize
        || markers.windows(2).any(|w| w[0] >= w[1])
        || markers.last().is_some_and(|x| *x >= frames)
    {
        return Err("invalid slice markers/key range".into());
    }
    Ok(markers
        .iter()
        .enumerate()
        .map(|(i, start)| {
            let mut z = SampleZone::new(asset);
            z.keys = [first_key + i as u8; 2];
            z.start_frame = *start;
            z.end_frame = Some(markers.get(i + 1).copied().unwrap_or(frames));
            z.group = i as u16;
            z
        })
        .collect())
}
type Region = BTreeMap<String, String>;
const OPCODES: &[&str] = &[
    "sample",
    "key",
    "lokey",
    "hikey",
    "pitch_keycenter",
    "lovel",
    "hivel",
    "volume",
    "offset",
    "end",
    "loop_mode",
    "loop_start",
    "loop_end",
    "trigger",
    "xfin_lovel",
    "xfin_hivel",
    "xfout_lovel",
    "xfout_hivel",
    "xf_velcurve",
    "seq_length",
    "seq_position",
];
fn key_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}
fn regions(text: &str, extra: &[&str]) -> Result<Vec<Region>, StateError> {
    if text.len() > 1024 * 1024 {
        return Err("SFZ text exceeds 1 MiB".into());
    }
    let mut stripped = String::new();
    for line in text.lines() {
        if line.trim_start().starts_with('#') {
            return Err("SFZ preprocessor/includes are unsupported".into());
        }
        let mut quoted = false;
        let mut end = line.len();
        let bytes = line.as_bytes();
        for i in 0..bytes.len() {
            if bytes[i] == b'"' {
                quoted = !quoted;
            }
            if !quoted && bytes.get(i..i + 2) == Some(b"//") {
                end = i;
                break;
            }
        }
        stripped.push_str(&line[..end]);
        stripped.push('\n');
    }
    let b = stripped.as_bytes();
    let mut pos = 0;
    let mut global = Region::new();
    let mut group = Region::new();
    let mut region: Option<Region> = None;
    let mut scope = 0;
    let mut sequence_group = 0usize;
    let mut output = vec![];
    let mut tokens = 0;
    while pos < b.len() {
        while pos < b.len() && b[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos == b.len() {
            break;
        }
        tokens += 1;
        if tokens > 16384 {
            return Err("SFZ token limit".into());
        }
        if b[pos] == b'<' {
            if let Some(r) = region.take() {
                output.push(r);
            }
            let end = stripped[pos..]
                .find('>')
                .map(|x| pos + x)
                .ok_or("unterminated SFZ header")?;
            match &stripped[pos + 1..end] {
                "global" => {
                    global.clear();
                    group.clear();
                    scope = 0;
                }
                "group" => {
                    group = global.clone();
                    sequence_group += 1;
                    group.insert("__sequence_group".into(), sequence_group.to_string());
                    scope = 1;
                }
                "region" => {
                    let mut r = global.clone();
                    r.extend(group.clone());
                    region = Some(r);
                    scope = 2;
                }
                h => return Err(format!("unsupported SFZ header: {h}").into()),
            }
            pos = end + 1;
            continue;
        }
        let start = pos;
        while pos < b.len() && key_byte(b[pos]) {
            pos += 1;
        }
        let key = &stripped[start..pos];
        while pos < b.len() && b[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if key.is_empty() || b.get(pos) != Some(&b'=') {
            return Err("expected SFZ opcode=value".into());
        }
        if !OPCODES.contains(&key) && !extra.contains(&key) {
            return Err(format!("unsupported SFZ opcode: {key}").into());
        }
        pos += 1;
        while pos < b.len() && b[pos].is_ascii_whitespace() {
            pos += 1;
        }
        let value = if b.get(pos) == Some(&b'"') {
            pos += 1;
            let start = pos;
            while pos < b.len() && b[pos] != b'"' {
                pos += 1;
            }
            if pos == b.len() {
                return Err("unterminated SFZ quoted value".into());
            }
            let value = stripped[start..pos].to_owned();
            pos += 1;
            value
        } else {
            let start = pos;
            while pos < b.len() {
                if b[pos] == b'<' {
                    break;
                }
                if b[pos].is_ascii_whitespace() {
                    let mut next = pos;
                    while next < b.len() && b[next].is_ascii_whitespace() {
                        next += 1;
                    }
                    let mut end = next;
                    while end < b.len() && key_byte(b[end]) {
                        end += 1;
                    }
                    while end < b.len() && b[end].is_ascii_whitespace() {
                        end += 1;
                    }
                    if b.get(next) == Some(&b'<') || b.get(end) == Some(&b'=') {
                        break;
                    }
                }
                pos += 1;
            }
            stripped[start..pos].trim().to_owned()
        };
        if value.is_empty() || value.len() > 4096 {
            return Err("empty/oversized SFZ value".into());
        }
        match scope {
            0 => {
                global.insert(key.into(), value);
            }
            1 => {
                group.insert(key.into(), value);
            }
            _ => {
                region
                    .as_mut()
                    .ok_or("opcode outside region")?
                    .insert(key.into(), value);
            }
        }
        if output.len() > 255 {
            return Err("SFZ region limit is 255".into());
        }
    }
    if let Some(r) = region {
        output.push(r);
    }
    if output.is_empty() || output.len() > 255 {
        return Err("SFZ requires 1..255 regions".into());
    }
    Ok(output)
}
fn note(s: &str) -> Result<u8, StateError> {
    if let Ok(n) = s.parse::<u8>() {
        if n <= 127 {
            return Ok(n);
        }
    }
    let b = s.as_bytes();
    let base = match b.first().map(u8::to_ascii_uppercase) {
        Some(b'C') => 0,
        Some(b'D') => 2,
        Some(b'E') => 4,
        Some(b'F') => 5,
        Some(b'G') => 7,
        Some(b'A') => 9,
        Some(b'B') => 11,
        _ => return Err("invalid SFZ note".into()),
    };
    let mut i = 1;
    let accidental = match b.get(i) {
        Some(b'#') => {
            i += 1;
            1
        }
        Some(b'b') => {
            i += 1;
            -1
        }
        _ => 0,
    };
    let octave = s.get(i..).ok_or("invalid SFZ octave")?.parse::<i16>()?;
    let n = (octave + 1) * 12 + base + accidental;
    if !(0..=127).contains(&n) {
        return Err("SFZ note outside MIDI range".into());
    }
    Ok(n as u8)
}
fn number(r: &Region, k: &str, default: usize) -> Result<usize, StateError> {
    r.get(k)
        .map(|s| s.parse::<usize>().map_err(|e| -> StateError { e.into() }))
        .unwrap_or(Ok(default))
}
fn fade(r: &Region, lo: &str, hi: &str) -> Result<Option<[u8; 2]>, StateError> {
    if !r.contains_key(lo) && !r.contains_key(hi) {
        return Ok(None);
    }
    let a = number(r, lo, 0)?;
    let b = number(r, hi, 127)?;
    if a >= b || b > 127 {
        return Err("invalid SFZ velocity fade".into());
    }
    Ok(Some([a as u8, b as u8]))
}
/// Import only the documented SFZ subset; reject unsupported semantics explicitly.
/// Paths must remain inside the SFZ directory. No includes, embedded samples or URLs.
pub fn import_sfz(path: &Path, product: &str) -> Result<(ProjectState, PathBuf), StateError> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    let regions = regions(std::str::from_utf8(&bytes)?, &[])?;
    let root = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let mut state = ProjectState::empty(
        product,
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("SFZ instrument"),
    );
    let mut total = 0;
    let mut sequence_ids = BTreeMap::new();
    let mut sequence_members: BTreeMap<u16, Vec<(usize, usize, usize)>> = BTreeMap::new();
    for (i, r) in regions.iter().enumerate() {
        let sample = r
            .get("sample")
            .ok_or("SFZ region requires sample")?
            .replace('\\', "/");
        let key = r.get("key").map(|s| note(s)).transpose()?;
        let center = r
            .get("pitch_keycenter")
            .map(|s| note(s))
            .transpose()?
            .or(key)
            .unwrap_or(60);
        let asset = Asset {
            id: format!("sfz-{i}"),
            path: sample,
            root_note: center,
        };
        state.assets.push(asset.clone());
        state.validate(product)?;
        let data = crate::read_audio_with_limit(
            state.resolve_asset(&root, &asset)?,
            128 * 1024 * 1024 - total,
        )?;
        total += data.samples.len() * 4;
        let mut z = SampleZone::new(&asset.id);
        z.keys = [
            r.get("lokey")
                .map(|s| note(s))
                .transpose()?
                .or(key)
                .unwrap_or(0),
            r.get("hikey")
                .map(|s| note(s))
                .transpose()?
                .or(key)
                .unwrap_or(127),
        ];
        let lo = number(r, "lovel", 1)?;
        let hi = number(r, "hivel", 127)?;
        if lo > hi || lo == 0 || hi > 127 {
            return Err("invalid SFZ velocity range".into());
        }
        z.velocities = [lo as u8, hi as u8];
        z.group = i as u16;
        if r.contains_key("seq_length") || r.contains_key("seq_position") {
            let length = number(r, "seq_length", 1)?;
            let position = number(r, "seq_position", 1)?;
            if length == 0 || length > 255 || position == 0 || position > length {
                return Err("invalid SFZ sequence".into());
            }
            let key = (
                r.get("__sequence_group").cloned().unwrap_or_default(),
                z.keys,
                z.velocities,
                center,
            );
            let group = *sequence_ids.entry(key).or_insert(i as u16);
            z.group = group;
            sequence_members
                .entry(group)
                .or_default()
                .push((position, length, i));
        }
        let db = r
            .get("volume")
            .map(|s| s.parse::<f32>())
            .transpose()?
            .unwrap_or(0.0);
        if !db.is_finite() {
            return Err("invalid SFZ volume".into());
        }
        z.gain = 10.0_f32.powf(db / 20.0);
        z.start_frame = number(r, "offset", 0)?;
        z.end_frame = r
            .get("end")
            .map(|s| {
                s.parse::<usize>()
                    .map_err(|e| -> StateError { e.into() })
                    .and_then(|n| n.checked_add(1).ok_or_else(|| "SFZ end overflow".into()))
            })
            .transpose()?;
        z.one_shot = false;
        match r.get("loop_mode").map(String::as_str).unwrap_or("no_loop") {
            "no_loop" => {}
            "one_shot" => z.one_shot = true,
            "loop_sustain" => {
                let a = number(r, "loop_start", 0)?;
                let b = number(r, "loop_end", data.frames().saturating_sub(1))?
                    .checked_add(1)
                    .ok_or("loop end overflow")?;
                z.loop_frames = Some([a, b]);
            }
            v => return Err(format!("unsupported SFZ loop_mode: {v}").into()),
        }
        match r.get("trigger").map(String::as_str).unwrap_or("attack") {
            "attack" => {}
            "release" => {
                z.release_trigger = true;
                z.one_shot = true;
                z.loop_frames = None;
            }
            v => return Err(format!("unsupported SFZ trigger: {v}").into()),
        }
        z.velocity_fade_in = fade(r, "xfin_lovel", "xfin_hivel")?;
        z.velocity_fade_out = fade(r, "xfout_lovel", "xfout_hivel")?;
        z.velocity_power = match r.get("xf_velcurve").map(String::as_str).unwrap_or("power") {
            "power" => true,
            "gain" => false,
            v => return Err(format!("unsupported SFZ velocity curve: {v}").into()),
        };
        let end = z.end_frame.unwrap_or(data.frames());
        if z.start_frame >= end
            || end > data.frames()
            || z.loop_frames
                .is_some_and(|[a, b]| a < z.start_frame || a >= b || b > end)
        {
            return Err("SFZ region/loop exceeds WAV bounds".into());
        }
        state.zones.push(z);
    }
    for members in sequence_members.values_mut() {
        members.sort_by_key(|m| m.0);
        let length = members[0].1;
        if members.len() != length
            || members
                .iter()
                .enumerate()
                .any(|(i, m)| m.0 != i + 1 || m.1 != length)
        {
            return Err("SFZ sequence must contain each position exactly once".into());
        }
    }
    for members in sequence_members.values() {
        let mut slots: Vec<_> = members.iter().map(|m| m.2).collect();
        slots.sort_unstable();
        let sorted: Vec<_> = members.iter().map(|m| state.zones[m.2].clone()).collect();
        for (slot, zone) in slots.into_iter().zip(sorted) {
            state.zones[slot] = zone;
        }
    }
    state
        .nodes
        .push(RackNode::new("sampler", ProcessorKind::Sampler));
    state.outputs.insert("sampler".into(), 1.0);
    state.validate(product)?;
    Ok((state, root))
}

/// Convert a bounded SFZ articulation subset into a disk-paged instrument recipe.
/// The original in-memory SFZ importer retains its existing opcode contract.
pub fn import_streamed_sfz(path: &Path) -> Result<soniclaw_stream::DiskInstrument, StateError> {
    use soniclaw_stream::{DiskAsset, DiskInstrument, DiskZone, ZoneRule};
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    let extra = ["sw_last", "sw_default", "ampeg_release"];
    let regions = regions(std::str::from_utf8(&bytes)?, &extra)?;
    let root = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let mut assets = Vec::new();
    let mut zones = Vec::new();
    let mut rules = Vec::new();
    let mut default_switch = None;
    let supported = [
        "sample",
        "key",
        "lokey",
        "hikey",
        "pitch_keycenter",
        "lovel",
        "hivel",
        "volume",
        "loop_mode",
        "loop_start",
        "loop_end",
        "sw_last",
        "sw_default",
        "ampeg_release",
        "__sequence_group",
    ];
    for (i, r) in regions.iter().enumerate() {
        if let Some(k) = r.keys().find(|k| !supported.contains(&k.as_str())) {
            return Err(format!("unsupported streamed SFZ opcode: {k}").into());
        }
        let rel = r
            .get("sample")
            .ok_or("streamed SFZ region requires sample")?
            .replace('\\', "/");
        let file = root.join(rel).canonicalize()?;
        if !file.starts_with(&root)
            || !file.is_file()
            || !file
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("wav"))
        {
            return Err(
                "streamed SFZ sample must be a local WAV inside the instrument folder".into(),
            );
        }
        let opt_note = |k| r.get(k).map(|s| note(s)).transpose();
        let key = opt_note("key")?;
        let center = opt_note("pitch_keycenter")?.or(key).unwrap_or(60);
        let asset = assets
            .iter()
            .position(|a: &DiskAsset| a.path == file && a.root_note == center)
            .unwrap_or_else(|| {
                let n = assets.len();
                assets.push(DiskAsset {
                    path: file,
                    root_note: center,
                });
                n
            });
        if assets.len() > 128 {
            return Err("streamed SFZ asset limit is 128".into());
        }
        let keys = [
            opt_note("lokey")?.or(key).unwrap_or(0),
            opt_note("hikey")?.or(key).unwrap_or(127),
        ];
        let lo = number(r, "lovel", 1)?;
        let hi = number(r, "hivel", 127)?;
        if lo == 0 || lo > hi || hi > 127 || keys[0] > keys[1] {
            return Err("invalid streamed note range".into());
        }
        let mode = r.get("loop_mode").map(String::as_str).unwrap_or("no_loop");
        let loop_frames = match mode {
            "one_shot" | "no_loop" => {
                if r.contains_key("loop_start") || r.contains_key("loop_end") {
                    return Err("unused loop bounds".into());
                }
                None
            }
            "loop_sustain" => {
                let a = number(r, "loop_start", 0)? as u64;
                let b = number(r, "loop_end", 0)?
                    .checked_add(1)
                    .ok_or("streamed loop end overflow")? as u64;
                if !r.contains_key("loop_end") || a >= b {
                    return Err("invalid streamed loop".into());
                }
                Some((a, b))
            }
            _ => return Err("unsupported streamed SFZ loop mode".into()),
        };
        let db = r
            .get("volume")
            .map(|s| s.parse::<f32>())
            .transpose()?
            .unwrap_or(0.);
        let gain = 10_f32.powf(db / 20.);
        if !(0. ..=8.).contains(&gain) {
            return Err("invalid streamed volume".into());
        }
        zones.push(DiskZone {
            asset,
            keys,
            velocities: [lo as u8, hi as u8],
            group: i as u8,
            gain,
            one_shot: mode == "one_shot",
            loop_frames,
        });
        if let Some(k) = opt_note("sw_default")? {
            if default_switch.is_some_and(|current| current != k) {
                return Err("conflicting default keyswitches".into());
            }
            default_switch = Some(k);
        }
        let release_ms = r
            .get("ampeg_release")
            .map(|s| s.parse::<f32>().map(|v| v * 1000.))
            .transpose()?;
        if release_ms.is_some_and(|v| !(1. ..=30000.).contains(&v)) {
            return Err("invalid articulation release".into());
        }
        rules.push(ZoneRule {
            keyswitch: opt_note("sw_last")?,
            release_ms,
        });
    }
    if default_switch.is_some_and(|k| !rules.iter().any(|r| r.keyswitch == Some(k))) {
        return Err("unknown default keyswitch".into());
    }
    Ok(DiskInstrument {
        assets,
        zones,
        rules,
        default_switch,
    })
}
