//! Direct trusted CLAP processing without an asynchronous audio bridge.
//! Each callback starts/stops its audio leases; the main owner can then safely revoke them.
use super::{live::*, *};
use rtrb::{Consumer, Producer, RingBuffer};
use std::{
    cell::UnsafeCell,
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};
pub(crate) struct DirectShared {
    busy: AtomicBool,
    state: UnsafeCell<Option<DirectState>>,
}
// SAFETY: only the successful busy CAS holder accesses state. Revocation sets the
// shared stop flag first, acquires busy off-thread and takes the state before unload.
unsafe impl Sync for DirectShared {}
unsafe impl Send for DirectShared {}
struct DirectState {
    leases: Vec<Lease>,
    controls: Consumer<Command>,
    events: Producer<RackEvent>,
    parameters: Vec<Vec<ParameterEvent>>,
    bypass: Vec<crate::session::BypassLine>,
    dry: Vec<f32>,
}
pub struct DirectProcessor {
    pub(crate) direct: Arc<DirectShared>,
    shared: Arc<Shared>,
    spec: ProcessSpec,
    latency: usize,
    pending: Vec<ParameterEvent>,
}
struct Busy<'a>(&'a AtomicBool);
impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
struct AudioFlag(bool);
impl Drop for AudioFlag {
    fn drop(&mut self) {
        AUDIO.set(self.0);
    }
}
impl DirectShared {
    pub(crate) fn revoke(&self) {
        while self
            .busy
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            std::thread::yield_now();
        }
        let _guard = Busy(&self.busy);
        unsafe {
            (*self.state.get()).take();
        }
    }
}
impl DirectProcessor {
    /// Sorted native-range events for node zero. Storage and merge are bounded at 64 events.
    pub fn process_automated(
        &mut self,
        b: &mut AudioBuffer<'_>,
        ctx: &ProcessContext<'_>,
        points: &[ParameterEvent],
    ) -> Result<(), Error> {
        if points.len() > 64
            || points.windows(2).any(|p| p[0].frame > p[1].frame)
            || points
                .iter()
                .any(|p| p.frame >= b.frames() || !p.value.is_finite())
        {
            b.clear();
            return Err(Error::InvalidEvents);
        }
        self.pending.clear();
        self.pending.extend_from_slice(points);
        self.process(b, ctx);
        self.pending.clear();
        if self.shared.stop.load(Ordering::Acquire) {
            Err(Error::InvalidEvents)
        } else {
            Ok(())
        }
    }

