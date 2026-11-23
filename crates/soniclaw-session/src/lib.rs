//! Configuration is portable; native recovery runs on the application's control
//! thread through `poll`, never inside an audio callback.
use serde::{Deserialize, Serialize};
pub type SessionError = Box<dyn std::error::Error>;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SessionSettings {
    pub schema_version: u32,
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub input_channels: [u16; 2],
    pub output_channels: [u16; 2],
    pub sample_rate: u32,
    pub buffer_frames: Option<u32>,
    pub auto_reconnect: bool,
}
impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            input_device: None,
            output_device: None,
            input_channels: [0, 0],
            output_channels: [0, 1],
            sample_rate: 48000,
            buffer_frames: None,
            auto_reconnect: true,
        }
    }
}
impl SessionSettings {
    pub fn validate(&self) -> Result<(), SessionError> {
        if self.schema_version != 1
            || !(8000..=192000).contains(&self.sample_rate)
            || self.output_channels[0] == self.output_channels[1]
            || self
                .input_channels
                .iter()
                .chain(&self.output_channels)
                .any(|v| *v >= 64)
            || self
                .buffer_frames
                .is_some_and(|n| !(16..=8192).contains(&n))
        {
            return Err("invalid or unsupported session settings".into());
        }
        Ok(())
    }
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, SessionError> {
        let settings: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        settings.validate()?;
        Ok(settings)
    }
    pub fn save(&self, path: impl AsRef<std::path::Path>) -> Result<(), SessionError> {
        self.validate()?;
        std::fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}
#[cfg(feature = "native")]
pub mod native {
    use super::*;
    use soniclaw_device::{DuplexHost, DuplexOptions, SessionControls, Telemetry};
    use soniclaw_engine::Engine;
    use soniclaw_record::{RecordSource, RecordSummary, Recorder};
    use std::{
        sync::{atomic::Ordering, Arc},
        time::{Duration, Instant},
    };
    pub struct ActiveSession {
        pub host: DuplexHost,
        pub recorder: Recorder,
    }
    pub struct SessionController {
        pub settings: SessionSettings,
        pub controls: Arc<SessionControls>,
        pub active: Option<ActiveSession>,
        pub status: String,
        pub last_recording_summary: Option<RecordSummary>,
        pub last_recording_error: Option<String>,
        factory: Box<dyn FnMut() -> Engine>,
        requested: Option<SessionSettings>,
        wanted: bool,
        last_poll: Instant,
    }
    impl SessionController {
        pub fn new(settings: SessionSettings, factory: impl FnMut() -> Engine + 'static) -> Self {
            Self {
                settings,
                controls: Arc::new(SessionControls::default()),
                active: None,
                status: "Stopped".into(),
                last_recording_summary: None,
                last_recording_error: None,
                factory: Box::new(factory),
                requested: None,
                wanted: false,
                last_poll: Instant::now(),
            }
        }
        pub fn start(&mut self) -> Result<(), SessionError> {
            self.stop();
            self.settings.validate()?;
            self.requested = Some(self.settings.clone());
            self.wanted = true;
            self.open()
        }
        fn open(&mut self) -> Result<(), SessionError> {
            let settings = self
                .requested
                .as_ref()
                .ok_or("start a session before opening audio")?;
            let options = DuplexOptions {
                output_bus: None,
                cue_outputs: Vec::new(),

                input_bus: None,
                host: None,
                input_occurrence: None,
                output_occurrence: None,
                output_hardware_channels: None,
                input_device: settings.input_device.clone(),
                output_device: settings.output_device.clone(),
                input_channels: settings.input_channels,
                output_channels: settings.output_channels,
                sample_rate: settings.sample_rate,
                buffer_frames: settings.buffer_frames,
                max_internal_frames: 256,
            };
            let (recorder, tap) = Recorder::new(128);
            match DuplexHost::start(
                (self.factory)(),
                &options,
                Arc::new(Telemetry::default()),
                self.controls.clone(),
                Some(tap),
            ) {
                Ok(host) => {
                    self.status = format!("Running · {}", host.output_name);
                    self.active = Some(ActiveSession { host, recorder });
                    Ok(())
                }
                Err(error) => {
                    self.status = format!("Waiting for audio: {error}");
                    Err(error)
                }
            }
        }
        pub fn stop(&mut self) {
            self.wanted = false;
            self.controls.monitor.store(false, Ordering::Relaxed);
            if let Some(mut active) = self.active.take() {
                let result = active.recorder.stop();
                if active.recorder.summary().captured_frames > 0 {
                    self.last_recording_summary = Some(active.recorder.summary());
                }
                if let Err(error) = result {
                    self.status = format!("Recording finalization failed: {error}");
                    self.last_recording_error = Some(error.to_string());
                    return;
                }
            }
            self.status = "Stopped".into();
        }
        /// Call from the UI/control thread. Failed sessions retry once per second.
        /// Driver stream errors trigger a fresh engine; recording finalizes first.
        pub fn poll(&mut self) {
            if self.last_poll.elapsed() < Duration::from_secs(1) {
                return;
            }
            self.last_poll = Instant::now();
            let missing = soniclaw_device::DeviceCatalog::enumerate()
                .ok()
                .is_some_and(|catalog| {
                    self.active.as_ref().is_some_and(|s| {
                        !catalog.outputs.iter().any(|d| d.name == s.host.output_name)
                            || s.host
                                .input_name
                                .as_ref()
                                .is_some_and(|name| !catalog.inputs.iter().any(|d| &d.name == name))
                    })
                });
            if missing
                || self
                    .active
                    .as_ref()
                    .is_some_and(|s| s.host.telemetry.stream_errors.load(Ordering::Relaxed) > 0)
            {
                let resume = self.wanted;
                self.stop();
                self.wanted = resume;
            }
            if self.wanted && self.active.is_none() && self.settings.auto_reconnect {
                let _ = self.open();
            }
        }
        pub fn record(
            &mut self,
            path: impl AsRef<std::path::Path>,
            source: RecordSource,
        ) -> Result<(), SessionError> {
            let active = self.active.as_mut().ok_or("start an audio session first")?;
            active
                .recorder
                .start(path, active.host.spec.sample_rate as u32, source)
                .map_err(|e| -> SessionError { e.to_string().into() })?;
            self.last_recording_error = None;
            Ok(())
        }
        pub fn stop_recording(&mut self) -> Result<(), SessionError> {
            if let Some(active) = self.active.as_mut() {
                let result = active.recorder.stop();
                self.last_recording_summary = Some(active.recorder.summary());
                if let Err(error) = result {
                    self.last_recording_error = Some(error.to_string());
                    return Err(error.to_string().into());
                }
            }
            Ok(())
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_roundtrip_and_version_validation() {
        let settings = SessionSettings::default();
        let bytes = serde_json::to_vec(&settings).unwrap();
        let mut loaded: SessionSettings = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(loaded, settings);
        loaded.schema_version = 99;
        assert!(loaded.validate().is_err());
    }
}
