use soniclaw::{
    core::*,
    studio::{
        self,
        project::{Project, ProjectError},
        recording::{self, Arm, Settings, Source},
        workspace::{self, Command, Kind},
    },
};
use std::{path::Path, sync::Arc};
fn run(root: &Path) -> Result<(), ProjectError> {
    std::fs::create_dir(root)?;
    let mut project = workspace::blank("Multichannel".into(), 48000, 128)?;
    workspace::apply(
        &mut project,
        Command::Add {
            id: "mic-b".into(),
            name: "Mic B".into(),
            kind: Kind::Audio,
        },
    )?;
    let bus = Arc::new(ChannelBus::new(4, 128)?);
    let settings = Settings {
        arms: vec![
            Arm {
                track: "audio-1".into(),
                source: Source::DryInput,
                monitor_input: true,
                channels: vec![0],
            },
            Arm {
                track: "mic-b".into(),
                source: Source::DryInput,
                monitor_input: true,
                channels: vec![2, 3],
            },
        ],
        ..Default::default()
    };
    let mut a = recording::prepare_with_inputs(
        &project,
        &mut studio::Builtins,
        settings,
        &root.join("take"),
        "take",
        0,
        Some(bus.clone()),
    )?;
    let mut input = vec![0.; 128 * 4];
    for pair in input.chunks_mut(4) {
        pair.copy_from_slice(&[0.125, 0.75, 0.25, -0.25]);
    }
    let mut pcm = vec![0.; 256];
    for _ in 0..16 {
        bus.write(&input)?;
        a.processor.process(
            &mut AudioBuffer::new(&mut pcm, 2)?,
            &ProcessContext {
                events: &[],
                transport: Default::default(),
            },
        );
    }
    drop(a.processor);
    let take = a.recorder.stop()?;
    assert_eq!(take.missing_frames, 0);
    assert_eq!(take.tracks[0].channels, 1);
    assert_eq!(take.tracks[1].channels, 2);
    for (name, expected) in [("audio-1", vec![0.125]), ("mic-b", vec![0.25, -0.25])] {
        let file = std::fs::read(root.join("take").join(format!("{name}.wav")))?;
        let values = file[44..]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect::<Vec<_>>();
        assert_eq!(&values[..expected.len()], &expected);
        assert_eq!(values.len(), take.frames as usize * expected.len());
    }
    project.takes.push(root.join("take"));
    assert_eq!(
        recording::place_input_take(&mut project, &root.join("take"), &a.schedule)?,
        2
    );
    assert!(recording::place_input_take(&mut project, &root.join("take"), &a.schedule).is_err());
    studio::sessions::save(&project, &root.join("song"))?;
    let reopened = Project::open(&root.join("song"))?;
    assert_eq!(reopened.document.tracks[0].streamed.len(), 1);
    assert_eq!(reopened.document.tracks[1].streamed.len(), 1);
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"physical_channels_simulated":4,"independent_stem_channels":[1,2],"frames":take.frames,"missing":take.missing_frames,"placed_regions":2,"portable_reopen":true,"hardware_opened":false}),
        )?,
    )?;
    Ok(())
}
fn main() -> Result<(), ProjectError> {
    run(Path::new(
        &std::env::args()
            .nth(1)
            .ok_or("usage: daw-multichannel NEW_DIRECTORY")?,
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_route_partition_and_allocations() {
        let mut p = workspace::blank("Routing".into(), 48000, 128).unwrap();
        p.document.tracks[0].input_gain = 1.;
        let bus = Arc::new(ChannelBus::new(4, 128).unwrap());
        let setup = studio::inputs::Setup {
            bus: bus.clone(),
            routes: std::collections::BTreeMap::from([("audio-1".into(), vec![2, 3])]),
        };
        let mut prepared = p
            .document
            .compile_inputs_at(&p.assets, &p.disk, &mut studio::Builtins, 0, &setup)
            .unwrap();
        let input = vec![0.25; 512];
        bus.write(&input).unwrap();
        let mut pcm = vec![0.; 256];
        let allocated = allocation_counter::measure(|| {
            prepared.process(
                &mut AudioBuffer::new(&mut pcm, 2).unwrap(),
                &ProcessContext {
                    events: &[],
                    transport: Default::default(),
                },
            )
        });
        assert_eq!(allocated.count_total, 0);
        assert!(pcm.iter().any(|s| *s > 0.));
    }
}
