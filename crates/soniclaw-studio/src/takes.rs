//! Transactional phase-locked audio comp groups and selectable loop MIDI passes.
use crate::{
    project::{Comp, Project, ProjectError},
    workspace, *,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AudioGroup {
    pub take: usize,
    pub destinations: BTreeMap<String, String>,
    pub original_mutes: BTreeMap<String, bool>,
    pub original_solos: BTreeMap<String, bool>,
    pub destination_solos: BTreeMap<String, bool>,
    pub active: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MidiSelection {
    pub fingerprint: String,
    pub pass: usize,
    pub track: String,
}
pub fn validate(p: &Project) -> Result<(), ProjectError> {
    if p.layout.take_groups.len() > 32 || p.layout.midi_selections.len() > 4096 {
        return Err("take selection metadata limit".into());
    }
    let mut active = BTreeSet::new();
    for (id, g) in &p.layout.take_groups {
        if !valid_id(id)
            || g.take >= p.takes.len()
            || g.destinations.is_empty()
            || g.destinations.len() > 32
            || g.original_mutes.len() > 32
            || g.original_solos
                .keys()
                .any(|id| !g.original_mutes.contains_key(id))
            || g.destination_solos
                .keys()
                .any(|id| !g.destinations.values().any(|v| v == id))
        {
            return Err("invalid audio take group".into());
        }
        let mut recipe = None;
        for (stem, dest) in &g.destinations {
            if !valid_id(stem)
                || !p.document.tracks.iter().any(|t| &t.id == dest)
                || !p.comps.contains_key(dest)
            {
                return Err("missing grouped comp destination".into());
            }
            let c = &p.comps[dest];
            if c.take != g.take || &c.stem != stem {
                return Err("grouped comp reference mismatch".into());
            }
            let bytes = serde_json::to_vec(&c.recipe)?;
            if recipe.as_ref().is_some_and(|old| old != &bytes) {
                return Err("grouped comp recipes must stay phase-locked".into());
            }
            recipe = Some(bytes);
        }
        if g.active
            && g.original_mutes.keys().any(|id| {
                p.document.automation.lanes.iter().any(|lane| {
                    lane.enabled
                        && (lane.parameter == format!("mix.{id}.mute")
                            || lane.parameter == format!("mix.{id}.solo"))
                })
            })
        {
            return Err("active grouped comp sources cannot have mute/solo automation".into());
        }
        for id in g.original_mutes.keys() {
            if !p.document.tracks.iter().any(|t| &t.id == id) || g.active && !active.insert(id) {
                return Err("missing or conflicting active audio comp sources".into());
            }
        }
    }
    for (id, s) in &p.layout.midi_selections {
        if s.fingerprint.len() != 64
            || !s.fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
            || s.pass > 4096
            || !p
                .document
                .tracks
                .iter()
                .any(|t| t.id == s.track && t.midi.iter().any(|c| &c.id == id))
        {
            return Err("invalid loop MIDI selection".into());
        }
    }
    Ok(())
}
pub fn set_active(p: &mut Project, id: &str, on: bool) -> Result<(), ProjectError> {
    let mut next = p.clone();
    let g = next
        .layout
        .take_groups
        .get_mut(id)
        .ok_or("unknown comp group")?;
    if g.active == on {
        return p.validate();
    }
    for strip in &mut next.document.mixer.strips {
        strip.mute = next
            .document
            .parameter_values
            .get(&format!("mix.{}.mute", strip.id))
            .map_or(strip.mute, |v| *v >= 0.5);
        strip.solo = next
            .document
            .parameter_values
            .get(&format!("mix.{}.solo", strip.id))
            .map_or(strip.solo, |v| *v >= 0.5);
        if g.original_mutes.contains_key(&strip.id) {
            if on {
                g.original_mutes.insert(strip.id.clone(), strip.mute);
                g.original_solos.insert(strip.id.clone(), strip.solo);
                strip.mute = true;
                strip.solo = false;
            } else {
                strip.mute = g.original_mutes[&strip.id];
                strip.solo = g.original_solos.get(&strip.id).copied().unwrap_or(false);
            }
        }
        if g.destinations.values().any(|id| id == &strip.id) {
            if on {
                strip.mute = false;
                strip.solo = g.destination_solos.get(&strip.id).copied().unwrap_or(false);
            } else {
                g.destination_solos.insert(strip.id.clone(), strip.solo);
                strip.mute = true;
                strip.solo = false;
            }
        }
    }
    for strip in &next.document.mixer.strips {
        if g.original_mutes.contains_key(&strip.id)
            || g.destinations.values().any(|id| id == &strip.id)
        {
            next.document.parameter_values.insert(
                format!("mix.{}.mute", strip.id),
                if strip.mute { 1. } else { 0. },
            );
            next.document.parameter_values.insert(
                format!("mix.{}.solo", strip.id),
                if strip.solo { 1. } else { 0. },
            );
        }
    }
    g.active = on;
    next.validate()?;
    *p = next;
    Ok(())
}
/// The same recipe is applied to every selected stem in one transaction. Destination IDs are stable across edits.
pub fn apply_group(
    p: &mut Project,
    id: &str,
    take_index: usize,
    recipe: &soniclaw_record::comp::Recipe,
) -> Result<(), ProjectError> {
    use sha2::{Digest, Sha256};
    if !valid_id(id) {
        return Err("invalid comp group identity".into());
    }
    let path = p
        .takes
        .get(take_index)
        .ok_or("missing grouped take")?
        .clone();
    let take = soniclaw_record::multitrack::Take::read(&path)?;
    recipe.validate(&take)?;
    let previous = p.layout.take_groups.get(id).cloned();
    if previous.as_ref().is_some_and(|g| g.take != take_index) {
        return Err("comp group belongs to another take".into());
    }
    let mut next = p.clone();
    next.layout.take_groups.remove(id);
    let mut group = previous.clone().unwrap_or(AudioGroup {
        take: take_index,
        destinations: BTreeMap::new(),
        original_mutes: BTreeMap::new(),
        original_solos: BTreeMap::new(),
        destination_solos: BTreeMap::new(),
        active: false,
    });
    let schedule = path.join("recording-schedule.json");
    let dry = if schedule.exists() {
        use std::io::Read;
        let mut bytes = vec![];
        std::fs::File::open(schedule)?
            .take(32769)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 32768 {
            return Err("recording schedule too large".into());
        }
        let schedule: crate::recording::Schedule = serde_json::from_slice(&bytes)?;
        schedule
            .settings
            .arms
            .into_iter()
            .filter(|a| a.source == crate::recording::Source::DryInput)
            .map(|a| a.track)
            .collect::<BTreeSet<_>>()
    } else {
        BTreeSet::new()
    };
    for stem in take.tracks.iter().filter(|t| t.id != "master") {
        let destination = group
            .destinations
            .entry(stem.id.clone())
            .or_insert_with(|| {
                format!(
                    "comp.{}",
                    &format!("{:x}", Sha256::digest(format!("{id}:{}", stem.id)))[..32]
                )
            })
            .clone();
        if let Some(strip) = next.document.mixer.strips.iter().find(|s| s.id == stem.id) {
            group
                .original_mutes
                .entry(stem.id.clone())
                .or_insert(strip.mute);
        }
        let first = !next.document.tracks.iter().any(|t| t.id == destination);
        next.apply_comp(
            &destination,
            Comp {
                take: take_index,
                stem: stem.id.clone(),
                recipe: recipe.clone(),
            },
        )?;
        if dry.contains(&stem.id) {
            let schedule: crate::recording::Schedule =
                serde_json::from_slice(&std::fs::read(path.join("recording-schedule.json"))?)?;
            let offset = schedule
                .calibration
                .as_ref()
                .map_or(0, |c| c.audio_offset_frames) as i128;
            if let Some(track) = next
                .document
                .tracks
                .iter_mut()
                .find(|t| t.id == destination)
            {
                for clip in &mut track.streamed {
                    let corrected = clip.source_offset as i128 + offset;
                    if corrected < 0 || corrected + clip.source_length as i128 > stem.frames as i128
                    {
                        return Err(
                            "loop comp lacks calibration pre-roll/tail; select a covered pass"
                                .into(),
                        );
                    }
                    clip.source_offset = corrected as u64;
                }
            }
        }
        if first {
            if p.document.automation.lanes.iter().any(|lane| {
                lane.enabled
                    && (lane.parameter == format!("mix.{}.mute", stem.id)
                        || lane.parameter == format!("mix.{}.solo", stem.id))
            }) {
                return Err(
                    "disable source mute/solo automation before activating a comp group".into(),
                );
            }
            let old = format!("mix.{}.", stem.id);
            let new = format!("mix.{destination}.");
            let values = p
                .document
                .parameter_values
                .iter()
                .filter_map(|(key, value)| {
                    key.strip_prefix(&old)
                        .map(|suffix| (format!("{new}{suffix}"), *value))
                })
                .collect::<Vec<_>>();
            for (key, value) in values {
                next.document.parameter_values.insert(key, value);
            }
            let lanes = p
                .document
                .automation
                .lanes
                .iter()
                .filter_map(|lane| {
                    lane.parameter
                        .strip_prefix(&old)
                        .filter(|suffix| *suffix != "mute" && *suffix != "solo")
                        .map(|suffix| {
                            let mut lane = lane.clone();
                            lane.parameter = format!("{new}{suffix}");
                            lane
                        })
                })
                .collect::<Vec<_>>();
            next.document.automation.lanes.extend(lanes);

            if let Some(source) = p.document.mixer.strips.iter().find(|s| s.id == stem.id) {
                if let Some(target) = next
                    .document
                    .mixer
                    .strips
                    .iter_mut()
                    .find(|s| s.id == destination)
                {
                    target.gain = source.gain;
                    target.pan = source.pan;
                    target.group = source.group;
                    target.sends = source.sends.clone();
                    group.destination_solos.insert(
                        destination.clone(),
                        p.document
                            .parameter_values
                            .get(&format!("mix.{}.solo", source.id))
                            .map_or(source.solo, |v| *v >= 0.5),
                    );
                }
            }
            if dry.contains(&stem.id) {
                copy_inserts(&mut next, &stem.id, &destination)?;
            }
        }
        if let Some(strip) = next
            .document
            .mixer
            .strips
            .iter_mut()
            .find(|s| s.id == destination)
        {
            strip.name = format!("{} comp", stem.id);
        }
    }
    if group.destinations.is_empty() {
        return Err("take contains no groupable track stems".into());
    }
    next.layout.take_groups.insert(id.into(), group);
    set_active(&mut next, id, true)?;
    *p = next;
    Ok(())
}
/// Dissolve the phase-lock binding while keeping the current audible mixer/comp state.
pub fn ungroup(p: &mut Project, id: &str) -> Result<(), ProjectError> {
    let mut next = p.clone();
    next.layout
        .take_groups
        .remove(id)
        .ok_or("unknown comp group")?;
    next.validate()?;
    *p = next;
    Ok(())
}
fn copy_inserts(p: &mut Project, source: &str, destination: &str) -> Result<(), ProjectError> {
    use sha2::{Digest, Sha256};
    let inserts = p
        .document
        .tracks
        .iter()
        .find(|t| t.id == source)
        .map_or_else(Vec::new, |t| t.inserts.clone());
    let mut copied = vec![];
    for mut d in inserts {
        let old = format!("{}.", d.id);
        d.id = format!(
            "compfx.{}",
            &format!("{:x}", Sha256::digest(format!("{destination}:{}", d.id)))[..32]
        );
        let new = format!("{}.", d.id);
        let parameters = p
            .document
            .parameter_values
            .iter()
            .filter_map(|(id, v)| {
                id.strip_prefix(&old)
                    .map(|suffix| (format!("{new}{suffix}"), *v))
            })
            .collect::<Vec<_>>();
        for (id, v) in parameters {
            p.document.parameter_values.insert(id, v);
        }
        let lanes = p
            .document
            .automation
            .lanes
            .iter()
            .filter_map(|lane| {
                lane.parameter.strip_prefix(&old).map(|suffix| {
                    let mut lane = lane.clone();
                    lane.parameter = format!("{new}{suffix}");
                    lane
                })
            })
            .collect::<Vec<_>>();
        p.document.automation.lanes.extend(lanes);
        copied.push(d);
    }
    p.document
        .tracks
        .iter_mut()
        .find(|t| t.id == destination)
        .ok_or("comp destination missing")?
        .inserts = copied;
    Ok(())
}
/// Remove only take-selection metadata made stale by a deliberate structural edit.
pub(crate) fn reconcile(p: &mut Project) {
    let invalid = p
        .layout
        .take_groups
        .iter()
        .filter(|(_, g)| {
            g.destinations
                .values()
                .any(|id| !p.document.tracks.iter().any(|t| &t.id == id))
                || g.original_mutes
                    .keys()
                    .any(|id| !p.document.tracks.iter().any(|t| &t.id == id))
        })
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    for id in invalid {
        if let Some(g) = p.layout.take_groups.remove(&id) {
            if g.active {
                for strip in &mut p.document.mixer.strips {
                    if let Some(m) = g.original_mutes.get(&strip.id) {
                        strip.mute = *m;
                        strip.solo = g.original_solos.get(&strip.id).copied().unwrap_or(false);
                        p.document.parameter_values.insert(
                            format!("mix.{}.mute", strip.id),
                            if strip.mute { 1. } else { 0. },
                        );
                        p.document.parameter_values.insert(
                            format!("mix.{}.solo", strip.id),
                            if strip.solo { 1. } else { 0. },
                        );
                    }
                    if g.destinations.values().any(|id| id == &strip.id) {
                        strip.mute = true;
                        strip.solo = false;
                        p.document
                            .parameter_values
                            .insert(format!("mix.{}.mute", strip.id), 1.);
                        p.document
                            .parameter_values
                            .insert(format!("mix.{}.solo", strip.id), 0.);
                    }
                }
            }
        }
    }
    p.layout.midi_selections.retain(|id, s| {
        p.document
            .tracks
            .iter()
            .any(|t| t.id == s.track && t.midi.iter().any(|c| &c.id == id))
    });
    workspace::reindex(&mut p.document);
}
