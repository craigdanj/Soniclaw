//! Control-thread DAW project/track commands. Immutable media is shared; models commit transactionally.
use crate::{
    project::{Project, ProjectError},
    *,
};
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub elastic_groups: Vec<crate::elastic::Group>,
    #[serde(default, skip_serializing_if = "crate::arrangement::Model::is_empty")]
    pub arrangement: crate::arrangement::Model,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub take_groups: BTreeMap<String, crate::takes::AudioGroup>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub midi_selections: BTreeMap<String, crate::takes::MidiSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calibration: Option<crate::timing::Calibration>,
    #[serde(default)]
    pub folders: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, Vec<u16>>,
    #[serde(default)]
    pub membership: BTreeMap<String, String>,
}
impl Layout {
    pub fn is_empty(&self) -> bool {
        self.folders.is_empty()
            && self.membership.is_empty()
            && self.inputs.is_empty()
            && self.calibration.is_none()
            && self.take_groups.is_empty()
            && self.midi_selections.is_empty()
            && self.arrangement.is_empty()
            && self.elastic_groups.is_empty()
    }
    pub fn validate(&self, d: &Document) -> Result<(), ProjectError> {
        self.arrangement.validate()?;
        if let Some(c) = &self.calibration {
            c.validate(d.sample_rate)?;
        }
        if self.inputs.iter().any(|(id, ch)| {
            !d.tracks.iter().any(|t| &t.id == id)
                || !(1..=2).contains(&ch.len())
                || ch.iter().any(|c| *c >= 64)
        }) || self.folders.len() > 128
            || self
                .folders
                .iter()
                .any(|(id, name)| !valid_id(id) || name.is_empty() || name.len() > 256)
            || self.membership.iter().any(|(track, folder)| {
                !d.tracks.iter().any(|t| &t.id == track) || !self.folders.contains_key(folder)
            })
        {
            return Err("invalid track folder layout".into());
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Audio,
    Instrument,
}
#[derive(Clone, Debug)]
pub enum Command {
    Add {
        id: String,
        name: String,
        kind: Kind,
    },
    Rename {
        id: String,
        name: String,
    },
    Delete {
        id: String,
    },
    Duplicate {
        id: String,
        new_id: String,
    },
    Move {
        id: String,
        index: usize,
    },
    Folder {
        id: String,
        name: String,
    },
    AssignFolder {
        track: String,
        folder: Option<String>,
    },
    AddMidi {
        track: String,
        id: String,
        start: f64,
        length: f64,
    },
}
fn parameters(t: &Track) -> Vec<String> {
    std::iter::once(format!("mix.{}.", t.id))
        .chain(
            t.instrument
                .iter()
                .chain(&t.inserts)
                .map(|d| format!("{}.", d.id)),
        )
        .collect()
}
/// Allocate a stable control-side ID suitable for newly recorded regions across reopened sessions.
pub fn identity(prefix: &str) -> Result<String, ProjectError> {
    if !valid_id(prefix) || prefix.len() > 8 {
        return Err("identity prefix must be 1..8 ID characters".into());
    }
    Ok(format!("{prefix}.{}", &crate::project::new_identity()[8..]))
}
pub(crate) fn reindex(d: &mut Document) {
    for (i, t) in d.tracks.iter_mut().enumerate() {
        for c in &mut t.audio {
            c.track = i as u16;
        }
        for c in &mut t.midi {
            c.track = i as u16;
        }
    }
}
fn duplicate(p: &mut Project, id: &str, new_id: &str) -> Result<(), ProjectError> {
    if !valid_id(new_id) || p.document.tracks.iter().any(|t| t.id == new_id) {
        return Err("invalid/duplicate track identity".into());
    }
    let index = p
        .document
        .tracks
        .iter()
        .position(|t| t.id == id)
        .ok_or("unknown track")?;
    let mut t = p.document.tracks[index].clone();
    if let Some(f) = p.freezes.get(id) {
        t = f
            .original
            .tracks
            .iter()
            .find(|t| t.id == id)
            .ok_or("missing frozen original")?
            .clone();
    }
    let mut prefixes = vec![(format!("mix.{id}."), format!("mix.{new_id}."))];
    let mut copied_sidechains = Vec::new();
    t.id = new_id.into();
    for (i, d) in t.instrument.iter_mut().chain(&mut t.inserts).enumerate() {
        let next = format!("{new_id}.processor{i}");
        if d.kind == "stock.sidechain" {
            let link = p
                .document
                .mix_routing
                .sidechains
                .iter()
                .find(|s| s.target == d.id)
                .ok_or("missing detector binding")?;
            copied_sidechains.push(crate::mix_graph::Sidechain {
                source: link.source.clone(),
                target: next.clone(),
            });
        }
        prefixes.push((format!("{}.", d.id), format!("{next}.")));
        if let Some(blob) = p.native_states.get(&d.id).cloned() {
            p.native_states.insert(next.clone(), blob);
        }
        d.id = next;
    }
    for (i, c) in t.audio.iter_mut().enumerate() {
        c.id = format!("{new_id}.audio{i}");
    }
    for (i, c) in t.midi.iter_mut().enumerate() {
        c.id = format!("{new_id}.midi{i}");
    }
    for (i, c) in t.streamed.iter_mut().enumerate() {
        c.id = format!("{new_id}.disk{i}");
    }
    let remap = |s: &str| {
        prefixes
            .iter()
            .find_map(|(from, to)| s.strip_prefix(from).map(|tail| format!("{to}{tail}")))
    };
    let mut source_values = p
        .freezes
        .get(id)
        .map_or_else(BTreeMap::new, |f| f.original.parameter_values.clone());
    source_values.extend(p.document.parameter_values.clone());
    let values: Vec<_> = source_values
        .iter()
        .filter_map(|(id, v)| remap(id).map(|id| (id, *v)))
        .collect();
    p.document.parameter_values.extend(values);
    let mut source_lanes: BTreeMap<String, soniclaw_automation::Lane> =
        p.freezes.get(id).map_or_else(BTreeMap::new, |f| {
            f.original
                .automation
                .lanes
                .iter()
                .map(|l| (l.parameter.clone(), l.clone()))
                .collect()
        });
    source_lanes.extend(
        p.document
            .automation
            .lanes
            .iter()
            .map(|l| (l.parameter.clone(), l.clone())),
    );
    let lanes: Vec<_> = source_lanes
        .values()
        .filter_map(|l| {
            let mut l = l.clone();
            l.parameter = remap(&l.parameter)?;
            Some(l)
        })
        .collect();
    p.document.automation.lanes.extend(lanes);
    let mut strip = p.document.mixer.strips[index].clone();
    strip.id = new_id.into();
    strip.name = format!("{} copy", strip.name);
    if let Some(out) = p.document.mix_routing.track_outputs.get(id).cloned() {
        p.document
            .mix_routing
            .track_outputs
            .insert(new_id.into(), out);
    }
    p.document.mix_routing.sidechains.extend(copied_sidechains);
    p.document.tracks.insert(index + 1, t);
    p.document.mixer.strips.insert(index + 1, strip);
    if let Some(f) = p.layout.membership.get(id).cloned() {
        p.layout.membership.insert(new_id.into(), f);
    }
    Ok(())
}
/// All errors leave the current project unchanged. Stop audio before structural commands.
pub fn apply(p: &mut Project, command: Command) -> Result<(), ProjectError> {
    let mut next = p.clone();
    match command {
        Command::Add { id, name, kind } => {
            if !valid_id(&id)
                || id.len() > 40
                || name.is_empty()
                || name.len() > 256
                || next.document.tracks.iter().any(|t| t.id == id)
            {
                return Err("invalid track name/identity".into());
            }
            next.document.tracks.push(Track {
                streamed: vec![],
                id: id.clone(),
                audio: vec![],
                midi: vec![],
                instrument: match kind {
                    Kind::Audio => None,
                    Kind::Instrument => Some(Descriptor {
                        id: format!("{id}.instrument"),
                        kind: "sine".into(),
                        state: serde_json::json!({}),
                    }),
                },
                inserts: vec![],
                input_gain: 0.,
                midi_input: matches!(kind, Kind::Instrument),
            });
            next.document.mixer.strips.push(soniclaw_mixer::Strip {
                id,
                name,
                gain: 1.,
                pan: 0.,
                mute: false,
                solo: false,
                group: None,
                sends: vec![0.; next.document.auxiliaries.len()],
            });
        }
        Command::Rename { id, name } => {
            if name.is_empty() || name.len() > 256 {
                return Err("invalid track name".into());
            }
            next.document
                .mixer
                .strips
                .iter_mut()
                .find(|s| s.id == id)
                .ok_or("unknown track")?
                .name = name;
        }
        Command::Delete { id } => {
            if next.document.tracks.len() == 1 {
                return Err("keep at least one track".into());
            }
            let index = next
                .document
                .tracks
                .iter()
                .position(|t| t.id == id)
                .ok_or("unknown track")?;
            if next
                .document
                .mix_routing
                .sidechains
                .iter()
                .any(|s| s.source == id)
            {
                return Err("disconnect sidechains before deleting their source track".into());
            }
            let removed: Vec<_> = next.document.tracks[index]
                .inserts
                .iter()
                .map(|d| d.id.clone())
                .collect();
            next.document
                .mix_routing
                .sidechains
                .retain(|s| !removed.contains(&s.target));
            next.document.mix_routing.track_outputs.remove(&id);
            let prefixes = parameters(&next.document.tracks[index]);
            next.document.tracks.remove(index);
            next.document.mixer.strips.remove(index);
            next.document
                .parameter_values
                .retain(|id, _| !prefixes.iter().any(|p| id.starts_with(p)));
            next.document
                .automation
                .lanes
                .retain(|l| !prefixes.iter().any(|p| l.parameter.starts_with(p)));
            next.freezes.remove(&id);
            next.comps.remove(&id);
            next.layout.membership.remove(&id);
        }
        Command::Duplicate { id, new_id } => {
            if new_id.len() > 40 {
                return Err("duplicate identity too long".into());
            }
            duplicate(&mut next, &id, &new_id)?;
        }
        Command::Move { id, index } => {
            if index >= next.document.tracks.len() {
                return Err("track destination out of range".into());
            }
            let old = next
                .document
                .tracks
                .iter()
                .position(|t| t.id == id)
                .ok_or("unknown track")?;
            let t = next.document.tracks.remove(old);
            let s = next.document.mixer.strips.remove(old);
            next.document.tracks.insert(index, t);
            next.document.mixer.strips.insert(index, s);
        }
        Command::Folder { id, name } => {
            if !valid_id(&id) || name.is_empty() || name.len() > 256 {
                return Err("invalid folder".into());
            }
            next.layout.folders.insert(id, name);
        }
        Command::AssignFolder { track, folder } => {
            if !next.document.tracks.iter().any(|t| t.id == track) {
                return Err("unknown track".into());
            }
            if let Some(f) = folder {
                if !next.layout.folders.contains_key(&f) {
                    return Err("unknown folder".into());
                }
                next.layout.membership.insert(track, f);
            } else {
                next.layout.membership.remove(&track);
            }
        }
        Command::AddMidi {
            track,
            id,
            start,
            length,
        } => {
            if !valid_id(&id)
                || next.document.tracks.iter().any(|t| {
                    t.audio.iter().any(|c| c.id == id)
                        || t.midi.iter().any(|c| c.id == id)
                        || t.streamed.iter().any(|c| c.id == id)
                })
            {
                return Err("clip identity collision".into());
            }
            let i = next
                .document
                .tracks
                .iter()
                .position(|t| t.id == track)
                .ok_or("unknown track")?;
            next.document.tracks[i].midi.push(MidiClip {
                controls: vec![],
                id,
                track: i as u16,
                start_beat: start,
                length_beats: length,
                source_offset: 0.,
                source_length: length,
                looping: false,
                notes: vec![],
                enabled: true,
            });
        }
    }
    crate::takes::reconcile(&mut next);
    reindex(&mut next.document);
    next.layout
        .inputs
        .retain(|id, _| next.document.tracks.iter().any(|t| &t.id == id));
    next.validate()?;
    *p = next;
    Ok(())
}
pub fn blank(name: String, rate: u32, block: usize) -> Result<Project, ProjectError> {
    if name.trim().is_empty() || name.len() > 256 {
        return Err("project name is empty/too long".into());
    }
    let d = Document {
        cues: Vec::new(),
        mix_routing: Default::default(),
        schema: 1,
        sample_rate: rate,
        max_block_frames: block,
        parameter_values: BTreeMap::new(),
        timeline: Default::default(),
        tracks: vec![Track {
            id: "audio-1".into(),
            streamed: vec![],
            audio: vec![],
            midi: vec![],
            instrument: None,
            inserts: vec![],
            input_gain: 0.,
            midi_input: false,
        }],
        mixer: MixerModel {
            schema: 1,
            strips: vec![soniclaw_mixer::Strip {
                id: "audio-1".into(),
                name: "Audio 1".into(),
                gain: 1.,
                pan: 0.,
                mute: false,
                solo: false,
                group: None,
                sends: vec![],
            }],
            groups: vec![],
            taps: vec![],
        },
        auxiliaries: vec![],
        automation: Default::default(),
    };
    Project::new(name, d, Assets::new(), DiskAssets::new())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn structural_commands_reindex_and_preserve_history_and_resources() {
        let mut p = blank("Song".into(), 48000, 256).unwrap();
        apply(
            &mut p,
            Command::Add {
                id: "keys".into(),
                name: "Keys".into(),
                kind: Kind::Instrument,
            },
        )
        .unwrap();
        apply(
            &mut p,
            Command::AddMidi {
                track: "keys".into(),
                id: "part".into(),
                start: 0.,
                length: 4.,
            },
        )
        .unwrap();
        apply(
            &mut p,
            Command::Folder {
                id: "band".into(),
                name: "Band".into(),
            },
        )
        .unwrap();
        apply(
            &mut p,
            Command::AssignFolder {
                track: "keys".into(),
                folder: Some("band".into()),
            },
        )
        .unwrap();
        let mut h = crate::history::History::default();
        h.edit(&mut p, "Duplicate", |p| {
            apply(
                p,
                Command::Duplicate {
                    id: "keys".into(),
                    new_id: "keys-copy".into(),
                },
            )
        })
        .unwrap();
        assert_eq!(p.document.tracks.len(), 3);
        assert_ne!(
            p.document.tracks[1].midi[0].id,
            p.document.tracks[2].midi[0].id
        );
        apply(
            &mut p,
            Command::Move {
                id: "keys-copy".into(),
                index: 0,
            },
        )
        .unwrap();
        assert_eq!(p.document.tracks[0].midi[0].track, 0);
        let before = p.document.to_json().unwrap();
        assert!(apply(
            &mut p,
            Command::Add {
                id: "keys".into(),
                name: "Bad".into(),
                kind: Kind::Audio
            }
        )
        .is_err());
        assert_eq!(p.document.to_json().unwrap(), before);
        h.undo(&mut p).unwrap();
        assert_eq!(p.document.tracks.len(), 2);
        p.document.compile(&p.assets, &mut Builtins).unwrap();
    }
}
