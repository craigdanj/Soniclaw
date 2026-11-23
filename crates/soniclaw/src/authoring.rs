//! Blocking sample authoring. Sources are read-only; exports require a new directory.
use crate::{io::AudioFile, state::*};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
#[derive(Clone, Debug)]
pub struct Source {
    pub path: PathBuf,
    pub root: Option<u8>,
    pub velocity: Option<[u8; 2]>,
    pub round_robin: Option<u8>,
    pub trim: Option<[usize; 2]>,
    pub loop_frames: Option<[usize; 2]>,
}
impl Source {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            root: None,
            velocity: None,
            round_robin: None,
            trim: None,
            loop_frames: None,
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct PitchEstimate {
    pub hz: f64,
    pub midi_note: u8,
    pub confidence: f64,
}
/// Monophonic autocorrelation estimate. Ambiguous material requires manual assignment.
pub fn estimate_root(audio: &AudioFile) -> Result<PitchEstimate, StateError> {
    if audio.channels == 0
        || audio.sample_rate < 8000
        || audio.samples.len() > 32 * 1024 * 1024
        || audio.samples.iter().any(|v| !v.is_finite())
    {
        return Err("invalid pitch input".into());
    }
    let stride = (audio.sample_rate as usize / 16000).max(1);
    let rate = audio.sample_rate as f64 / stride as f64;
    let mut mono: Vec<f64> = audio
        .samples
        .chunks_exact(audio.channels as usize)
        .step_by(stride)
        .take((rate * 0.25) as usize)
        .map(|f| f.iter().map(|v| *v as f64).sum::<f64>() / audio.channels as f64)
        .collect();
    let max_lag = (rate / 30.) as usize;
    if mono.len() < max_lag * 2 {
        return Err("pitch estimation needs at least two 30 Hz periods".into());
    }
    let mean = mono.iter().sum::<f64>() / mono.len() as f64;
    for v in &mut mono {
        *v -= mean;
    }
    let mut best = (0, 0f64);
    let mut correlations = vec![0.; max_lag + 1];
    for lag in ((rate / 2000.) as usize).max(1)..=max_lag {
        let (mut dot, mut a, mut b) = (0., 0., 0.);
        for i in 0..mono.len() - max_lag {
            dot += mono[i] * mono[i + lag];
            a += mono[i] * mono[i];
            b += mono[i + lag] * mono[i + lag];
        }
        let score = dot / (a * b).sqrt().max(1e-30);
        correlations[lag] = score;
        if score > best.1 + 0.002 {
            best = (lag, score)
        }
    }
    if best.1 < 0.85 || best.0 == 0 {
        return Err("unreliable pitch; assign a root manually".into());
    }
    let correction = if best.0 > 0 && best.0 < max_lag {
        let (a, b, c) = (
            correlations[best.0 - 1],
            correlations[best.0],
            correlations[best.0 + 1],
        );
        let d = a - 2. * b + c;
        if d.abs() > 1e-12 {
            (0.5 * (a - c) / d).clamp(-1., 1.)
        } else {
            0.
        }
    } else {
        0.
    };
    let hz = rate / (best.0 as f64 + correction);
    let note = (69. + 12. * (hz / 440.).log2()).round();
    if !(0.0..=127.).contains(&note) {
        return Err("pitch outside MIDI range".into());
    }
    Ok(PitchEstimate {
        hz,
        midi_note: note as u8,
        confidence: best.1,
    })
}
/// Quietest mono-average sign crossing near a proposed trim/loop boundary.
pub fn nearest_zero_crossing(
    audio: &AudioFile,
    frame: usize,
    radius: usize,
) -> Result<usize, StateError> {
    if audio.channels == 0
        || !audio.samples.len().is_multiple_of(audio.channels as usize)
        || audio.samples.is_empty()
        || audio.samples.iter().any(|v| !v.is_finite())
        || frame >= audio.frames()
        || radius > 65536
    {
        return Err("invalid zero crossing search".into());
    }
    let mean = |i: usize| {
        audio.samples[i * audio.channels as usize..(i + 1) * audio.channels as usize]
            .iter()
            .map(|v| *v as f64)
            .sum::<f64>()
            / audio.channels as f64
    };
    let mut best = None;
    for i in
        frame.saturating_sub(radius).max(1)..=frame.saturating_add(radius).min(audio.frames() - 1)
    {
        let (a, b) = (mean(i - 1), mean(i));
        if a.signum() != b.signum() || b == 0. {
            let cost = b.abs() + i.abs_diff(frame) as f64 * 1e-12;
            if best.is_none_or(|(_, v)| cost < v) {
                best = Some((i, cost));
            }
        }
    }
    best.map(|(i, _)| i)
        .ok_or_else(|| "no crossing in window".into())
}
type Tokens = (Option<u8>, Option<[u8; 2]>, Option<u8>);
fn filename(path: &Path) -> Result<Tokens, StateError> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("invalid filename")?;
    let (mut root, mut vel, mut rr) = (None, None, None);
    for token in stem.split('_') {
        if let Some(n) = token
            .strip_prefix('n')
            .filter(|s| s.bytes().next().is_some_and(|b| b.is_ascii_digit()))
        {
            if root.is_some() {
                return Err("duplicate root token".into());
            }
            root = Some(n.parse()?);
        }
        if let Some(v) = token
            .strip_prefix('v')
            .filter(|s| s.bytes().next().is_some_and(|b| b.is_ascii_digit()))
        {
            let (a, b) = v.split_once('-').ok_or("use v1-127 velocity token")?;
            if vel.is_some() {
                return Err("duplicate velocity token".into());
            }
            vel = Some([a.parse()?, b.parse()?]);
        }
        if let Some(v) = token
            .strip_prefix("rr")
            .filter(|s| s.bytes().next().is_some_and(|b| b.is_ascii_digit()))
        {
            if rr.is_some() {
                return Err("duplicate RR token".into());
            }
            rr = Some(v.parse()?);
        }
    }
    Ok((root, vel, rr))
}
#[derive(Debug, Serialize)]
pub struct ExportReport {
    pub schema: u32,
    pub product: String,
    pub files: BTreeMap<String, String>,
    pub decoded_pcm_bytes: usize,
    pub regions: usize,
    pub estimates: Vec<Option<PitchEstimate>>,
}
struct Loaded {
    audio: AudioFile,
    root: u8,
    vel: [u8; 2],
    rr: u8,
    trim: [usize; 2],
    loop_frames: Option<[usize; 2]>,
    estimate: Option<PitchEstimate>,
}
/// Normalized PCM16 copies, native project, SFZ and SHA-256 receipt. Existing output is refused.
pub fn export(
    sources: &[Source],
    destination: &Path,
    product: &str,
    peak: f32,
) -> Result<ExportReport, StateError> {
    use sha2::{Digest, Sha256};
    if sources.is_empty()
        || sources.len() > 255
        || !peak.is_finite()
        || !(0.01..=1.).contains(&peak)
    {
        return Err("invalid authoring request".into());
    }
    let mut loaded = Vec::new();
    let mut total = 0;
    for s in sources {
        let mut audio = crate::io::read_wav_with_limit(&s.path, 128 * 1024 * 1024 - total)?;
        if audio.channels > 2 || audio.samples.is_empty() {
            return Err("expected nonempty mono/stereo WAV".into());
        }
        total += audio.samples.len() * 4;
        let (n, v, r) = filename(&s.path)?;
        let estimate = if s.root.or(n).is_none() {
            Some(estimate_root(&audio)?)
        } else {
            None
        };
        let root = s.root.or(n).or(estimate.map(|e| e.midi_note)).unwrap();
        let vel = s.velocity.or(v).unwrap_or([1, 127]);
        let rr = s.round_robin.or(r).unwrap_or(1);
        let trim = s.trim.unwrap_or([0, audio.frames()]);
        if root > 127
            || vel[0] == 0
            || vel[0] > vel[1]
            || vel[1] > 127
            || rr == 0
            || trim[0] >= trim[1]
            || trim[1] > audio.frames()
            || s.loop_frames
                .is_some_and(|[a, b]| a < trim[0] || a >= b || b > trim[1])
        {
            return Err("invalid zone/source bounds".into());
        }
        let max = audio.samples
            [trim[0] * audio.channels as usize..trim[1] * audio.channels as usize]
            .iter()
            .map(|v| v.abs())
            .fold(0f32, f32::max);
        if max > 0. {
            let gain = peak as f64 / max as f64;
            for v in &mut audio.samples {
                *v = (*v as f64 * gain).clamp(-1., 1.) as f32;
            }
        }
        loaded.push(Loaded {
            audio,
            root,
            vel,
            rr,
            trim,
            loop_frames: s.loop_frames,
            estimate,
        });
    }
    loaded.sort_by_key(|s| (s.root, s.vel, s.rr));
    let roots: Vec<_> = loaded
        .iter()
        .map(|s| s.root)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut groups: BTreeMap<(u8, [u8; 2]), Vec<usize>> = BTreeMap::new();
    for (i, s) in loaded.iter().enumerate() {
        groups.entry((s.root, s.vel)).or_default().push(i);
    }
    for ((root, vel), indices) in &groups {
        for (i, index) in indices.iter().enumerate() {
            if loaded[*index].rr as usize != i + 1 {
                return Err("RR alternatives must be unique/contiguous from rr1".into());
            }
        }
        if groups
            .keys()
            .any(|(r, v)| r == root && v != vel && v[0] <= vel[1] && vel[0] <= v[1])
        {
            return Err("overlapping velocity layers".into());
        }
    }
    let mut state = ProjectState::empty(product, "Authored instrument");
    let mut sfz = String::new();
    for (g, ((root, vel), indices)) in groups.iter().enumerate() {
        let p = roots.iter().position(|r| r == root).unwrap();
        let low = if p == 0 {
            0
        } else {
            (roots[p - 1] as u16 + *root as u16) / 2 + 1
        };
        let high = if p + 1 == roots.len() {
            127
        } else {
            (*root as u16 + roots[p + 1] as u16) / 2
        };
        sfz.push_str(&format!("<group> seq_length={}\n", indices.len()));
        for (index, i) in indices.iter().enumerate() {
            let s = &loaded[*i];
            let id = format!("sample-{i:03}");
            let path = format!("{id}.wav");
            state.assets.push(Asset {
                id: id.clone(),
                path: path.clone(),
                root_note: *root,
            });
            let mut z = SampleZone::new(&id);
            z.keys = [low as u8, high as u8];
            z.velocities = *vel;
            z.group = g as u16;
            z.one_shot = false;
            z.start_frame = s.trim[0];
            z.end_frame = Some(s.trim[1]);
            z.loop_frames = s.loop_frames;
            state.zones.push(z);
            sfz.push_str(&format!("<region> sample={path} pitch_keycenter={root} lokey={low} hikey={high} lovel={} hivel={} seq_position={} offset={} end={} ",vel[0],vel[1],index+1,s.trim[0],s.trim[1]-1));
            if let Some([a, b]) = s.loop_frames {
                sfz.push_str(&format!(
                    "loop_mode=loop_sustain loop_start={a} loop_end={}\n",
                    b - 1
                ));
            } else {
                sfz.push_str("loop_mode=no_loop\n");
            }
        }
    }
    state
        .nodes
        .push(RackNode::new("sampler", ProcessorKind::Sampler));
    state.outputs.insert("sampler".into(), 1.);
    let json = state.to_json()?;
    std::fs::create_dir(destination)?;
    let result = (|| {
        let mut files = BTreeMap::new();
        for (i, s) in loaded.iter().enumerate() {
            crate::io::write_wav(
                destination.join(format!("sample-{i:03}.wav")),
                s.audio.sample_rate,
                s.audio.channels,
                &s.audio.samples,
            )?;
        }
        std::fs::write(destination.join("instrument.json"), json)?;
        std::fs::write(destination.join("instrument.sfz"), sfz)?;
        for entry in std::fs::read_dir(destination)? {
            let p = entry?.path();
            files.insert(
                p.file_name().unwrap().to_string_lossy().into_owned(),
                format!("{:x}", Sha256::digest(std::fs::read(&p)?)),
            );
        }
        let report = ExportReport {
            schema: 1,
            product: product.into(),
            files,
            decoded_pcm_bytes: total,
            regions: sources.len(),
            estimates: loaded.iter().map(|s| s.estimate).collect(),
        };
        std::fs::write(
            destination.join("manifest.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        Ok(report)
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(destination);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pitch_crossing_tokens() {
        let audio = AudioFile {
            sample_rate: 48000,
            channels: 1,
            samples: (0..12000)
                .map(|i| (i as f32 * std::f32::consts::TAU * 440. / 48000.).sin() * 0.2)
                .collect(),
        };
        let p = estimate_root(&audio).unwrap();
        assert_eq!(p.midi_note, 69);
        assert!((p.hz - 440.).abs() < 3.);
        let f = nearest_zero_crossing(&audio, 100, 20).unwrap();
        assert!(f.abs_diff(109) <= 1);
        assert!(estimate_root(&AudioFile {
            samples: vec![0.; 12000],
            ..audio
        })
        .is_err());
        assert_eq!(
            filename(Path::new("Piano_n60_v1-80_rr2.wav")).unwrap(),
            (Some(60), Some([1, 80]), Some(2))
        );
        assert!(filename(Path::new("n60_n64.wav")).is_err());
    }
    #[test]
    fn export_roundtrip_rr_and_no_overwrite() {
        let root = std::env::temp_dir().join(format!(
            "soniclaw-authoring-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let pcm: Vec<_> = (0..12000)
            .map(|i| (i as f32 * 440. * std::f32::consts::TAU / 48000.).sin() * 0.1)
            .collect();
        let sources: Vec<_> = (1..=2)
            .map(|rr| {
                let p = root.join(format!("s_n69_v1-127_rr{rr}.wav"));
                crate::io::write_wav(&p, 48000, 1, &pcm).unwrap();
                Source::new(p)
            })
            .collect();
        let out = root.join("export");
        let report = export(&sources, &out, "test", 0.8).unwrap();
        assert_eq!(report.regions, 2);
        let native =
            ProjectState::from_json(&std::fs::read_to_string(out.join("instrument.json")).unwrap())
                .unwrap();
        let (imported, _) =
            crate::sampling::import_sfz(&out.join("instrument.sfz"), "test").unwrap();
        assert_eq!(native.zones[0].group, native.zones[1].group);
        assert_eq!(imported.zones[0].group, imported.zones[1].group);
        assert_eq!(native.zones[0].keys, imported.zones[0].keys);
        let wav = crate::io::read_wav(out.join("sample-000.wav")).unwrap();
        assert!((wav.samples.iter().map(|v| v.abs()).fold(0f32, f32::max) - 0.8).abs() < 0.0001);
        assert!(export(&sources, &out, "test", 0.8).is_err());
        assert!(out.join("manifest.json").exists());
        let invalid = [Source {
            round_robin: Some(2),
            ..sources[0].clone()
        }];
        assert!(export(&invalid, &root.join("bad"), "test", 0.8).is_err());
        assert!(!root.join("bad").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
