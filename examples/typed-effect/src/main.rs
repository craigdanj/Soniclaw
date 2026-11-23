use soniclaw::{params::*, prelude::*};
use soniclaw_typed_effect::*;
use std::path::Path;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    let out = Path::new(
        args.get(1)
            .ok_or("usage: soniclaw-typed-effect NEW-DIRECTORY [built-clap-library]")?,
    );
    std::fs::create_dir(out)?;
    let controls = Controls::new()?;
    let snapshot = Snapshot::capture("org.soniclaw.typed-effect", 1, &controls.raw_parameters())?;
    std::fs::write(
        out.join("declarations.json"),
        serde_json::to_vec_pretty(&controls.declarations())?,
    )?;
    std::fs::write(
        out.join("snapshot.json"),
        serde_json::to_vec_pretty(&snapshot)?,
    )?;
    let gain = controls.gain.clone();
    let mut processor = soniclaw::plugin::HostedProcessor(create_with(controls));
    let spec = ProcessSpec {
        sample_rate: 48000.,
        max_block_frames: 256,
        channels: 2,
    };
    processor.prepare(spec)?;
    let mut data = [1.; 512];
    let measure = allocation_counter::measure(|| {
        processor.process(
            &mut AudioBuffer::new(&mut data, 2).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Default::default(),
            },
        )
    });
    if data.iter().any(|v| (*v - 0.5).abs() > 1e-6) || measure.count_total != 0 {
        return Err("typed native processor output/allocation mismatch".into());
    }
    gain.set(1.)?;
    let mut report = serde_json::json!({"typed_dsp_gain":0.5,"callback_allocations":measure.count_total,"normalized_gain_default":snapshot.values["master.gain"],"declaration_count":5});
    if let Some(path) = args.get(2) {
        // SAFETY: caller explicitly supplied a trusted native CLAP library for validation.
        let library = unsafe { soniclaw_host::ClapLibrary::load(Path::new(path)) }
            .map_err(|e| e.to_string())?;
        let mut instance = library
            .instantiate("org.soniclaw.typed-effect")
            .map_err(|e| e.to_string())?;
        let parameters = instance.parameters().to_vec();
        let id = |name: &str| parameters.iter().find(|p| p.name == name).unwrap().id;
        let cutoff = instance
            .parameter_text(id("Cutoff"), 1. / 3.)
            .map_err(|e| e.to_string())?;
        let mode = instance
            .parameter_text(id("Mode"), 2.)
            .map_err(|e| e.to_string())?;
        let parsed = instance
            .parameter_from_text(id("Cutoff"), "1000 Hz")
            .map_err(|e| e.to_string())?;
        if !cutoff.contains("200.")
            || mode != "High pass"
            || !parameters
                .iter()
                .filter(|p| ["Mode", "Stages", "Bypass"].contains(&p.name.as_str()))
                .all(|p| p.flags & clap_sys::ext::params::CLAP_PARAM_IS_STEPPED != 0)
        {
            return Err("exported typed metadata/format mismatch".into());
        }
        instance.activate(spec).map_err(|e| e.to_string())?;
        let output = instance
            .run_audio(|audio| {
                let mut output = [1f32; 512];
                audio
                    .process_block(
                        &mut output,
                        &[],
                        &[
                            soniclaw_host::ParameterEvent {
                                frame: 128,
                                id: id("Gain"),
                                value: 0.4,
                            },
                            soniclaw_host::ParameterEvent {
                                frame: 192,
                                id: id("Bypass"),
                                value: 1.,
                            },
                        ],
                        Transport::default(),
                    )
                    .unwrap();
                output
            })
            .map_err(|e| e.to_string())?;
        instance.deactivate();
        if output[..256].iter().any(|v| (*v - 0.5).abs() > 1e-6)
            || output[256..384].iter().any(|v| (*v - 0.8).abs() > 1e-6)
            || output[384..].iter().any(|v| (*v - 1.).abs() > 1e-6)
        {
            return Err("exported typed sample-offset automation mismatch".into());
        }
        report["clap"] = serde_json::json!({"parameters":parameters,"cutoff_default_text":cutoff,"mode_text":mode,"parsed_1000_hz_normalized":parsed,"gain_change_frame":128,"bypass_change_frame":192});
    }
    std::fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    println!("{report}");
    Ok(())
}
