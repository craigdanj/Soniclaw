use super::*;
use std::{path::PathBuf, time::Duration};
struct DelayFixture {
    frames: usize,
    data: Vec<f32>,
    pos: usize,
}
impl AudioProcessor for DelayFixture {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        self.data = vec![0.; self.frames * s.channels];
        Ok(())
    }
    fn reset(&mut self) {
        self.data.fill(0.);
        self.pos = 0;
    }
    fn latency_frames(&self) -> usize {
        self.frames
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        for s in b.samples_mut() {
            std::mem::swap(s, &mut self.data[self.pos]);
            self.pos = (self.pos + 1) % self.data.len();
        }
    }
}
fn descriptor(id: &str, n: usize) -> Descriptor {
    Descriptor {
        id: id.into(),
        kind: "test.delay".into(),
        state: serde_json::json!({"frames":n}),
    }
}
fn factory(d: &Descriptor) -> Result<Built, Error> {
    if d.kind == "test.delay" {
        Ok(Built {
            processor: Box::new(DelayFixture {
                frames: d.state["frames"].as_u64().unwrap() as usize,
                data: vec![],
                pos: 0,
            }),
            parameters: vec![],
        })
    } else {
        Builtins.create(d)
    }
}
fn fixture() -> project::Project {
    let mut p = workspace::blank("Routed latency".into(), 48000, 64).unwrap();
    let mut data = vec![0.; 128];
    data[7] = 1.;
    p.assets.insert(
        "impulse".into(),
        soniclaw_clips::AudioAsset {
            sample_rate: 48000,
            channels: 1,
            samples: data.into(),
        },
    );
    p.document.tracks[0].audio.push(soniclaw_clips::AudioClip {
        id: "impulse".into(),
        asset: "impulse".into(),
        track: 0,
        start_frame: 0,
        length_frames: 128,
        source_offset: 0,
        source_length: 128,
        looping: false,
        gain: 1.,
        fade_in: 0,
        fade_out: 0,
        enabled: true,
        edits: None,
    });
    p.document.tracks[0]
        .inserts
        .push(descriptor("track-delay", 13));
    let mut b = mix_graph::Bus::new("bus".into(), "Bus".into());
    b.inserts.push(descriptor("bus-delay", 7));
    p.document.mix_routing.buses.push(b);
    p.document
        .mix_routing
        .track_outputs
        .insert("audio-1".into(), mix_graph::Destination::Bus("bus".into()));
    p.document
        .mix_routing
        .master
        .inserts
        .push(descriptor("master-delay", 5));
    p.document.auxiliaries.push(Auxiliary {
        inserts: vec![descriptor("return-delay", 3)],
        return_gain: 1.,
    });
    p.document.mixer.taps.push(soniclaw_mixer::Tap::PostFader);
    p.document.mixer.strips[0].sends.push(1.);
    p
}
fn folder() -> PathBuf {
    std::env::temp_dir().join(format!("soniclaw-delivery-{}", project::new_identity()))
}
fn wait<T: Send + 'static>(t: &mut jobs::Ticket<T>) -> Result<T, jobs::Error> {
    for _ in 0..1000 {
        if let Some(r) = t.poll(0) {
            return r;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("job timeout")
}
#[test]
fn same_pass_stems_trim_each_compiled_latency() {
    let p = fixture();
    let session = p
        .document
        .compile_offline(&p.assets, &p.disk, &mut factory)
        .unwrap();
    assert_eq!(session.latency_frames(), 25);
    assert_eq!(
        session
            .routed_taps()
            .iter()
            .map(|t| t.latency_frames)
            .collect::<Vec<_>>(),
        vec![13, 20, 16]
    );
    drop(session);
    let path = folder();
    let result_path = path.clone();
    let scheduler = jobs::Scheduler::new(Default::default()).unwrap();
    let mut job = scheduler
        .submit(
            "route-test",
            0,
            jobs::Priority::Normal,
            1024 * 1024,
            move |ctx| {
                export::render(
                    &p,
                    export::Settings {
                        length_frames: 128,
                        tail_frames: 32,
                        routed_stems: vec![
                            delivery::Source::TrackPost("audio-1".into()),
                            delivery::Source::Bus("bus".into()),
                            delivery::Source::Aux(0),
                        ],
                        ..Default::default()
                    },
                    &path,
                    &mut factory,
                    &ctx,
                )
                .map_err(|e| e.to_string())
            },
        )
        .unwrap();
    let report = wait(&mut job).unwrap();
    assert_eq!(report.files.len(), 4);
    for file in &report.files {
        assert_eq!(file.frames, 160);
        let mut reader = hound::WavReader::open(result_path.join(&file.file)).unwrap();
        let pcm = reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            pcm.as_chunks::<2>()
                .0
                .iter()
                .position(|f| f.iter().any(|v| v.abs() > 1e-6)),
            Some(7),
            "{}",
            file.role
        );
    }
    std::fs::remove_dir_all(result_path).unwrap();
}
#[test]
fn cancellation_after_preparation_removes_only_new_output() {
    let p = fixture();
    let path = folder();
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("keep.txt"), "keep").unwrap();
    let output = path.join("render");
    let worker_output = output.clone();
    let scheduler = jobs::Scheduler::new(Default::default()).unwrap();
    let mut job = scheduler
        .submit(
            "cancel-test",
            0,
            jobs::Priority::Normal,
            1024 * 1024,
            move |ctx| {
                let cancel = ctx.cancel_flag();
                let mut f = |d: &Descriptor| {
                    cancel.store(true, Ordering::Release);
                    factory(d)
                };
                export::render(
                    &p,
                    export::Settings {
                        length_frames: 128,
                        tail_frames: 0,
                        routed_stems: vec![delivery::Source::Bus("bus".into())],
                        ..Default::default()
                    },
                    &worker_output,
                    &mut f,
                    &ctx,
                )
                .map_err(|e| e.to_string())
            },
        )
        .unwrap();
    assert!(matches!(wait(&mut job), Err(jobs::Error::Cancelled)));
    assert!(!output.exists());
    assert_eq!(
        std::fs::read_to_string(path.join("keep.txt")).unwrap(),
        "keep"
    );
    std::fs::remove_dir_all(path).unwrap();
}
