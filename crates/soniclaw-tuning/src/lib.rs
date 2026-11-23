//! Parse on the control thread; prepared instruments perform one immutable table lookup per key.
use serde::{Deserialize, Serialize};
use soniclaw_core::Error;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Raw", into = "Raw")]
pub struct Tuning {
    name: String,
    frequencies: [Option<f64>; 128],
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    schema: u32,
    name: String,
    frequencies: Vec<Option<f64>>,
}
impl TryFrom<Raw> for Tuning {
    type Error = Error;
    fn try_from(r: Raw) -> Result<Self, Error> {
        if r.schema != 1 {
            return Err(Error::InvalidParameter);
        }
        Self::new(
            &r.name,
            r.frequencies
                .try_into()
                .map_err(|_| Error::InvalidParameter)?,
        )
    }
}
impl From<Tuning> for Raw {
    fn from(t: Tuning) -> Self {
        Self {
            schema: 1,
            name: t.name,
            frequencies: t.frequencies.to_vec(),
        }
    }
}
impl Default for Tuning {
    fn default() -> Self {
        Self::equal_temperament(69, 440.).expect("standard tuning")
    }
}
impl Tuning {
    pub fn new(name: &str, frequencies: [Option<f64>; 128]) -> Result<Self, Error> {
        let t = Self {
            name: name.into(),
            frequencies,
        };
        t.validate()?;
        Ok(t)
    }
    pub fn validate(&self) -> Result<(), Error> {
        if self.name.len() > 256
            || self.frequencies.iter().all(Option::is_none)
            || self
                .frequencies
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || !(0.01..=1e6).contains(v))
        {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
    pub fn equal_temperament(reference_note: u8, hz: f64) -> Result<Self, Error> {
        if reference_note > 127 || !hz.is_finite() || hz <= 0. {
            return Err(Error::InvalidParameter);
        }
        Self::new(
            "12-tone equal temperament",
            std::array::from_fn(|k| Some(hz * 2f64.powf((k as f64 - reference_note as f64) / 12.))),
        )
    }
    pub fn frequency(&self, key: u8) -> Option<f64> {
        self.frequencies.get(key as usize).copied().flatten()
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn to_json(&self) -> Result<String, Error> {
        self.validate()?;
        serde_json::to_string_pretty(self).map_err(|_| Error::InvalidParameter)
    }
    pub fn from_json(text: &str) -> Result<Self, Error> {
        if text.len() > 32768 {
            return Err(Error::InvalidParameter);
        }
        serde_json::from_str(text).map_err(|_| Error::InvalidParameter)
    }
    pub fn from_scala(scale: &Scale, mapping: &KeyboardMapping) -> Result<Self, Error> {
        if !(0.01..=1e6).contains(&mapping.hz) {
            return Err(Error::InvalidParameter);
        }
        let base = mapping
            .degree(mapping.reference)
            .ok_or(Error::InvalidParameter)?;
        let offset = scale.cents(base);
        let mut frequencies = [None; 128];
        for key in mapping.first..=mapping.last {
            if let Some(degree) = mapping.degree(key) {
                let hz = mapping.hz * 2f64.powf((scale.cents(degree) - offset) / 1200.);
                frequencies[key as usize] = if hz.is_finite() && (0.01..=1e6).contains(&hz) {
                    Some(hz)
                } else {
                    None
                };
            }
        }
        Self::new(&scale.name, frequencies)
    }
}
fn lines(text: &str) -> Result<Vec<&str>, Error> {
    if text.len() > 65536 {
        return Err(Error::InvalidParameter);
    }
    Ok(text
        .lines()
        .filter(|l| !l.trim_start().starts_with('!'))
        .map(|l| l.split('!').next().unwrap_or("").trim())
        .collect())
}
#[derive(Clone, Debug)]
pub struct Scale {
    name: String,
    intervals: Vec<f64>,
}
impl Scale {
    pub fn new(name: &str, intervals: Vec<f64>) -> Result<Self, Error> {
        if name.len() > 256
            || intervals.is_empty()
            || intervals.len() > 128
            || intervals.iter().any(|v| !v.is_finite() || v.abs() > 9600.)
            || intervals.last().is_none_or(|v| *v <= 0.)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            name: name.into(),
            intervals,
        })
    }
    /// A bounded UTF-8 Scala subset: ratios/integer ratios/decimal cents, comments and trailing pitch labels.
    pub fn from_scl(text: &str) -> Result<Self, Error> {
        let l = lines(text)?;
        let name = *l.first().ok_or(Error::InvalidParameter)?;
        let mut rest = l.iter().skip(1).filter(|s| !s.is_empty());
        let n = rest
            .next()
            .ok_or(Error::InvalidParameter)?
            .parse::<usize>()
            .map_err(|_| Error::InvalidParameter)?;
        if !(1..=128).contains(&n) {
            return Err(Error::InvalidParameter);
        }
        let mut values = Vec::with_capacity(n);
        for line in rest {
            let token = line
                .split_whitespace()
                .next()
                .ok_or(Error::InvalidParameter)?;
            let cents = if token.contains('.') {
                token.parse::<f64>().map_err(|_| Error::InvalidParameter)?
            } else {
                let (a, b) = token.split_once('/').unwrap_or((token, "1"));
                let a = a.parse::<u32>().map_err(|_| Error::InvalidParameter)?;
                let b = b.parse::<u32>().map_err(|_| Error::InvalidParameter)?;
                if a == 0 || b == 0 {
                    return Err(Error::InvalidParameter);
                }
                1200. * (a as f64 / b as f64).log2()
            };
            values.push(cents);
            if values.len() > n {
                return Err(Error::InvalidParameter);
            }
        }
        if values.len() != n {
            return Err(Error::InvalidParameter);
        }
        Self::new(name, values)
    }
    pub fn steps(&self) -> usize {
        self.intervals.len()
    }
    pub fn cents(&self, degree: i32) -> f64 {
        let n = self.steps() as i32;
        let q = degree.div_euclid(n);
        let r = degree.rem_euclid(n);
        q as f64 * self.intervals[self.steps() - 1]
            + if r == 0 {
                0.
            } else {
                self.intervals[r as usize - 1]
            }
    }
}
#[derive(Clone, Debug)]
pub struct KeyboardMapping {
    first: u8,
    last: u8,
    middle: u8,
    reference: u8,
    hz: f64,
    octave: i32,
    entries: Vec<Option<i32>>,
}
impl KeyboardMapping {
    pub fn linear(reference: u8, hz: f64) -> Result<Self, Error> {
        if reference > 127 || !hz.is_finite() || hz <= 0. {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            first: 0,
            last: 127,
            middle: 60,
            reference,
            hz,
            octave: 0,
            entries: vec![],
        })
    }
    pub fn from_kbm(text: &str, scale: &Scale) -> Result<Self, Error> {
        let l = lines(text)?;
        let tokens: Vec<_> = l
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.split_whitespace().next().unwrap())
            .collect();
        if tokens.len() < 7 {
            return Err(Error::InvalidParameter);
        }
        let number = |i: usize| {
            tokens[i]
                .parse::<i32>()
                .map_err(|_| Error::InvalidParameter)
        };
        let size = number(0)?;
        let first = number(1)?;
        let last = number(2)?;
        let middle = number(3)?;
        let reference = number(4)?;
        let hz = tokens[5]
            .parse::<f64>()
            .map_err(|_| Error::InvalidParameter)?;
        let mut octave = number(6)?;
        if octave == 0 {
            octave = scale.steps() as i32
        }
        if !(0..=128).contains(&size)
            || tokens.len() != 7 + size as usize
            || !(0..=127).contains(&first)
            || !(first..=127).contains(&last)
            || !(0..=127).contains(&middle)
            || !(0..=127).contains(&reference)
            || !hz.is_finite()
            || hz <= 0.
            || octave.unsigned_abs() > 4096
        {
            return Err(Error::InvalidParameter);
        }
        let entries = tokens[7..]
            .iter()
            .map(|t| {
                if t.eq_ignore_ascii_case("x") {
                    Ok(None)
                } else {
                    let v = t.parse::<i32>().map_err(|_| Error::InvalidParameter)?;
                    if v.unsigned_abs() > 4096 {
                        return Err(Error::InvalidParameter);
                    }
                    Ok(Some(v))
                }
            })
            .collect::<Result<_, _>>()?;
        let m = Self {
            first: first as u8,
            last: last as u8,
            middle: middle as u8,
            reference: reference as u8,
            hz,
            octave,
            entries,
        };
        if m.degree(m.reference).is_none() {
            return Err(Error::InvalidParameter);
        }
        Ok(m)
    }
    fn degree(&self, key: u8) -> Option<i32> {
        let d = key as i32 - self.middle as i32;
        if self.entries.is_empty() {
            Some(d)
        } else {
            let n = self.entries.len() as i32;
            self.entries[d.rem_euclid(n) as usize].map(|v| v + d.div_euclid(n) * self.octave)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn equal_temperament_json_and_invalid() {
        let t = Tuning::default();
        assert_eq!(t.frequency(69), Some(440.));
        assert!((t.frequency(81).unwrap() - 880.).abs() < 1e-10);
        assert_eq!(Tuning::from_json(&t.to_json().unwrap()).unwrap(), t);
        let mut raw: serde_json::Value = serde_json::from_str(&t.to_json().unwrap()).unwrap();
        raw["frequencies"][0] = serde_json::json!(-1);
        assert!(Tuning::from_json(&raw.to_string()).is_err());
        assert!(Tuning::new("empty", [None; 128]).is_err());
    }
    #[test]
    fn scala_ratios_nonmonotonic_and_negative_degrees() {
        let s = Scale::from_scl("! test\nJust triad\n3\n5/4 third\n3/2 fifth\n2\n").unwrap();
        let t = Tuning::from_scala(&s, &KeyboardMapping::linear(60, 256.).unwrap()).unwrap();
        assert!((t.frequency(61).unwrap() - 320.).abs() < 1e-9);
        assert!((t.frequency(59).unwrap() - 192.).abs() < 1e-9);
        assert!(Scale::from_scl("x\n2\n1/0\n2/1").is_err());
        let s = Scale::from_scl("\n2\n-5.0\n2/1").unwrap();
        assert_eq!(s.cents(1), -5.);
    }
    #[test]
    fn kbm_gaps_reference_and_repetition() {
        let s = Scale::from_scl("triad\n3\n5/4\n3/2\n2/1").unwrap();
        let m = KeyboardMapping::from_kbm("4\n48\n84\n60\n60\n256.0\n0\n0\nx\n1\n2", &s).unwrap();
        let t = Tuning::from_scala(&s, &m).unwrap();
        assert_eq!(t.frequency(61), None);
        assert_eq!(t.frequency(47), None);
        assert!((t.frequency(64).unwrap() - 512.).abs() < 1e-9);
        assert!((t.frequency(56).unwrap() - 128.).abs() < 1e-9);
        assert!(KeyboardMapping::from_kbm("1\n0\n127\n60\n60\n440\n3\nx", &s).is_err());
    }
}
