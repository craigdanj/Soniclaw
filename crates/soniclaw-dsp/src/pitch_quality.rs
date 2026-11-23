//! Explicit quality presets for the prepared streaming algorithms.
use crate::streaming::{
    FormantSettings, PitchSettings, PitchShifter, StretchSettings, TimeStretcher,
};
use serde::{Deserialize, Serialize};
use soniclaw_core::Error;
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    LowLatency,
    Music,
    Voice,
}
impl Quality {
    pub fn pitch(self, semitones: f32) -> Result<PitchShifter, Error> {
        let mut p = PitchShifter::new(
            semitones,
            PitchSettings {
                window_frames: match self {
                    Self::LowLatency => 512,
                    Self::Music => 2048,
                    Self::Voice => 4096,
                },
                ..Default::default()
            },
        )?;
        if matches!(self, Self::Voice) {
            p.set_formants(Some(FormantSettings::default()))?;
        }
        Ok(p)
    }
    pub fn stretch(
        self,
        ratio: f64,
        channels: usize,
        maximum: usize,
    ) -> Result<TimeStretcher, Error> {
        let window = match self {
            Self::LowLatency => 512,
            Self::Music => 1024,
            Self::Voice => 2048,
        };
        TimeStretcher::new(StretchSettings {
            ratio,
            channels,
            window_frames: window,
            search_frames: window / 8,
            max_block_frames: maximum,
            transient_ratio: Some(if matches!(self, Self::LowLatency) {
                2.
            } else {
                4.
            }),
            ..Default::default()
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use soniclaw_core::*;
    #[test]
    fn unity_is_unchanged_and_controls_validate() {
        let sp = ProcessSpec {
            sample_rate: 48000.,
            max_block_frames: 127,
            channels: 1,
        };
        let render = |enabled| {
            let mut p = Quality::LowLatency.pitch(0.).unwrap();
            if enabled {
                p.set_formants(Some(FormantSettings::default())).unwrap();
            }
            p.prepare(sp).unwrap();
            let mut pcm: Vec<f32> = (0..4096).map(|i| 0.1 * (i as f32 * 0.1).sin()).collect();
            for b in pcm.chunks_mut(127) {
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
            pcm
        };
        assert_eq!(render(false), render(true));
        let mut p = Quality::Voice.pitch(7.).unwrap();
        assert!(p
            .set_formants(Some(FormantSettings {
                amount: f32::NAN,
                ..Default::default()
            }))
            .is_err());
        assert!(p.formants().is_some());
    }
    #[test]
    fn formant_shift_is_finite_and_partition_invariant() {
        let render = |block: usize| {
            let mut p = Quality::LowLatency.pitch(5.).unwrap();
            p.set_formants(Some(FormantSettings {
                shift_semitones: -2.,
                ..Default::default()
            }))
            .unwrap();
            p.prepare(ProcessSpec {
                sample_rate: 48000.,
                max_block_frames: 128,
                channels: 1,
            })
            .unwrap();
            let mut data: Vec<f32> = (0..8192)
                .map(|i| 0.2 * (i as f32 * 0.051).sin() + 0.1 * (i as f32 * 0.07).sin())
                .collect();
            for b in data.chunks_mut(block) {
                p.process(
                    &mut AudioBuffer::new(b, 1).unwrap(),
                    &ProcessContext {
                        events: &[],
                        transport: Transport::default(),
                    },
                );
            }
            assert!(data.iter().all(|x| x.is_finite()));
            data
        };
        assert_eq!(render(128), render(1));
    }
}
