//! Project-wide, control-thread history. Assets are shared; files and native DSP histories are never undone.
use crate::{
    project::{Project, ProjectError},
    *,
};
use std::{collections::VecDeque, io::Write};
const META_LIMIT: usize = 16 * 1024 * 1024;
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub entries: usize,
    pub retained_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            entries: 32,
            retained_bytes: 256 * 1024 * 1024,
        }
    }
}
struct Limited(Vec<u8>);
impl Write for Limited {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if self
            .0
            .len()
            .checked_add(b.len())
            .is_none_or(|n| n > META_LIMIT)
        {
            return Err(std::io::Error::other("history metadata exceeds 16 MiB"));
        }
        self.0.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn metadata(p: &Project) -> Result<Vec<u8>, ProjectError> {
    let mut out = Limited(Vec::new());
    serde_json::to_writer(
        &mut out,
        &(
            &p.native_states,
            &p.project_id,
            &p.layout,
            &p.name,
            &p.document,
            &p.disk,
            &p.freezes,
            &p.takes,
            &p.midi_takes,
            &p.comps,
            &p.warps,
        ),
    )?;
    Ok(out.0)
}
struct Entry {
    label: String,
    project: Project,
    metadata: Vec<u8>,
}
fn entry(project: Project, label: &str) -> Result<Entry, ProjectError> {
    if label.is_empty() || label.len() > 256 {
        return Err("invalid history label".into());
    }
    let bytes = metadata(&project)?;
    Ok(Entry {
        label: label.into(),
        project,
        metadata: bytes,
    })
}
fn same(a: &Entry, p: &Project) -> Result<bool, ProjectError> {
    if a.metadata != metadata(p)? || a.project.assets.len() != p.assets.len() {
        return Ok(false);
    }
    Ok(a.project.assets.iter().all(|(id, x)| {
        p.assets.get(id).is_some_and(|y| {
            x.channels == y.channels
                && x.sample_rate == y.sample_rate
                && Arc::ptr_eq(&x.samples, &y.samples)
        })
    }))
}
/// Metadata and asset-identity equality for control-side change detection; not a validation substitute.
pub fn equivalent(a: &Project, b: &Project) -> Result<bool, ProjectError> {
    same(&entry(a.clone(), "Compare")?, b)
}
fn bytes<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> Result<usize, ProjectError> {
    let mut total = 0usize;
    let mut samples = BTreeSet::new();
    let mut native = BTreeSet::new();
    for e in entries {
        total = total
            .checked_add(
                e.metadata
                    .len()
                    .checked_mul(8)
                    .ok_or("history size overflow")?
                    + e.label.len()
                    + std::mem::size_of::<Entry>(),
            )
            .ok_or("history size overflow")?;
        for b in e.project.native_states.values() {
            if native.insert((b.bytes().as_ptr() as usize, b.bytes().len())) {
                total = total
                    .checked_add(b.bytes().len())
                    .ok_or("native history size overflow")?;
            }
        }
        for (id, a) in &e.project.assets {
            total = total
                .checked_add(id.len() + 128)
                .ok_or("history size overflow")?;
            let key = (
                Arc::as_ptr(&a.samples) as *const f32 as usize,
                a.samples.len(),
            );
            if samples.insert(key) {
                total = total
                    .checked_add(
                        a.samples
                            .len()
                            .checked_mul(4)
                            .ok_or("history PCM overflow")?,
                    )
                    .ok_or("history size overflow")?;
            }
        }
    }
    Ok(total)
}
#[derive(Clone, Copy, Debug)]
pub struct Stats {
    pub undo: usize,
    pub redo: usize,
    pub estimated_retained_bytes: usize,
    pub grouped: bool,
}
pub struct History {
    undo: VecDeque<Entry>,
    redo: Vec<Entry>,
    group: Option<Entry>,
    limits: Limits,
}
impl Default for History {
    fn default() -> Self {
        Self::new(Limits::default()).expect("default history limits")
    }
}
impl History {
    pub fn new(limits: Limits) -> Result<Self, ProjectError> {
        if !(1..=256).contains(&limits.entries)
            || !(1024..=1024 * 1024 * 1024).contains(&limits.retained_bytes)
        {
            return Err("invalid history limits".into());
        }
        Ok(Self {
            undo: VecDeque::new(),
            redo: vec![],
            group: None,
            limits,
        })
    }
    pub fn clear(&mut self) -> Result<(), ProjectError> {
        if self.group.is_some() {
            return Err("cancel or finish the history group first".into());
        }
        self.undo.clear();
        self.redo.clear();
        Ok(())
    }
    pub fn stats(&self) -> Result<Stats, ProjectError> {
        Ok(Stats {
            undo: self.undo.len(),
            redo: self.redo.len(),
            estimated_retained_bytes: bytes(
                self.undo.iter().chain(&self.redo).chain(self.group.iter()),
            )?,
            grouped: self.group.is_some(),
        })
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.group
            .is_none()
            .then(|| self.undo.back().map(|e| e.label.as_str()))
            .flatten()
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.group
            .is_none()
            .then(|| self.redo.last().map(|e| e.label.as_str()))
            .flatten()
    }
    fn push(&mut self, e: Entry) -> Result<(), ProjectError> {
        if bytes([&e])? > self.limits.retained_bytes {
            return Err("one project snapshot exceeds the history memory limit".into());
        }
        let mut skip = self
            .undo
            .len()
            .saturating_add(1)
            .saturating_sub(self.limits.entries);
        while bytes(self.undo.iter().skip(skip).chain([&e]))? > self.limits.retained_bytes {
            skip += 1;
        }
        for _ in 0..skip {
            self.undo.pop_front();
        }
        self.redo.clear();
        self.undo.push_back(e);
        Ok(())
    }
    /// Adopt edits made by a validated editor. On error restore previous and leave history unchanged.
    pub fn record(
        &mut self,
        previous: Project,
        current: &mut Project,
        label: &str,
    ) -> Result<bool, ProjectError> {
        let result = (|| {
            current.validate()?;
            previous.validate()?;
            let e = entry(previous.clone(), label)?;
            if same(&e, current)? {
                return Ok(false);
            }
            if self.group.is_none() {
                self.push(e)?;
            }
            Ok(true)
        })();
        if result.is_err() {
            *current = previous;
        }
        result
    }
    /// Edit an isolated candidate; failed edits or preparation never replace current.
    pub fn edit_prepared<T>(
        &mut self,
        current: &mut Project,
        label: &str,
        edit: impl FnOnce(&mut Project) -> Result<(), ProjectError>,
        prepare: impl FnOnce(&Project) -> Result<T, ProjectError>,
    ) -> Result<Option<T>, ProjectError> {
        current.validate()?;
        let mut next = current.clone();
        edit(&mut next)?;
        next.validate()?;
        let e = entry(current.clone(), label)?;
        if same(&e, &next)? {
            return Ok(None);
        }
        let prepared = prepare(&next)?;
        if self.group.is_none() {
            self.push(e)?;
        }
        *current = next;
        Ok(Some(prepared))
    }
    pub fn edit(
        &mut self,
        current: &mut Project,
        label: &str,
        edit: impl FnOnce(&mut Project) -> Result<(), ProjectError>,
    ) -> Result<bool, ProjectError> {
        Ok(self
            .edit_prepared(current, label, edit, |_| Ok(()))?
            .is_some())
    }
    pub fn begin_group(&mut self, current: &Project, label: &str) -> Result<(), ProjectError> {
        if self.group.is_some() {
            return Err("nested history groups are unsupported".into());
        }
        current.validate()?;
        let e = entry(current.clone(), label)?;
        if bytes(self.undo.iter().chain(&self.redo).chain([&e]))? > self.limits.retained_bytes {
            return Err("history group exceeds retained-memory limit".into());
        }
        self.group = Some(e);
        Ok(())
    }
    pub fn finish_group(&mut self, current: &mut Project) -> Result<bool, ProjectError> {
        let e = self.group.take().ok_or("no active history group")?;
        let result = (|| {
            current.validate()?;
            Ok(!same(&e, current)?)
        })();
        match result {
            Ok(false) => Ok(false),
            Ok(true) => {
                let previous = e.project.clone();
                if let Err(error) = self.push(e) {
                    *current = previous;
                    return Err(error);
                }
                Ok(true)
            }
            Err(error) => {
                *current = e.project;
                Err(error)
            }
        }
    }
    pub fn cancel_group(&mut self, current: &mut Project) -> Result<(), ProjectError> {
        let e = self.group.as_ref().ok_or("no active history group")?;
        e.project.validate()?;
        *current = self.group.take().unwrap().project;
        Ok(())
    }
    pub fn undo_prepared<T>(
        &mut self,
        current: &mut Project,
        prepare: impl FnOnce(&Project) -> Result<T, ProjectError>,
    ) -> Result<Option<T>, ProjectError> {
        if self.group.is_some() {
            return Err("finish or cancel history group first".into());
        }
        let Some(target) = self.undo.back() else {
            return Ok(None);
        };
        target.project.validate()?;
        current.validate()?;
        let replacement = entry(current.clone(), &target.label)?;
        if bytes(
            self.undo
                .iter()
                .take(self.undo.len() - 1)
                .chain(&self.redo)
                .chain([&replacement]),
        )? > self.limits.retained_bytes
        {
            return Err("undo cannot retain current project within memory limit".into());
        }
        let prepared = prepare(&target.project)?;
        let target = self.undo.pop_back().unwrap();
        self.redo.push(replacement);
        *current = target.project;
        Ok(Some(prepared))
    }
    pub fn redo_prepared<T>(
        &mut self,
        current: &mut Project,
        prepare: impl FnOnce(&Project) -> Result<T, ProjectError>,
    ) -> Result<Option<T>, ProjectError> {
        if self.group.is_some() {
            return Err("finish or cancel history group first".into());
        }
        let Some(target) = self.redo.last() else {
            return Ok(None);
        };
        target.project.validate()?;
        current.validate()?;
        let replacement = entry(current.clone(), &target.label)?;
        if bytes(
            self.undo
                .iter()
                .chain(self.redo.iter().take(self.redo.len() - 1))
                .chain([&replacement]),
        )? > self.limits.retained_bytes
        {
            return Err("redo cannot retain current project within memory limit".into());
        }
        let prepared = prepare(&target.project)?;
        let target = self.redo.pop().unwrap();
        self.undo.push_back(replacement);
        *current = target.project;
        Ok(Some(prepared))
    }
    pub fn undo(&mut self, current: &mut Project) -> Result<bool, ProjectError> {
        Ok(self.undo_prepared(current, |_| Ok(()))?.is_some())
    }
    pub fn redo(&mut self, current: &mut Project) -> Result<bool, ProjectError> {
        Ok(self.redo_prepared(current, |_| Ok(()))?.is_some())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Project {
        let (d, a) = crate::tests::fixture();
        Project::new("History".into(), d, a, Default::default()).unwrap()
    }
    #[test]
    fn grouped_project_edits_undo_redo_and_failed_prepare_are_atomic() {
        let mut p = fixture();
        let original = p.document.to_json().unwrap();
        let arc = p.assets["a"].samples.clone();
        let mut h = History::default();
        h.begin_group(&p, "Arrangement and mix").unwrap();
        h.edit(&mut p, "Gain", |p| {
            p.document.mixer.strips[0].gain = 0.3;
            Ok(())
        })
        .unwrap();
        h.edit(&mut p, "Move", |p| {
            p.document.tracks[0].audio[0].start_frame = 128;
            Ok(())
        })
        .unwrap();
        assert!(h.undo(&mut p).is_err());
        assert!(h.finish_group(&mut p).unwrap());
        assert_eq!(h.stats().unwrap().undo, 1);
        assert!(h.undo(&mut p).unwrap());
        assert_eq!(original, p.document.to_json().unwrap());
        assert!(Arc::ptr_eq(&arc, &p.assets["a"].samples));
        assert!(h.redo(&mut p).unwrap());
        assert_eq!(p.document.tracks[0].audio[0].start_frame, 128);
        let before = p.document.to_json().unwrap();
        assert!(h
            .edit_prepared(
                &mut p,
                "Bad factory",
                |p| {
                    p.document.mixer.strips[0].gain = 0.7;
                    Ok(())
                },
                |_| Err::<(), ProjectError>("processor refused preparation".into())
            )
            .is_err());
        assert_eq!(before, p.document.to_json().unwrap());
        let stats = h.stats().unwrap();
        assert!(h
            .undo_prepared(&mut p, |_| Err::<(), ProjectError>("prepare failed".into()))
            .is_err());
        assert_eq!(h.stats().unwrap().undo, stats.undo);
        h.begin_group(&p, "Cancel").unwrap();
        h.edit(&mut p, "Name", |p| {
            p.name = "Changed".into();
            Ok(())
        })
        .unwrap();
        h.cancel_group(&mut p).unwrap();
        assert_eq!(p.name, "History");
    }
    #[test]
    fn branch_noop_limits_and_invalid_editor_commit() {
        let mut p = fixture();
        let mut h = History::new(Limits {
            entries: 2,
            retained_bytes: 1024 * 1024,
        })
        .unwrap();
        for name in ["one", "two", "three"] {
            h.edit(&mut p, "Name", |p| {
                p.name = name.into();
                Ok(())
            })
            .unwrap();
        }
        assert_eq!(h.stats().unwrap().undo, 2);
        assert!(!h.edit(&mut p, "No change", |_| Ok(())).unwrap());
        h.undo(&mut p).unwrap();
        h.edit(&mut p, "Branch", |p| {
            p.name = "branch".into();
            Ok(())
        })
        .unwrap();
        assert!(h.redo_label().is_none());
        let previous = p.clone();
        p.document.tracks[0].audio[0].length_frames = 0;
        assert!(h.record(previous, &mut p, "Invalid editor").is_err());
        assert!(p.document.tracks[0].audio[0].length_frames > 0);
        let mut tiny = History::new(Limits {
            entries: 2,
            retained_bytes: 1024,
        })
        .unwrap();
        let before = p.name.clone();
        assert!(tiny
            .edit(&mut p, "Too large", |p| {
                p.name = "rejected".into();
                Ok(())
            })
            .is_err());
        assert_eq!(p.name, before);
    }
}
