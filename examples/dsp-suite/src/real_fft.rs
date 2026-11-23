use super::*;
use soniclaw_dsp::advanced::spectral::{
    AnalysisWindow, RealFft, WindowNormalization, WindowSampling, WindowShape,
};

pub fn run(out: &Path) -> Result<serde_json::Value> {
    let n = 4096;
    let bin = 100;
    let mut fft = RealFft::new(n)?;
    let mut bins = vec![0.; fft.bins()];
    let mut cases = Vec::new();
    for shape in [
        WindowShape::Hann,
        WindowShape::BlackmanHarris,
        WindowShape::FlatTop,
        WindowShape::Kaiser { beta: 9. },
    ] {
        let window = AnalysisWindow::new(
            n,
            shape,
            WindowSampling::Periodic,
            WindowNormalization::None,
        )?;
        let mut amplitudes = Vec::new();
        for offset in [0., 0.5] {
            let tone: Vec<_> = (0..n)
                .map(|i| {
                    (std::f64::consts::TAU * i as f64 * (bin as f64 + offset) / n as f64).cos()
                        as f32
                        * 0.5
                })
                .collect();
            assert_eq!(
                allocation_counter::measure(|| fft
                    .magnitudes(&tone, Some(&window), &mut bins)
                    .unwrap())
                .count_total,
                0
            );
            amplitudes.push(bins[bin]);
        }
        assert!((amplitudes[0] - 0.5).abs() < 1e-5);
        cases.push(serde_json::json!({"window":format!("{shape:?}"), "bin_center_amplitude":amplitudes[0], "half_bin_amplitude":amplitudes[1]}));
    }
    let mut csv = String::from("frequency_hz,peak_amplitude\n");
    for (i, amplitude) in bins.iter().enumerate() {
        use std::fmt::Write as _;
        writeln!(csv, "{},{}", i as f64 * 48000. / n as f64, amplitude)?;
    }
    std::fs::write(out.join("kaiser-spectrum.csv"), csv)?;
    Ok(serde_json::json!({"size":n,"cases":cases,"callback_allocations":0}))
}
