//! Continuous offline processing and aligned WAV stem export. Blocking control/worker APIs.
use crate::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub schema: u32,
    pub sample_rate: f64,
    pub channels: usize,
    pub frames: usize,
    pub tail_frames: usize,
    pub block_frames: usize,
    pub tempo: f64,
    pub compensate_latency: bool,
}
impl Default for Plan {
    fn default() -> Self {
        Self {
            schema: 1,
            sample_rate: 48000.,
            channels: 2,
            frames: 48000,
            tail_frames: 0,
            block_frames: 256,
            tempo: 120.,
            compensate_latency: true,
        }
    }
}
#[derive(Debug)]
pub enum RenderError {
    InvalidPlan,
    InvalidInput,
    EventDensity,
    Cancelled,
    Processor(Error),
    Io(String),
}
impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for RenderError {}
impl From<Error> for RenderError {
    fn from(e: Error) -> Self {
        Self::Processor(e)
    }
}
pub type Result<T> = std::result::Result<T, RenderError>;
impl Plan {
    pub fn validate(&self) -> Result<ProcessSpec> {
        let spec = ProcessSpec {
            sample_rate: self.sample_rate,
            channels: self.channels,
            max_block_frames: self.block_frames,
        };
        spec.validate()?;
        if self.schema != 1
            || self.frames == 0
            || self.block_frames > 8192
            || !self.tempo.is_finite()
            || !(1.0..=1000.0).contains(&self.tempo)
            || self
                .frames
                .checked_add(self.tail_frames)
                .and_then(|n| n.checked_mul(self.channels))
                .is_none_or(|n| n > 64 * 1024 * 1024)
        {
            return Err(RenderError::InvalidPlan);
        }
        Ok(spec)
    }
}
pub struct Rendered {
    pub samples: Vec<f32>,
    pub frames: usize,
    pub sample_rate: f64,
    pub channels: usize,
    pub trimmed_latency: usize,
    pub blocks: usize,
}
impl Rendered {
    pub fn write_wav(&self, path: &Path) -> Result<()> {
        crate::io::write_wav(
            path,
            self.sample_rate.round() as u32,
            u16::try_from(self.channels).map_err(|_| RenderError::InvalidPlan)?,
            &self.samples,
        )
        .map_err(|e| RenderError::Io(e.to_string()))
    }
}
pub fn render(
    processor: &mut dyn AudioProcessor,
    plan: &Plan,
    input: &[f32],
    events: &[Event],
) -> Result<Rendered> {
    render_cancellable(processor, plan, input, events, &AtomicBool::new(false))
}
pub fn render_cancellable(
    processor: &mut dyn AudioProcessor,
    plan: &Plan,
    input: &[f32],
    events: &[Event],
    cancel: &AtomicBool,
) -> Result<Rendered> {
    let spec = plan.validate()?;
    if !input.is_empty() && input.len() != plan.frames * plan.channels {
        return Err(RenderError::InvalidInput);
    }
    if events.len() > 1_048_576 || crate::core::validate_events(events, plan.frames).is_err() {
        return Err(RenderError::InvalidInput);
    }
    processor.prepare(spec)?;
    let latency = if plan.compensate_latency {
        processor.latency_frames()
    } else {
        0
    };
    if latency > 1_048_576 {
        return Err(RenderError::InvalidPlan);
    }
    let frames = plan.frames + plan.tail_frames;
    let total = frames
        .checked_add(latency)
        .ok_or(RenderError::InvalidPlan)?;
    if total
        .checked_mul(plan.channels)
        .is_none_or(|n| n > 64 * 1024 * 1024)
    {
        return Err(RenderError::InvalidPlan);
    }
    let mut output = vec![0.; total * plan.channels];
    let mut scratch = vec![0.; plan.block_frames * plan.channels];
    let mut staged = [Event {
        frame: 0,
        message: MidiMessage::AllNotesOff,
    }; 1024];
    let mut event = 0;
    let mut cursor = 0;
    let mut blocks = 0;
    while cursor < total {
        if cancel.load(Ordering::Acquire) {
            return Err(RenderError::Cancelled);
        }
        let count = plan.block_frames.min(total - cursor);
        let data = &mut scratch[..count * plan.channels];
        data.fill(0.);
        if !input.is_empty() && cursor < plan.frames {
            let n = count.min(plan.frames - cursor) * plan.channels;
            data[..n].copy_from_slice(&input[cursor * plan.channels..cursor * plan.channels + n]);
        }
        let mut used = 0;
        while event < events.len() && events[event].frame < cursor + count {
            if used == staged.len() {
                return Err(RenderError::EventDensity);
            }
            staged[used] = Event {
                frame: events[event].frame - cursor,
                message: events[event].message,
            };
            used += 1;
            event += 1;
        }
        let context = ProcessContext {
            events: &staged[..used],
            transport: Transport {
                sample_position: cursor as u64,
                tempo: plan.tempo,
                playing: true,
            },
        };
        processor.process(&mut AudioBuffer::new(data, plan.channels)?, &context);
        if data.iter().any(|x| !x.is_finite()) {
            return Err(RenderError::Processor(Error::InvalidBuffer));
        }
        output[cursor * plan.channels..(cursor + count) * plan.channels].copy_from_slice(data);
        cursor += count;
        blocks += 1;
    }
    if latency > 0 {
        output.copy_within(latency * plan.channels.., 0);
        output.truncate(frames * plan.channels);
    }
    Ok(Rendered {
        samples: output,
        frames,
        sample_rate: plan.sample_rate,
        channels: plan.channels,
        trimmed_latency: latency,
        blocks,
    })
}
pub struct Stem<'a> {
    pub name: &'a str,
    pub processor: &'a mut dyn AudioProcessor,
    pub input: &'a [f32],
    pub events: &'a [Event],
}
/// New-directory transaction: incomplete exports remain in a temporary sibling and are removed on error.
pub fn export_stems(
    stems: &mut [Stem<'_>],
    plan: &Plan,
    destination: &Path,
) -> Result<Vec<PathBuf>> {
    plan.validate()?;
    if destination.exists() || stems.is_empty() || stems.len() > 128 {
        return Err(RenderError::InvalidInput);
    }
    let mut names = std::collections::BTreeSet::new();
    for stem in stems.iter() {
        if stem.name.is_empty()
            || stem.name.len() > 64
            || !stem
                .name
                .bytes()
                .all(|x| x.is_ascii_alphanumeric() || b"._-".contains(&x))
            || !names.insert(stem.name.to_ascii_lowercase())
        {
            return Err(RenderError::InvalidInput);
        }
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| RenderError::Io(e.to_string()))?
        .as_nanos();
    let temporary = destination.with_file_name(format!(".stems-{}-{stamp}", std::process::id()));
    std::fs::create_dir(&temporary).map_err(|e| RenderError::Io(e.to_string()))?;
    let result = (|| {
        let mut entries = vec![];
        for stem in stems.iter_mut() {
            let rendered = render(stem.processor, plan, stem.input, stem.events)?;
            let filename = format!("{}.wav", stem.name);
            rendered.write_wav(&temporary.join(&filename))?;
            entries.push(serde_json::json!({"name":stem.name,"file":filename,"frames":rendered.frames,"trimmed_latency":rendered.trimmed_latency}));
        }
        let json = serde_json::json!({"schema":1,"plan":plan,"stems":entries});
        std::fs::write(
            temporary.join("stems.json"),
            serde_json::to_vec_pretty(&json).map_err(|e| RenderError::Io(e.to_string()))?,
        )
        .map_err(|e| RenderError::Io(e.to_string()))?;
        std::fs::rename(&temporary, destination).map_err(|e| RenderError::Io(e.to_string()))?;
        Ok(stems
            .iter()
            .map(|s| destination.join(format!("{}.wav", s.name)))
            .collect())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&temporary);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Counter {
        value: f32,
    }
    impl AudioProcessor for Counter {
        fn prepare(&mut self, _: ProcessSpec) -> std::result::Result<(), Error> {
            self.value = 0.;
            Ok(())
        }
        fn reset(&mut self) {
            self.value = 0.;
        }
        fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
            for f in b.samples_mut().as_chunks_mut::<2>().0.iter_mut() {
                self.value += 1.;
                f.fill(self.value);
            }
        }
    }
    #[test]
    fn continuous_state_tail_and_partition_invariance() {
        let mut p = Counter { value: 0. };
        let mut plan = Plan {
            frames: 1000,
            tail_frames: 19,
            block_frames: 64,
            ..Default::default()
        };
        let a = render(&mut p, &plan, &[], &[]).unwrap();
        plan.block_frames = 257;
        let b = render(&mut p, &plan, &[], &[]).unwrap();
        assert_eq!(a.samples, b.samples);
        assert_eq!(a.samples.last(), Some(&1019.));
        assert_eq!(a.frames, 1019);
    }
    #[test]
    fn cancellation_and_invalid_input_are_explicit() {
        let mut p = Counter { value: 0. };
        assert!(matches!(
            render_cancellable(&mut p, &Plan::default(), &[], &[], &AtomicBool::new(true)),
            Err(RenderError::Cancelled)
        ));
        assert!(render(&mut p, &Plan::default(), &[1.], &[]).is_err());
    }
}

#[cfg(test)]
mod export_tests {
    use super::*;
    struct Delay {
        data: [f32; 6],
        cursor: usize,
    }
    impl AudioProcessor for Delay {
        fn prepare(&mut self, _: ProcessSpec) -> std::result::Result<(), Error> {
            self.data.fill(0.);
            self.cursor = 0;
            Ok(())
        }
        fn reset(&mut self) {
            self.data.fill(0.);
            self.cursor = 0;
        }
        fn latency_frames(&self) -> usize {
            3
        }
        fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
            for x in b.samples_mut() {
                std::mem::swap(x, &mut self.data[self.cursor]);
                self.cursor = (self.cursor + 1) % 6;
            }
        }
    }
    #[test]
    fn declared_latency_is_trimmed_without_losing_the_end() {
        let mut p = Delay {
            data: [0.; 6],
            cursor: 0,
        };
        let input = [1., -1., 2., -2., 3., -3., 4., -4.];
        let rendered = render(
            &mut p,
            &Plan {
                frames: 4,
                block_frames: 2,
                ..Default::default()
            },
            &input,
            &[],
        )
        .unwrap();
        assert_eq!(rendered.samples, input);
        assert_eq!(rendered.trimmed_latency, 3);
    }
    struct Bad;
    impl AudioProcessor for Bad {
        fn prepare(&mut self, _: ProcessSpec) -> std::result::Result<(), Error> {
            Ok(())
        }
        fn reset(&mut self) {}
        fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
            b.samples_mut().fill(f32::NAN);
        }
    }
    #[test]
    fn failed_stem_export_does_not_commit_partial_files() {
        let parent = std::env::temp_dir().join(format!("soniclaw-stems-{}", std::process::id()));
        std::fs::create_dir_all(&parent).unwrap();
        let destination = parent.join("export");
        let mut first = Delay {
            data: [0.; 6],
            cursor: 0,
        };
        let mut second = Bad;
        let mut stems = [
            Stem {
                name: "first",
                processor: &mut first,
                input: &[],
                events: &[],
            },
            Stem {
                name: "second",
                processor: &mut second,
                input: &[],
                events: &[],
            },
        ];
        assert!(export_stems(
            &mut stems,
            &Plan {
                frames: 4,
                block_frames: 2,
                ..Default::default()
            },
            &destination
        )
        .is_err());
        assert!(!destination.exists());
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 0);
        std::fs::remove_dir(parent).unwrap();
    }
}
