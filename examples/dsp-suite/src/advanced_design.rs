use super::*;
use soniclaw_dsp::filter_design::{
    design_iir, fir_at, half_band, iir_at, least_squares, Filter, FirBand, IirFamily, IirSpec,
};

pub fn run(out: &Path) -> Result<serde_json::Value> {
    let families = [
        IirFamily::Butterworth,
        IirFamily::ChebyshevI,
        IirFamily::ChebyshevII,
        IirFamily::Elliptic,
    ];
    let mut cases = Vec::new();
    for family in families {
        let d = design_iir(IirSpec {
            sample_rate: 48_000.,
            passband_hz: 6_000.,
            stopband_hz: 10_000.,
            passband_ripple_db: 0.5,
            stopband_attenuation_db: 60.,
            family,
        })?;
        let metric = |hz| 20. * iir_at(&d.sections, hz / 48_000.).norm().log10();
        cases.push(serde_json::json!({
            "family": format!("{family:?}"), "order": d.order, "pass_edge_db": metric(6000.),
            "stop_edge_db": metric(10_000.),
            "reference_frequencies_hz": [1000., 4000., 6000., 8500., 10000., 14000., 21000.],
            "response_db": ([1000., 4000., 6000., 8500., 10000., 14000., 21000.].map(metric))
        }));
        if family == IirFamily::Elliptic {
            let f = Filter::iir(48_000., d.sections)?;
            let mut impulse = vec![0.; 512];
            impulse[0] = 1.;
            render(out, "elliptic-impulse", Box::new(f), impulse)?;
        }
    }
    let taps = least_squares(
        63,
        &[
            FirBand {
                start: 0.,
                end: 0.15,
                gain: 1.,
                weight: 1.,
            },
            FirBand {
                start: 0.22,
                end: 0.5,
                gain: 0.,
                weight: 4.,
            },
        ],
    )?;
    let half = half_band(31, 0.12)?;
    let mut impulse = vec![0.; 256];
    impulse[0] = 1.;
    let (report, _) = render(
        out,
        "half-band-impulse",
        Box::new(Filter::fir(48_000., half.clone())?),
        impulse,
    )?;
    Ok(serde_json::json!({
        "iir": cases, "least_squares_taps": taps, "half_band": report, "half_band_edge_gain": fir_at(&half, 0.25).norm()
    }))
}
