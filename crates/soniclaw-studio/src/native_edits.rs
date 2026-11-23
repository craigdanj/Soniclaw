//! Control-thread native-editor gestures shared by applications and host adapters.
use crate::{
    project::{Project, ProjectError},
    *,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Begin,
    Value,
    End,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    pub parameter: String,
    pub phase: Phase,
    pub value: Option<f32>,
}
impl Edit {
    pub fn validate(&self) -> Result<(), ProjectError> {
        if self.parameter.is_empty()
            || self.parameter.len() > 128
            || !self
                .parameter
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            || match (self.phase, self.value) {
                (Phase::Value, Some(v)) => !v.is_finite() || !(0.0..=1.0).contains(&v),
                (Phase::Begin | Phase::End, None) => false,
                _ => true,
            }
        {
            return Err("invalid native editor gesture".into());
        }
        Ok(())
    }
}
#[derive(Default)]
pub struct Gestures {
    active: BTreeSet<String>,
}
impl Gestures {
    pub fn active(&self) -> bool {
        !self.active.is_empty()
    }
    /// Apply on the GUI/control thread at the observed playhead beat. Native timestamps are not assumed to be sample clocks.
    pub fn apply(
        &mut self,
        edit: &Edit,
        p: &mut Project,
        control: Option<&Controller>,
        writer: Option<&mut automation_write::Writer>,
        history: &mut history::History,
        beat: f64,
    ) -> Result<(), ProjectError> {
        edit.validate()?;
        if !beat.is_finite() || beat < 0. {
            return Err("invalid gesture time".into());
        }
        if edit.phase != Phase::End && !self.active.contains(&edit.parameter) {
            if self.active.len() >= 64 {
                return Err("at most 64 simultaneous native gestures".into());
            }
            if !history.stats()?.grouped {
                history.begin_group(p, "Native editor gesture")?;
            }
            self.active.insert(edit.parameter.clone());
        }
        if let Some(value) = edit.value {
            if let Some(c) = control {
                let target = c
                    .parameters()
                    .iter()
                    .find(|v| v.info().id == edit.parameter)
                    .ok_or("unknown native host control")?;
                target.set_normalized(value)?;
            }
            p.document
                .parameter_values
                .insert(edit.parameter.clone(), value);
        }
        if let Some(writer) = writer.filter(|w| w.parameter() == edit.parameter) {
            let gesture = control.and_then(|c| c.gestures().get(writer.lane_index()));
            match edit.phase {
                Phase::Value => {
                    let value = edit.value.expect("validated");
                    writer.input(beat, value, true)?;
                    if let Some(g) = gesture {
                        g.set(value)?;
                    }
                }
                Phase::End => {
                    writer.end();
                    if let Some(g) = gesture {
                        g.end();
                    }
                }
                Phase::Begin => {}
            }
        }
        if edit.phase == Phase::End {
            self.active.remove(&edit.parameter);
        }
        Ok(())
    }
    pub fn finish(
        &mut self,
        p: &mut Project,
        history: &mut history::History,
    ) -> Result<(), ProjectError> {
        self.active.clear();
        if history.stats()?.grouped {
            history.finish_group(p)?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grouped_editor_values_undo_redo() {
        let mut p = workspace::blank("Editor".into(), 48000, 128).unwrap();
        let id = "native.gain.7";
        let mut h = history::History::default();
        let mut g = Gestures::default();
        for (phase, value) in [
            (Phase::Begin, None),
            (Phase::Value, Some(0.2)),
            (Phase::Value, Some(0.8)),
            (Phase::End, None),
        ] {
            g.apply(
                &Edit {
                    parameter: id.into(),
                    phase,
                    value,
                },
                &mut p,
                None,
                None,
                &mut h,
                0.5,
            )
            .unwrap();
        }
        g.finish(&mut p, &mut h).unwrap();
        assert_eq!(p.document.parameter_values[id], 0.8);
        assert_eq!(h.stats().unwrap().undo, 1);
        h.undo(&mut p).unwrap();
        assert!(!p.document.parameter_values.contains_key(id));
        h.redo(&mut p).unwrap();
        assert_eq!(p.document.parameter_values[id], 0.8);
        assert!(!g.active());
    }
}
