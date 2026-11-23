use soniclaw::{
    core::*,
    studio::{
        self,
        mix_graph::{self, Bus, Destination, Draft, Sidechain},
        project::{Project, ProjectError},
        workspace::{self, Kind},
        Built, Descriptor, Factory,
    },
};
use std::{path::Path, sync::Arc};
struct TestDelay {
    frames: usize,
    data: Vec<f32>,
    cursor: usize,
}
impl AudioProcessor for TestDelay {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        self.data = vec![0.; self.frames * s.channels];
        self.cursor = 0;
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        for x in b.samples_mut() {
            std::mem::swap(x, &mut self.data[self.cursor]);
            self.cursor = (self.cursor + 1) % self.data.len();
        }
    }
    fn reset(&mut self) {
        self.data.fill(0.);
        self.cursor = 0;
    }
    fn latency_frames(&self) -> usize {
        self.frames
    }
}
fn factory(d: &Descriptor) -> Result<Built, Error> {
    if d.kind == "test.delay" {
        Ok(Built {
            processor: Box::new(TestDelay {
                frames: d.state["frames"].as_u64().ok_or(Error::InvalidParameter)? as usize,
                data: vec![],
                cursor: 0,
            }),
            parameters: vec![],
        })
    } else {
        studio::Builtins.create(d)
    }
}
fn delay(id: &str, frames: usize) -> Descriptor {
    Descriptor {
        id: id.into(),
        kind: "test.delay".into(),
        state: serde_json::json!({"frames":frames}),
    }
}
fn fixture(samples: Vec<f32>) -> Result<Project, ProjectError> {
    let mut p = workspace::blank("Submix song".into(), 48000, 128)?;
    workspace::apply(
        &mut p,
        workspace::Command::Add {
            id: "second".into(),
            name: "Second".into(),
            kind: Kind::Audio,
        },
    )?;
    let frames = samples.len() as u64;
    p.assets.insert(
        "signal".into(),
        soniclaw::clips::AudioAsset {
            sample_rate: 48000,
            channels: 1,
            samples: Arc::from(samples),
        },
    );
    for (i, t) in p.document.tracks.iter_mut().enumerate() {
        t.audio.push(soniclaw::clips::AudioClip {
            id: format!("clip-{i}"),
            asset: "signal".into(),
            track: i as u16,
            start_frame: 0,
            length_frames: frames,
            source_offset: 0,
            source_length: frames,
            looping: false,
            gain: 1.,
            fade_in: 0,
            fade_out: 0,
            enabled: true,
            edits: None,
        });
    }
    p.validate()?;
    Ok(p)
}
fn block(p: &Project) -> Result<(Vec<f32>, usize, u64), ProjectError> {
    let mut prepared = p.document.compile(&p.assets, &mut factory)?;
    let latency = prepared.latency_frames();
    let mut out = vec![0.; 256];
    let allocations = allocation_counter::measure(|| {
        prepared.process(
            &mut AudioBuffer::new(&mut out, 2).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Transport {
                    playing: true,
                    ..Default::default()
                },
            },
        )
    });
    if !prepared.mix_meters().is_empty() {
        let m = prepared
            .mix_meters()
            .iter()
            .find(|m| m.id == "master")
            .ok_or("master meter missing")?;
        let peak = m.take_peaks();
        let max = out.iter().copied().map(f32::abs).fold(0., f32::max);
        assert!((peak[0] - max).abs() < 1e-6);
        assert_eq!(m.take_peaks(), [0.; 2]);
    }
    Ok((out, latency, allocations.count_total))
}
fn exercise(root: &Path) -> Result<serde_json::Value, ProjectError> {
    std::fs::create_dir(root)?;
    let mut impulse = vec![0.; 512];
    impulse[0] = 1.;
    let mut p = fixture(impulse)?;
    p.document.tracks[1].inserts.push(delay("track-delay", 3));
    let mut drums = Bus::new("drums".into(), "Drums".into());
    drums.gain = 0.5;
    drums.inserts.push(delay("bus-delay", 17));
    drums.output = Destination::Bus("all".into());
    let mut all = Bus::new("all".into(), "All buses".into());
    all.inserts.push(delay("parent-delay", 11));
    p.document.mix_routing.buses = vec![drums, all];
    p.document.mix_routing.track_outputs.insert(
        p.document.tracks[0].id.clone(),
        Destination::Bus("drums".into()),
    );
    p.document
        .mix_routing
        .master
        .inserts
        .push(delay("master-delay", 5));
    p.validate()?;
    let (out, latency, allocations) = block(&p)?;
    assert_eq!(latency, 36);
    assert_eq!(allocations, 0);
    assert!(out[..72].iter().all(|v| v.abs() < 1e-7));
    assert!((out[72] - 1.5 * 0.5f32.sqrt()).abs() < 1e-6);
    assert!(out[74..].iter().all(|v| v.abs() < 1e-6));
    let mut aux = p.clone();
    aux.document
        .mixer
        .taps
        .push(soniclaw::mixer::Tap::PostFader);
    aux.document.mixer.strips[0].sends.push(1.);
    aux.document.mixer.strips[1].sends.push(0.);
    aux.document.auxiliaries.push(studio::Auxiliary {
        inserts: vec![delay("aux-delay", 23)],
        return_gain: 0.25,
    });
    let (with_aux, aux_latency, aux_alloc) = block(&aux)?;
    assert_eq!(aux_latency, 36);
    assert_eq!(aux_alloc, 0);
    assert!((with_aux[72] - 1.75 * 0.5f32.sqrt()).abs() < 1e-6);
    assert!(with_aux[..72].iter().all(|v| v.abs() < 1e-7));
    let mut solo = p.clone();
    solo.document.mix_routing.buses[0].solo = true;
    let (one, _, _) = block(&solo)?;
    assert!((one[72] - 0.5 * 0.5f32.sqrt()).abs() < 1e-6);
    let mut mute = p.clone();
    mute.document.mix_routing.buses[0].mute = true;
    let (one, _, _) = block(&mute)?;
    assert!((one[72] - 0.5f32.sqrt()).abs() < 1e-6);
    let mut cycle = Draft::new(&p)?;
    cycle.document.mix_routing.buses[1].output = Destination::Bus("drums".into());
    let before = p.document.to_json()?;
    assert!(cycle.apply(&mut p, &mut factory).is_err());
    assert_eq!(p.document.to_json()?, before);
    let stale = Draft::new(&p)?;
    p.name = "Edited".into();
    assert!(stale.apply(&mut p, &mut factory).is_err());
    let mut draft = Draft::new(&p)?;
    draft.document.mix_routing.buses[0].gain = 0.25;
    let mut history = studio::history::History::new(Default::default())?;
    history.edit_prepared(
        &mut p,
        "Bus fader",
        |p| {
            drop(draft.apply(p, &mut factory)?);
            Ok(())
        },
        |_| Ok(()),
    )?;
    history.undo(&mut p)?;
    assert_eq!(p.document.mix_routing.buses[0].gain, 0.5);
    history.redo(&mut p)?;
    p.document
        .parameter_values
        .insert("bus.drums.gain".into(), 0.2);
    let d = Draft::new(&p)?;
    assert!((d.document.mix_routing.buses[0].gain - 0.8).abs() < 1e-6);
    studio::sessions::save(&p, &root.join("song"))?;
    let reopened = Project::open(&root.join("song"))?;
    assert_eq!(reopened.document.mix_routing.buses.len(), 2);
    let (_, latency2, _) = block(&reopened)?;
    assert_eq!(latency2, 36);
    let mut sc = fixture(vec![1.; 8192])?;
    let target = sc.document.tracks[0].id.clone();
    sc.document.mixer.strips[1].mute = true;
    let mut descriptor = mix_graph::sidechain_descriptor("duck".into())?;
    descriptor.state["threshold"] = (-20.).into();
    descriptor.state["ratio"] = 4.into();
    descriptor.state["attack_ms"] = 0.into();
    sc.document.tracks[0].inserts.push(descriptor);
    sc.document.mix_routing.sidechains.push(Sidechain {
        source: "second".into(),
        target: "duck".into(),
    });
    let (ducked, _, sc_alloc) = block(&sc)?;
    let expected = 0.5f32.sqrt() * 10f32.powf(-15. / 20.);
    assert!((ducked[0] - expected).abs() < 1e-6);
    assert_eq!(sc_alloc, 0);
    let mut copied = sc.clone();
    let id = copied.document.tracks[0].id.clone();
    workspace::apply(
        &mut copied,
        workspace::Command::Duplicate {
            id,
            new_id: "copy".into(),
        },
    )?;
    assert_eq!(copied.document.mix_routing.sidechains.len(), 2);
    assert!(copied
        .document
        .compile(&copied.assets, &mut factory)
        .is_ok());
    let mut delayed = sc.clone();
    delayed.document.tracks[0]
        .inserts
        .push(delay("not-supported", 1));
    assert!(delayed
        .document
        .compile(&delayed.assets, &mut factory)
        .is_err());
    let mut sc_cycle = sc.clone();
    sc_cycle.document.tracks[1]
        .inserts
        .push(mix_graph::sidechain_descriptor("reverse".into())?);
    sc_cycle.document.mix_routing.sidechains.push(Sidechain {
        source: target,
        target: "reverse".into(),
    });
    assert!(sc_cycle.validate().is_err());
    Ok(
        serde_json::json!({"nested_bus_count":2,"parallel_impulse_arrival":36,"reported_latency":latency,"first_impulse_amplitude":out[72],"submix_and_master_processing":true,"aux_return_compensated":true,"sidechain_target_duplicate_remapped":true,"submix_solo_and_mute":true,"callback_allocations":allocations,"master_peak_meter_verified":true,"sidechain_first_sample":ducked[0],"sidechain_expected":expected,"muted_source_drives_current_block_detector":true,"sidechain_callback_allocations":sc_alloc,"nonzero_sidechain_latency_rejected":true,"audio_and_sidechain_cycles_rejected":true,"stale_draft_rejected":true,"fader_values_materialized_for_edit":true,"undo_redo_and_portable_routing":true,"hardware_opened":false}),
    )
}
fn main() -> Result<(), ProjectError> {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: daw-mixing NEW_DIRECTORY")?,
    );
    let report = exercise(&root)?;
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn submix_sidechain_workflow() {
        let root = std::env::temp_dir().join(workspace::identity("mixing").unwrap());
        exercise(&root).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
