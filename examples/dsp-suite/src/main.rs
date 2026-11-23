mod advanced_design;
mod ambisonics;
mod amp;
mod articulations;
mod binaural;
mod block_dsp;
mod block_view;
mod cabinet_blend;
mod composition;
mod convolution_update_queue;
mod coupled_instrument;
mod dither;
mod double_precision;
mod dsp_utilities;
mod dynamics;
mod engine_precision;
mod feedback_patch;
mod filter_design;
mod first_order_tpt;
mod fm_operators;
mod fractional_delay;
mod granular_fx;
mod half_band_design;
mod hpss;
mod hybrid_voice;
mod iir_oversampling;
mod interpolation;
mod ir_capture;
mod linear_phase;
mod logarithmic_ramp;
mod lookup_transform;
mod mastering;
mod multimic_sampler;
mod nam_lstm;
mod nam_wavenet;
mod optimization;
mod physical;
mod pitch;
mod pitch_correction;
mod playback;
mod polyphonic_analysis;
mod real_fft;
mod reed_waveguide;
mod restoration;
mod simd_vectors;
mod spatial;
mod spectral_morph;
mod spectral_resynthesis;
mod stereo_panner;
mod stream_resample;
mod unified_oversampling;
mod wave_digital;
mod wavetable_author;
use soniclaw_core::*;
use std::{path::Path, sync::Arc};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn spec() -> ProcessSpec {
    ProcessSpec {
        sample_rate: 48000.,
        max_block_frames: 128,
        channels: 1,
    }
}
fn context(sample: usize) -> ProcessContext<'static> {
    ProcessContext {
        events: &[],
        transport: Transport {
            sample_position: sample as u64,
            ..Default::default()
        },
    }
}
fn render(
    out: &Path,
    name: &str,
    processor: Box<dyn AudioProcessor>,
    input: Vec<f32>,
) -> Result<(serde_json::Value, Vec<f32>)> {
    render_channels(out, name, processor, input, 1)
}
fn render_channels(
    out: &Path,
    name: &str,
    mut processor: Box<dyn AudioProcessor>,
    input: Vec<f32>,
    channels: usize,
) -> Result<(serde_json::Value, Vec<f32>)> {
    processor.prepare(ProcessSpec { channels, ..spec() })?;
    let mut pcm = input;
    let mut allocations = 0;
    let mut frame = 0;
    for b in pcm.chunks_mut(128 * channels) {
        allocations += allocation_counter::measure(|| {
            processor.process(&mut AudioBuffer::new(b, channels).unwrap(), &context(frame))
        })
        .count_total;
        frame += b.len() / channels;
    }
    assert_eq!(allocations, 0);
    assert!(pcm.iter().all(|x| x.is_finite()));
    let peak = pcm.iter().map(|v| v.abs()).fold(0f32, f32::max);
    assert!(peak > 1e-7);
    let file = format!("{name}.wav");
    let mut w = hound::WavWriter::create(
        out.join(&file),
        hound::WavSpec {
            channels: channels as u16,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    for x in &pcm {
        w.write_sample(*x)?
    }
    w.finalize()?;
    Ok((
        serde_json::json!({"name":name,"audio":file,"latency_frames":processor.latency_frames(),"peak":peak,"callback_allocations":allocations}),
        pcm,
    ))
}
fn oscillators(out: &Path) -> Result<serde_json::Value> {
    use soniclaw_dsp::{advanced::wavetable::WavetableBank, oscillators::*};
    let mut cases = vec![];
    let bank = Arc::new(WavetableBank::basic(512)?);
    for (name, c) in [
        (
            "pwm",
            Controls {
                shape: Shape::Pulse,
                pulse_width: 0.23,
                ..Default::default()
            },
        ),
        (
            "fm",
            Controls {
                fm_hz: 110.,
                fm_depth_hz: 180.,
                ..Default::default()
            },
        ),
        (
            "pm-warp",
            Controls {
                shape: Shape::Sine,
                pm_hz: 100.,
                pm_depth_cycles: 0.5,
                warp: 0.7,
                ..Default::default()
            },
        ),
        (
            "sync",
            Controls {
                sync_ratio: 3.3,
                ..Default::default()
            },
        ),
        (
            "wavetable",
            Controls {
                shape: Shape::Wavetable,
                morph: 0.4,
                ..Default::default()
            },
        ),
    ] {
        cases.push(
            render(
                out,
                name,
                Box::new(Oscillator::new(
                    Config {
                        controls: c,
                        ..Default::default()
                    },
                    Some(bank.clone()),
                )?),
                vec![0.; 12288],
            )?
            .0,
        );
    }
    let mut residuals = vec![];
    for enabled in [false, true] {
        let c = Controls {
            frequency: 851. * 48000. / 4096.,
            shape: Shape::Pulse,
            ..Default::default()
        };
        let mut cfg = Config {
            controls: c,
            ..Default::default()
        };
        if !enabled {
            cfg.oversampling = None;
        }
        let (_, pcm) = render(
            out,
            if enabled {
                "pulse-oversampled"
            } else {
                "pulse-naive"
            },
            Box::new(Oscillator::new(cfg, None)?),
            vec![0.; 12288],
        )?;
        residuals.push(
            soniclaw_quality::Spectrum::coherent(&pcm[8192..])?
                .distortion(&[851], 2)?
                .thdn_db,
        );
    }
    assert!(residuals[0] - residuals[1] > 3.);
    Ok(
        serde_json::json!({"cases":cases,"pulse_alias_fixture":{"fft":4096,"bin":851,"naive_residual_db":residuals[0],"oversampled_residual_db":residuals[1]}}),
    )
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let feature = args.get(1).ok_or("FEATURE NEW_DIRECTORY")?;
    let out = Path::new(args.get(2).ok_or("NEW_DIRECTORY")?);
    std::fs::create_dir(out)?;
    let data = match feature.as_str() {
        "simd-vectors" => simd_vectors::run(out)?,
        "engine-precision" => engine_precision::run(out)?,
        "stereo-panner" => stereo_panner::run(out)?,
        "logarithmic-ramp" => logarithmic_ramp::run(out)?,
        "first-order-tpt" => first_order_tpt::run(out)?,
        "block-view" => block_view::run(out)?,
        "lookup-transform" => lookup_transform::run(out)?,
        "convolution-update-queue" => convolution_update_queue::run(out)?,
        "half-band-design" => half_band_design::run(out)?,
        "oscillators" => oscillators(out)?,
        "pitch" => pitch::run(out)?,
        "physical" => physical::run(out)?,
        "amp" => amp::run(out)?,
        "spatial" => spatial::run(out)?,
        "dynamics" => dynamics::run(out)?,
        "playback" => playback::run(out)?,
        "optimization" => optimization::run(out)?,
        "filter-design" => filter_design::run(out)?,
        "fractional-delay" => fractional_delay::run(out)?,
        "iir-oversampling" => iir_oversampling::run(out)?,
        "stream-resample" => stream_resample::run(out)?,
        "linear-phase" => linear_phase::run(out)?,
        "block-dsp" => block_dsp::run(out)?,
        "dither" => dither::run(out)?,
        "restoration" => restoration::run(out)?,
        "hybrid-voice" => hybrid_voice::run(out)?,
        "wavetable-author" => wavetable_author::run(out)?,
        "spectral-resynthesis" => spectral_resynthesis::run(out)?,
        "articulations" => articulations::run(out)?,
        "nam-lstm" => nam_lstm::run(out)?,
        "coupled-instrument" => coupled_instrument::run(out)?,
        "composition" => composition::run(out)?,
        "advanced-design" => advanced_design::run(out)?,
        "unified-oversampling" => unified_oversampling::run(out)?,
        "real-fft" => real_fft::run(out)?,
        "interpolation" => interpolation::run(out)?,
        "dsp-utilities" => dsp_utilities::run(out)?,
        "double-precision" => double_precision::run(out)?,
        "ir-capture" => ir_capture::run(out, &args[3..])?,
        "fm-operators" => fm_operators::run(out)?,
        "wave-digital" => wave_digital::run(out)?,
        "spectral-morph" => spectral_morph::run(out)?,
        "polyphonic-analysis" => polyphonic_analysis::run(out, &args[3..])?,
        "binaural" => binaural::run(out, &args[3..])?,
        "reed-waveguide" => reed_waveguide::run(out)?,
        "mastering" => mastering::run(out, &args[3..])?,
        "nam-wavenet" => nam_wavenet::run(out, &args[3..])?,
        "multimic-sampler" => multimic_sampler::run(out)?,
        "pitch-correction" => pitch_correction::run(out)?,
        "cabinet-blend" => cabinet_blend::run(out, &args[3..])?,
        "hpss" => hpss::run(out, &args[3..])?,
        "granular-fx" => granular_fx::run(out)?,
        "ambisonics" => ambisonics::run(out)?,
        "feedback-patch" => feedback_patch::run(out)?,
        _ => return Err("unknown DSP example".into()),
    };
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"feature":feature,"results":data}),
        )?,
    )?;
    Ok(())
}
