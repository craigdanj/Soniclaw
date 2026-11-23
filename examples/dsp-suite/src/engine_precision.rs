use super::*;
use soniclaw::{
    clips::AudioClip,
    dsp::{filter_design::Filter64, precision::Gain64},
    engine::Engine64,
    graph::Graph64,
    io::{read_wav64_with_limit, write_wav64},
    studio::{
        precision::{Asset64, PreparedSession64},
        Document, Track,
    },
};
const DELTA: f64 = 1. / (1u64 << 40) as f64;

fn render_session(out: &Path) -> Result<serde_json::Value> {
    let source = out.join("source-f64.wav");
    write_wav64(&source, 48000, 2, &[1. + DELTA, 0., 0., 0., 0., 0.])?;
    let data = read_wav64_with_limit(&source, 48)?;
    assert_eq!(data.samples[0] - 1., DELTA);
    assert!(read_wav64_with_limit(&source, 47).is_err());
    let asset = |samples| Asset64::new(48000, 2, samples);
    let assets = std::collections::BTreeMap::from([
        (String::from("positive"), asset(data.samples.into())?),
        (
            String::from("negative"),
            asset(Arc::from([-1., 0., 0., 0., 0., 0.]))?,
        ),
    ]);
    let mut document: Document = serde_json::from_value(serde_json::json!({
        "schema":1, "sample_rate":48000, "max_block_frames":128,
        "timeline":soniclaw::transport::Timeline::default(),
        "tracks":[], "mixer":{"schema":1,"strips":[],"groups":[],"taps":[]},
        "auxiliaries":[], "automation":soniclaw::automation::Document::default()
    }))?;
    for id in ["positive", "negative"] {
        document.tracks.push(Track {
            streamed: Vec::new(),
            id: id.into(),
            audio: vec![AudioClip {
                edits: None,
                id: id.into(),
                asset: id.into(),
                track: 0,
                start_frame: 0,
                length_frames: 3,
                source_offset: 0,
                source_length: 3,
                looping: false,
                gain: 1.,
                fade_in: 0,
                fade_out: 0,
                enabled: true,
            }],
            midi: Vec::new(),
            instrument: None,
            inserts: Vec::new(),
            input_gain: 0.,
            midi_input: false,
        });
        document.mixer.strips.push(soniclaw::mixer::Strip {
            id: id.into(),
            name: id.into(),
            gain: 1.,
            pan: -1.,
            mute: false,
            solo: false,
            group: None,
            sends: Vec::new(),
        });
    }
    let session = PreparedSession64::new(&document, &assets, |_| Err(Error::InvalidParameter))?;
    let mut engine = Engine64::default();
    engine.push(session);
    let mut graph = Graph64::default();
    let a = graph.add(Gain64::new(1.)?);
    let b = graph.add(Filter64::fir(48000., vec![0., 1., 0.])?);
    graph.set_input(a, 0.5)?;
    graph.set_input(b, 0.5)?;
    graph.set_output(a, 1.)?;
    graph.set_output(b, 1.)?;
    engine.push(graph);
    engine.prepare(ProcessSpec {
        channels: 2,
        ..spec()
    })?;
    engine.set_transport(Transport {
        playing: true,
        ..Default::default()
    })?;
    let mut block = [0.; 6];
    let callback = allocation_counter::measure(|| {
        engine.process(&mut block, &[]).unwrap();
    });
    assert_eq!(callback.count_total, 0);
    assert_eq!(engine.latency_frames(), 1);
    assert_eq!(block, [0., 0., DELTA, 0., 0., 0.]);
    let result = out.join("render-f64.wav");
    write_wav64(&result, 48000, 2, &block)?;
    assert_eq!(read_wav64_with_limit(result, 48)?.samples, block);
    Ok(
        serde_json::json!({ "input":"source-f64.wav","output":"render-f64.wav",
        "residual":block[2], "latency_frames":1, "callback_allocations":callback.count_total }),
    )
}

pub fn run(out: &Path) -> Result<serde_json::Value> {
    Ok(serde_json::json!({ "session":render_session(out)? }))
}
