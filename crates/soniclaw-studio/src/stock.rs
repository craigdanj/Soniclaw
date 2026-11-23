//! Stock processor catalog and asset-aware factory. All construction belongs off callback.
use crate::{
    project::{Project, ProjectError},
    *,
};
use soniclaw_dsp::*;
pub const EFFECTS: &[(&str, EffectKind)] = &[
    ("gain", EffectKind::Gain),
    ("eq", EffectKind::Eq),
    ("lowpass", EffectKind::LowPass),
    ("compressor", EffectKind::Compressor),
    ("gate", EffectKind::Gate),
    ("limiter", EffectKind::Limiter),
    ("chorus", EffectKind::Chorus),
    ("reverb", EffectKind::Reverb),
    ("delay", EffectKind::Delay),
    ("drive", EffectKind::Drive),
    ("stereo-width", EffectKind::StereoWidth),
    ("state-variable", EffectKind::StateVariable),
    ("ladder", EffectKind::Ladder),
    ("multiband", EffectKind::Multiband),
    ("dynamic-eq", EffectKind::DynamicEq),
    ("rms-compressor", EffectKind::RmsCompressor),
    ("lookahead-limiter", EffectKind::LookaheadLimiter),
    ("transient-shaper", EffectKind::TransientShaper),
    ("spectral-gate", EffectKind::SpectralGate),
    ("fdn-reverb", EffectKind::FdnReverb),
    ("phaser", EffectKind::Phaser),
    ("flanger", EffectKind::Flanger),
    ("tremolo", EffectKind::Tremolo),
    ("auto-pan", EffectKind::AutoPan),
    ("multi-tap-delay", EffectKind::MultiTapDelay),
    ("pitch-shift", EffectKind::PitchShift),
    ("frequency-shift", EffectKind::FrequencyShift),
    ("spectral-freeze", EffectKind::SpectralFreeze),
    ("robotisation", EffectKind::Robotisation),
    ("crossover-low", EffectKind::CrossoverLow),
    ("crossover-mid", EffectKind::CrossoverMid),
    ("crossover-high", EffectKind::CrossoverHigh),
];
pub const INSTRUMENTS: &[&str] = &["Sine keys", "Saw bass", "Square lead", "Slow pad"];
pub struct AssetsFactory<'a> {
    pub assets: &'a Assets,
}
impl crate::Factory for AssetsFactory<'_> {
    fn create(&mut self, d: &Descriptor) -> Result<Built, Error> {
        if d.kind == "stock.sampler" {
            build(d, Some(self.assets))
        } else {
            Builtins.create(d)
        }
    }
}
fn number(d: &Descriptor, key: &str, default: f32) -> Result<f32, Error> {
    d.state.get(key).map_or(Ok(default), |v| {
        v.as_f64()
            .filter(|v| v.is_finite())
            .map(|v| v as f32)
            .ok_or(Error::InvalidParameter)
    })
}
pub(crate) fn build(d: &Descriptor, assets: Option<&Assets>) -> Result<Built, Error> {
    let object = d.state.as_object().ok_or(Error::InvalidParameter)?;
    let allowed: &[&str] = match d.kind.as_str() {
        "stock.fx" => &["effect", "values"],
        "stock.synth" => &["waveform", "attack", "decay", "sustain", "release"],
        "stock.sampler" => &[
            "asset",
            "root",
            "one_shot",
            "loop",
            "loop_crossfade",
            "attack_ms",
            "release_ms",
            "bend_range",
        ],
        _ => return Err(Error::InvalidParameter),
    };
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(Error::InvalidParameter);
    }
    let mut parameters = vec![];
    let processor: Box<dyn AudioProcessor> = match d.kind.as_str() {
        "stock.fx" => {
            let name = d.state["effect"].as_str().ok_or(Error::InvalidParameter)?;
            let kind = EFFECTS
                .iter()
                .find(|(id, _)| *id == name)
                .ok_or(Error::InvalidParameter)?
                .1;
            let effect = AutomatedEffect::new(kind, &d.id)?;
            if let Some(values) = d.state.get("values") {
                let values = values.as_object().ok_or(Error::InvalidParameter)?;
                for (key, value) in values {
                    let id = format!("{}.{}", d.id, key);
                    let p = effect
                        .parameters()
                        .iter()
                        .find(|p| p.info().id == id)
                        .ok_or(Error::InvalidParameter)?;
                    p.set(value.as_f64().ok_or(Error::InvalidParameter)? as f32)?;
                }
            }
            parameters = effect.parameters().to_vec();
            Box::new(effect)
        }
        "stock.synth" => {
            let wave = match d.state["waveform"].as_str().unwrap_or("sine") {
                "sine" => Waveform::Sine,
                "saw" => Waveform::Saw,
                "square" => Waveform::Square,
                _ => return Err(Error::InvalidParameter),
            };
            Box::new(ExpressiveSynth::<32>::new(
                wave,
                AdsrSettings {
                    attack: number(d, "attack", 0.008)?,
                    decay: number(d, "decay", 0.14)?,
                    sustain: number(d, "sustain", 0.65)?,
                    release: number(d, "release", 0.25)?,
                },
            )?)
        }
        "stock.sampler" => {
            let id = d.state["asset"].as_str().ok_or(Error::InvalidParameter)?;
            let a = assets
                .and_then(|a| a.get(id))
                .ok_or(Error::InvalidParameter)?;
            let root = d.state.get("root").map_or(Ok(60), |v| {
                v.as_u64()
                    .filter(|n| *n <= 127)
                    .map(|n| n as u8)
                    .ok_or(Error::InvalidParameter)
            })?;
            let sample = Sample::new(
                a.samples.clone(),
                a.channels as usize,
                a.sample_rate as f64,
                root,
            )?;
            let mut zone = soniclaw_sampler::Zone::single(0, root);
            zone.low_key = 0;
            zone.high_key = 127;
            zone.one_shot = d
                .state
                .get("one_shot")
                .map_or(Ok(false), |v| v.as_bool().ok_or(Error::InvalidParameter))?;
            if let Some(value) = d.state.get("loop") {
                let frames: Option<[usize; 2]> =
                    serde_json::from_value(value.clone()).map_err(|_| Error::InvalidParameter)?;
                if let Some([a, b]) = frames {
                    zone.loop_frames = Some((a, b));
                    zone.loop_crossfade = d.state.get("loop_crossfade").map_or(Ok(0), |v| {
                        v.as_u64()
                            .and_then(|v| usize::try_from(v).ok())
                            .ok_or(Error::InvalidParameter)
                    })?;
                }
            }
            if d.state.get("loop_crossfade").is_some() && zone.loop_frames.is_none() {
                return Err(Error::InvalidParameter);
            }
            let bank = Arc::new(soniclaw_sampler::SampleBank::new(
                vec![sample],
                vec![zone],
                256 * 1024 * 1024,
            )?);
            let mut sampler = soniclaw_sampler::MultiSampler::<64>::new(bank);
            sampler.set_quality(resample::ResampleQuality::SincFast);
            sampler.set_envelope(number(d, "attack_ms", 0.)?, number(d, "release_ms", 80.)?)?;
            sampler.set_pitch_bend_range(number(d, "bend_range", 2.)?)?;
            Box::new(sampler)
        }
        _ => unreachable!(),
    };
    Ok(Built {
        processor,
        parameters,
    })
}
pub fn instrument(index: usize, id: &str) -> Result<Descriptor, ProjectError> {
    if !valid_id(id) {
        return Err("invalid instrument identity".into());
    }
    let (wave, attack, decay, sustain, release) = match index {
        0 => ("sine", 0.005, 0.25, 0.6, 0.3),
        1 => ("saw", 0.003, 0.18, 0.45, 0.12),
        2 => ("square", 0.008, 0.15, 0.7, 0.25),
        3 => ("saw", 0.6, 0.5, 0.7, 1.8),
        _ => return Err("unknown stock instrument".into()),
    };
    Ok(Descriptor {
        id: id.into(),
        kind: "stock.synth".into(),
        state: serde_json::json!({"waveform":wave,"attack":attack,"decay":decay,"sustain":sustain,"release":release}),
    })
}
pub fn effect(name: &str, id: &str) -> Result<Descriptor, ProjectError> {
    if !valid_id(id) || !EFFECTS.iter().any(|(key, _)| *key == name) {
        return Err("unknown stock effect/identity".into());
    }
    Ok(Descriptor {
        id: id.into(),
        kind: "stock.fx".into(),
        state: serde_json::json!({"effect":name,"values":{}}),
    })
}
fn remove_namespace(p: &mut Project, id: &str) {
    let prefix = format!("{id}.");
    p.document
        .parameter_values
        .retain(|key, _| !key.starts_with(&prefix));
    p.document
        .automation
        .lanes
        .retain(|lane| !lane.parameter.starts_with(&prefix));
}
pub fn set_instrument(p: &mut Project, track: &str, d: Descriptor) -> Result<(), ProjectError> {
    if !matches!(d.kind.as_str(), "stock.synth" | "stock.sampler") {
        return Err("expected a stock instrument".into());
    }
    let mut next = p.clone();
    if next.freezes.contains_key(track) {
        return Err("unfreeze before replacing an instrument".into());
    }
    let i = next
        .document
        .tracks
        .iter()
        .position(|t| t.id == track)
        .ok_or("track missing")?;
    if let Some(old) = next.document.tracks[i].instrument.take() {
        remove_namespace(&mut next, &old.id);
    }
    let mut f = AssetsFactory {
        assets: &next.assets,
    };
    let mut built = f.create(&d)?;
    built.processor.prepare(next.document.spec())?;
    next.document.tracks[i].instrument = Some(d);
    next.validate()?;
    *p = next;
    Ok(())
}
pub fn add_effect(p: &mut Project, track: &str, d: Descriptor) -> Result<(), ProjectError> {
    if d.kind != "stock.fx" {
        return Err("expected a stock effect".into());
    }
    let mut next = p.clone();
    if next.freezes.contains_key(track) {
        return Err("unfreeze before editing inserts".into());
    }
    let mut built = AssetsFactory {
        assets: &next.assets,
    }
    .create(&d)?;
    built.processor.prepare(next.document.spec())?;
    next.document
        .tracks
        .iter_mut()
        .find(|t| t.id == track)
        .ok_or("track missing")?
        .inserts
        .push(d);
    next.validate()?;
    *p = next;
    Ok(())
}
pub fn install_sample(
    p: &mut Project,
    track: &str,
    asset_id: &str,
    asset: soniclaw_clips::AudioAsset,
    root: u8,
) -> Result<(), ProjectError> {
    if !valid_id(asset_id) || p.assets.contains_key(asset_id) {
        return Err("sample asset identity collision".into());
    }
    let mut next = p.clone();
    next.assets.insert(asset_id.into(), asset);
    let d = Descriptor {
        id: workspace::identity("sampler")?,
        kind: "stock.sampler".into(),
        state: serde_json::json!({"asset":asset_id,"root":root,"one_shot":false}),
    };
    set_instrument(&mut next, track, d)?;
    *p = next;
    Ok(())
}
/// Move a stock insert one slot (-1/+1), or remove it (0), including its parameter lanes.
pub fn edit_insert(
    p: &mut Project,
    track: &str,
    id: &str,
    direction: i8,
) -> Result<(), ProjectError> {
    let mut next = p.clone();
    if next.freezes.contains_key(track) || !(-1..=1).contains(&direction) {
        return Err("invalid/frozen insert edit".into());
    }
    let t = next
        .document
        .tracks
        .iter_mut()
        .find(|t| t.id == track)
        .ok_or("track missing")?;
    let i = t
        .inserts
        .iter()
        .position(|d| d.id == id && d.kind == "stock.fx")
        .ok_or("select a stock insert")?;
    if direction == 0 {
        t.inserts.remove(i);
        remove_namespace(&mut next, id);
    } else {
        let to = i
            .checked_add_signed(direction as isize)
            .filter(|n| *n < t.inserts.len())
            .ok_or("insert is already at chain boundary")?;
        t.inserts.swap(i, to);
    }
    next.validate()?;
    *p = next;
    Ok(())
}
