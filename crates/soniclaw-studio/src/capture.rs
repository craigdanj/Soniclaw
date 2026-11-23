//! Callback-only capture staging. Atomic samples keep sharing safe without callback mutexes.
use super::*;
use soniclaw_record::multitrack::{BoundaryKind, LoopBoundary, Tap, TrackSpec};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RecordingSource {
    Track(String),
    Master,
    Input,
    InputChannels(Vec<u16>),
}
pub(crate) struct CaptureStore {
    pub ids: Vec<String>,
    pub input: Option<Arc<ChannelBus>>,
    data: Vec<Vec<AtomicU32>>,
    offset: AtomicUsize,
    positions: Vec<AtomicU64>,
    clock_flags: Vec<std::sync::atomic::AtomicU8>,
    pub enabled: AtomicBool,
    pub clock_enabled: AtomicBool,
    pub track_latency: usize,
}
impl CaptureStore {
    pub fn new(
        tracks: &[TrackRuntime],
        ids: Vec<String>,
        maximum: usize,
        track_latency: usize,
        input: Option<Arc<ChannelBus>>,
    ) -> Self {
        Self {
            input,
            ids,
            data: tracks
                .iter()
                .map(|_| (0..maximum * 2).map(|_| AtomicU32::new(0)).collect())
                .collect(),
            offset: AtomicUsize::new(0),
            positions: (0..maximum).map(|_| AtomicU64::new(0)).collect(),
            clock_flags: (0..maximum)
                .map(|_| std::sync::atomic::AtomicU8::new(0))
                .collect(),
            enabled: AtomicBool::new(false),
            clock_enabled: AtomicBool::new(false),
            track_latency,
        }
    }
    pub fn input_offset(&self) -> usize {
        self.offset.load(Ordering::Relaxed) / 2
    }
    pub fn begin(&self) {
        self.offset.store(0, Ordering::Relaxed);
    }
    pub fn clock(
        &self,
        offset: usize,
        count: usize,
        source: u64,
        playing: bool,
        looped: bool,
        at_start: bool,
    ) {
        if !self.enabled.load(Ordering::Relaxed) && !self.clock_enabled.load(Ordering::Relaxed) {
            return;
        }
        for f in 0..count {
            self.positions[offset + f].store(
                source + if playing { f as u64 } else { 0 },
                Ordering::Relaxed,
            );
            self.clock_flags[offset + f].store(
                if !playing {
                    0
                } else if f == 0 && looped {
                    2
                } else if f == 0 && at_start {
                    3
                } else {
                    1
                },
                Ordering::Relaxed,
            );
        }
    }
    pub fn write(&self, tracks: &[TrackRuntime], samples: usize) {
        if !self.enabled.load(Ordering::Relaxed) {
            self.offset.fetch_add(samples, Ordering::Release);
            return;
        }
        let offset = self.offset.load(Ordering::Relaxed);
        for (source, destination) in tracks.iter().zip(&self.data) {
            if offset + samples > destination.len() {
                return;
            }
            for (s, d) in source.pcm[..samples]
                .iter()
                .zip(&destination[offset..offset + samples])
            {
                d.store(s.to_bits(), Ordering::Relaxed);
            }
        }
        self.offset.store(offset + samples, Ordering::Release);
    }
    pub(crate) fn read(&self, index: usize, destination: &mut [f32]) {
        for (d, s) in destination.iter_mut().zip(&self.data[index]) {
            *d = f32::from_bits(s.load(Ordering::Relaxed));
        }
    }
    pub(crate) fn clock_at(&self, frame: usize) -> Option<(u64, bool, bool)> {
        if frame * 2 >= self.offset.load(Ordering::Acquire) {
            return None;
        }
        let flag = self.clock_flags[frame].load(Ordering::Relaxed);
        Some((
            self.positions[frame].load(Ordering::Relaxed),
            flag != 0,
            flag == 2,
        ))
    }
    pub fn specs(
        &self,
        sources: &[(String, RecordingSource)],
        budget: usize,
    ) -> Result<Vec<TrackSpec>, Error> {
        sources
            .iter()
            .map(|(id, source)| {
                let latency_frames = match source {
                    RecordingSource::Track(id) => {
                        if !self.ids.contains(id) {
                            return Err(Error::InvalidParameter);
                        }
                        self.track_latency
                    }
                    RecordingSource::Master => budget,
                    RecordingSource::Input => 0,
                    RecordingSource::InputChannels(channels) => {
                        let bus = self.input.as_ref().ok_or(Error::InvalidRouting)?;
                        if !(1..=2).contains(&channels.len())
                            || channels.iter().any(|&c| c as usize >= bus.channels())
                        {
                            return Err(Error::InvalidRouting);
                        }
                        0
                    }
                };
                Ok(TrackSpec {
                    id: id.clone(),
                    channels: if let RecordingSource::InputChannels(ch) = source {
                        ch.len()
                    } else {
                        2
                    },
                    latency_frames,
                })
            })
            .collect()
    }
}
pub(crate) struct RecordingBinding {
    pub tap: Tap,
    sources: Vec<usize>,
    channels: Vec<Option<Vec<u16>>>,
    scratch: Vec<Vec<f32>>,
    origin: Option<u64>,
    previous: Option<(u64, bool)>,
}
impl RecordingBinding {
    pub fn new(
        tap: Tap,
        sources: &[RecordingSource],
        capture: &CaptureStore,
        spec: ProcessSpec,
        budget: usize,
    ) -> Result<Self, Error> {
        let names: Vec<_> = tap
            .config()
            .tracks
            .iter()
            .zip(sources)
            .map(|(t, s)| (t.id.clone(), s.clone()))
            .collect();
        let expected = capture.specs(&names, budget)?;
        if tap.config().sample_rate as f64 != spec.sample_rate
            || tap.config().max_block_frames < spec.max_block_frames
            || sources.len() != tap.config().tracks.len()
            || expected
                .iter()
                .zip(&tap.config().tracks)
                .any(|(a, b)| a.channels != b.channels || a.latency_frames != b.latency_frames)
        {
            return Err(Error::InvalidSpec);
        }
        let indices = sources
            .iter()
            .map(|s| match s {
                RecordingSource::Track(id) => capture
                    .ids
                    .iter()
                    .position(|s| s == id)
                    .ok_or(Error::InvalidParameter),
                RecordingSource::Master => Ok(usize::MAX),
                RecordingSource::Input => Ok(usize::MAX - 1),
                RecordingSource::InputChannels(_) => Ok(usize::MAX - 2),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            tap,
            scratch: expected
                .iter()
                .map(|t| vec![0.; spec.max_block_frames * t.channels])
                .collect(),
            channels: sources
                .iter()
                .map(|s| {
                    if let RecordingSource::InputChannels(ch) = s {
                        Some(ch.clone())
                    } else {
                        None
                    }
                })
                .collect(),
            sources: indices,
            origin: None,
            previous: None,
        })
    }
    pub fn capture(
        &mut self,
        store: &CaptureStore,
        clock: u64,
        input: &[f32],
        master: &[f32],
    ) -> Result<(), Error> {
        let len = master.len();
        if store.offset.load(Ordering::Acquire) != len {
            return Err(Error::NotPrepared);
        }
        for (i, (index, destination)) in self.sources.iter().zip(&mut self.scratch).enumerate() {
            if *index == usize::MAX - 2 {
                let bus = store.input.as_ref().ok_or(Error::InvalidRouting)?;
                let channels = self.channels[i].as_ref().ok_or(Error::InvalidRouting)?;
                for f in 0..len / 2 {
                    for (c, &ch) in channels.iter().enumerate() {
                        destination[f * channels.len() + c] = bus.sample(f, ch as usize);
                    }
                }
                continue;
            }
            if *index == usize::MAX {
                destination[..len].copy_from_slice(master);
            } else if *index == usize::MAX - 1 {
                destination[..len].copy_from_slice(input);
            } else {
                for (s, d) in store.data[*index][..len]
                    .iter()
                    .zip(&mut destination[..len])
                {
                    *d = f32::from_bits(s.load(Ordering::Relaxed));
                }
            }
        }
        let mut views: [&[f32]; 32] = [&[]; 32];
        for (i, (view, data)) in views.iter_mut().zip(&self.scratch).enumerate() {
            *view = &data[..len / 2 * self.tap.config().tracks[i].channels];
        }
        let origin = *self.origin.get_or_insert(clock);
        let mut boundaries = [LoopBoundary {
            sample: 0,
            source_frame: 0,
            kind: BoundaryKind::Loop,
        }; 1024];
        let mut count = 0;
        for f in 0..len / 2 {
            let source = store.positions[f].load(Ordering::Relaxed);
            let flag = store.clock_flags[f].load(Ordering::Relaxed);
            let playing = flag != 0;
            let discontinuous = self.previous.is_some_and(|(previous, was_playing)| {
                source != previous + if was_playing { 1 } else { 0 }
            });
            let kind = if flag == 2
                || flag == 3 && self.previous.is_none_or(|(_, was_playing)| !was_playing)
            {
                Some(BoundaryKind::Loop)
            } else if self
                .previous
                .is_none_or(|(_, was_playing)| was_playing != playing)
                || discontinuous
            {
                Some(BoundaryKind::Transport)
            } else {
                None
            };
            if let Some(kind) = kind {
                boundaries[count] = LoopBoundary {
                    sample: clock - origin + f as u64,
                    source_frame: source,
                    kind,
                };
                count += 1;
            }
            self.previous = Some((source, playing));
        }
        self.tap.capture_with_loops(
            clock - origin,
            &views[..self.sources.len()],
            &boundaries[..count],
        )
    }
}
