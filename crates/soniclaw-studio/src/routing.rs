//! Stopped, transactional routing drafts for the studio's stereo track -> auxiliary -> master topology.
use crate::{
    project::{Project, ProjectError},
    *,
};
use soniclaw_mixer::Tap;
#[derive(Clone, Debug)]
pub enum Edit {
    AddAux {
        tap: Tap,
        return_gain: f32,
    },
    RemoveAux {
        index: usize,
    },
    Tap {
        index: usize,
        tap: Tap,
    },
    Send {
        track: String,
        aux: usize,
        gain: f32,
    },
    Return {
        aux: usize,
        gain: f32,
    },
    Insert {
        aux: usize,
        descriptor: Descriptor,
    },
    RemoveInsert {
        aux: usize,
        index: usize,
    },
}
#[derive(Clone)]
pub struct Draft {
    pub document: Document,
    project_id: String,
    base: String,
}
impl Draft {
    pub fn new(p: &Project) -> Result<Self, ProjectError> {
        p.validate()?;
        Ok(Self {
            document: p.document.clone(),
            project_id: p.project_id.clone(),
            base: p.document.to_json()?,
        })
    }
    pub fn edit(&mut self, edit: Edit) -> Result<(), ProjectError> {
        let mut d = self.document.clone();
        match edit {
            Edit::AddAux { tap, return_gain } => {
                if d.auxiliaries.len() >= 4 {
                    return Err("studio supports at most four auxiliaries".into());
                }
                d.auxiliaries.push(Auxiliary {
                    inserts: vec![],
                    return_gain,
                });
                d.mixer.taps.push(tap);
                for s in &mut d.mixer.strips {
                    s.sends.push(0.);
                }
            }
            Edit::RemoveAux { index } => {
                if index >= d.auxiliaries.len() {
                    return Err("unknown auxiliary".into());
                }
                let removed = d.auxiliaries.remove(index);
                d.mixer.taps.remove(index);
                for s in &mut d.mixer.strips {
                    s.sends.remove(index);
                }
                let remap = |id: &str| -> Option<String> {
                    for t in &d.tracks {
                        let prefix = format!("mix.{}.send", t.id);
                        if let Some(tail) = id.strip_prefix(&prefix) {
                            if let Ok(i) = tail.parse::<usize>() {
                                return if i == index {
                                    None
                                } else {
                                    Some(format!("{prefix}{}", if i > index { i - 1 } else { i }))
                                };
                            }
                        }
                    }
                    if removed
                        .inserts
                        .iter()
                        .any(|x| id.starts_with(&format!("{}.", x.id)))
                    {
                        None
                    } else {
                        Some(id.into())
                    }
                };
                d.parameter_values = d
                    .parameter_values
                    .iter()
                    .filter_map(|(id, v)| remap(id).map(|id| (id, *v)))
                    .collect();
                d.automation.lanes = d
                    .automation
                    .lanes
                    .iter()
                    .filter_map(|l| {
                        let mut l = l.clone();
                        l.parameter = remap(&l.parameter)?;
                        Some(l)
                    })
                    .collect();
            }
            Edit::Tap { index, tap } => {
                *d.mixer.taps.get_mut(index).ok_or("unknown auxiliary")? = tap
            }
            Edit::Send { track, aux, gain } => {
                let strip = d
                    .mixer
                    .strips
                    .iter_mut()
                    .find(|t| t.id == track)
                    .ok_or("unknown track")?;
                *strip.sends.get_mut(aux).ok_or("unknown send")? = gain;
                d.parameter_values
                    .insert(format!("mix.{track}.send{aux}"), gain / 4.);
            }
            Edit::Return { aux, gain } => {
                d.auxiliaries
                    .get_mut(aux)
                    .ok_or("unknown auxiliary")?
                    .return_gain = gain
            }
            Edit::Insert { aux, descriptor } => d
                .auxiliaries
                .get_mut(aux)
                .ok_or("unknown auxiliary")?
                .inserts
                .push(descriptor),
            Edit::RemoveInsert { aux, index } => {
                let a = d.auxiliaries.get_mut(aux).ok_or("unknown auxiliary")?;
                if index >= a.inserts.len() {
                    return Err("unknown insert".into());
                }
                let id = a.inserts.remove(index).id;
                d.parameter_values
                    .retain(|p, _| !p.starts_with(&format!("{id}.")));
                d.automation
                    .lanes
                    .retain(|p| !p.parameter.starts_with(&format!("{id}.")));
            }
        }
        d.validate()?;
        self.document = d;
        Ok(())
    }
    /// Build before replacing the model. Failure leaves the project unchanged; caller owns stopped publication.
    pub fn commit(
        self,
        p: &mut Project,
        factory: &mut impl Factory,
    ) -> Result<PreparedSession, ProjectError> {
        if p.project_id != self.project_id || p.document.to_json()? != self.base {
            return Err("stale routing draft; reload after project edits".into());
        }
        let mut next = p.clone();
        next.document = self.document;
        next.validate()?;
        let prepared = next
            .document
            .compile_streamed(&next.assets, &next.disk, factory)?;
        *p = next;
        Ok(prepared)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removal_remaps_lanes_values_and_failed_prepare_is_atomic() {
        let (d, a) = crate::tests::fixture();
        let mut p = Project::new("Routing".into(), d, a, Default::default()).unwrap();
        let mut r = Draft::new(&p).unwrap();
        for _ in 0..2 {
            r.edit(Edit::AddAux {
                tap: Tap::PreFader,
                return_gain: 1.,
            })
            .unwrap();
        }
        r.edit(Edit::Send {
            track: "audio".into(),
            aux: 1,
            gain: 0.6,
        })
        .unwrap();
        r.document.automation.lanes.push(soniclaw_automation::Lane {
            parameter: "mix.audio.send1".into(),
            enabled: true,
            mode: soniclaw_automation::Mode::Read,
            points: vec![],
        });
        r.edit(Edit::RemoveAux { index: 0 }).unwrap();
        assert_eq!(r.document.mixer.strips[0].sends, [0.6]);
        assert_eq!(r.document.parameter_values["mix.audio.send0"], 0.15);
        assert_eq!(r.document.automation.lanes[0].parameter, "mix.audio.send0");
        let before = p.document.to_json().unwrap();
        r.edit(Edit::Insert {
            aux: 0,
            descriptor: Descriptor {
                id: "bad".into(),
                kind: "unknown".into(),
                state: serde_json::json!({}),
            },
        })
        .unwrap();
        assert!(r.commit(&mut p, &mut Builtins).is_err());
        assert_eq!(p.document.to_json().unwrap(), before);
    }
}
