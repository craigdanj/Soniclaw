//! Single-voice region playback with reverse, wrap/ping-pong loops and overlap crossfades.
use crate::{resample::ResampleQuality, Sample};
use serde::{Deserialize, Serialize};
use soniclaw_core::*;
use std::sync::Arc;
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    Linear,
    SincFast,
    SincBest,
    Prepared,
}
impl Interpolation {
    fn quality(self) -> ResampleQuality {
        match self {
            Self::Linear => ResampleQuality::Linear,
            Self::SincFast => ResampleQuality::SincFast,
            Self::SincBest => ResampleQuality::SincBest,
            Self::Prepared => ResampleQuality::SincFast,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopMode {
    Wrap,
    PingPong,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Loop {
    pub start: usize,
    pub end: usize,
    pub mode: LoopMode,
    pub crossfade_frames: usize,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema: u32,
    pub start: usize,
    pub end: usize,
    pub reverse: bool,
    pub speed: f64,
    pub gain: f32,
    pub interpolation: Interpolation,
    pub loop_region: Option<Loop>,
}
impl Config {
    fn validate(self, s: &Sample) -> Result<(), Error> {
        if self.schema != 1
            || self.start >= self.end
            || self.end > s.frames()
            || s.frames() > 16_777_216
            || s.data().iter().any(|x| x.abs() > 16.)
            || !self.speed.is_finite()
            || !(0.125..=8.).contains(&self.speed)
            || !self.gain.is_finite()
            || !(0. ..=1.).contains(&self.gain)
        {
            return Err(Error::InvalidParameter);
        }
        if let Some(l) = self.loop_region {
            if l.start < self.start
                || l.end > self.end
                || l.start >= l.end
                || l.end - l.start < 16
                || l.crossfade_frames > (l.end - l.start) / 4
                || l.mode == LoopMode::PingPong && l.crossfade_frames != 0
            {
                return Err(Error::InvalidParameter);
            }
        }
        Ok(())
    }
}
pub struct RegionPlayer {
    sample: Arc<Sample>,
    tables: Option<Arc<crate::prepared_resample::PreparedSinc>>,
    config: Config,
    spec: Option<ProcessSpec>,
    position: f64,
    direction: f64,
    increment: f64,
    active: bool,
    released: bool,
    wrapped: bool,
}
impl RegionPlayer {
    pub fn new(sample: Arc<Sample>, config: Config) -> Result<Self, Error> {
        config.validate(&sample)?;
        Ok(Self {
            sample,
            tables: None,
            config,
            spec: None,
            position: 0.,
            direction: 1.,
            increment: 1.,
            active: false,
            released: false,
            wrapped: false,
        })
    }
    /// Control-thread constructor: share immutable coefficient tables across prepared voices.
    pub fn new_with_tables(
        sample: Arc<Sample>,
        config: Config,
        tables: Arc<crate::prepared_resample::PreparedSinc>,
    ) -> Result<Self, Error> {
        let mut p = Self::new(sample, config)?;
        p.tables = Some(tables);
        Ok(p)
    }
    fn maximum_increment(&self) -> f64 {
        if matches!(self.config.interpolation, Interpolation::Prepared) {
            8.
        } else {
            32.
        }
    }
    pub fn config(&self) -> Config {
        self.config
    }
    pub fn start(&mut self) -> Result<(), Error> {
        if self.spec.is_none() {
            return Err(Error::NotPrepared);
        }
        self.position = if self.config.reverse {
            (self.config.end - 1) as f64
        } else {
            self.config.start as f64
        };
        self.direction = if self.config.reverse { -1. } else { 1. };
        self.active = true;
        self.released = false;
        self.wrapped = false;
        Ok(())
    }
    pub fn release(&mut self) {
        self.released = true;
    }
    pub fn is_active(&self) -> bool {
        self.active
    }
    pub fn position(&self) -> f64 {
        self.position
    }
    pub fn set_speed(&mut self, speed: f64) -> Result<(), Error> {
        if !speed.is_finite()
            || !(0.125..=8.).contains(&speed)
            || self.spec.is_some_and(|s| {
                speed * self.sample.sample_rate() / s.sample_rate > self.maximum_increment()
            })
        {
            return Err(Error::InvalidParameter);
        }
        self.config.speed = speed;
        if let Some(s) = self.spec {
            self.increment = speed * self.sample.sample_rate() / s.sample_rate;
        }
        Ok(())
    }
    pub fn seek(&mut self, frame: f64) -> Result<(), Error> {
        if !frame.is_finite() || frame < self.config.start as f64 || frame >= self.config.end as f64
        {
            return Err(Error::InvalidParameter);
        }
        self.position = frame;
        self.wrapped = false;
        Ok(())
    }
    fn read(&self, position: f64, channel: usize) -> f32 {
        let looping = self.config.loop_region.filter(|_| !self.released);
        let quality = self.config.interpolation.quality();
        let read = |mut index: isize| {
            if let Some(l) = looping {
                let lo = l.start as isize;
                let hi = l.end as isize;
                let inside = self.position >= l.start as f64 && self.position < l.end as f64;
                if inside {
                    match l.mode {
                        LoopMode::Wrap => {
                            if index >= hi || index < lo && self.wrapped {
                                index = lo + (index - lo).rem_euclid(hi - lo);
                            }
                        }
                        LoopMode::PingPong => {
                            let span = hi - lo - 1;
                            let phase = (index - lo).rem_euclid(2 * span);
                            index = lo
                                + if phase <= span {
                                    phase
                                } else {
                                    2 * span - phase
                                };
                        }
                    }
                }
            }
            if index < self.config.start as isize || index >= self.config.end as isize {
                0.
            } else {
                self.sample.data()[index as usize * self.sample.channels()
                    + channel.min(self.sample.channels() - 1)]
            }
        };
        if matches!(self.config.interpolation, Interpolation::Prepared) {
            self.tables.as_ref().expect("prepared tables").interpolate(
                position,
                self.increment,
                read,
            )
        } else {
            quality.interpolate(position, self.increment, read)
        }
    }
    pub fn next_frame(&mut self, frame: &mut [f32]) {
        if !self.active || self.spec.is_none_or(|s| frame.len() != s.channels) {
            frame.fill(0.);
            return;
        }
        let looping = self.config.loop_region.filter(|_| !self.released);
        for (ch, x) in frame.iter_mut().enumerate() {
            let mut value = self.read(self.position, ch);
            if let Some(l) = looping.filter(|l| l.mode == LoopMode::Wrap && l.crossfade_frames > 0)
            {
                let n = l.crossfade_frames as f64;
                let overlap = if self.direction > 0.
                    && self.position >= l.end as f64 - n
                    && self.position < l.end as f64
                {
                    Some((
                        l.start as f64 + self.position - (l.end as f64 - n),
                        (self.position - (l.end as f64 - n)) / n,
                    ))
                } else if self.direction < 0.
                    && self.position < l.start as f64 + n
                    && self.position >= l.start as f64
                {
                    Some((
                        l.end as f64 - n + self.position - l.start as f64,
                        (l.start as f64 + n - self.position) / n,
                    ))
                } else {
                    None
                };
                if let Some((position, t)) = overlap {
                    value = value * (1. - t as f32) + self.read(position, ch) * t as f32;
                }
            }
            *x = value * self.config.gain;
        }
        self.position += self.direction * self.increment;
        if let Some(l) = looping {
            let lo = l.start as f64;
            let hi = l.end as f64;
            match l.mode {
                LoopMode::Wrap => {
                    let n = l.crossfade_frames as f64;
                    let period = hi - lo - n;
                    if self.direction > 0. && self.position >= hi {
                        self.position = lo + n + (self.position - hi).rem_euclid(period);
                        self.wrapped = true;
                    } else if self.direction < 0. && self.position < lo {
                        self.position = lo + (self.position - lo).rem_euclid(period);
                        self.wrapped = true;
                    }
                }
                LoopMode::PingPong => {
                    let high = hi - 1.;
                    if self.position > high && self.direction > 0.
                        || self.position < lo && self.direction < 0.
                    {
                        let span = high - lo;
                        let phase = (self.position - lo).rem_euclid(2. * span);
                        let direction = self.direction;
                        self.position = lo
                            + if phase <= span {
                                phase
                            } else {
                                2. * span - phase
                            };
                        self.direction = if phase < span { direction } else { -direction };
                        self.wrapped = true;
                    }
                }
            }
        }
        if self.position < self.config.start as f64 || self.position >= self.config.end as f64 {
            self.active = false;
        }
    }
}
impl AudioProcessor for RegionPlayer {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        let increment = self.config.speed * self.sample.sample_rate() / s.sample_rate;
        if !(8000. ..=192000.).contains(&s.sample_rate)
            || s.channels > 2
            || s.channels < self.sample.channels()
            || s.max_block_frames > 8192
            || increment <= 0.
            || increment > self.maximum_increment()
            || increment == 0.
        {
            return Err(Error::InvalidSpec);
        }
        if matches!(self.config.interpolation, Interpolation::Prepared) && self.tables.is_none() {
            self.tables = Some(Arc::new(crate::prepared_resample::PreparedSinc::new(
                Default::default(),
            )?));
        }
        self.spec = Some(s);
        self.increment = increment;
        self.reset();
        Ok(())
    }
    fn reset(&mut self) {
        self.active = false;
        self.position = self.config.start as f64;
        self.released = false;
        self.wrapped = false;
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let Some(s) = self.spec else {
            b.samples_mut().fill(0.);
            return;
        };
        if b.channels() != s.channels || b.frames() > s.max_block_frames {
            b.samples_mut().fill(0.);
            return;
        }
        for f in b.samples_mut().chunks_exact_mut(s.channels) {
            self.next_frame(f);
        }
    }
}
/// Offline bounded overlap matcher: scores 64-sample tails against the loop start.
/// radius <=256, channel valid, start+64<=ideal_end, all candidates inside PCM.
pub fn match_loop_end(
    sample: &Sample,
    start: usize,
    ideal_end: usize,
    radius: usize,
    channel: usize,
) -> Result<usize, Error> {
    if radius > 256
        || channel >= sample.channels()
        || ideal_end > sample.frames()
        || start
            .checked_add(64)
            .is_none_or(|x| x > ideal_end.saturating_sub(radius))
        || ideal_end + radius > sample.frames()
    {
        return Err(Error::InvalidParameter);
    }
    let mut best = ideal_end;
    let mut error = f64::INFINITY;
    for end in ideal_end - radius..=ideal_end + radius {
        let score = (0..64)
            .map(|i| {
                let a = sample.data()[(start + i) * sample.channels() + channel];
                let b = sample.data()[(end - 64 + i) * sample.channels() + channel];
                (a as f64 - b as f64).powi(2)
            })
            .sum::<f64>();
        if score < error {
            error = score;
            best = end;
        }
    }
    Ok(best)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config {
        Config {
            schema: 1,
            start: 0,
            end: 128,
            reverse: false,
            speed: 1.,
            gain: 1.,
            interpolation: Interpolation::Linear,
            loop_region: None,
        }
    }
    fn player(config: Config) -> RegionPlayer {
        let sample = Arc::new(
            Sample::new(
                (0..128).map(|i| i as f32 / 128.).collect::<Vec<_>>(),
                1,
                48000.,
                60,
            )
            .unwrap(),
        );
        let mut p = RegionPlayer::new(sample, config).unwrap();
        p.prepare(ProcessSpec {
            sample_rate: 48000.,
            max_block_frames: 128,
            channels: 1,
        })
        .unwrap();
        p.start().unwrap();
        p
    }
    #[test]
    fn prepared_tables_are_shared_and_rate_is_guarded() {
        let sample = Arc::new(Sample::new(vec![0.2; 128], 1, 96000., 60).unwrap());
        let tables =
            Arc::new(crate::prepared_resample::PreparedSinc::new(Default::default()).unwrap());
        let mut p = RegionPlayer::new_with_tables(
            sample,
            Config {
                interpolation: Interpolation::Prepared,
                loop_region: Some(Loop {
                    start: 0,
                    end: 128,
                    mode: LoopMode::Wrap,
                    crossfade_frames: 0,
                }),
                ..config()
            },
            tables.clone(),
        )
        .unwrap();
        p.prepare(ProcessSpec {
            sample_rate: 48000.,
            max_block_frames: 128,
            channels: 1,
        })
        .unwrap();
        assert_eq!(Arc::strong_count(&tables), 2);
        p.start().unwrap();
        assert!(p.set_speed(8.).is_err());
        assert_eq!(p.config().speed, 1.);
        for _ in 0..256 {
            assert_eq!(
                allocation_counter::measure(|| p.next_frame(&mut [0.])).count_total,
                0
            );
        }
        p.reset();
        assert_eq!(Arc::strong_count(&tables), 2);
    }
    #[test]
    fn loop_search_finds_period_and_rejects_outside_windows() {
        let sample = Sample::new(
            (0..256)
                .map(|i| ((i % 64) as f32 * std::f32::consts::TAU / 64.).sin())
                .collect::<Vec<_>>(),
            1,
            48000.,
            60,
        )
        .unwrap();
        assert_eq!(match_loop_end(&sample, 0, 132, 8, 0).unwrap(), 128);
        assert!(match_loop_end(&sample, 0, 20, 8, 0).is_err());
        assert!(match_loop_end(&sample, usize::MAX, 132, 8, 0).is_err());
        assert!(match_loop_end(&sample, 0, 250, 8, 0).is_err());
    }
    #[test]
    fn high_rate_small_loop_stays_inside() {
        for mode in [LoopMode::Wrap, LoopMode::PingPong] {
            for reverse in [false, true] {
                let mut p = player(Config {
                    start: 0,
                    end: 16,
                    reverse,
                    speed: 8.,
                    loop_region: Some(Loop {
                        start: 0,
                        end: 16,
                        mode,
                        crossfade_frames: 0,
                    }),
                    ..config()
                });
                p.prepare(ProcessSpec {
                    sample_rate: 12000.,
                    max_block_frames: 128,
                    channels: 1,
                })
                .unwrap();
                p.start().unwrap();
                for _ in 0..1024 {
                    p.next_frame(&mut [0.]);
                    assert!(p.is_active());
                    assert!((0. ..16.).contains(&p.position()));
                }
            }
        }
    }
    #[test]
    fn reverse_and_pingpong_endpoints() {
        let mut p = player(Config {
            reverse: true,
            ..config()
        });
        let mut f = [0.];
        p.next_frame(&mut f);
        assert_eq!(f[0], 127. / 128.);
        let mut p = player(Config {
            loop_region: Some(Loop {
                start: 0,
                end: 16,
                mode: LoopMode::PingPong,
                crossfade_frames: 0,
            }),
            ..config()
        });
        let mut values = vec![];
        for _ in 0..34 {
            p.next_frame(&mut f);
            values.push(f[0] * 128.);
        }
        assert_eq!(&values[13..19], &[13., 14., 15., 14., 13., 12.]);
        assert_eq!(&values[29..33], &[1., 0., 1., 2.]);
        p.release();
        for _ in 0..128 {
            p.next_frame(&mut f);
        }
        assert!(!p.is_active());
    }
    #[test]
    fn overlap_skip_and_callback_partition() {
        let render = |block: usize| {
            let mut p = player(Config {
                interpolation: Interpolation::SincFast,
                speed: 1.2,
                loop_region: Some(Loop {
                    start: 16,
                    end: 112,
                    mode: LoopMode::Wrap,
                    crossfade_frames: 16,
                }),
                ..config()
            });
            let mut data = vec![0.; 1024];
            for b in data.chunks_mut(block) {
                assert_eq!(
                    allocation_counter::measure(|| p.process(
                        &mut AudioBuffer::new(b, 1).unwrap(),
                        &ProcessContext {
                            events: &[],
                            transport: Transport::default()
                        }
                    ))
                    .count_total,
                    0
                );
            }
            assert!(p.is_active());
            assert!(data.iter().all(|x| x.is_finite()));
            assert!(p.set_speed(f64::NAN).is_err());
            data
        };
        assert_eq!(render(128), render(1));
        let mut p = player(Config {
            loop_region: Some(Loop {
                start: 0,
                end: 128,
                mode: LoopMode::Wrap,
                crossfade_frames: 16,
            }),
            ..config()
        });
        for _ in 0..128 {
            p.next_frame(&mut [0.]);
        }
        assert_eq!(p.position(), 16.);
    }
}
