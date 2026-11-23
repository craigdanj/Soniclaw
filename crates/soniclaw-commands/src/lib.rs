//! Control-thread commands. Applications own document semantics and rebuild audio after edits.
use serde::{Deserialize, Serialize};
use soniclaw_core::Error;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
fn valid(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}
#[derive(Clone, Debug)]
pub struct Command {
    pub id: String,
    pub label: String,
    pub category: String,
    pub enabled: bool,
    pub checked: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shortcut {
    pub key: String,
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
    pub command: bool,
}
impl Shortcut {
    pub fn new(key: &str) -> Self {
        Self {
            key: key.into(),
            control: false,
            alt: false,
            shift: false,
            command: false,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keymap {
    pub schema: u32,
    pub bindings: BTreeMap<String, Shortcut>,
}
impl Default for Keymap {
    fn default() -> Self {
        Self {
            schema: 1,
            bindings: BTreeMap::new(),
        }
    }
}
#[derive(Default)]
pub struct Registry {
    commands: BTreeMap<String, Command>,
    keymap: Keymap,
}
impl Registry {
    pub fn register(&mut self, command: Command) -> Result<(), Error> {
        if !valid(&command.id)
            || command.label.is_empty()
            || command.label.len() > 256
            || command.category.len() > 128
            || self.commands.len() >= 1024
            || self.commands.contains_key(&command.id)
        {
            return Err(Error::InvalidParameter);
        }
        self.commands.insert(command.id.clone(), command);
        Ok(())
    }
    pub fn commands(&self) -> impl Iterator<Item = &Command> {
        self.commands.values()
    }
    pub fn get(&self, id: &str) -> Option<&Command> {
        self.commands.get(id)
    }
    pub fn state(&mut self, id: &str, enabled: bool, checked: bool) -> Result<(), Error> {
        let c = self.commands.get_mut(id).ok_or(Error::InvalidParameter)?;
        c.enabled = enabled;
        c.checked = checked;
        Ok(())
    }
    pub fn set_keymap(&mut self, map: Keymap) -> Result<(), Error> {
        if map.schema != 1 || map.bindings.len() > 1024 {
            return Err(Error::InvalidParameter);
        }
        let mut seen = BTreeSet::new();
        for (id, key) in &map.bindings {
            if !self.commands.contains_key(id)
                || key.key.is_empty()
                || key.key.len() > 32
                || !key.key.bytes().all(|c| c.is_ascii_alphanumeric())
                || !seen.insert(key)
            {
                return Err(Error::InvalidParameter);
            }
        }
        self.keymap = map;
        Ok(())
    }
    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }
    pub fn keymap_json(&self) -> Result<String, Error> {
        serde_json::to_string_pretty(&self.keymap).map_err(|_| Error::InvalidParameter)
    }
    pub fn load_keymap(&mut self, json: &str) -> Result<(), Error> {
        if json.len() > 256 * 1024 {
            return Err(Error::InvalidParameter);
        }
        self.set_keymap(serde_json::from_str(json).map_err(|_| Error::InvalidParameter)?)
    }
    /// Exact modifier match; caller suppresses routing while editing text or during modal UI.
    pub fn route(&self, key: &Shortcut) -> Option<&str> {
        self.keymap
            .bindings
            .iter()
            .find_map(|(id, k)| (k == key && self.commands[id].enabled).then_some(id.as_str()))
    }
    pub fn invoke<T: Clone, E>(
        &self,
        id: &str,
        history: &mut History<T>,
        document: &mut T,
        edit: impl FnOnce(&mut T) -> Result<(), E>,
    ) -> Result<bool, E> {
        if !self.get(id).is_some_and(|c| c.enabled) {
            return Ok(false);
        }
        history.edit(document, edit)?;
        Ok(true)
    }
}
struct Entry<T> {
    label: String,
    value: T,
}
/// Snapshot history with a bounded entry count. T's memory and validation remain application-owned.
pub struct History<T> {
    undo: VecDeque<Entry<T>>,
    redo: Vec<Entry<T>>,
    group: Option<Entry<T>>,
    capacity: usize,
}
impl<T: Clone> History<T> {
    pub fn new(capacity: usize) -> Result<Self, Error> {
        if !(1..=1024).contains(&capacity) {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            group: None,
            capacity,
        })
    }
    pub fn can_undo(&self) -> bool {
        self.group.is_none() && !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        self.group.is_none() && !self.redo.is_empty()
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.back().map(|e| e.label.as_str())
    }
    pub fn record(&mut self, previous: T, label: impl Into<String>) {
        if self.group.is_some() {
            return;
        }
        self.push(Entry {
            label: label.into(),
            value: previous,
        });
        self.redo.clear();
    }
    fn push(&mut self, e: Entry<T>) {
        if self.undo.len() == self.capacity {
            self.undo.pop_front();
        }
        self.undo.push_back(e);
    }
    pub fn edit<E>(
        &mut self,
        document: &mut T,
        apply: impl FnOnce(&mut T) -> Result<(), E>,
    ) -> Result<(), E> {
        let old = document.clone();
        match apply(document) {
            Ok(()) => {
                self.record(old, "Edit");
                Ok(())
            }
            Err(e) => {
                *document = old;
                Err(e)
            }
        }
    }
    pub fn begin_group(&mut self, document: &T, label: impl Into<String>) -> Result<(), Error> {
        if self.group.is_some() {
            return Err(Error::InvalidParameter);
        }
        self.group = Some(Entry {
            label: label.into(),
            value: document.clone(),
        });
        Ok(())
    }
    pub fn finish_group(&mut self) -> Result<(), Error> {
        let e = self.group.take().ok_or(Error::InvalidParameter)?;
        self.push(e);
        self.redo.clear();
        Ok(())
    }
    pub fn cancel_group(&mut self, document: &mut T) -> Result<(), Error> {
        *document = self.group.take().ok_or(Error::InvalidParameter)?.value;
        Ok(())
    }
    pub fn undo(&mut self, document: &mut T) -> bool {
        if self.group.is_some() {
            return false;
        }
        if let Some(e) = self.undo.pop_back() {
            self.redo.push(Entry {
                label: e.label,
                value: std::mem::replace(document, e.value),
            });
            true
        } else {
            false
        }
    }
    pub fn redo(&mut self, document: &mut T) -> bool {
        if self.group.is_some() {
            return false;
        }
        if let Some(e) = self.redo.pop() {
            self.push(Entry {
                label: e.label,
                value: std::mem::replace(document, e.value),
            });
            true
        } else {
            false
        }
    }
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.group = None;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grouped_undo_redo_branch_and_capacity() {
        let mut h = History::new(2).unwrap();
        let mut d = 0;
        h.begin_group(&d, "Drag").unwrap();
        h.edit(&mut d, |v| {
            *v = 1;
            Ok::<_, ()>(())
        })
        .unwrap();
        h.edit(&mut d, |v| {
            *v = 2;
            Ok::<_, ()>(())
        })
        .unwrap();
        assert!(!h.undo(&mut d));
        h.finish_group().unwrap();
        assert_eq!(h.undo_label(), Some("Drag"));
        assert!(h.undo(&mut d));
        assert_eq!(d, 0);
        assert!(h.redo(&mut d));
        assert_eq!(d, 2);
        h.begin_group(&d, "Cancel").unwrap();
        d = 5;
        h.cancel_group(&mut d).unwrap();
        assert_eq!(d, 2);
        h.record(d, "A");
        d = 3;
        h.record(d, "B");
        d = 4;
        assert!(h.undo(&mut d));
        assert_eq!(d, 3);
        h.edit(&mut d, |v| {
            *v = 8;
            Ok::<_, ()>(())
        })
        .unwrap();
        assert!(!h.can_redo());
        assert!(h.undo(&mut d));
        assert!(h.undo(&mut d));
        assert!(!h.undo(&mut d));
    }
    #[test]
    fn failed_edits_roll_back() {
        let mut h = History::new(8).unwrap();
        let mut d = vec![1];
        assert_eq!(
            h.edit(&mut d, |d| {
                d.push(2);
                Err::<(), _>("bad")
            }),
            Err("bad")
        );
        assert_eq!(d, vec![1]);
        assert!(!h.can_undo());
    }
    #[test]
    fn keymap_conflicts_unknown_commands_and_disabled_dispatch() {
        let mut r = Registry::default();
        for id in ["edit.add", "edit.other"] {
            r.register(Command {
                id: id.into(),
                label: id.into(),
                category: "Edit".into(),
                enabled: true,
                checked: false,
            })
            .unwrap();
        }
        let mut k = Shortcut::new("K");
        k.control = true;
        let map = Keymap {
            schema: 1,
            bindings: BTreeMap::from([("edit.add".into(), k.clone())]),
        };
        r.set_keymap(map).unwrap();
        let json = r.keymap_json().unwrap();
        r.load_keymap(&json).unwrap();
        assert_eq!(r.route(&k), Some("edit.add"));
        let mut invalid = r.keymap.clone();
        invalid.bindings.insert("edit.other".into(), k.clone());
        assert!(r.set_keymap(invalid).is_err());
        assert_eq!(r.route(&k), Some("edit.add"));
        r.state("edit.add", false, true).unwrap();
        assert_eq!(r.route(&k), None);
        let mut h = History::new(8).unwrap();
        let mut d = 0;
        assert!(!r
            .invoke("edit.add", &mut h, &mut d, |_| Ok::<_, ()>(()))
            .unwrap());
    }
}
