use soniclaw_device::*;
use soniclaw_engine::Engine;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
fn selector(name: &str) -> DeviceSelector {
    DeviceSelector {
        name: name.into(),
        occurrence: 0,
    }
}
fn inventory() -> OutputInventory {
    OutputInventory {
        inputs: vec![],
        default_input: None,
        host: "Mock".into(),
        hosts: vec!["Mock".into()],
        default_output: Some(selector("Speakers")),
        outputs: ["Speakers", "USB"]
            .into_iter()
            .map(|name| OutputDevice {
                selector: selector(name),
                default_rate: 48000,
                default_channels: 2,
                configs: vec![OutputRange {
                    channels: 2,
                    min_rate: 44100,
                    max_rate: 96000,
                    buffer_range: Some((64, 1024)),
                    format: OutputFormat::F32,
                }],
            })
            .collect(),
        warnings: vec![],
    }
}
#[derive(Clone)]
struct Sim {
    inventory: OutputInventory,
    fail: bool,
    plans: Vec<OutputPlan>,
    enumerations: usize,
}
struct Mock {
    sim: Arc<Mutex<Sim>>,
    drops: Arc<AtomicUsize>,
    thread: std::thread::ThreadId,
}
struct Stream {
    drops: Arc<AtomicUsize>,
    thread: std::thread::ThreadId,
}
impl Drop for Stream {
    fn drop(&mut self) {
        assert_eq!(self.thread, std::thread::current().id());
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
impl OutputBackend for Mock {
    type Stream = Stream;
    fn enumerate(&mut self, _: &DevicePreferences) -> Result<OutputInventory, DeviceError> {
        assert_eq!(self.thread, std::thread::current().id());
        let mut s = self.sim.lock().unwrap();
        s.enumerations += 1;
        Ok(s.inventory.clone())
    }
    fn open(
        &mut self,
        p: &OutputPlan,
        b: OutputEngine,
        _: Arc<Telemetry>,
    ) -> Result<Stream, DeviceError> {
        assert_eq!(self.thread, std::thread::current().id());
        let mut s = self.sim.lock().unwrap();
        s.plans.push(p.clone());
        if s.fail {
            return Err("mock open failure".into());
        }
        let mut engine = b.engine;
        engine.prepare(p.spec())?;
        Ok(Stream {
            drops: self.drops.clone(),
            thread: self.thread,
        })
    }
    fn close(&mut self, stream: Stream) -> Option<RecordingReport> {
        drop(stream);
        if self
            .sim
            .lock()
            .unwrap()
            .plans
            .last()
            .is_some_and(|p| p.duplex.is_some())
        {
            Some(RecordingReport {
                captured_frames: 64,
                written_frames: 60,
                dropped_frames: 4,
                write_errors: 1,
                error: Some("simulated finalization failure".into()),
            })
        } else {
            None
        }
    }
}
fn setup(p: DevicePreferences) -> (DeviceManager<Mock>, Arc<Mutex<Sim>>, Arc<AtomicUsize>) {
    let sim = Arc::new(Mutex::new(Sim {
        inventory: inventory(),
        fail: false,
        plans: vec![],
        enumerations: 0,
    }));
    let drops = Arc::new(AtomicUsize::new(0));
    (
        DeviceManager::new(
            Mock {
                sim: sim.clone(),
                drops: drops.clone(),
                thread: std::thread::current().id(),
            },
            p,
        )
        .unwrap(),
        sim,
        drops,
    )
}
fn build(_: &OutputPlan, _: Arc<Telemetry>) -> Result<OutputEngine, DeviceError> {
    Ok(OutputEngine {
        output_bus: None,

        input_bus: None,
        engine: Engine::default(),
        receivers: vec![],
    })
}
#[test]
fn selects_rates_channels_buffers_and_formats_without_opening() {
    let mut p = DevicePreferences {
        output: Some(selector("USB")),
        sample_rate: Some(44100),
        buffer_frames: Some(128),
        ..Default::default()
    };
    let c = inventory();
    let plan = negotiate_output(&p, &c).unwrap();
    assert_eq!(plan.output.name, "USB");
    assert_eq!(plan.spec().sample_rate, 44100.0);
    assert_eq!(plan.buffer_frames, Some(128));
    p.sample_rate = Some(192000);
    assert!(negotiate_output(&p, &c).is_err());
    p.sample_rate = Some(48000);
    p.channels = Some(1);
    assert!(negotiate_output(&p, &c).is_err());
    p.channels = None;
    p.buffer_frames = Some(32);
    assert!(negotiate_output(&p, &c).is_err());
    p.buffer_frames = None;
    let mut c = c;
    c.outputs[1].configs[0].format = OutputFormat::I16;
    assert_eq!(negotiate_output(&p, &c).unwrap().format, OutputFormat::I16);
}
#[test]
fn disconnect_waits_then_reconnects_only_after_interval() {
    let (mut m, sim, drops) = setup(DevicePreferences {
        output: Some(selector("USB")),
        ..Default::default()
    });
    m.start(Duration::ZERO, &mut build).unwrap();
    sim.lock()
        .unwrap()
        .inventory
        .outputs
        .retain(|d| d.selector.name != "USB");
    assert!(m.poll(Duration::from_secs(1), &mut build));
    assert_eq!(m.status().state, ManagerState::Waiting);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    sim.lock().unwrap().inventory = inventory();
    assert!(!m.poll(Duration::from_millis(1500), &mut build));
    assert!(m.poll(Duration::from_secs(2), &mut build));
    assert!(m.is_running());
    assert_eq!(m.status().reconnects, 1);
    m.stop();
    let checks = sim.lock().unwrap().enumerations;
    assert!(!m.poll(Duration::from_secs(5), &mut build));
    assert_eq!(sim.lock().unwrap().enumerations, checks);
}
#[test]
fn stream_error_rebuilds_and_manual_stop_cancels_retries() {
    let (mut m, sim, drops) = setup(Default::default());
    m.start(Duration::ZERO, &mut build).unwrap();
    m.telemetry().stream_errors.fetch_add(1, Ordering::Relaxed);
    assert!(m.poll(Duration::from_secs(1), &mut build));
    assert_eq!(m.status().reconnects, 1);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    sim.lock().unwrap().fail = true;
    m.telemetry().stream_errors.fetch_add(1, Ordering::Relaxed);
    m.poll(Duration::from_secs(2), &mut build);
    assert_eq!(m.status().state, ManagerState::Waiting);
    m.stop();
    assert!(!m.poll(Duration::from_secs(99), &mut build));
}
#[test]
fn fallback_is_explicit_and_returns_to_preferred_device() {
    let mut p = DevicePreferences {
        output: Some(selector("USB")),
        ..Default::default()
    };
    let mut c = inventory();
    c.outputs.retain(|d| d.selector.name != "USB");
    assert!(negotiate_output(&p, &c).is_err());
    p.fallback_to_default = true;
    assert!(negotiate_output(&p, &c).unwrap().used_fallback);
    let (mut m, sim, _) = setup(p);
    sim.lock().unwrap().inventory = c;
    m.start(Duration::ZERO, &mut build).unwrap();
    assert_eq!(m.status().active.as_ref().unwrap().output.name, "Speakers");
    sim.lock().unwrap().inventory = inventory();
    m.poll(Duration::from_secs(1), &mut build);
    assert_eq!(m.status().active.as_ref().unwrap().output.name, "USB");
}
#[test]
fn disabled_reconnect_requires_manual_retry() {
    let (mut m, sim, _) = setup(DevicePreferences {
        reconnect: false,
        ..Default::default()
    });
    m.start(Duration::ZERO, &mut build).unwrap();
    m.telemetry().stream_errors.fetch_add(1, Ordering::Relaxed);
    m.poll(Duration::from_secs(1), &mut build);
    assert_eq!(m.status().state, ManagerState::Failed);
    assert!(!m.poll(Duration::from_secs(2), &mut build));
    m.start(Duration::from_secs(3), &mut build).unwrap();
    assert_eq!(sim.lock().unwrap().plans.len(), 2);
}
#[test]
fn preferences_apply_transactionally_and_default_device_changes_restart() {
    let (mut m, sim, _) = setup(Default::default());
    m.start(Duration::ZERO, &mut build).unwrap();
    let old = m.preferences().clone();
    let invalid = DevicePreferences {
        max_internal_frames: 0,
        ..old.clone()
    };
    assert!(m
        .apply(invalid, Duration::from_secs(1), &mut build)
        .is_err());
    assert_eq!(m.preferences(), &old);
    assert!(m.is_running());
    sim.lock().unwrap().inventory.default_output = Some(selector("USB"));
    m.poll(Duration::from_secs(1), &mut build);
    assert_eq!(m.status().active.as_ref().unwrap().output.name, "USB");
    m.apply(
        DevicePreferences {
            sample_rate: Some(44100),
            buffer_frames: Some(128),
            ..old
        },
        Duration::from_secs(2),
        &mut build,
    )
    .unwrap();
    assert_eq!(m.status().active.as_ref().unwrap().sample_rate, 44100);
}
#[test]
fn preferences_roundtrip_and_reject_corrupt_unknown_and_oversized_files() {
    let root = std::env::temp_dir().join(format!("soniclaw-device-prefs-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("prefs.json");
    let p = DevicePreferences {
        output: Some(selector("USB")),
        ..Default::default()
    };
    p.save(&path).unwrap();
    assert_eq!(DevicePreferences::load(&path).unwrap(), p);
    for bytes in [
        b"invalid".to_vec(),
        vec![b' '; 16385],
        serde_json::to_vec(&serde_json::json!({"schema_version":99})).unwrap(),
    ] {
        std::fs::write(&path, bytes).unwrap();
        assert!(DevicePreferences::load(&path).is_err());
    }
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn duplicate_names_use_occurrence_and_unknown_buffer_range_is_a_request() {
    let mut c = inventory();
    c.outputs[1].selector = DeviceSelector {
        name: "Speakers".into(),
        occurrence: 1,
    };
    c.outputs[1].configs[0].buffer_range = None;
    let p = DevicePreferences {
        output: Some(c.outputs[1].selector.clone()),
        buffer_frames: Some(32),
        ..Default::default()
    };
    let plan = negotiate_output(&p, &c).unwrap();
    assert_eq!(plan.output.occurrence, 1);
    assert_eq!(plan.buffer_frames, Some(32));
}

#[test]
fn unified_negotiates_shared_rate_and_hardware_maps() {
    let mut c = inventory();
    c.outputs[0].configs[0].channels = 8;
    c.outputs[0].default_channels = 8;
    let mut input = c.outputs[1].clone();
    input.selector = selector("Input");
    input.configs[0].channels = 4;
    c.inputs = vec![input];
    c.default_input = Some(selector("Input"));
    let mut p = DevicePreferences {
        duplex: Some(DuplexRoute {
            input_count: None,
            input_channels: [2, 3],
            output_channels: [2, 7],
            ..Default::default()
        }),
        ..Default::default()
    };
    let plan = negotiate_output(&p, &c).unwrap();
    assert_eq!(plan.spec().channels, 2);
    assert_eq!(plan.hardware_channels, 8);
    assert_eq!(
        plan.duplex.as_ref().unwrap().input.as_ref().unwrap().name,
        "Input"
    );
    p.sample_rate = Some(192000);
    assert!(negotiate_output(&p, &c).is_err());
    p.sample_rate = Some(48000);
    p.duplex.as_mut().unwrap().input_channels = [4, 4];
    assert!(negotiate_output(&p, &c).is_err());
}
#[test]
fn input_loss_closes_take_and_retries_both_streams() {
    let p = DevicePreferences {
        duplex: Some(Default::default()),
        ..Default::default()
    };
    let (mut m, sim, drops) = setup(p);
    {
        let mut s = sim.lock().unwrap();
        let mut input = s.inventory.outputs[1].clone();
        input.selector = selector("Input");
        s.inventory.inputs = vec![input];
        s.inventory.default_input = Some(selector("Input"));
    }
    m.start(Duration::ZERO, &mut build).unwrap();
    sim.lock().unwrap().inventory.inputs.clear();
    m.poll(Duration::from_secs(1), &mut build);
    assert_eq!(m.status().state, ManagerState::Waiting);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    let report = m.status().last_recording.as_ref().unwrap();
    assert_eq!(report.captured_frames, 64);
    assert!(report.error.as_ref().unwrap().contains("simulated"));
    {
        let mut s = sim.lock().unwrap();
        let mut input = s.inventory.outputs[1].clone();
        input.selector = selector("Input");
        s.inventory.inputs = vec![input];
    }
    m.poll(Duration::from_secs(2), &mut build);
    assert!(m.is_running());
    assert_eq!(m.status().reconnects, 1);
    assert!(m.status().last_recording.as_ref().unwrap().error.is_some());
    m.stop();
}
#[test]
fn unified_output_recording_can_disable_input_and_old_preferences_load() {
    let p = DevicePreferences {
        duplex: Some(DuplexRoute {
            input_count: None,
            use_default_input: false,
            ..Default::default()
        }),
        ..Default::default()
    };
    let plan = negotiate_output(&p, &inventory()).unwrap();
    assert!(plan.duplex.unwrap().input.is_none());
    let old = serde_json::to_string(&DevicePreferences::default()).unwrap();
    assert!(!old.contains("duplex"));
    let loaded: DevicePreferences = serde_json::from_str(&old).unwrap();
    assert!(loaded.duplex.is_none());
    let bad = DevicePreferences {
        duplex: Some(DuplexRoute {
            input_count: None,
            output_channels: [0, 0],
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(bad.validate().is_err());
}

#[test]
fn unified_layout_selection_accounts_for_requested_buffer() {
    let mut c = inventory();
    let device = &mut c.outputs[0];
    device.configs[0].buffer_range = Some((32, 64));
    device.configs.push(OutputRange {
        channels: 4,
        min_rate: 44100,
        max_rate: 96000,
        buffer_range: Some((128, 512)),
        format: OutputFormat::F32,
    });
    let p = DevicePreferences {
        duplex: Some(DuplexRoute {
            input_count: None,
            use_default_input: false,
            ..Default::default()
        }),
        buffer_frames: Some(256),
        ..Default::default()
    };
    let plan = negotiate_output(&p, &c).unwrap();
    assert_eq!(plan.hardware_channels, 4);
    assert_eq!(plan.channels, 2);
    assert_eq!(plan.buffer_frames, Some(256));
}
