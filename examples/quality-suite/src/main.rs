use soniclaw::dsp::OversampledDrive;
use soniclaw::{prelude::*, quality::*};
use std::{collections::BTreeMap, sync::Arc};
fn lowpass() -> Box<dyn AudioProcessor> {
    Box::new(LowPass::new(Arc::new(
        Parameter::new(ParameterInfo {
            id: "cutoff",
            name: "Cutoff",
            min: 20.,
            max: 20000.,
            default: 4000.,
            unit: "Hz",
        })
        .unwrap(),
    )))
}
fn measurements() -> Result<BTreeMap<String, f64>, Box<dyn std::error::Error>> {
    let n = 8192;
    let pure = coherent_sine(n, 113, 0.5, 0.)?;
    let d = Spectrum::coherent(&pure)?.distortion(&[113], 16)?;
    let mut values = BTreeMap::from([("pure_thdn_db".into(), d.thdn_db)]);
    let harmonic = coherent_sine(n, 226, 0.005, 0.3)?;
    let distorted: Vec<_> = pure.iter().zip(harmonic).map(|(a, b)| *a + b).collect();
    values.insert(
        "known_thd_db".into(),
        Spectrum::coherent(&distorted)?
            .distortion(&[113], 16)?
            .thd_db,
    );
    let noise = deterministic_noise(n, 0.001, 42)?;
    let noisy: Vec<_> = pure.iter().zip(noise).map(|(a, b)| *a + b).collect();
    let d = Spectrum::coherent(&noisy)?.distortion(&[113], 16)?;
    values.insert("known_snr_db".into(), d.snr_db);
    values.insert("noise_dbfs".into(), d.noise_dbfs);
    let a = coherent_sine(n, 127, 0.2, 0.)?;
    let b = coherent_sine(n, 199, 0.2, 0.)?;
    let nonlinear: Vec<_> = a
        .into_iter()
        .zip(b)
        .map(|(a, b)| {
            let x = a + b;
            x + 0.3 * x * x * x
        })
        .collect();
    values.insert(
        "cubic_imd_db".into(),
        Spectrum::coherent(&nonlinear)?.ratio_db(&[127, 199], &[55, 271, 453, 525])?,
    );
    let response = response(lowpass, 48000, &[100., 4000., 12000.], 48000, 48000)?;
    for (name, r) in ["lowpass_100_db", "lowpass_cutoff_db", "lowpass_stop_db"]
        .into_iter()
        .zip(response)
    {
        values.insert(name.into(), r.gain_db);
    }
    let tone = coherent_sine(n, 1900, 0.5, 0.)?;
    let naive: Vec<_> = tone
        .iter()
        .map(|x| (*x * 5.).tanh() / 5f32.tanh())
        .collect();
    let repeated: Vec<_> = tone.iter().cycle().take(n * 3).copied().collect();
    let wet = render(&mut OversampledDrive::new(5., 1.)?, &repeated, 48000., 256)?;
    let alias = alias_bin(1900 * 3, n)?;
    let base = Spectrum::coherent(&naive)?.ratio_db(&[1900], &[alias])?;
    let oversampled = Spectrum::coherent(&wet[n * 2..])?.ratio_db(&[1900], &[alias])?;
    values.insert("drive_alias_reduction_db".into(), base - oversampled);
    let mut impulse = vec![0.; 48000];
    impulse[0] = 1.;
    let out = render(lowpass().as_mut(), &impulse, 48000., 256)?;
    values.insert("impulse_peak".into(), statistics(&out)?.peak);
    values.insert("impulse_tail_rms".into(), statistics(&out[40000..])?.rms);
    let sweep = log_sweep(48000, 48000, 20., 20000., 0.5)?;
    let output = render(lowpass().as_mut(), &sweep, 48000., 256)?;
    values.insert("filtered_sweep_rms".into(), statistics(&output)?.rms);
    Ok(values)
}
fn defaults() -> Suite {
    Suite {
        schema: 1,
        limits: [
            ("pure_thdn_db", -300., -100.),
            ("known_thd_db", -40.01, -39.99),
            ("known_snr_db", 54., 58.),
            ("noise_dbfs", -66., -63.),
            ("cubic_imd_db", -39., -36.),
            ("lowpass_100_db", -0.05, 0.05),
            ("lowpass_cutoff_db", -3.05, -2.97),
            ("lowpass_stop_db", -80., -20.),
            ("drive_alias_reduction_db", 8., 140.),
            ("impulse_peak", 0.01, 1.1),
            ("impulse_tail_rms", 0., 1e-10),
            ("filtered_sweep_rms", 0.05, 0.5),
        ]
        .into_iter()
        .map(|(name, minimum, maximum)| Limit {
            name: name.into(),
            minimum,
            maximum,
        })
        .collect(),
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().collect();
    let out = std::path::PathBuf::from(
        args.get(1)
            .ok_or("usage: soniclaw-quality-suite NEW_DIRECTORY [LIMITS_JSON]")?,
    );
    std::fs::create_dir(&out)?;
    let suite = if let Some(path) = args.get(2) {
        Suite::from_json(&std::fs::read_to_string(path)?)?
    } else {
        defaults()
    };
    let values = measurements()?;
    let report = suite.evaluate(&values)?;
    std::fs::write(out.join("limits.json"), suite.to_json()?)?;
    std::fs::write(out.join("report.json"), report.to_json()?)?;
    std::fs::write(
        out.join("measurements.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"sample_rate":48000,"fft_size":8192,"coherent_no_window":true,"values":values,"version":env!("CARGO_PKG_VERSION")}),
        )?,
    )?;
    println!(
        "{} checks: {}",
        report.checks.len(),
        if report.passed() { "PASS" } else { "FAIL" }
    );
    if !report.passed() {
        return Err("audio-quality regression threshold failed; see report.json".into());
    }
    Ok(())
}
