//! Parameter state, not a full project format or DSP history. Restore off-thread
//! while stopped: scalar atomics do not give a coherent multi-parameter update.
use serde::{Deserialize, Serialize};
use soniclaw_core::Parameter;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub schema_version: u32,
    pub product_id: String,
    pub name: String,
    pub parameters: BTreeMap<String, f32>,
}
pub type StateError = Box<dyn std::error::Error>;
impl Preset {
    pub fn capture(
        product_id: &str,
        name: &str,
        parameters: &[Arc<Parameter>],
    ) -> Result<Self, StateError> {
        let mut values = BTreeMap::new();
        for p in parameters {
            if values.insert(p.info().id.to_owned(), p.get()).is_some() {
                return Err("duplicate parameter ID".into());
            }
        }
        Ok(Self {
            schema_version: 1,
            product_id: product_id.into(),
            name: name.into(),
            parameters: values,
        })
    }
    pub fn to_json(&self) -> Result<String, StateError> {
        Ok(serde_json::to_string_pretty(self)?)
    }
    pub fn from_json(json: &str) -> Result<Self, StateError> {
        Ok(serde_json::from_str(json)?)
    }
    pub fn restore(
        &self,
        expected_product: &str,
        parameters: &[Arc<Parameter>],
    ) -> Result<(), StateError> {
        if self.schema_version != 1 || self.product_id != expected_product {
            return Err("incompatible preset schema or product".into());
        }
        let mut ids = BTreeSet::new();
        for p in parameters {
            let info = p.info();
            if !ids.insert(info.id) {
                return Err("duplicate parameter ID".into());
            }
            let v = self
                .parameters
                .get(info.id)
                .ok_or("preset is missing a parameter")?;
            if !v.is_finite() || !(info.min..=info.max).contains(v) {
                return Err("preset parameter is out of range".into());
            }
        }
        if ids.len() != self.parameters.len() {
            return Err("preset contains unknown parameters".into());
        }
        // Everything is checked before mutation. Individual stores are atomic.
        for p in parameters {
            p.set(self.parameters[p.info().id])?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use soniclaw_core::ParameterInfo;
    #[test]
    fn invalid_restore_does_not_partially_apply() {
        let params: Vec<_> = ["a", "b"]
            .map(|id| {
                Arc::new(
                    Parameter::new(ParameterInfo {
                        id,
                        name: id,
                        min: 0.0,
                        max: 1.0,
                        default: 0.5,
                        unit: "",
                    })
                    .unwrap(),
                )
            })
            .into();
        let mut preset = Preset::capture("test", "init", &params).unwrap();
        preset.parameters.insert("a".into(), 0.9);
        preset.parameters.insert("b".into(), 2.0);
        assert!(preset.restore("test", &params).is_err());
        assert_eq!(params[0].get(), 0.5);
        preset.parameters.insert("b".into(), 0.1);
        let preset = Preset::from_json(&preset.to_json().unwrap()).unwrap();
        preset.restore("test", &params).unwrap();
        assert_eq!(params[0].get(), 0.9);
        assert!(preset.restore("another-product", &params).is_err());
    }
}
