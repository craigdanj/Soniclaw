use super::*;
use soniclaw_dsp::ir_capture::{recover, CaptureOptions, Sweep};

pub fn run(out: &Path, args: &[String]) -> Result<serde_json::Value> {
    let sweep = || Sweep {
        sample_rate: 48_000,
        frames: 96_000,
        first_hz: 20.,
        last_hz: 20_000.,
        amplitude: 0.5,
        fade_frames: 480,
    };
    let (reference, recording, rate, ir_frames, search_frames, fixture) = match args {
        [mode] if mode == "--sweep-only" => {
            let sweep = sweep();
            ir_wav(out.join("sweep.wav"), &sweep.generate()?, sweep.sample_rate)?;
            return Ok(
                serde_json::json!({"sample_rate":sweep.sample_rate,"sweep_frames":sweep.frames}),
            );
        }
        [mode, source, recorded, length, search] if mode == "--recover" => {
            let (source, rate) = read_mono(source)?;
            let (recorded, recorded_rate) = read_mono(recorded)?;
            if rate != recorded_rate {
                return Err("the WAV sample rates must match".into());
            }
            (
                source,
                recorded,
                rate,
                length.parse()?,
                search.parse()?,
                false,
            )
        }
        [] => {
            let sweep = sweep();
            let x = sweep.generate()?;
            let mut y = vec![0.; x.len() + 2048];
            for (i, &v) in x.iter().enumerate() {
                y[i + 441] += 0.8 * v;
                y[i + 669] -= 0.3 * v;
            }
            ir_wav(out.join("sweep.wav"), &x, sweep.sample_rate)?;
            ir_wav(out.join("synthetic-capture.wav"), &y, sweep.sample_rate)?;
            (x, y, sweep.sample_rate, 1024, 800, true)
        }
        _ => {
            return Err(
                "use --sweep-only or --recover SWEEP.wav RECORDING.wav IR_FRAMES SEARCH_FRAMES"
                    .into(),
            )
        }
    };
    let ir = recover(
        &reference,
        &recording,
        CaptureOptions {
            impulse_frames: ir_frames,
            search_frames,
            pre_roll_frames: search_frames.min(16),
            floor_db: -110.,
            tail_fade_frames: (ir_frames / 8).min(480),
        },
    )?;
    if fixture {
        assert_eq!(ir.strongest_frame, 441);
        assert!(ir.impulse[16] > 0.6 && ir.impulse[244] < -0.24);
        let engine = soniclaw_dsp::Convolution::new(ir.impulse.clone(), f64::from(rate), 128)?;
        let mut excitation = vec![0.; 2048];
        excitation[0] = 1.;
        render(out, "convolver-check", Box::new(engine), excitation)?;
    }
    ir_wav(out.join("captured-ir.wav"), &ir.impulse, rate)?;
    Ok(
        serde_json::json!({"rate":rate,"strongest_frame":ir.strongest_frame,
        "start_frame":ir.start_frame,"ir_frames":ir.impulse.len()}),
    )
}

pub(super) fn read_mono(fname: &str) -> Result<(Vec<f32>, u32)> {
    let mut reader = hound::WavReader::open(fname)?;
    let spec = reader.spec();
    if spec.channels != 1
        || reader.duration() > 1 << 22
        || !(8000..=192000).contains(&spec.sample_rate)
    {
        return Err("use a bounded mono WAV at 8–192 kHz".into());
    }
    let data = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<std::result::Result<Vec<_>, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 2f64.powi(i32::from(spec.bits_per_sample) - 1);
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| (f64::from(v) / scale) as f32))
                .collect::<std::result::Result<Vec<_>, _>>()?
        }
    };
    Ok((data, spec.sample_rate))
}

pub(super) fn ir_wav(path: impl AsRef<Path>, data: &[f32], rate: u32) -> Result<()> {
    let mut writer = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 1,
            sample_rate: rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    for &v in data {
        writer.write_sample(v)?;
    }
    writer.finalize()?;
    Ok(())
}