    pub(crate) fn pair(
        mut instances: Vec<ClapInstance>,
        spec: ProcessSpec,
    ) -> Result<(LiveRack, Self), HostError> {
        spec.validate()?;
        if spec.channels != 2
            || spec.max_block_frames > 8192
            || instances.is_empty()
            || instances.len() > MAX_NODES
        {
            return Err("direct rack needs stereo, 1..8 nodes, maximum 8192 frames".into());
        }
        let mut leases = Vec::new();
        let mut latency = 0;
        for instance in &mut instances {
            if instance.active {
                return Err("direct rack needs deactivated instances".into());
            }
            instance.activate(spec)?;
            if instance.latency > 1_048_576 {
                return Err("latency exceeds bypass memory limit".into());
            }
            latency += instance.latency;
            leases.push(Lease {
                plugin: instance.plugin,
                context: &*instance.context,
                parameters: instance.parameters.clone(),
                resources: instance.resources.take().expect("activated"),
                spec,
                input_channels: instance.input_channels,
                note_dialect: instance.note_dialect,
                latency: instance.latency,
                params: unsafe {
                    (*instance.plugin).get_extension.expect("checked")(
                        instance.plugin,
                        CLAP_EXT_PARAMS.as_ptr(),
                    )
                    .cast()
                },
            });
        }
        let (commands, controls) = RingBuffer::new(64);
        let (event_tx, events) = RingBuffer::new(128);
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            panic: AtomicBool::new(false),
            flush: AtomicU32::new(0),
            bypass: AtomicU32::new(0),
            stats: Arc::new(Stats::default()),
        });
        let count = instances.len();
        let parameters = (0..count).map(|_| Vec::with_capacity(64)).collect();
        let bypass = leases
            .iter()
            .map(|l| crate::session::BypassLine::new(l.latency))
            .collect::<Result<Vec<_>, _>>()?;
        let direct = Arc::new(DirectShared {
            busy: AtomicBool::new(false),
            state: UnsafeCell::new(Some(DirectState {
                leases,
                controls,
                events: event_tx,
                parameters,
                bypass,
                dry: vec![0.0; spec.max_block_frames * 2],
            })),
        });
        Ok((
            LiveRack {
                instances,
                editors: (0..count).map(|_| None).collect(),
                commands,
                events,
                shared: shared.clone(),
                worker: None,
                rate: spec.sample_rate,
                direct: Some(direct.clone()),
                editor_events: Vec::with_capacity(128),
            },
            Self {
                direct,
                shared,
                spec,
                latency,
                pending: Vec::with_capacity(64),
            },
        ))
    }
}
impl Drop for DirectProcessor {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
    }
}
impl AudioProcessor for DirectProcessor {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        if s.channels != self.spec.channels
            || s.sample_rate != self.spec.sample_rate
            || s.max_block_frames > self.spec.max_block_frames
        {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
    fn reset(&mut self) {
        self.shared.panic.store(true, Ordering::Release);
    }
    fn latency_frames(&self) -> usize {
        self.latency
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        if self
            .direct
            .busy
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            b.clear();
            return;
        }
        let _guard = Busy(&self.direct.busy);
        if self.shared.stop.load(Ordering::Acquire)
            || b.channels() != 2
            || b.frames() > self.spec.max_block_frames
            || ctx.events.len() > 1024
            || validate_events(ctx.events, b.frames()).is_err()
        {
            b.clear();
            return;
        }
        let start = Instant::now();
        let frames = b.frames();
        let _flag = AudioFlag(AUDIO.replace(true));
        // SAFETY: acquired exclusive state guard; owner may only revoke after this guard exits.
        let Some(state) = (unsafe { &mut *self.direct.state.get() }).as_mut() else {
            b.clear();
            return;
        };
        for p in &mut state.parameters {
            p.clear();
        }
        for _ in 0..64 {
            let Ok(c) = state.controls.pop() else {
                break;
            };
            state.parameters[c.node].push(ParameterEvent {
                frame: 0,
                id: c.id,
                value: c.value,
            });
        }
        for point in &self.pending {
            let valid = state.leases[0].parameters.iter().any(|p| {
                p.id == point.id
                    && p.flags & CLAP_PARAM_IS_READONLY == 0
                    && (p.min..=p.max).contains(&point.value)
            });
            if !valid || state.parameters[0].len() >= 64 {
                self.shared.stop.store(true, Ordering::Release);
                b.clear();
                return;
            }
            state.parameters[0].push(*point);
        }
        state.parameters[0].sort_unstable_by_key(|p| p.frame);
        let panic = self.shared.panic.swap(false, Ordering::AcqRel);
        let flush = self.shared.flush.swap(0, Ordering::AcqRel);
        for (index, (lease, parameters)) in
            state.leases.iter_mut().zip(&state.parameters).enumerate()
        {
            state.dry[..b.samples().len()].copy_from_slice(b.samples());
            let mut audio = HostedAudio {
                plugin: lease.plugin,
                _context: unsafe { &*lease.context },
                parameters: &lease.parameters,
                resources: std::mem::replace(&mut lease.resources, Resources::empty()),
                spec: lease.spec,
                input_channels: lease.input_channels,
                note_dialect: lease.note_dialect,
                processing: false,
                steady: ctx.transport.sample_position.min(i64::MAX as u64) as i64,
            };
            if unsafe { (*audio.plugin).start_processing.expect("checked")(audio.plugin) } {
                audio.processing = true;
                if flush & (1 << index) != 0 && !lease.params.is_null() {
                    audio.resources.out.clear();
                    let empty = Vec::<*const clap_event_header>::new();
                    let ie = clap_input_events {
                        ctx: (&empty as *const Vec<*const clap_event_header>)
                            .cast_mut()
                            .cast(),
                        size: Some(event_size),
                        get: Some(event_get),
                    };
                    let mut output = OutputContext {
                        events: &mut audio.resources.out,
                        frames: frames.max(1),
                        dropped: &mut audio.resources.dropped,
                    };
                    let oe = clap_output_events {
                        ctx: (&mut output as *mut OutputContext<'_>).cast(),
                        try_push: Some(event_push),
                    };
                    unsafe {
                        if let Some(f) = (*lease.params).flush {
                            f(audio.plugin, &ie, &oe);
                        }
                    }
                    for event in audio.output_events() {
                        if state
                            .events
                            .push(RackEvent {
                                node: index,
                                event: *event,
                            })
                            .is_err()
                        {
                            self.shared
                                .stats
                                .event_drops
                                .fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                let events = if lease.note_dialect == 0 {
                    &[][..]
                } else if panic && frames > 0 {
                    &[Event {
                        frame: 0,
                        message: MidiMessage::AllNotesOff,
                    }][..]
                } else {
                    ctx.events
                };
                if frames > 0
                    && (audio
                        .process_block(b.samples_mut(), events, parameters, ctx.transport)
                        .is_err()
                        || b.samples().iter().any(|x| !x.is_finite()))
                {
                    b.clear();
                    self.shared
                        .stats
                        .process_errors
                        .fetch_add(1, Ordering::Relaxed);
                    self.shared.panic.store(true, Ordering::Release);
                }
                for event in audio.output_events() {
                    if state
                        .events
                        .push(RackEvent {
                            node: index,
                            event: *event,
                        })
                        .is_err()
                    {
                        self.shared
                            .stats
                            .event_drops
                            .fetch_add(1, Ordering::Relaxed);
                    }
                }
                audio.stop();
            } else {
                b.clear();
                self.shared
                    .stats
                    .process_errors
                    .fetch_add(1, Ordering::Relaxed);
            }
            state.bypass[index].apply(
                &state.dry[..b.samples().len()],
                b.samples_mut(),
                self.shared.bypass.load(Ordering::Acquire) & (1 << index) != 0,
            );
            lease.resources = std::mem::replace(&mut audio.resources, Resources::empty());
        }
        self.shared.stats.blocks.fetch_add(1, Ordering::Relaxed);
        let ns = start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        self.shared
            .stats
            .max_callback_ns
            .fetch_max(ns, Ordering::Relaxed);
        if ns as f64 > frames as f64 / self.spec.sample_rate * 1e9 {
            self.shared
                .stats
                .deadline_misses
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}
/// Offline/control-thread impulse alignment tool. No audio hardware is opened.
/// Returns the strongest finite captured impulse at/after the reference impulse.
pub fn impulse_latency(
    reference: &[f32],
    captured: &[f32],
    max_lag: usize,
) -> Result<usize, Error> {
    if reference.is_empty()
        || captured.is_empty()
        || reference.len() > 1_048_576
        || captured.len() > 1_048_576
        || reference.iter().chain(captured).any(|x| !x.is_finite())
    {
        return Err(Error::InvalidBuffer);
    }
    let peak = |data: &[f32]| {
        data.iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .map(|(i, x)| (i, x.abs()))
            .expect("nonempty")
    };
    let (a, x) = peak(reference);
    let (b, y) = peak(captured);
    if x == 0.0 || y == 0.0 || b < a || b - a > max_lag {
        return Err(Error::InvalidBuffer);
    }
    Ok(b - a)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn impulse_alignment_limits() {
        assert_eq!(impulse_latency(&[0., 1., 0.], &[0., 0., 0., 1.], 8), Ok(2));
        assert!(impulse_latency(&[1.], &[0., 0.], 8).is_err());
        assert!(impulse_latency(&[1.], &[0., 1.], 0).is_err());
    }
}
