//! Explicit session trust and stopped-graph recovery for Studio child hosts.
use super::*;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, marker::PhantomData, rc::Rc};
#[derive(Default)]
pub struct TrustedHelpers {
    paths: BTreeSet<PathBuf>,
    thread: PhantomData<Rc<()>>,
}
impl TrustedHelpers {
    /// # Safety
    /// Every executable and dependency must remain explicitly trusted. Control thread only.
    pub unsafe fn new(paths: &[PathBuf]) -> Result<Self, HostError> {
        if paths.len() > 256 {
            return Err("too many approved helpers".into());
        }
        Ok(Self {
            paths: paths
                .iter()
                .map(|p| p.canonicalize())
                .collect::<Result<_, _>>()?,
            thread: PhantomData,
        })
    }
    pub fn check(&self, path: &Path) -> Result<PathBuf, HostError> {
        let p = path.canonicalize()?;
        if !p.is_file() || !self.paths.contains(&p) {
            return Err("helper requires explicit session approval".into());
        }
        Ok(p)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub helper: PathBuf,
    pub timeout_ms: u64,
    pub recovery_budget: u8,
    pub dry_on_fault: bool,
}
impl Policy {
    pub fn from_descriptor(d: &Descriptor) -> Result<Option<Self>, HostError> {
        let Some(v) = d.state.get("isolation") else {
            return Ok(None);
        };
        let p: Self = serde_json::from_value(v.clone())?;
        if !p.helper.is_absolute()
            || !(50..=10000).contains(&p.timeout_ms)
            || !(1..=8).contains(&p.recovery_budget)
        {
            return Err("invalid isolation policy".into());
        }
        Ok(Some(p))
    }
    /// Explicit recovery consumes a persisted attempt before building a fresh stopped graph.
    pub fn request_recovery(d: &mut Descriptor) -> Result<(), HostError> {
        let p = Self::from_descriptor(d)?.ok_or("not isolated")?;
        let n = d
            .state
            .get("isolation_attempts")
            .map_or(Ok(0), |v| v.as_u64().ok_or("invalid recovery count"))?;
        if n >= p.recovery_budget as u64 {
            return Err("recovery budget exhausted; explicitly reset the policy".into());
        }
        d.state["isolation_attempts"] = serde_json::json!(n + 1);
        d.state["quarantined"] = serde_json::json!(false);
        Ok(())
    }
}
#[cfg(feature = "isolation")]
impl NativeFactory<'_> {
    pub fn create_isolated(
        &mut self,
        d: &Descriptor,
        product: &Product,
        state: Option<Blob>,
    ) -> Result<Built, HostError> {
        if product.format != PluginFormat::Vst3 {
            return Err(
                "Studio isolation requires stereo VST3; CLAP child hosting uses host::isolation"
                    .into(),
            );
        }
        if d.state.get("buses").is_some() {
            return Err("isolated Studio instance must use stereo buses".into());
        }
        if d.state["quarantined"].as_bool() == Some(true) {
            return Err("instance quarantined; explicitly recover or bypass".into());
        }
        let policy = Policy::from_descriptor(d)?.ok_or("missing policy")?;
        let helper = self.helpers.check(&policy.helper)?;
        // SAFETY: separate capabilities approve the module and helper executable/dependencies.
        let mut p = unsafe {
            self.loader.load_isolated_vst3(
                product,
                ProcessSpec {
                    max_block_frames: self.spec.max_block_frames.max(soniclaw_vst3::BLOCK),
                    ..self.spec
                },
                soniclaw_vst3::isolation::Config {
                    helper,
                    response_timeout_ms: policy.timeout_ms,
                    max_recoveries: policy.recovery_budget,
                },
            )
        }?;
        if let Some(s) = state.filter(|s| s.matches(d)) {
            let m = s.metadata();
            if m.opaque {
                p.restore_state(&soniclaw_vst3::State {
                    schema: 1,
                    path: product.path.clone(),
                    class_id: product.id.clone(),
                    version: m.identity.version.clone(),
                    bytes: s.bytes().to_vec(),
                    sha256: m.sha256.clone(),
                })?
            }
            for (&id, &v) in &m.parameters {
                p.set_parameter(id, v)?
            }
        }
        let parameters: Vec<ParameterInfo> = p
            .parameters()?
            .into_iter()
            .map(|p| ParameterInfo {
                id: p.id,
                name: p.name,
                min: 0.,
                max: 1.,
                default: p.default,
                value: p.value,
                flags: if p.is_read_only { READ_ONLY } else { 0 },
            })
            .collect();
        let version = p.info()?.version;
        let opaque = d.state["state_mode"].as_str() != Some("parameters");
        let last_good = Blob::new(
            Identity {
                format: "vst3".into(),
                module: product.path.clone(),
                product: product.id.clone(),
                version: version.clone(),
            },
            parameters
                .iter()
                .filter(|p| p.flags & READ_ONLY == 0)
                .map(|p| (p.id, p.value))
                .collect(),
            if opaque {
                Some(p.save_state()?.bytes)
            } else {
                None
            },
        )?;
        let config = soniclaw_vst3::musical::Config {
            timeline: self.timeline.clone(),
            recording: false,
        };
        let processor = if self.mode == Mode::Offline {
            automation::NativeProcessor::Offline(Box::new(p.start_offline_musical(config)?))
        } else {
            start_live(&mut p, config)?
        };
        let controls = automation::controls(d, &parameters)?;
        let handles = controls.iter().map(|c| c.parameter.clone()).collect();
        let failures = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let inner = automation::Automated::new(processor, controls.clone(), failures.clone());
        let latency = inner.latency_frames();
        let protected = Protected {
            inner,
            failures: failures.clone(),
            line: vec![0.; latency * 2],
            pos: 0,
            dry: vec![0.; self.spec.max_block_frames * 2],
            dry_on_fault: policy.dry_on_fault && d.state["role"].as_str() != Some("instrument"),
        };
        self.owners.push(Owner {
            id: d.id.clone(),
            entry: RackEntry {
                path: product.path.clone(),
                plugin_id: product.id.clone(),
            },
            restore_window: false,
            format: product.format,
            backend: Backend::IsolatedVst3(Box::new(p)),
            parameters,
            latency,
            version,
            opaque,
            controls,
            automation_failures: failures,
            targets: Default::default(),
            editor_active: Default::default(),
            editor_edits: Vec::with_capacity(256),
            last_good: Some(last_good),
            fault: None,
        });
        Ok(Built {
            processor: Box::new(protected),
            parameters: handles,
        })
    }
}
// Keep the large live packet construction out of the offline factory stack frame.
#[cfg(feature = "isolation")]
fn start_live(
    p: &mut soniclaw_vst3::isolation::Isolated,
    c: soniclaw_vst3::musical::Config,
) -> Result<automation::NativeProcessor, HostError> {
    Ok(automation::NativeProcessor::Vst3(Box::new(
        p.start_buffered()?.with_music(c)?,
    )))
}
#[cfg(feature = "isolation")]
struct Protected {
    inner: automation::Automated,
    failures: Arc<std::sync::atomic::AtomicU64>,
    line: Vec<f32>,
    pos: usize,
    dry: Vec<f32>,
    dry_on_fault: bool,
}
#[cfg(feature = "isolation")]
impl AudioProcessor for Protected {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        if s.channels != 2 || s.max_block_frames * 2 > self.dry.len() {
            return Err(Error::InvalidSpec);
        }
        self.inner.prepare(s)
    }
    fn reset(&mut self) {
        self.inner.reset();
        self.line.fill(0.);
        self.pos = 0;
    }
    fn latency_frames(&self) -> usize {
        self.inner.latency_frames()
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, c: &ProcessContext<'_>) {
        if b.channels() != 2 || b.samples().len() > self.dry.len() {
            b.clear();
            return;
        }
        for (x, y) in b.samples().iter().zip(&mut self.dry) {
            *y = *x;
            if !self.line.is_empty() {
                std::mem::swap(y, &mut self.line[self.pos]);
                self.pos = (self.pos + 1) % self.line.len();
            }
        }
        if self.failures.load(Ordering::Acquire) == 0 {
            self.inner.process(b, c)
        }
        if self.failures.load(Ordering::Acquire) > 0 {
            if self.dry_on_fault {
                let n = b.samples().len();
                b.samples_mut().copy_from_slice(&self.dry[..n])
            } else {
                b.clear()
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_budget_is_durable() {
        let mut d = Descriptor {
            id: "p".into(),
            kind: "vst3".into(),
            state: serde_json::json!({"isolation":{"helper":"/explicit/helper","timeout_ms":100,"recovery_budget":1,"dry_on_fault":true},"quarantined":true}),
        };
        Policy::request_recovery(&mut d).unwrap();
        assert_eq!(d.state["isolation_attempts"], 1);
        assert_eq!(d.state["quarantined"], false);
        assert!(Policy::request_recovery(&mut d).is_err());
    }
    #[test]
    fn saved_helper_path_does_not_authorize_execution() {
        assert!(TrustedHelpers::default()
            .check(Path::new("/bin/sh"))
            .is_err());
    }
}
