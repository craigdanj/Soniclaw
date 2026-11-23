//! Development-only stress audits and single-writer callback telemetry.
use serde::{Deserialize, Serialize};
use soniclaw_core::*;
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};
#[derive(Default)]
pub struct CallbackStats {
    blocks: AtomicU64,
    frames: AtomicU64,
    overruns: AtomicU64,
    max_ns: AtomicU64,
    nonfinite: AtomicU64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StatsSnapshot {
    pub blocks: u64,
    pub frames: u64,
    pub overruns: u64,
    pub max_ns: u64,
    pub nonfinite: u64,
}
impl CallbackStats {
    pub fn record(&self, frames: usize, rate: f64, ns: u64, bad: usize) {
        self.blocks.fetch_add(1, Ordering::Relaxed);
        self.frames.fetch_add(frames as u64, Ordering::Relaxed);
        self.max_ns.fetch_max(ns, Ordering::Relaxed);
        self.nonfinite.fetch_add(bad as u64, Ordering::Relaxed);
        if rate.is_finite() && rate > 0.0 && ns as f64 > frames as f64 / rate * 1e9 {
            self.overruns.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            blocks: self.blocks.load(Ordering::Relaxed),
            frames: self.frames.load(Ordering::Relaxed),
            overruns: self.overruns.load(Ordering::Relaxed),
            max_ns: self.max_ns.load(Ordering::Relaxed),
            nonfinite: self.nonfinite.load(Ordering::Relaxed),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditConfig {
    pub sample_rates: Vec<f64>,
    pub block_sizes: Vec<usize>,
    pub frames: usize,
    pub channels: usize,
}
impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            sample_rates: vec![44100.0, 48000.0, 96000.0],
            block_sizes: vec![1, 17, 64, 257],
            frames: 4096,
            channels: 2,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditCase {
    pub rate: f64,
    pub max_block: usize,
    pub latency: usize,
    pub stats: StatsSnapshot,
    pub peak: f32,
    pub allocations: Option<u64>,
    pub allocation_balance: Option<i64>,
    pub max_block_ratio: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditReport {
    pub cases: Vec<AuditCase>,
    pub finite: bool,
    pub allocation_clean: Option<bool>,
}
pub fn audit(
    mut factory: impl FnMut() -> Result<Box<dyn AudioProcessor>, Error>,
    c: &AuditConfig,
) -> Result<AuditReport, Error> {
    if c.sample_rates.is_empty()
        || c.sample_rates.len() > 16
        || c.block_sizes.is_empty()
        || c.block_sizes.len() > 16
        || !(1..=2).contains(&c.channels)
        || !(1024..=1_048_576).contains(&c.frames)
        || c.block_sizes.iter().any(|n| !(1..=8192).contains(n))
    {
        return Err(Error::InvalidSpec);
    }
    let mut cases = vec![];
    for &rate in &c.sample_rates {
        for &size in &c.block_sizes {
            let mut p = factory()?;
            p.prepare(ProcessSpec {
                sample_rate: rate,
                max_block_frames: size,
                channels: c.channels,
            })?;
            let mut data = vec![0.0; size * c.channels];
            let stats = CallbackStats::default();
            let mut peak = 0.0_f32;
            let mut worst = 0.0_f64;
            let mut run = || {
                let (mut at, mut block) = (0, 0);
                while at < c.frames {
                    let count =
                        (if block % 3 == 1 { size / 2 + 1 } else { size }).min(c.frames - at);
                    let samples = &mut data[..count * c.channels];
                    for (i, f) in samples.chunks_exact_mut(c.channels).enumerate() {
                        f.fill(
                            0.1 * (std::f32::consts::TAU * 997.0 * (at + i) as f32 / rate as f32)
                                .sin()
                                + 1e-30,
                        );
                    }
                    let events = [Event {
                        frame: 0,
                        message: if at == 0 {
                            MidiMessage::NoteOnId {
                                channel: 1,
                                note: 60,
                                velocity: 0.8,
                                note_id: 71,
                            }
                        } else {
                            MidiMessage::AllNotesOff
                        },
                    }];
                    let start = Instant::now();
                    p.process(
                        &mut AudioBuffer::new(samples, c.channels).unwrap(),
                        &ProcessContext {
                            events: if at == 0 || at + count == c.frames {
                                &events
                            } else {
                                &[]
                            },
                            transport: Transport {
                                sample_position: at as u64,
                                tempo: 120.0 + block as f64 % 7.0,
                                playing: true,
                            },
                        },
                    );
                    let ns = start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
                    let bad = samples.iter().filter(|x| !x.is_finite()).count();
                    for x in samples.iter() {
                        if x.is_finite() {
                            peak = peak.max(x.abs());
                        }
                    }
                    stats.record(count, rate, ns, bad);
                    worst = worst.max(ns as f64 * rate / (count as f64 * 1e9));
                    at += count;
                    block += 1;
                }
                p.reset();
            };
            #[cfg(feature = "allocations")]
            let (allocations, balance) = {
                let a = allocation_counter::measure(&mut run);
                (Some(a.count_total), Some(a.count_current as i64))
            };
            #[cfg(not(feature = "allocations"))]
            let (allocations, balance) = {
                run();
                (None, None)
            };
            cases.push(AuditCase {
                rate,
                max_block: size,
                latency: p.latency_frames(),
                stats: stats.snapshot(),
                peak,
                allocations,
                allocation_balance: balance,
                max_block_ratio: worst,
            });
        }
    }
    let finite = cases.iter().all(|c| c.stats.nonfinite == 0);
    let clean = cases
        .iter()
        .all(|c| c.allocations == Some(0) && c.allocation_balance == Some(0));
    Ok(AuditReport {
        cases,
        finite,
        allocation_clean: if cfg!(feature = "allocations") {
            Some(clean)
        } else {
            None
        },
    })
}
pub fn audit_state(
    p: &mut soniclaw_plugin::PluginInstance,
) -> Result<(), Box<dyn std::error::Error>> {
    let state = p.save_state()?;
    let ids: Vec<_> = p.parameters().iter().map(std::sync::Arc::as_ptr).collect();
    p.load_state(&state)?;
    if ids
        != p.parameters()
            .iter()
            .map(std::sync::Arc::as_ptr)
            .collect::<Vec<_>>()
    {
        return Err("parameter identities changed".into());
    }
    let before = p.save_state()?;
    for invalid in ["", "{", "null", r#"{"schema_version":999}"#] {
        if p.load_state(invalid).is_ok() || p.save_state()? != before {
            return Err("invalid state changed current state".into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Bad;
    impl AudioProcessor for Bad {
        fn prepare(&mut self, _: ProcessSpec) -> Result<(), Error> {
            Ok(())
        }
        fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
            b.samples_mut().fill(f32::NAN)
        }
        fn reset(&mut self) {}
    }
    #[test]
    fn detects_nonfinite_across_variable_blocks() {
        let r = audit(
            || Ok(Box::new(Bad)),
            &AuditConfig {
                sample_rates: vec![48000.0],
                block_sizes: vec![17],
                frames: 1024,
                channels: 2,
            },
        )
        .unwrap();
        assert!(!r.finite);
        assert_eq!(r.cases[0].stats.nonfinite, 2048);
        assert!(r.cases[0].stats.blocks > 60);
    }
}
