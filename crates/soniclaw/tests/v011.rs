use soniclaw::{prelude::*, sampler::*, sampling::*};
use std::{path::PathBuf, sync::Arc};
fn spec() -> ProcessSpec {
    ProcessSpec {
        sample_rate: 48000.0,
        max_block_frames: 128,
        channels: 2,
    }
}
fn sample(value: f32) -> soniclaw::dsp::Sample {
    soniclaw::dsp::Sample::new(vec![value; 128], 1, 48000.0, 60).unwrap()
}
fn render(zones: Vec<Zone>, samples: Vec<soniclaw::dsp::Sample>, events: &[Event]) -> Vec<f32> {
    let mut p = MultiSampler::<16>::new(Arc::new(SampleBank::new(samples, zones, 65536).unwrap()));
    p.prepare(spec()).unwrap();
    let mut data = vec![0.0; 256];
    let a = allocation_counter::measure(|| {
        p.process(
            &mut AudioBuffer::new(&mut data, 2).unwrap(),
            &ProcessContext {
                events,
                transport: Default::default(),
            },
        )
    });
    assert_eq!(a.count_total, 0);
    data
}
fn on(v: u8) -> Event {
    Event {
        frame: 0,
        message: MidiMessage::NoteOn {
            channel: 0,
            note: 60,
            velocity: v,
        },
    }
}
#[test]
fn velocity_crossfades_layer_and_follow_endpoints() {
    let mut a = Zone::single(0, 60);
    a.velocity_fade_out = Some([1, 127]);
    a.round_robin_group = 1;
    let mut b = Zone::single(1, 60);
    b.velocity_fade_in = Some([1, 127]);
    b.round_robin_group = 2;
    let low = render(
        vec![a.clone(), b.clone()],
        vec![sample(1.0), sample(-1.0)],
        &[on(1)],
    );
    let mid = render(
        vec![a.clone(), b.clone()],
        vec![sample(1.0), sample(-1.0)],
        &[on(64)],
    );
    let high = render(vec![a, b], vec![sample(1.0), sample(-1.0)], &[on(127)]);
    assert!(low[0] > 0.0);
    assert!(mid[0].abs() < 1e-6);
    assert!(high[0] < -0.9);
}
#[test]
fn release_uses_note_on_velocity_and_waits_for_sustain_up() {
    let mut release = Zone::single(1, 60);
    release.release_trigger = true;
    release.round_robin_group = 1;
    let events = [
        on(127),
        Event {
            frame: 8,
            message: MidiMessage::ControlChange {
                channel: 0,
                controller: 64,
                value: 127,
            },
        },
        Event {
            frame: 16,
            message: MidiMessage::NoteOff {
                channel: 0,
                note: 60,
                velocity: 1,
            },
        },
        Event {
            frame: 32,
            message: MidiMessage::ControlChange {
                channel: 0,
                controller: 64,
                value: 0,
            },
        },
    ];
    let out = render(
        vec![Zone::single(0, 60), release],
        vec![sample(0.1), sample(0.5)],
        &events,
    );
    assert!(out[40] < 0.11);
    assert!(out[70] > 0.55);
}
#[test]
fn identified_release_is_not_duplicated_and_choke_does_not_trigger_release() {
    let mut r = Zone::single(0, 60);
    r.release_trigger = true;
    let on = Event {
        frame: 0,
        message: MidiMessage::NoteOnId {
            channel: 1,
            note: 60,
            velocity: 1.0,
            note_id: 8,
        },
    };
    let target = NoteTarget {
        channel: -1,
        key: -1,
        note_id: 8,
    };
    let off = Event {
        frame: 8,
        message: MidiMessage::NoteOffId { target },
    };
    let twice = Event {
        frame: 16,
        message: MidiMessage::NoteOffId { target },
    };
    let out = render(vec![r.clone()], vec![sample(0.2)], &[on, off, twice]);
    assert!((out[40] - 0.2).abs() < 1e-5);
    let out = render(
        vec![r],
        vec![sample(0.2)],
        &[
            on,
            Event {
                frame: 8,
                message: MidiMessage::NoteChoke { target },
            },
        ],
    );
    assert!(out.iter().all(|x| *x == 0.0));
}
struct Temp(PathBuf);
impl Temp {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("soniclaw-sfz-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        soniclaw::io::write_wav(p.join("tone sample.wav"), 48000, 1, &[0.2; 256]).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn sfz_subset_imports_inheritance_notes_paths_endpoints_and_release() {
    let t = Temp::new("import");
    let path = t.0.join("test.sfz");
    std::fs::write(&path,"<global> lovel=1 hivel=127\n<group> pitch_keycenter=C4\n<region> sample=\"tone sample.wav\" key=C4 offset=2 end=127 xfin_lovel=1 xfin_hivel=64\n<region> sample=\"tone sample.wav\" key=C4 trigger=release volume=-6").unwrap();
    let (s, root) = import_sfz(&path, "test.sfz").unwrap();
    assert_eq!(s.zones.len(), 2);
    assert_eq!(s.zones[0].keys, [60, 60]);
    assert_eq!(s.zones[0].end_frame, Some(128));
    assert!(s.zones[1].release_trigger);
    let descriptor = soniclaw::plugin::PluginDescriptor {
        id: "test.sfz",
        name: "SFZ",
        vendor: "Test",
        version: "0.146.0",
        is_instrument: true,
        accepts_midi: true,
    };
    let mut p = soniclaw::project::create_plugin(descriptor, s, root).unwrap();
    p.activate(spec()).unwrap();
    p.process(&mut [0.0; 256], &[on(127)]).unwrap();
}
#[test]
fn sfz_rejects_unsupported_and_escaping_data() {
    let t = Temp::new("reject");
    let path = t.0.join("bad.sfz");
    for text in [
        "#include \"other.sfz\"",
        "<region> sample=../escape.wav",
        "<region> sample=\"tone sample.wav\" cutoff=1000",
        "<region> sample=\"tone sample.wav\" trigger=legato",
        "<region> sample=\"tone sample.wav\" end=99999",
    ] {
        std::fs::write(&path, text).unwrap();
        assert!(import_sfz(&path, "test.sfz").is_err(), "{text}");
    }
}
#[test]
fn energy_transients_generate_scoped_slice_ranges() {
    let mut samples = vec![0.0; 24000];
    for start in [0, 4800, 9600, 14400, 19200] {
        samples[start..start + 240].fill(0.8);
    }
    let audio = soniclaw::io::AudioFile {
        sample_rate: 48000,
        channels: 1,
        samples,
    };
    let markers = detect_transients(
        &audio,
        TransientOptions {
            pre_roll_ms: 0.0,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(markers, vec![0, 4800, 9600, 14400, 19200]);
    let zones = sliced_zones("kick", &markers, audio.frames(), 36).unwrap();
    assert_eq!(zones[4].keys, [40, 40]);
    assert_eq!(zones[4].end_frame, Some(24000));
    assert!(sliced_zones("kick", &markers, 24000, 126).is_err());
}
