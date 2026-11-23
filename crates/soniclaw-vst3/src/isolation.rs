//! Explicit child-process isolation and checkpoint-based recovery. All methods are control-thread operations.
use super::*;
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub helper: PathBuf,
    pub response_timeout_ms: u64,
    pub max_recoveries: u8,
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        if !self.helper.is_absolute()
            || !self.helper.is_file()
            || !(50..=10000).contains(&self.response_timeout_ms)
            || !(1..=8).contains(&self.max_recoveries)
        {
            return Err("isolation requires an explicit executable path, timeout50..10000ms and recovery budget1..8".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub schema: u32,
    pub state: State,
    pub parameters: BTreeMap<u32, f64>,
}
impl Checkpoint {
    pub fn validate(&self) -> Result<()> {
        self.state.validate()?;
        if self.schema != 1
            || self.parameters.len() > 4096
            || self
                .parameters
                .values()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return Err("invalid isolated checkpoint".into());
        }
        Ok(())
    }
}
impl Checkpoint {
    pub fn from_json(text: &str) -> Result<Self> {
        if text.len() > 24 * 1024 * 1024 {
            return Err("isolated checkpoint JSON exceeds24MiB".into());
        }
        let cp: Self = serde_json::from_str(text)?;
        cp.validate()?;
        Ok(cp)
    }
    pub fn to_json(&self) -> Result<String> {
        self.validate()?;
        Ok(serde_json::to_string_pretty(self)?)
    }
}
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub enum Phase {
    Ready,
    Running,
    Stopped,
    Faulted,
    Quarantined,
}
#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub phase: Phase,
    pub recovery_attempts: u8,
    pub helper_pid: Option<u32>,
    pub last_error: Option<String>,
    pub audio: Snapshot,
}
/// Main-thread owner. Native DSP and module loading execute in the explicitly selected helper.
#[cfg(feature = "isolation")]
pub struct Isolated {
    instance: Option<Instance>,
    path: PathBuf,
    class: String,
    spec: ProcessSpec,
    config: Config,
    checkpoint: Option<Checkpoint>,
    controls: BTreeMap<u32, f64>,
    attempts: u8,
    phase: Phase,
    error: Option<String>,
}
#[cfg(feature = "isolation")]
impl Isolated {
    /// # Safety
    /// Select a trusted helper executable and native module/dependencies on the main thread.
    /// Process isolation contains crashes; it is not an operating-system security sandbox.
    pub unsafe fn load(
        path: &Path,
        class: Option<&str>,
        spec: ProcessSpec,
        mut config: Config,
    ) -> Result<Self> {
        config.validate()?;
        config.helper = config.helper.canonicalize()?;
        let instance = unsafe { Instance::load_internal(path, class, spec, true, Some(&config)) }?;
        let info = instance.info()?;
        Ok(Self {
            instance: Some(instance),
            path: info.path,
            class: info.uid,
            spec,
            config,
            checkpoint: None,
            controls: BTreeMap::new(),
            attempts: 0,
            phase: Phase::Ready,
            error: None,
        })
    }
    fn owner(&self) -> Result<&Instance> {
        self.instance
            .as_ref()
            .ok_or_else(|| err("isolated owner unavailable; recover or reconstruct"))
    }
    fn owner_mut(&mut self) -> Result<&mut Instance> {
        self.instance
            .as_mut()
            .ok_or_else(|| err("isolated owner unavailable; recover or reconstruct"))
    }
    pub fn start_offline_musical(
        &mut self,
        config: musical::Config,
    ) -> Result<musical_offline::OfflineProcessor> {
        if matches!(self.phase, Phase::Faulted | Phase::Quarantined) {
            return Err("recover isolated owner first".into());
        }
        let p = self.owner_mut()?.start_offline_musical(config)?;
        self.phase = Phase::Running;
        Ok(p)
    }
    pub fn offline_errors(&self) -> u64 {
        self.instance.as_ref().map_or(0, |p| p.offline_errors())
    }
    pub fn info(&self) -> Result<PluginInfo> {
        self.owner()?.info()
    }
    pub fn latency_frames(&self) -> Result<usize> {
        self.owner()?.latency_frames()
    }
    pub fn restore_state(&mut self, s: &State) -> Result<()> {
        self.owner_mut()?.restore_state(s)
    }
    pub fn save_state(&mut self) -> Result<State> {
        self.owner_mut()?.save_state()
    }
    pub fn parameters(&self) -> Result<Vec<Parameter>> {
        self.owner()?.parameters()
    }
    pub fn helper_pid(&self) -> Option<u32> {
        self.instance
            .as_ref()
            .and_then(|i| i.lock().ok().and_then(|p| p.isolation_pid()))
    }
    pub fn status(&self) -> Status {
        let audio = self
            .instance
            .as_ref()
            .map(|i| i.stats())
            .unwrap_or_else(|| Shared::default().snapshot());
        Status {
            phase: if audio.process_errors > 0 && self.phase == Phase::Running {
                Phase::Faulted
            } else {
                self.phase
            },
            recovery_attempts: self.attempts,
            helper_pid: self.helper_pid(),
            last_error: self.error.clone(),
            audio,
        }
    }
    pub fn start_buffered(&mut self) -> Result<Processor> {
        if matches!(self.phase, Phase::Faulted | Phase::Quarantined) {
            return Err("recover the failed isolated owner before starting".into());
        }
        let processor = self.owner_mut()?.start_buffered()?;
        self.phase = Phase::Running;
        Ok(processor)
    }
    pub fn set_parameter(&mut self, id: u32, value: f64) -> Result<()> {
        self.owner_mut()?.set_parameter(id, value)?;
        self.controls.insert(id, value);
        Ok(())
    }
    /// Snapshot opaque state plus normalized controls while stopped. Does not persist DSP history.
    pub fn checkpoint(&mut self) -> Result<Checkpoint> {
        let state = self.owner_mut()?.save_state()?;
        let parameters = self
            .parameters()?
            .into_iter()
            .filter(|p| !p.is_read_only)
            .map(|p| (p.id, p.value))
            .collect();
        let cp = Checkpoint {
            schema: 1,
            state,
            parameters,
        };
        cp.validate()?;
        self.controls.clear();
        self.checkpoint = Some(cp.clone());
        Ok(cp)
    }
    pub fn restore_checkpoint(&mut self, cp: Checkpoint) -> Result<()> {
        cp.validate()?;
        self.owner_mut()?.restore_state(&cp.state)?;
        for (&id, &value) in &cp.parameters {
            self.owner_mut()?.set_parameter(id, value)?;
        }
        self.controls.clear();
        self.checkpoint = Some(cp);
        Ok(())
    }
    pub fn stop(&mut self) -> Result<()> {
        let result = self.owner_mut()?.stop();
        if result.is_ok() && self.phase != Phase::Faulted {
            self.phase = Phase::Stopped;
        }
        result
    }
    pub fn poll(&mut self) -> Result<()> {
        if matches!(self.phase, Phase::Faulted | Phase::Quarantined) {
            return Err(err(self
                .error
                .as_deref()
                .unwrap_or("isolated helper failed")));
        }
        let result = if self.owner()?.stats().process_errors > 0
            || (self.phase == Phase::Running && self.owner()?.shared.stop.load(Ordering::Acquire))
        {
            Err(err("isolated audio operation failed or timed out"))
        } else {
            self.owner_mut()?.poll()
        };
        if let Err(error) = result {
            self.error = Some(error.to_string());
            self.phase = Phase::Faulted;
            let _ = self.owner_mut()?.stop();
            return Err(error);
        }
        Ok(())
    }
    /// Explicit recovery on main: revoke old callbacks, spawn a fresh helper, restore checkpoint and later controls.
    /// Every attempt consumes budget, including a failed spawn/reload. Returns stopped/ready; obtain a new callback.
    pub fn recover(&mut self) -> Result<()> {
        if self.attempts >= self.config.max_recoveries {
            self.phase = Phase::Quarantined;
            if let Some(owner) = &mut self.instance {
                let _ = owner.stop();
            }
            return Err("isolated product recovery budget exhausted; reconstruct explicitly to clear quarantine".into());
        }
        self.attempts += 1;
        if let Some(mut old) = self.instance.take() {
            let _ = old.stop();
            drop(old);
        }
        self.phase = Phase::Faulted;
        // SAFETY: original load contract asserts continued trust of these canonical paths and dependencies.
        let result = (|| -> Result<()> {
            let mut fresh = unsafe {
                Instance::load_internal(
                    &self.path,
                    Some(&self.class),
                    self.spec,
                    true,
                    Some(&self.config),
                )
            }?;
            if let Some(cp) = &self.checkpoint {
                fresh.restore_state(&cp.state)?;
                for (&id, &value) in &cp.parameters {
                    fresh.set_parameter(id, value)?;
                }
            }
            for (&id, &value) in &self.controls {
                fresh.set_parameter(id, value)?;
            }
            self.instance = Some(fresh);
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.phase = Phase::Ready;
                self.error = None;
                Ok(())
            }
            Err(e) => {
                self.error = Some(e.to_string());
                if self.attempts >= self.config.max_recoveries {
                    self.phase = Phase::Quarantined;
                }
                Err(e)
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "isolation")]
    #[test]
    fn full_musical_context_survives_helper_protocol() {
        use vst3_host::{
            plugin::{MusicalTransport, TransportPosition},
            process_isolation::HostCommand,
        };
        let context = MusicalTransport {
            position: TransportPosition {
                samples: 123456,
                quarter_notes: 9.5,
            },
            tempo: 137.25,
            numerator: 7,
            denominator: 8,
            bar_start: 7.,
            cycle: Some([3., 17.]),
            playing: true,
            recording: true,
        };
        let encoded = serde_json::to_string(&HostCommand::SetMusicalTransport { context }).unwrap();
        let HostCommand::SetMusicalTransport { context: c } =
            serde_json::from_str(&encoded).unwrap()
        else {
            panic!("context command changed")
        };
        assert_eq!(c.position, context.position);
        assert_eq!(c.tempo.to_bits(), context.tempo.to_bits());
        assert_eq!((c.numerator, c.denominator), (7, 8));
        assert_eq!(c.bar_start, 7.);
        assert_eq!(c.cycle, Some([3., 17.]));
        assert!(c.playing && c.recording);
    }
    #[test]
    fn invalid_policy_rejected_without_native_execution() {
        let c = Config {
            helper: PathBuf::from("relative"),
            response_timeout_ms: 500,
            max_recoveries: 3,
        };
        assert!(c.validate().is_err());
    }
}
