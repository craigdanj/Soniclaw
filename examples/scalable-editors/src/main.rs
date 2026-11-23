use soniclaw_clips::*;
use soniclaw_ui::{egui, timeline::*};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: scalable-editors NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&path)?;
    let mut d = Document {
        schema: 1,
        sample_rate: 48000,
        channels: 2,
        timeline: Default::default(),
        audio: vec![],
        midi: vec![MidiClip {
            controls: vec![],
            id: "all-pitches".into(),
            track: 31,
            start_beat: 0.,
            length_beats: 8.,
            source_offset: 0.,
            source_length: 8.,
            looping: false,
            notes: (0..128)
                .map(|key| Note {
                    id: key as u32,
                    beat: (key % 16) as f64 * 0.25,
                    length: 0.125,
                    channel: 0,
                    key,
                    velocity: 0.8,
                })
                .collect(),
            enabled: true,
        }],
    };
    let mut editor = TimelineEditor::default();
    editor.view.arrangement_scroll[1] = 31. * 48.;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut out = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000., 700.),
            )),
            ..Default::default()
        },
        |ui| {
            editor.arrangement(ui, &mut d, 64).unwrap();
        },
    );
    out.textures_delta.clear();
    let visible_tracks = editor.last_visible;
    assert_eq!(visible_tracks, 1);
    editor.select_notes(&d, "all-pitches", 0., 8., 0, 127, false)?;
    assert_eq!(editor.selection.notes.len(), 128);
    editor.apply(
        &mut d,
        Action::MoveNotes {
            beats: 0.25,
            semitones: 0,
        },
    )?;
    assert!(editor.undo(&mut d));
    assert!(editor.redo(&mut d));
    let mut visibility = Vec::new();
    for key in [0, 127] {
        editor.view.piano_scroll[1] = (127 - key) as f32 * editor.view.key_height;
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000., 700.),
                )),
                ..Default::default()
            },
            |ui| {
                editor.piano_roll(ui, &mut d, "all-pitches").unwrap();
            },
        );
        out.textures_delta.clear();
        assert!(editor.last_visible > 0);
        visibility.push((key, editor.last_visible));
    }
    std::fs::write(path.join("arrangement.json"), d.to_json()?)?;
    std::fs::write(path.join("editor-view.json"), editor.view.to_json()?)?;
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"track_32_visible":visible_tracks,"selected_notes":128,"endpoint_viewports":visibility,"grouped_undo_redo":true,"native_window_tested":false}),
        )?,
    )?;
    Ok(())
}
