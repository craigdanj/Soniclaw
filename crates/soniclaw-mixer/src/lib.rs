//! Stereo mixing; route/compensate processors upstream and apply a limiter downstream if required.
use serde::{Deserialize, Serialize};
use soniclaw_core::Error;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc,
};
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Tap {
    PreFader,
    PostFader,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Strip {
    pub id: String,
    pub name: String,
    pub gain: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub group: Option<usize>,
    pub sends: Vec<f32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub gain: f32,
    pub mute: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub schema: u32,
    pub strips: Vec<Strip>,
    pub groups: Vec<Group>,
    pub taps: Vec<Tap>,
}
fn gain(v: f32) -> bool {
    v.is_finite() && (0.0..=4.0).contains(&v)
}
impl Model {
    pub fn validate(&self) -> Result<(), Error> {
        if self.schema != 1
            || self.strips.is_empty()
            || self.strips.len() > 64
            || self.groups.len() > 16
            || self.taps.len() > 4
        {
            return Err(Error::InvalidRouting);
        }
        let mut ids = std::collections::BTreeSet::new();
        for c in &self.strips {
            if c.id.is_empty()
                || c.id.len() > 128
                || c.name.len() > 256
                || !ids.insert(&c.id)
                || !gain(c.gain)
                || !c.pan.is_finite()
                || c.pan.abs() > 1.
                || c.group.is_some_and(|g| g >= self.groups.len())
                || c.sends.len() != self.taps.len()
                || c.sends.iter().any(|v| !gain(*v))
            {
                return Err(Error::InvalidParameter);
            }
        }
        if self.groups.iter().any(|g| !gain(g.gain)) {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
    pub fn to_json(&self) -> Result<String, Error> {
        self.validate()?;
        serde_json::to_string_pretty(self).map_err(|_| Error::InvalidParameter)
    }
    pub fn from_json(json: &str) -> Result<Self, Error> {
        if json.len() > 128 * 1024 {
            return Err(Error::InvalidParameter);
        }
        let m: Self = serde_json::from_str(json).map_err(|_| Error::InvalidParameter)?;
        m.validate()?;
        Ok(m)
    }
    pub fn compile(&self, rate: f64, max_frames: usize) -> Result<Mixer, Error> {
        self.validate()?;
        if !rate.is_finite()
            || !(8000.0..=384000.).contains(&rate)
            || !(1..=8192).contains(&max_frames)
        {
            return Err(Error::InvalidSpec);
        }
        let controls = self
            .strips
            .iter()
            .map(|s| Arc::new(Controls::new(s)))
            .collect::<Vec<_>>();
        let groups = self
            .groups
            .iter()
            .map(|g| {
                Arc::new(GroupControls {
                    gain: AtomicU32::new(g.gain.to_bits()),
                    mute: AtomicBool::new(g.mute),
                })
            })
            .collect();
        Ok(Mixer {
            smooth: self
                .strips
                .iter()
                .map(|s| Smooth {
                    gain: s.gain * s.group.map(|g| self.groups[g].gain).unwrap_or(1.),
                    pan: s.pan,
                    gate: if s.mute
                        || s.group.is_some_and(|g| self.groups[g].mute)
                        || (self.strips.iter().any(|c| c.solo) && !s.solo)
                    {
                        0.
                    } else {
                        1.
                    },
                    sends: std::array::from_fn(|i| s.sends.get(i).copied().unwrap_or(0.)),
                })
                .collect(),
            controls,
            groups,
            model: self.clone(),
            max_frames,
            coefficient: (1. - (-1. / (rate * 0.005)).exp()) as f32,
        })
    }
}
pub struct Controls {
    gain: AtomicU32,
    pan: AtomicU32,
    mute: AtomicBool,
    solo: AtomicBool,
    sends: [AtomicU32; 4],
    peak: [AtomicU32; 2],
}
impl Controls {
    pub fn send(&self, index: usize) -> Option<f32> {
        self.sends
            .get(index)
            .map(|v| f32::from_bits(v.load(Ordering::Relaxed)))
    }
    fn new(s: &Strip) -> Self {
        Self {
            gain: AtomicU32::new(s.gain.to_bits()),
            pan: AtomicU32::new(s.pan.to_bits()),
            mute: AtomicBool::new(s.mute),
            solo: AtomicBool::new(s.solo),
            sends: std::array::from_fn(|i| {
                AtomicU32::new(s.sends.get(i).copied().unwrap_or(0.).to_bits())
            }),
            peak: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }
    pub fn gain(&self) -> f32 {
        f32::from_bits(self.gain.load(Ordering::Relaxed))
    }
    pub fn pan(&self) -> f32 {
        f32::from_bits(self.pan.load(Ordering::Relaxed))
    }
    pub fn muted(&self) -> bool {
        self.mute.load(Ordering::Relaxed)
    }
    pub fn soloed(&self) -> bool {
        self.solo.load(Ordering::Relaxed)
    }
    pub fn set_gain(&self, v: f32) -> Result<(), Error> {
        if !gain(v) {
            return Err(Error::InvalidParameter);
        }
        self.gain.store(v.to_bits(), Ordering::Relaxed);
        Ok(())
    }
    pub fn set_pan(&self, v: f32) -> Result<(), Error> {
        if !v.is_finite() || v.abs() > 1. {
            return Err(Error::InvalidParameter);
        }
        self.pan.store(v.to_bits(), Ordering::Relaxed);
        Ok(())
    }
    pub fn set_mute(&self, v: bool) {
        self.mute.store(v, Ordering::Relaxed)
    }
    pub fn set_solo(&self, v: bool) {
        self.solo.store(v, Ordering::Relaxed)
    }
    pub fn set_send(&self, index: usize, v: f32) -> Result<(), Error> {
        if index >= 4 || !gain(v) {
            return Err(Error::InvalidParameter);
        }
        self.sends[index].store(v.to_bits(), Ordering::Relaxed);
        Ok(())
    }
    pub fn take_peaks(&self) -> [f32; 2] {
        std::array::from_fn(|i| f32::from_bits(self.peak[i].swap(0, Ordering::Relaxed)))
    }
}
pub struct GroupControls {
    gain: AtomicU32,
    mute: AtomicBool,
}
impl GroupControls {
    pub fn gain(&self) -> f32 {
        f32::from_bits(self.gain.load(Ordering::Relaxed))
    }
    pub fn muted(&self) -> bool {
        self.mute.load(Ordering::Relaxed)
    }
    pub fn set_gain(&self, v: f32) -> Result<(), Error> {
        if !gain(v) {
            return Err(Error::InvalidParameter);
        }
        self.gain.store(v.to_bits(), Ordering::Relaxed);
        Ok(())
    }
    pub fn set_mute(&self, v: bool) {
        self.mute.store(v, Ordering::Relaxed)
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Value {
    Gain(f32),
    Pan(f32),
    Mute(bool),
    Solo(bool),
    Send(usize, f32),
}
#[derive(Clone, Copy, Debug)]
pub struct Change {
    pub frame: usize,
    pub strip: usize,
    pub value: Value,
}
struct Smooth {
    gain: f32,
    pan: f32,
    gate: f32,
    sends: [f32; 4],
}
pub struct Mixer {
    model: Model,
    controls: Vec<Arc<Controls>>,
    groups: Vec<Arc<GroupControls>>,
    smooth: Vec<Smooth>,
    max_frames: usize,
    coefficient: f32,
}
impl Mixer {
    pub fn controls(&self) -> &[Arc<Controls>] {
        &self.controls
    }
    pub fn groups(&self) -> &[Arc<GroupControls>] {
        &self.groups
    }
    pub fn model(&self) -> &Model {
        &self.model
    }
    /// Capture current atomic targets off the callback. Meter and smoothing state are not persisted.
    pub fn snapshot(&self) -> Model {
        let mut m = self.model.clone();
        for (s, c) in m.strips.iter_mut().zip(&self.controls) {
            s.gain = c.gain();
            s.pan = c.pan();
            s.mute = c.muted();
            s.solo = c.soloed();
            for (i, v) in s.sends.iter_mut().enumerate() {
                *v = f32::from_bits(c.sends[i].load(Ordering::Relaxed));
            }
        }
        for (g, c) in m.groups.iter_mut().zip(&self.groups) {
            g.gain = f32::from_bits(c.gain.load(Ordering::Relaxed));
            g.mute = c.mute.load(Ordering::Relaxed);
        }
        m
    }
    pub fn process(
        &mut self,
        inputs: &[&[f32]],
        main: &mut [f32],
        sends: &mut [&mut [f32]],
        changes: &[Change],
    ) -> Result<(), Error> {
        self.process_inner(inputs, main, sends, changes, &mut [], &[])
    }
    /// Prepared post-strip outputs share the exact main/send smoothing, pan, mute and solo decisions.
    pub fn process_routed(
        &mut self,
        inputs: &[&[f32]],
        main: &mut [f32],
        sends: &mut [&mut [f32]],
        changes: &[Change],
        post: &mut [&mut [f32]],
        audible: &[bool],
    ) -> Result<(), Error> {
        self.process_inner(inputs, main, sends, changes, post, audible)
    }
    /// Initialize a prepared bus-solo mask before playback, without changing persisted track controls.
    pub fn initialize_audible(&mut self, audible: &[bool]) -> Result<(), Error> {
        if audible.len() != self.smooth.len() {
            return Err(Error::InvalidRouting);
        }
        for (s, a) in self.smooth.iter_mut().zip(audible) {
            if !a {
                s.gate = 0.;
            }
        }
        Ok(())
    }
    fn process_inner(
        &mut self,
        inputs: &[&[f32]],
        main: &mut [f32],
        sends: &mut [&mut [f32]],
        changes: &[Change],
        post_outputs: &mut [&mut [f32]],
        audible: &[bool],
    ) -> Result<(), Error> {
        main.fill(0.);
        for s in sends.iter_mut() {
            s.fill(0.);
        }
        for p in post_outputs.iter_mut() {
            p.fill(0.);
        }
        let frames = main.len() / 2;
        if !main.len().is_multiple_of(2)
            || frames > self.max_frames
            || inputs.len() != self.controls.len()
            || sends.len() != self.model.taps.len()
            || inputs
                .iter()
                .any(|i| i.len() != main.len() || i.iter().any(|v| !v.is_finite() || v.abs() > 1e6))
            || sends.iter().any(|s| s.len() != main.len())
            || (!post_outputs.is_empty()
                && (post_outputs.len() != inputs.len()
                    || post_outputs.iter().any(|p| p.len() != main.len())))
            || (!audible.is_empty() && audible.len() != inputs.len())
            || changes.len() > 1024
        {
            return Err(Error::InvalidBuffer);
        }
        let mut last = 0;
        for e in changes {
            if e.frame >= frames || e.frame < last || e.strip >= inputs.len() {
                return Err(Error::InvalidEvents);
            }
            last = e.frame;
            let ok = match e.value {
                Value::Gain(v) => gain(v),
                Value::Pan(v) => v.is_finite() && v.abs() <= 1.,
                Value::Send(i, v) => i < sends.len() && gain(v),
                _ => true,
            };
            if !ok {
                return Err(Error::InvalidEvents);
            }
        }
        let mut target_gain = [0.; 64];
        let mut pan = [0.; 64];
        let mut mute = [false; 64];
        let mut solo = [false; 64];
        let mut send = [[0.; 4]; 64];
        let mut group_gain = [1.; 64];
        let mut group_mute = [false; 64];
        let mut peaks = [[0f32; 2]; 64];
        for (i, c) in self.controls.iter().enumerate() {
            target_gain[i] = c.gain();
            pan[i] = c.pan();
            mute[i] = c.muted();
            solo[i] = c.soloed();
            for (j, s) in send[i].iter_mut().enumerate().take(sends.len()) {
                *s = f32::from_bits(c.sends[j].load(Ordering::Relaxed));
            }
            if let Some(g) = self.model.strips[i].group {
                group_gain[i] = f32::from_bits(self.groups[g].gain.load(Ordering::Relaxed));
                group_mute[i] = self.groups[g].mute.load(Ordering::Relaxed);
            }
        }
        let mut cursor = 0;
        for frame in 0..frames {
            while cursor < changes.len() && changes[cursor].frame == frame {
                let e = changes[cursor];
                let c = &self.controls[e.strip];
                match e.value {
                    Value::Gain(v) => {
                        target_gain[e.strip] = v;
                        c.set_gain(v)?
                    }
                    Value::Pan(v) => {
                        pan[e.strip] = v;
                        c.set_pan(v)?
                    }
                    Value::Mute(v) => {
                        mute[e.strip] = v;
                        c.set_mute(v)
                    }
                    Value::Solo(v) => {
                        solo[e.strip] = v;
                        c.set_solo(v)
                    }
                    Value::Send(j, v) => {
                        send[e.strip][j] = v;
                        c.set_send(j, v)?
                    }
                }
                cursor += 1;
            }
            let any_solo = solo[..inputs.len()].iter().any(|v| *v);
            for (i, input) in inputs.iter().enumerate() {
                let s = &mut self.smooth[i];
                let gate = if mute[i]
                    || group_mute[i]
                    || (any_solo && !solo[i])
                    || (!audible.is_empty() && !audible[i])
                {
                    0.
                } else {
                    1.
                };
                let k = self.coefficient;
                s.gain += (target_gain[i] * group_gain[i] - s.gain) * k;
                s.pan += (pan[i] - s.pan) * k;
                s.gate += (gate - s.gate) * k;
                let angle = (s.pan + 1.) * std::f32::consts::FRAC_PI_4;
                let pre = [
                    input[frame * 2] * angle.cos() * s.gate,
                    input[frame * 2 + 1] * angle.sin() * s.gate,
                ];
                for ch in 0..2 {
                    let post = pre[ch] * s.gain;
                    main[frame * 2 + ch] += post;
                    if !post_outputs.is_empty() {
                        post_outputs[i][frame * 2 + ch] = post;
                    }
                    peaks[i][ch] = peaks[i][ch].max(post.abs());
                }
                for (j, output) in sends.iter_mut().enumerate() {
                    s.sends[j] += (send[i][j] - s.sends[j]) * k;
                    let tap = match self.model.taps[j] {
                        Tap::PreFader => 1.,
                        Tap::PostFader => s.gain,
                    };
                    output[frame * 2] += pre[0] * tap * s.sends[j];
                    output[frame * 2 + 1] += pre[1] * tap * s.sends[j];
                }
            }
        }
        for (i, c) in self.controls.iter().enumerate() {
            for ch in 0..2 {
                c.peak[ch].fetch_max(peaks[i][ch].to_bits(), Ordering::Relaxed);
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn model() -> Model {
        Model {
            schema: 1,
            strips: (0..2)
                .map(|i| Strip {
                    id: i.to_string(),
                    name: i.to_string(),
                    gain: 1.,
                    pan: 0.,
                    mute: false,
                    solo: false,
                    group: None,
                    sends: vec![0.5],
                })
                .collect(),
            groups: vec![],
            taps: vec![Tap::PostFader],
        }
    }
    #[test]
    fn pan_sends_meters_and_no_alloc() {
        let mut m = model().compile(48000., 128).unwrap();
        let a = [1.; 256];
        let b = [0.; 256];
        let mut out = [0.; 256];
        let mut aux = [0.; 256];
        let count = allocation_counter::measure(|| {
            m.process(&[&a, &b], &mut out, &mut [&mut aux], &[])
                .unwrap()
        });
        assert_eq!(count.count_total, 0);
        assert!((out[0] - 0.5f32.sqrt()).abs() < 1e-6);
        assert!((aux[0] - out[0] * 0.5).abs() < 1e-6);
        assert_eq!(m.controls()[0].take_peaks(), [out[0]; 2]);
        assert_eq!(m.controls()[0].take_peaks(), [0.; 2]);
    }
    #[test]
    fn sample_automation_partition_and_invalid_is_transactional() {
        let model = model();
        let mut a = model.compile(48000., 64).unwrap();
        let mut b = model.compile(48000., 64).unwrap();
        let input = [1.; 128];
        let zero = [0.; 128];
        let mut full = [0.; 128];
        let mut aux = [0.; 128];
        let changes = [
            Change {
                frame: 17,
                strip: 0,
                value: Value::Gain(0.),
            },
            Change {
                frame: 31,
                strip: 1,
                value: Value::Solo(true),
            },
        ];
        a.process(&[&input, &zero], &mut full, &mut [&mut aux], &changes)
            .unwrap();
        let mut split = [0.; 128];
        let mut tmp = [0.; 32];
        for n in 0..4 {
            let events = changes
                .iter()
                .filter(|e| e.frame >= n * 16 && e.frame < (n + 1) * 16)
                .map(|e| Change {
                    frame: e.frame - n * 16,
                    ..*e
                })
                .collect::<Vec<_>>();
            b.process(
                &[&input[n * 32..(n + 1) * 32], &zero[n * 32..(n + 1) * 32]],
                &mut split[n * 32..(n + 1) * 32],
                &mut [&mut tmp],
                &events,
            )
            .unwrap();
        }
        assert_eq!(full, split);
        let before = a.snapshot().to_json().unwrap();
        assert!(a
            .process(
                &[&input, &zero],
                &mut full,
                &mut [&mut aux],
                &[Change {
                    frame: 65,
                    strip: 0,
                    value: Value::Gain(3.)
                }]
            )
            .is_err());
        assert!(full.iter().all(|v| *v == 0.));
        assert_eq!(before, a.snapshot().to_json().unwrap());
    }
    #[test]
    fn persistence_and_group_mute() {
        let mut model = model();
        model.groups.push(Group {
            gain: 0.5,
            mute: true,
        });
        model.strips[0].group = Some(0);
        let round = Model::from_json(&model.to_json().unwrap()).unwrap();
        let mut m = round.compile(48000., 1024).unwrap();
        let mut out = [0.; 2048];
        let mut aux = [0.; 2048];
        for _ in 0..8 {
            m.process(&[&[1.; 2048], &[0.; 2048]], &mut out, &mut [&mut aux], &[])
                .unwrap();
        }
        assert!(out[2047].abs() < 1e-6);
    }
}
