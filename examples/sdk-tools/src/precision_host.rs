//! Native precision test for the explicit trusted reference modules.
use super::*;
const DELTA: f64 = 1. / (1u64 << 40) as f64;
fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn native(kind: &str, path: &Path) -> Result<serde_json::Value> {
    use soniclaw::host::ClapLibrary;
    let s = ProcessSpec {
        sample_rate: 48000.,
        channels: 2,
        max_block_frames: 128,
    };
    let input = [1. + DELTA, -1. - DELTA, 0.5, 0.25];
    let expected = input.map(|x| x * 0.5);
    let mut output = input;
    let count = match kind {
        // SAFETY: Examples accept only the owned reference binaries built from this workspace.
        "clap" => {
            let lib = unsafe { ClapLibrary::load(path) }.map_err(text)?;
            let mut product = lib
                .instantiate("org.soniclaw.precision-gain")
                .map_err(text)?;
            assert!(product.supports_f64());
            product.activate(s).map_err(text)?;
            let (actual, allocation) = product
                .run_audio(move |audio| {
                    let n = allocation_counter::measure(|| {
                        audio
                            .process_block64(&mut output, &[], &[], Transport::default())
                            .unwrap();
                    });
                    let mut narrow = [1f32, -1., 0.5, 0.25];
                    audio
                        .process_block(&mut narrow, &[], &[], Transport::default())
                        .unwrap();
                    assert_eq!(narrow, [0.5, -0.5, 0.25, 0.125]);
                    (output, n.count_total)
                })
                .map_err(text)?;
            output = actual;
            let mut legacy = lib.instantiate("org.sonora.reference").map_err(text)?;
            assert!(!legacy.supports_f64());
            legacy.activate(s).map_err(text)?;
            legacy
                .run_audio(|audio| {
                    let mut invalid = [1.; 4];
                    assert_eq!(
                        audio.process_block64(&mut invalid, &[], &[], Transport::default()),
                        Err(Error::InvalidSpec)
                    );
                    assert_eq!(invalid, [0.; 4]);
                })
                .map_err(text)?;
            allocation
        }
        #[cfg(feature = "vst3-tools")]
        "vst3" => {
            use soniclaw::host::formats::{products, PluginFormat};
            let ctx = ProcessContext {
                events: &[],
                transport: Transport::default(),
            };
            let variants =
                unsafe { products(PluginFormat::Vst3, path) }.map_err(|e| e.to_string())?;
            let product = variants
                .iter()
                .find(|p| p.name == "Soniclaw Precision Gain")
                .ok_or("precision class missing")?;
            let mut owner = unsafe { soniclaw::vst3::Instance::load(path, Some(&product.id), s) }
                .map_err(|e| e.to_string())?;
            let mut precision = owner.start_direct64().map_err(|e| e.to_string())?;
            let n = allocation_counter::measure(|| {
                precision.process(&mut AudioBuffer64::new(&mut output, 2).unwrap(), &ctx);
            });
            assert_eq!(owner.direct_errors(), 0);
            owner.stop().map_err(|e| e.to_string())?;
            let mut narrow = [1f32, -1., 0.5, 0.25];
            let mut p32 = owner.start_direct().map_err(|e| e.to_string())?;
            p32.process(&mut AudioBuffer::new(&mut narrow, 2)?, &ctx);
            assert_eq!(narrow, [0.5, -0.5, 0.25, 0.125]);
            owner.stop().map_err(|e| e.to_string())?;
            n.count_total
        }
        _ => return Err("expected clap or vst3 (build with vst3-tools)".into()),
    };
    assert_eq!(output, expected);
    assert_eq!(count, 0);
    Ok(
        serde_json::json!({ "format":kind, "sample_precision":64, "expected":expected,
        "actual":output, "f32_compatibility":true, "callback_allocations":count }),
    )
}

pub fn run(path: &Path, args: &[std::ffi::OsString]) -> Result<()> {
    if args.is_empty() || !args.len().is_multiple_of(2) {
        return Err("expected one or more format and owned reference path pairs".into());
    }
    let fixtures = args
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[kind, file]| {
            native(
                kind.to_str().ok_or("format must be UTF-8")?,
                Path::new(file),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let data = serde_json::json!({ "version":env!("CARGO_PKG_VERSION"), "fixtures":fixtures });
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(&serde_json::to_vec_pretty(&data)?)?;
    Ok(())
}
