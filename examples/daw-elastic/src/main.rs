use soniclaw::{
    stream::clips::DiskClip,
    studio::{
        self,
        elastic::{self, Group, Member},
        project::{Project, ProjectError},
        tempo_cache::{Marker, Mode, Recipe},
        workspace::{self, Kind},
    },
};
use std::{path::Path, sync::atomic::AtomicBool};
fn exercise(root: &Path) -> Result<serde_json::Value, ProjectError> {
    std::fs::create_dir(root)?;
    let mut p = workspace::blank("Linked microphones".into(), 48000, 128)?;
    for i in 1..4 {
        workspace::apply(
            &mut p,
            workspace::Command::Add {
                id: format!("mic-{i}"),
                name: format!("Mic {i}"),
                kind: Kind::Audio,
            },
        )?;
    }
    let hits = [7000usize, 19000, 31000, 43000];
    let mut pcm = vec![0f32; 48000];
    for at in hits {
        for j in 0..600 {
            pcm[at + j] = (j as f32 * 0.15).sin() * (-(j as f32) / 140.).exp() * 0.5;
        }
    }
    let mut group = Group {
        id: "drums".into(),
        reference: 0,
        members: vec![],
        recipe: Recipe {
            schema: 1,
            mode: Mode::PreservePitch,
            markers: vec![
                Marker {
                    source_frame: 0,
                    beat: 0.,
                },
                Marker {
                    source_frame: 48000,
                    beat: 2.,
                },
            ],
        },
    };
    for (i, gain) in [1., -0.5, 0.25, -1.].iter().enumerate() {
        let source = format!("source-{i}");
        let original = format!("original-{i}");
        let path = root.join(format!("mic-{i}.wav"));
        let mut w = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )?;
        for x in &pcm {
            w.write_sample(x * gain)?;
        }
        w.finalize()?;
        p.disk.insert(source.clone(), path);
        p.document.tracks[i].streamed.push(DiskClip {
            id: original.clone(),
            asset: source.clone(),
            start_frame: 0,
            length_frames: 48000,
            source_offset: 0,
            source_length: 48000,
            looping: false,
            gain: 1.,
            fade_in: 0,
            fade_out: 0,
            enabled: true,
            edits: None,
        });
        group.members.push(Member {
            source,
            track: p.document.tracks[i].id.clone(),
            region: format!("elastic-{i}"),
            asset: format!("cache-{i}"),
            replace: Some(original),
        });
    }
    p.validate()?;
    let onsets = elastic::detect(&p.disk["source-0"], Default::default())?;
    assert_eq!(onsets.len(), 4);
    for (&onset, hit) in onsets.iter().zip(hits) {
        assert!(onset.abs_diff(hit as u64) < 32);
    }
    let anchors = group.quantize(&p, &onsets, 0.5)?;
    assert_eq!(anchors, 3);
    let ready = group.build(&p, &root.join("linked"), &AtomicBool::new(false))?;
    let mut history = studio::history::History::new(Default::default())?;
    history.edit(&mut p, "Quantize linked microphones", move |p| {
        ready.apply(p)
    })?;
    assert_eq!(p.layout.elastic_groups.len(), 1);
    assert!(p.document.tracks.iter().all(|t| !t.streamed[0].enabled));
    let read = |id: &str| -> Result<Vec<f32>, ProjectError> {
        Ok(hound::WavReader::open(&p.disk[id])?
            .into_samples::<f32>()
            .collect::<Result<Vec<_>, _>>()?)
    };
    let a = read("cache-0")?;
    let b = read("cache-1")?;
    let c = read("cache-2")?;
    let d = read("cache-3")?;
    assert_eq!(a.len(), 48000);
    let residual = a
        .iter()
        .zip(&b)
        .map(|(x, y)| (y + x * 0.5).abs())
        .fold(0., f32::max);
    assert!(residual < 1e-6);
    assert!(a.iter().zip(&c).all(|(x, y)| (y - x * 0.25).abs() < 1e-6));
    assert!(a.iter().zip(&d).all(|(x, y)| (y + x).abs() < 1e-6));
    for at in [12000usize, 24000, 36000] {
        assert!(
            a[at..at + 256].iter().any(|v| v.abs() > 0.02),
            "missing aligned transient at {at}"
        );
    }
    history.undo(&mut p)?;
    assert!(p.layout.elastic_groups.is_empty());
    assert!(p.document.tracks.iter().all(|t| t.streamed[0].enabled));
    history.redo(&mut p)?;
    studio::sessions::save(&p, &root.join("song"))?;
    let reopened = Project::open(&root.join("song"))?;
    assert_eq!(reopened.layout.elastic_groups[0], group);
    let mut invalid = p.clone();
    invalid.document.tracks[0]
        .streamed
        .retain(|c| c.id != "elastic-0");
    assert!(invalid.validate().is_err());
    let mut stale = p.clone();
    let ready = group.build(&stale, &root.join("stale"), &AtomicBool::new(false))?;
    stale.name = "Changed during render".into();
    assert!(ready.apply(&mut stale).is_err());
    assert!(group
        .build(&p, &root.join("cancelled"), &AtomicBool::new(true))
        .is_err());
    assert!(!root.join("cancelled").exists());
    let mut changed = p.clone();
    let mut wrong = group.clone();
    wrong.members[1].source = "cache-1".into();
    assert!(wrong.validate(&changed).is_err());
    elastic::dissolve(&mut changed, "drums")?;
    assert!(changed.layout.elastic_groups.is_empty());
    assert_eq!(changed.warps.len(), 4);
    Ok(
        serde_json::json!({"microphones":4,"detected_onsets":onsets,"snapped_interior_anchors":anchors,"rendered_frames":a.len(),"phase_residual_max":residual,"aligned_transient_samples":[12000,24000,36000],"original_regions_disabled_and_undo_restores":true,"group_recipes_portable":true,"stale_job_rejected":true,"cancelled_folder_removed":true,"dissolve_keeps_audio":true,"hardware_opened":false}),
    )
}
fn main() -> Result<(), ProjectError> {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: daw-elastic NEW_DIRECTORY")?,
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
    fn phase_linked_workflow() {
        let root = std::env::temp_dir().join(workspace::identity("elastic").unwrap());
        exercise(&root).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
