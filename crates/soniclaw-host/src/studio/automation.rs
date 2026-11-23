//! Native controls use stable normalized Studio parameters. Callback event storage is prepared.
use super::*;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
#[derive(Clone)]
pub struct Control {
    pub native_id: u32,
    pub min: f64,
    pub max: f64,
    pub parameter: Arc<soniclaw_core::Parameter>,
}
impl Control {
    pub fn native_value(&self) -> f64 {
        self.min + self.parameter.normalized() as f64 * (self.max - self.min)
    }
}
pub fn controls(d: &Descriptor, parameters: &[ParameterInfo]) -> Result<Vec<Control>, HostError> {
    let selected = if let Some(ids) = d.state.get("automation_ids") {
        Some(
            ids.as_array()
                .ok_or("automation_ids must be an array")?
                .iter()
                .map(|v| {
                    v.as_u64()
                        .filter(|v| *v <= u32::MAX as u64)
                        .map(|v| v as u32)
                        .ok_or("invalid native parameter ID")
                })
                .collect::<Result<Vec<_>, _>>()?,
        )
    } else {
        None
    };
    if selected.as_ref().is_some_and(|s| {
        s.len() > 64 || s.iter().collect::<std::collections::BTreeSet<_>>().len() != s.len()
    }) {
        return Err("at most 64 unique native automation IDs".into());
    }
    let mut result = Vec::new();
    for p in parameters {
        if selected.as_ref().is_some_and(|s| !s.contains(&p.id))
            || p.flags & clap_sys::ext::params::CLAP_PARAM_IS_READONLY != 0
            || p.max <= p.min
        {
            continue;
        }
        if result.len() == 64 {
            return Err("select at most 64 writable native automation_ids".into());
        }
        if ![p.min, p.max, p.default, p.value]
            .iter()
            .all(|v| v.is_finite())
        {
            return Err("nonfinite native metadata".into());
        }
        let id = format!("{}.native.{}", d.id, p.id);
        let default = ((p.default - p.min) / (p.max - p.min)).clamp(0., 1.) as f32;
        let parameter = Arc::new(soniclaw_core::Parameter::new(
            soniclaw_core::ParameterInfo {
                id: &id,
                name: &p.name,
                min: 0.,
                max: 1.,
                default,
                unit: "",
            },
        )?);
        parameter.set_normalized(((p.value - p.min) / (p.max - p.min)).clamp(0., 1.) as f32)?;
        result.push(Control {
            native_id: p.id,
            min: p.min,
            max: p.max,
            parameter,
        });
    }
    if selected.as_ref().is_some_and(|s| {
        s.iter()
            .any(|id| !result.iter().any(|p| &p.native_id == id))
    }) {
        return Err("automation selection includes unknown/read-only/fixed-range ID".into());
    }
    Ok(result)
}
pub enum NativeProcessor {
    #[cfg(feature = "vst3")]
    Buses(Box<super::bus_adapter::Adapter>),
    Clap(crate::direct::DirectProcessor),
    #[cfg(feature = "vst3")]
    Vst3(Box<soniclaw_vst3::Processor>),
    #[cfg(feature = "vst3")]
    Offline(Box<soniclaw_vst3::musical_offline::OfflineProcessor>),
}
pub struct Automated {
    inner: NativeProcessor,
    controls: Vec<Control>,
    last: Vec<f64>,
    pub failures: Arc<AtomicU64>,
}
impl Automated {
    pub fn new(inner: NativeProcessor, controls: Vec<Control>, failures: Arc<AtomicU64>) -> Self {
        let last = vec![f64::NAN; controls.len()];
        Self {
            inner,
            controls,
            last,
            failures,
        }
    }
}
impl AudioProcessor for Automated {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        match &mut self.inner {
            NativeProcessor::Clap(p) => p.prepare(s),
            #[cfg(feature = "vst3")]
            NativeProcessor::Vst3(p) => p.prepare(s),
            #[cfg(feature = "vst3")]
            NativeProcessor::Offline(p) => p.prepare(s),
            #[cfg(feature = "vst3")]
            NativeProcessor::Buses(p) => p.prepare(s),
        }
    }
    fn reset(&mut self) {
        self.last.fill(f64::NAN);
        match &mut self.inner {
            NativeProcessor::Clap(p) => p.reset(),
            #[cfg(feature = "vst3")]
            NativeProcessor::Vst3(p) => p.reset(),
            #[cfg(feature = "vst3")]
            NativeProcessor::Offline(p) => p.reset(),
            #[cfg(feature = "vst3")]
            NativeProcessor::Buses(p) => p.reset(),
        }
    }
    fn latency_frames(&self) -> usize {
        match &self.inner {
            NativeProcessor::Clap(p) => p.latency_frames(),
            #[cfg(feature = "vst3")]
            NativeProcessor::Vst3(p) => p.latency_frames(),
            #[cfg(feature = "vst3")]
            NativeProcessor::Offline(p) => p.latency_frames(),
            #[cfg(feature = "vst3")]
            NativeProcessor::Buses(p) => p.latency_frames(),
        }
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        if b.frames() == 0 {
            return;
        }
        let mut points = [ParameterEvent {
            frame: 0,
            id: 0,
            value: 0.,
        }; 64];
        let mut count = 0;
        for (i, c) in self.controls.iter().enumerate() {
            let value = c.native_value();
            if value != self.last[i] {
                points[count] = ParameterEvent {
                    frame: 0,
                    id: c.native_id,
                    value,
                };
                count += 1;
                self.last[i] = value;
            }
        }
        let result = match &mut self.inner {
            NativeProcessor::Clap(p) => p.process_automated(b, ctx, &points[..count]),
            #[cfg(feature = "vst3")]
            NativeProcessor::Vst3(p) => {
                let mut normalized = [soniclaw_vst3::musical::ParameterPoint::default(); 64];
                for (o, p) in normalized.iter_mut().zip(&points[..count]) {
                    o.id = p.id;
                    o.value = self
                        .controls
                        .iter()
                        .find(|c| c.native_id == p.id)
                        .expect("selected control")
                        .parameter
                        .normalized() as f64;
                }
                p.process_automated(b, ctx, &normalized[..count])
                    .and_then(|()| {
                        if p.faulted() {
                            Err(Error::NotPrepared)
                        } else {
                            Ok(())
                        }
                    })
            }
            #[cfg(feature = "vst3")]
            NativeProcessor::Offline(p) => {
                let mut normalized = [soniclaw_vst3::musical::ParameterPoint::default(); 64];
                for (o, p) in normalized.iter_mut().zip(&points[..count]) {
                    o.id = p.id;
                    o.value = self
                        .controls
                        .iter()
                        .find(|c| c.native_id == p.id)
                        .expect("selected control")
                        .parameter
                        .normalized() as f64;
                }
                p.process_automated(b, ctx, &normalized[..count])
            }
            #[cfg(feature = "vst3")]
            NativeProcessor::Buses(p) => {
                let mut normalized = [soniclaw_vst3::musical::ParameterPoint::default(); 64];
                for (o, p) in normalized.iter_mut().zip(&points[..count]) {
                    o.id = p.id;
                    o.value = self
                        .controls
                        .iter()
                        .find(|c| c.native_id == p.id)
                        .expect("selected control")
                        .parameter
                        .normalized() as f64;
                }
                p.process_automated(b, ctx, &normalized[..count])
            }
        };
        if result.is_err() {
            self.failures.fetch_add(1, Ordering::Relaxed);
            b.clear();
        }
    }
}
/// Metadata-free normalized handles keep saved lanes resolvable while bypassed. They do not control DSP.
pub fn placeholders(
    d: &Descriptor,
    state: Option<&studio::native_state::Blob>,
) -> Result<Vec<Arc<soniclaw_core::Parameter>>, HostError> {
    let ids = if let Some(v) = d.state.get("automation_ids") {
        v.as_array()
            .ok_or("invalid automation selection")?
            .iter()
            .map(|v| {
                v.as_u64()
                    .filter(|v| *v <= u32::MAX as u64)
                    .map(|v| v as u32)
                    .ok_or("invalid native ID")
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        state.map_or_else(Vec::new, |s| {
            s.metadata().parameters.keys().copied().collect()
        })
    };
    if ids.len() > 64 || ids.iter().collect::<std::collections::BTreeSet<_>>().len() != ids.len() {
        return Err("invalid placeholder native selection".into());
    }
    ids.into_iter()
        .map(|id| {
            let id = format!("{}.native.{id}", d.id);
            Ok(Arc::new(soniclaw_core::Parameter::new(
                soniclaw_core::ParameterInfo {
                    id: &id,
                    name: "Bypassed native control",
                    min: 0.,
                    max: 1.,
                    default: 0.5,
                    unit: "",
                },
            )?))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_ids_ranges_and_readonly_selection() {
        let d = Descriptor {
            id: "gain".into(),
            kind: "clap".into(),
            state: serde_json::json!({"automation_ids":[7]}),
        };
        let p = ParameterInfo {
            id: 7,
            name: "Gain".into(),
            min: -12.,
            max: 12.,
            default: 0.,
            value: 6.,
            flags: 0,
        };
        let c = controls(&d, std::slice::from_ref(&p)).unwrap();
        assert_eq!(c[0].parameter.info().id, "gain.native.7");
        assert_eq!(c[0].parameter.normalized(), 0.75);
        c[0].parameter.set_normalized(0.25).unwrap();
        assert_eq!(c[0].native_value(), -6.);
        let mut p = p;
        p.flags = clap_sys::ext::params::CLAP_PARAM_IS_READONLY;
        assert!(controls(&d, &[p]).is_err());
    }
}
