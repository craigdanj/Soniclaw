//! Offline specification-driven IIR and weighted least-squares FIR design.
use super::{butterworth, check_frequency, iir_at, Response, Section};
use rustfft::num_complex::Complex64;
use soniclaw_core::Error;
use std::f64::consts::{FRAC_PI_2, LN_10, PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IirFamily {
    Butterworth,
    ChebyshevI,
    ChebyshevII,
    Elliptic,
}

/// Edge frequencies in hertz. Ripple and attenuation are positive decibel values.
#[derive(Clone, Copy, Debug)]
pub struct IirSpec {
    pub sample_rate: f64,
    pub passband_hz: f64,
    pub stopband_hz: f64,
    pub passband_ripple_db: f64,
    pub stopband_attenuation_db: f64,
    pub family: IirFamily,
}

#[derive(Debug)]
pub struct IirDesign {
    pub order: usize,
    pub sections: Vec<Section>,
}

/// Find the first order up to 20 that meets the requested edges.
pub fn design_iir(spec: IirSpec) -> Result<IirDesign, Error> {
    let IirSpec {
        sample_rate,
        passband_hz,
        stopband_hz,
        passband_ripple_db: rp,
        stopband_attenuation_db: rs,
        family,
    } = spec;
    check_frequency(sample_rate, passband_hz)?;
    check_frequency(sample_rate, stopband_hz)?;
    if passband_hz == stopband_hz || !(0.01..=6.).contains(&rp) || !(12. ..=140.).contains(&rs) {
        return Err(Error::InvalidParameter);
    }
    let low = passband_hz < stopband_hz;
    let response = if low {
        Response::LowPass
    } else {
        Response::HighPass
    };
    let wp = (PI * passband_hz / sample_rate).tan();
    let ws = (PI * stopband_hz / sample_rate).tan();
    let epsilon_sq = (rp * LN_10 / 10.).exp_m1();
    let stop_sq = (rs * LN_10 / 10.).exp_m1();
    let edge_ratio = if low { ws / wp } else { wp / ws };
    let order_at_least = match family {
        IirFamily::Butterworth => (stop_sq / epsilon_sq).ln() / (2. * edge_ratio.ln()),
        IirFamily::ChebyshevI | IirFamily::ChebyshevII => {
            (stop_sq / epsilon_sq).sqrt().acosh() / edge_ratio.acosh()
        }
        IirFamily::Elliptic => {
            let m = edge_ratio.recip().powi(2);
            let m1 = epsilon_sq / stop_sq;
            complete(1. - m) * complete(m1) / (complete(m) * complete(1. - m1))
        }
    };
    let first = (order_at_least - 1e-10).ceil().max(1.) as usize;
    for order in first..=20 {
        let sections = if family == IirFamily::Butterworth {
            let shift = epsilon_sq.powf(-0.5 / order as f64);
            let cutoff = if low { wp * shift } else { wp / shift }.atan() * sample_rate / PI;
            butterworth(sample_rate, cutoff, order, response)?
        } else {
            let groups = prototype(order, family, epsilon_sq, stop_sq)?;
            let critical = if family == IirFamily::ChebyshevII {
                ws
            } else {
                wp
            };
            digital_sections(groups, critical, low, order, rp, family)?
        };
        let pass = iir_at(&sections, passband_hz / sample_rate).norm();
        let stop = iir_at(&sections, stopband_hz / sample_rate).norm();
        if pass >= 10f64.powf(-rp / 20.) * (1. - 1e-6)
            && stop <= 10f64.powf(-rs / 20.) * (1. + 1e-6)
        {
            return Ok(IirDesign { order, sections });
        }
    }
    Err(Error::InvalidParameter)
}

struct RootGroup {
    pole: Complex64,
    zero: Option<Complex64>,
    pair: bool,
}

fn prototype(
    order: usize,
    family: IirFamily,
    ep2: f64,
    stop2: f64,
) -> Result<Vec<RootGroup>, Error> {
    let n = order as f64;
    if family == IirFamily::Elliptic && order > 1 {
        return elliptic(order, ep2, stop2);
    }
    let inverse = family == IirFamily::ChebyshevII;
    let mu = if inverse {
        stop2.sqrt().asinh()
    } else {
        ep2.sqrt().recip().asinh()
    } / n;
    Ok((0..order.div_ceil(2))
        .map(|i| {
            let theta = PI * (2 * i + 1) as f64 / (2. * n);
            let pair = i < order / 2;
            let pole = Complex64::new(
                -mu.sinh() * theta.sin(),
                if pair { mu.cosh() * theta.cos() } else { 0. },
            );
            RootGroup {
                pole: if inverse { pole.inv() } else { pole },
                zero: (inverse && pair).then(|| Complex64::new(0., theta.cos().recip())),
                pair,
            }
        })
        .collect())
}

// K with the complementary parameter as input to preserve small stopband ratios.
fn complete(complement: f64) -> f64 {
    let (mut arithmetic, mut geometric) = (1., complement.sqrt());
    for _ in 0..32 {
        if arithmetic - geometric <= arithmetic * 2e-15 {
            break;
        }
        (arithmetic, geometric) = (
            (arithmetic + geometric) * 0.5,
            (arithmetic * geometric).sqrt(),
        );
    }
    FRAC_PI_2 / arithmetic
}

fn jacobi(u: f64, complement: f64) -> (f64, f64, f64) {
    let (mut a, mut b) = (1., complement.sqrt());
    let mut corrections = [0.; 32];
    let mut steps = 0;
    for correction in &mut corrections {
        let c = (a - b) * 0.5;
        if c <= a * 2e-15 {
            break;
        }
        (a, b) = ((a + b) * 0.5, (a * b).sqrt());
        *correction = c / a;
        steps += 1;
    }
    let mut phase = u * a * (1_u64 << steps) as f64;
    for ratio in corrections[..steps].iter().rev() {
        phase = 0.5 * (phase + (ratio * phase.sin()).asin());
    }
    let (s, c) = phase.sin_cos();
    (s, c, (c * c + complement * s * s).sqrt())
}

fn bisect(mut lo: f64, mut hi: f64, below: impl Fn(f64) -> bool) -> f64 {
    for _ in 0..72 {
        let mid = (lo + hi) * 0.5;
        if below(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) * 0.5
}

/// Cauer prototype using a degree equation and real Jacobi functions.
fn elliptic(order: usize, ep2: f64, stop2: f64) -> Result<Vec<RootGroup>, Error> {
    let n = order as f64;
    let m1 = ep2 / stop2;
    let k1 = complete(1. - m1);
    let degree_ratio = complete(m1) / (n * k1);
    // Solve in log-odds space so m and 1-m are each well-conditioned.
    let parameters = |log_odds: f64| {
        let odds = log_odds.exp();
        (odds / (1. + odds), 1. / (1. + odds))
    };
    let exponent = bisect(-80., 36., |x| {
        let (m, mc) = parameters(x);
        complete(m) / complete(mc) > degree_ratio
    });
    let (m, mc) = parameters(exponent);
    if exponent >= 35.99 || exponent <= -79.99 {
        return Err(Error::InvalidParameter);
    }
    let k = complete(mc);
    let r = bisect(0., complete(m1), |u| {
        let (s, c, _) = jacobi(u, m1);
        s * ep2.sqrt() < c
    });
    let (sv, cv, dv) = jacobi(k * r / (n * k1), m);
    Ok((1 - order % 2..order)
        .step_by(2)
        .map(|j| {
            let (s, c, d) = jacobi(j as f64 * k / n, mc);
            let pair = j != 0;
            let denominator = 1. - (d * sv).powi(2);
            RootGroup {
                pole: Complex64::new(-c * d * sv * cv / denominator, s * dv / denominator),
                zero: pair.then(|| Complex64::new(0., (m.sqrt() * s).recip())),
                pair,
            }
        })
        .collect())
}

fn digital_sections(
    groups: Vec<RootGroup>,
    critical: f64,
    low: bool,
    order: usize,
    rp: f64,
    family: IirFamily,
) -> Result<Vec<Section>, Error> {
    let z = |root: Complex64| {
        let analog = if low {
            root * critical
        } else {
            root.inv() * critical
        };
        (1. + analog) / (1. - analog)
    };
    let polynomial = |root: Complex64, pair| {
        [
            1.,
            if pair { -2. * root.re } else { -root.re },
            if pair { root.norm_sqr() } else { 0. },
        ]
    };
    let anchor = if low { 1. } else { -1. };
    let at_anchor = |p: [f64; 3]| p[0] + anchor * p[1] + p[2];
    let mut stages: Vec<_> = groups
        .into_iter()
        .map(|group| {
            let pole = z(group.pole);
            let zero = group.zero.map_or(Complex64::new(-anchor, 0.), z);
            let a = polynomial(pole, group.pair);
            let b = polynomial(zero, group.pair);
            let gain = at_anchor(a) / at_anchor(b);
            Section::new(b.map(|x| x * gain), a).map(|s| (pole.norm_sqr(), s))
        })
        .collect::<Result<_, _>>()?;
    stages.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut sections: Vec<Section> = stages.into_iter().map(|(_, section)| section).collect();
    if order.is_multiple_of(2) && matches!(family, IirFamily::ChebyshevI | IirFamily::Elliptic) {
        sections[0].b = sections[0].b.map(|x| x * 10f64.powf(-rp / 20.));
    }
    Ok(sections)
}

/// One inclusive frequency band, in cycles/sample (0 through Nyquist at 0.5).
#[derive(Clone, Copy, Debug)]
pub struct FirBand {
    pub start: f64,
    pub end: f64,
    pub gain: f64,
    pub weight: f64,
}

/// Odd, symmetric FIR minimizing weighted squared error in the specified bands.
pub fn least_squares(taps: usize, bands: &[FirBand]) -> Result<Vec<f64>, Error> {
    validate_taps(taps)?;
    if bands.is_empty() || bands.len() > 32 {
        return Err(Error::InvalidParameter);
    }
    let mut edge = 0.;
    for b in bands {
        if !(edge..=0.5).contains(&b.start)
            || !(b.start..=0.5).contains(&b.end)
            || b.end == b.start
            || !(-2. ..=2.).contains(&b.gain)
            || !(1e-4..=1e4).contains(&b.weight)
        {
            return Err(Error::InvalidParameter);
        }
        edge = b.end;
    }
    let indices: Vec<_> = (0..=taps / 2).collect();
    fit(taps, &indices, bands, 0.)
}

/// Linear-phase half-band with an exact center coefficient of 0.5 and alternating zero taps.
pub fn half_band(taps: usize, transition_width: f64) -> Result<Vec<f64>, Error> {
    validate_taps(taps)?;
    if !taps.is_multiple_of(2)
        && (taps - 3).is_multiple_of(4)
        && (0.01..=0.4).contains(&transition_width)
    {
        let indices: Vec<_> = (1..=taps / 2).step_by(2).collect();
        let band = FirBand {
            start: 0.,
            end: 0.25 - transition_width * 0.5,
            gain: 1.,
            weight: 1.,
        };
        fit(taps, &indices, &[band], 0.5)
    } else {
        Err(Error::InvalidParameter)
    }
}

fn validate_taps(taps: usize) -> Result<(), Error> {
    if (3..=511).contains(&taps) && !taps.is_multiple_of(2) {
        Ok(())
    } else {
        Err(Error::InvalidParameter)
    }
}

fn fit(taps: usize, indices: &[usize], bands: &[FirBand], center: f64) -> Result<Vec<f64>, Error> {
    let n = indices.len();
    let mut normal = vec![vec![0.; n]; n];
    let mut rhs = vec![0.; n];
    for band in bands {
        let integrals: Vec<_> = (0..taps)
            .map(|k| {
                if k == 0 {
                    return band.end - band.start;
                }
                let k = TAU * k as f64;
                ((k * band.end).sin() - (k * band.start).sin()) / k
            })
            .collect();
        for (row, &i) in indices.iter().enumerate() {
            rhs[row] += band.weight * (band.gain - center) * integrals[i];
            for (col, &j) in indices[..=row].iter().enumerate() {
                normal[row][col] +=
                    0.5 * band.weight * (integrals[i.abs_diff(j)] + integrals[i + j]);
            }
        }
    }
    // Cholesky on the positive-definite Gram matrix. Near-singular designs are rejected.
    let floor = normal[0][0] * 1e-13;
    for row in 0..n {
        for col in 0..=row {
            let product: f64 = (0..col).map(|k| normal[row][k] * normal[col][k]).sum();
            let residual = normal[row][col] - product;
            normal[row][col] = if row == col {
                if !residual.is_finite() || residual <= floor {
                    return Err(Error::InvalidParameter);
                }
                residual.sqrt()
            } else {
                residual / normal[col][col]
            };
        }
        let product: f64 = rhs[..row]
            .iter()
            .zip(&normal[row])
            .map(|(a, b)| a * b)
            .sum();
        rhs[row] = (rhs[row] - product) / normal[row][row];
    }
    for i in (0..n).rev() {
        let product: f64 = (i + 1..n).map(|row| normal[row][i] * rhs[row]).sum();
        rhs[i] = (rhs[i] - product) / normal[i][i];
    }
    let middle = taps / 2;
    let mut output = vec![0.; taps];
    output[middle] = center;
    for (distance, coefficient) in indices.iter().copied().zip(rhs) {
        if !coefficient.is_finite() || coefficient.abs() > 16. {
            return Err(Error::InvalidParameter);
        }
        if distance == 0 {
            output[middle] = coefficient;
        } else {
            output[middle - distance] = coefficient * 0.5;
            output[middle + distance] = coefficient * 0.5;
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::super::fir_at;
    use super::*;

    #[test]
    fn reference_iir_orders_and_response() {
        // Independent SciPy 1.17-style signal.iirdesign reference, checked at linear frequencies.
        for (family, order) in [
            (IirFamily::Butterworth, 13),
            (IirFamily::ChebyshevI, 8),
            (IirFamily::ChebyshevII, 8),
            (IirFamily::Elliptic, 5),
        ] {
            for (pass, stop) in [(6000., 10000.), (18000., 14000.)] {
                let design = design_iir(IirSpec {
                    sample_rate: 48000.,
                    passband_hz: pass,
                    stopband_hz: stop,
                    passband_ripple_db: 0.5,
                    stopband_attenuation_db: 60.,
                    family,
                })
                .unwrap();
                assert_eq!(design.order, order, "{family:?}");
                let low = pass < stop;
                for i in 0..=8192 {
                    let f = 24000. * f64::from(i) / 8192.;
                    let gain = iir_at(&design.sections, f / 48000.).norm();
                    if (low && f <= pass) || (!low && f >= pass) {
                        assert!(
                            (0.944_060_87..=1.000_001).contains(&gain),
                            "{family:?} {f} {gain}"
                        );
                    } else if (low && f >= stop) || (!low && f <= stop) {
                        assert!(gain <= 0.001_001, "{family:?} {f} {gain}");
                    }
                }
            }
        }
    }

    #[test]
    fn fir_symmetry_and_half_band_complement() {
        let h = least_squares(
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
        )
        .unwrap();
        assert_eq!(
            h.iter().collect::<Vec<_>>(),
            h.iter().rev().collect::<Vec<_>>()
        );
        assert!((fir_at(&h, 0.1).norm() - 1.).abs() < 0.001);
        assert!(fir_at(&h, 0.28).norm() < 0.001);
        let half = half_band(31, 0.12).unwrap();
        assert_eq!(half[15], 0.5);
        assert!(half
            .iter()
            .enumerate()
            .all(|(i, &x)| i.is_multiple_of(2) || i == 15 || x == 0.));
        assert!((fir_at(&half, 0.25).norm() - 0.5).abs() < 1e-13);
        assert!((fir_at(&half, 0.1).norm() - 1.).abs() < 0.002);
        assert!(fir_at(&half, 0.4).norm() < 0.002);
    }
}
