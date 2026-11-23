//! Deliberately faults only an explicitly selected owned fixture in a child process.
use soniclaw::{
    core::*,
    studio::{self, project::ProjectError, Descriptor, Factory},
};
use soniclaw_host::{
    formats::TrustedLoader,
    studio::{
        isolation_adapter::{Policy, TrustedHelpers},
        Backend, Mode, NativeFactory,
    },
};
use std::{path::PathBuf, time::Duration};
fn context(sample: u64) -> ProcessContext<'static> {
    ProcessContext {
        events: &[],
        transport: Transport {
            sample_position: sample,
            playing: true,
            ..Default::default()
        },
    }
}
fn main() -> Result<(), ProjectError> {
    let a: Vec<_> = std::env::args().collect();
    let out = PathBuf::from(a.get(1).ok_or("NEW_FOLDER ABS_OWNED_FIXTURE ABS_HELPER")?);
    std::fs::create_dir(&out)?;
    let module = PathBuf::from(a.get(2).ok_or("fixture")?).canonicalize()?;
    let helper = PathBuf::from(a.get(3).ok_or("helper")?).canonicalize()?;
    // SAFETY: the caller explicitly selects this owned module and its native dependencies.
    let loader = unsafe { TrustedLoader::new(std::slice::from_ref(&module)) }?;
    // SAFETY: the caller separately selects the trusted helper and its dependencies.
    let helpers = unsafe { TrustedHelpers::new(std::slice::from_ref(&helper)) }?;
    let mut p = studio::workspace::blank("Isolation".into(), 48000, 128)?;
    let mut d = Descriptor {
        id: "isolated".into(),
        kind: "vst3".into(),
        state: serde_json::json!({"path":module,"class_id":"53434C5748535450524F543100000001","state_mode":"parameters","automation_ids":[9,11],"isolation":Policy{helper:helper.clone(),timeout_ms:500,recovery_budget:4,dry_on_fault:true}}),
    };
    let mut reports = vec![];
    for (fault, instrument, dry) in [
        (0.25f32, false, true),
        (0.75, false, true),
        (0.25, false, false),
        (0.25, true, true),
    ] {
        d.state.as_object_mut().unwrap().remove("state_mode");
        if !dry {
            d.state["state_mode"] = serde_json::json!("parameters");
        }
        d.state["role"] = serde_json::json!(if instrument { "instrument" } else { "effect" });
        d.state["isolation"]["dry_on_fault"] = serde_json::json!(dry);
        let mut owners = vec![];
        let mut buses = Default::default();
        let mut built = {
            let mut f = NativeFactory {
                helpers: &helpers,
                bus_graph: &mut buses,
                spec: p.document.spec(),
                mode: Mode::Live,
                timeline: &p.document.timeline,
                assets: &p.assets,
                states: &p.native_states,
                loader: &loader,
                owners: &mut owners,
                last_error: None,
            };
            f.create(&d)?
        };
        built.processor.prepare(p.document.spec())?;
        let old_pid = owners[0].isolated_pid().ok_or("no helper PID")?;
        let mut allocations = 0;
        let mut peak = 0f32;
        for n in 0..64 {
            let mut data = [0.25; 256];
            allocations += allocation_counter::measure(|| {
                built.processor.process(
                    &mut AudioBuffer::new(&mut data, 2).unwrap(),
                    &context(n * 128),
                )
            })
            .count_total;
            peak = peak.max(data.iter().copied().fold(0f32, |a, b| a.max(b.abs())));
            std::thread::sleep(Duration::from_millis(8));
        }
        if let Backend::IsolatedVst3(r) = &owners[0].backend {
            let metadata = r.parameters()?;
            assert!((metadata.iter().find(|p| p.id == 600).unwrap().value - 0.12).abs() < 1e-6);
            assert!((metadata.iter().find(|p| p.id == 607).unwrap().value - 1. / 16.).abs() < 1e-6);
            eprintln!(
                "healthy status {:?}, failures {}",
                r.status(),
                owners[0]
                    .automation_failures
                    .load(std::sync::atomic::Ordering::Acquire)
            );
        }
        assert_eq!(allocations, 0);
        assert!((peak - 0.125).abs() < 1e-6, "healthy peak {peak}");
        owners[0].set_parameter(0, 11, fault as f64)?;
        for n in 64..200 {
            let mut data = [0.25; 256];
            built
                .processor
                .process(&mut AudioBuffer::new(&mut data, 2)?, &context(n * 128));
            std::thread::sleep(Duration::from_millis(8));
            if owners[0].poll().is_err() {
                break;
            }
        }
        assert!(owners[0].poll().is_err(), "helper did not fault");
        let mut data = [0.25; 256];
        built
            .processor
            .process(&mut AudioBuffer::new(&mut data, 2)?, &context(25600));
        assert_eq!(data, [if dry && !instrument { 0.25 } else { 0. }; 256]);
        let fallback_sample = data[0];
        // Reset the owned fixture's deliberate crash switch in the host targets before recovery.
        owners[0].set_parameter(0, 11, 0.)?;
        let state = owners[0].snapshot()?;
        assert_eq!(state.metadata().opaque, dry);
        assert!(owners[0].fault.is_some());
        p.native_states.insert(d.id.clone(), state);
        d.state["native_state"] = serde_json::json!(true);
        d.state["quarantined"] = serde_json::json!(true);
        drop(built);
        owners.clear();
        Policy::request_recovery(&mut d)?;
        let mut fresh = {
            let mut f = NativeFactory {
                helpers: &helpers,
                bus_graph: &mut buses,
                spec: p.document.spec(),
                mode: Mode::Offline,
                timeline: &p.document.timeline,
                assets: &p.assets,
                states: &p.native_states,
                loader: &loader,
                owners: &mut owners,
                last_error: None,
            };
            f.create(&d)?
        };
        fresh.processor.prepare(p.document.spec())?;
        let new_pid = owners[0].isolated_pid().ok_or("new PID")?;
        assert_ne!(old_pid, new_pid);
        let mut data = [0.25; 256];
        fresh
            .processor
            .process(&mut AudioBuffer::new(&mut data, 2)?, &context(0));
        assert_eq!(data, [0.125; 256]);
        assert_eq!(
            owners[0]
                .automation_failures
                .load(std::sync::atomic::Ordering::Acquire),
            0
        );
        reports.push(serde_json::json!({"fault":fault,"parent_survived":true,"old_pid":old_pid,"new_pid":new_pid,"callback_allocations":allocations,"healthy_peak":peak,"dry_requested":dry,"instrument":instrument,"fallback_sample":fallback_sample,"checkpoint_restored":true,"opaque_checkpoint":dry,"offline_peak":data[0]}));
        drop(fresh);
        owners.clear();
    }
    p.document.tracks[0].inserts.push(d);
    p.save_new(&out.join("project"))?;
    let reopened = studio::project::Project::open(&out.join("project"))?;
    assert_eq!(
        reopened.document.tracks[0].inserts[0].state["isolation_attempts"],
        4
    );
    // Exercise the complete Studio graph and exclusive offline export cleanup as well as the adapter.
    p.assets.insert(
        "signal".into(),
        soniclaw::clips::AudioAsset {
            sample_rate: 48000,
            channels: 1,
            samples: vec![0.25; 1024].into(),
        },
    );
    p.document.tracks[0].audio.push(soniclaw::clips::AudioClip {
        id: "clip".into(),
        asset: "signal".into(),
        track: 0,
        start_frame: 0,
        length_frames: 1024,
        source_offset: 0,
        source_length: 1024,
        looping: false,
        gain: 1.,
        fade_in: 0,
        fade_out: 0,
        enabled: true,
        edits: None,
    });
    let mut owners = vec![];
    let mut buses = Default::default();
    {
        let mut f = NativeFactory {
            helpers: &Default::default(),
            bus_graph: &mut buses,
            spec: p.document.spec(),
            mode: Mode::Offline,
            timeline: &p.document.timeline,
            assets: &p.assets,
            states: &p.native_states,
            loader: &loader,
            owners: &mut owners,
            last_error: None,
        };
        assert!(f.create_detailed(&p.document.tracks[0].inserts[0]).is_err());
    }
    assert!(owners.is_empty());
    let mut offline = {
        let mut f = NativeFactory {
            helpers: &helpers,
            bus_graph: &mut buses,
            spec: p.document.spec(),
            mode: Mode::Offline,
            timeline: &p.document.timeline,
            assets: &p.assets,
            states: &p.native_states,
            loader: &loader,
            owners: &mut owners,
            last_error: None,
        };
        p.document.compile_offline(&p.assets, &p.disk, &mut f)?
    };
    let mut data = [0.; 256];
    offline.render_block(&mut AudioBuffer::new(&mut data, 2)?, &context(0))?;
    assert!(data.iter().any(|v| v.abs() > 0.05));
    drop(offline);
    owners.clear();
    p.document
        .parameter_values
        .insert("isolated.native.11".into(), 0.25);
    let pool = studio::jobs::Scheduler::new(Default::default())?;
    let path = out.join("failed-export");
    let check = path.clone();
    let mut ticket = pool.submit(
        "export-fault",
        0,
        studio::jobs::Priority::Normal,
        64 * 1024 * 1024,
        move |ctx| {
            // SAFETY: these canonical paths were explicitly approved by the caller before moving to this worker.
            let loader = unsafe { TrustedLoader::new(std::slice::from_ref(&module)) }
                .map_err(|e| e.to_string())?;
            // SAFETY: separate helper approval from the same explicit invocation.
            let helpers = unsafe { TrustedHelpers::new(std::slice::from_ref(&helper)) }
                .map_err(|e| e.to_string())?;
            let result = soniclaw_host::studio::render_export_with_helpers(
                &p,
                studio::export::Settings {
                    length_frames: 256,
                    tail_frames: 0,
                    ..Default::default()
                },
                &path,
                &loader,
                &helpers,
                &ctx,
            );
            let message = result
                .err()
                .ok_or("faulty export was accepted")?
                .to_string();
            if !message.contains("native offline processing failed") || path.exists() {
                return Err(format!("native export failure/cleanup mismatch: {message}"));
            }
            Ok(())
        },
    )?;
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(r) = ticket.poll(0) {
            r?;
            break;
        }
        if std::time::Instant::now() > deadline {
            return Err("offline failure test timed out".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!check.exists());
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"version":"0.146.0","cases":reports,"save_reopen":true,"unapproved_helper_denied":true,"studio_graph_audio":true,"offline_fault_export_removed":true}),
        )?,
    )?;
    Ok(())
}
