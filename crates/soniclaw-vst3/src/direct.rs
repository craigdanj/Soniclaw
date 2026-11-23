//! Prepared native processor path. No backend processing queues or meters on the callback.
use super::*;
use soniclaw_core::ProcessContext as CoreContext;
use std::{
    cell::{Cell, UnsafeCell},
    ptr,
};
use vst3::{
    Class, ComPtr, ComWrapper,
    Steinberg::{Vst::*, *},
};
const QUEUES: usize = 64;
struct ValueQueue {
    id: Cell<u32>,
    point: Cell<Option<(i32, f64)>>,
}
impl Class for ValueQueue {
    type Interfaces = (IParamValueQueue,);
}
impl IParamValueQueueTrait for ValueQueue {
    unsafe fn getParameterId(&self) -> u32 {
        self.id.get()
    }
    unsafe fn getPointCount(&self) -> i32 {
        i32::from(self.point.get().is_some())
    }
    unsafe fn getPoint(&self, index: i32, offset: *mut i32, value: *mut f64) -> i32 {
        if index != 0 || offset.is_null() || value.is_null() {
            return kResultFalse;
        }
        if let Some((o, v)) = self.point.get() {
            unsafe {
                *offset = o;
                *value = v;
            }
            kResultOk
        } else {
            kResultFalse
        }
    }
    unsafe fn addPoint(&self, offset: i32, value: f64, index: *mut i32) -> i32 {
        if index.is_null() || !value.is_finite() || !(0.0..=1.0).contains(&value) || offset < 0 {
            return kResultFalse;
        }
        self.point.set(Some((offset, value)));
        unsafe {
            *index = 0;
        }
        kResultOk
    }
}
struct Changes {
    queues: Vec<ComWrapper<ValueQueue>>,
    used: Cell<usize>,
}
impl Changes {
    fn new() -> Self {
        Self {
            queues: (0..QUEUES)
                .map(|_| {
                    ComWrapper::new(ValueQueue {
                        id: Cell::new(0),
                        point: Cell::new(None),
                    })
                })
                .collect(),
            used: Cell::new(0),
        }
    }
    fn clear(&self) {
        for q in &self.queues[..self.used.get()] {
            q.point.set(None);
        }
        self.used.set(0);
    }
    fn push(&self, id: u32, value: f64) -> bool {
        let mut index = 0;
        let q = unsafe { self.addParameterData(&id, &mut index) };
        if q.is_null() {
            false
        } else {
            {
                self.queues[index as usize].point.set(Some((0, value)));
                true
            }
        }
    }
}
impl Class for Changes {
    type Interfaces = (IParameterChanges,);
}
impl IParameterChangesTrait for Changes {
    unsafe fn getParameterCount(&self) -> i32 {
        self.used.get() as i32
    }
    unsafe fn getParameterData(&self, index: i32) -> *mut IParamValueQueue {
        if index < 0 || index as usize >= self.used.get() {
            return ptr::null_mut();
        }
        self.queues[index as usize]
            .as_com_ref::<IParamValueQueue>()
            .map_or(ptr::null_mut(), |p| p.as_ptr())
    }
    unsafe fn addParameterData(&self, id: *const u32, index: *mut i32) -> *mut IParamValueQueue {
        if id.is_null() || index.is_null() {
            return ptr::null_mut();
        }
        let id = unsafe { *id };
        let used = self.used.get();
        let slot = if let Some(i) = self.queues[..used].iter().position(|q| q.id.get() == id) {
            i
        } else {
            if used == QUEUES {
                return ptr::null_mut();
            }
            self.queues[used].id.set(id);
            self.queues[used].point.set(None);
            self.used.set(used + 1);
            used
        };
        unsafe {
            *index = slot as i32;
            self.getParameterData(slot as i32)
        }
    }
}
struct EmptyEvents;
impl Class for EmptyEvents {
    type Interfaces = (IEventList,);
}
impl IEventListTrait for EmptyEvents {
    unsafe fn getEventCount(&self) -> i32 {
        0
    }
    unsafe fn getEvent(&self, _: i32, _: *mut vst3::Steinberg::Vst::Event) -> i32 {
        kResultFalse
    }
    unsafe fn addEvent(&self, _: *mut vst3::Steinberg::Vst::Event) -> i32 {
        kResultFalse
    }
}
#[derive(Clone, Copy)]
pub(super) struct Command {
    pub id: u32,
    pub value: f64,
}
// The type selects both the sample layout and the VST3 union member.
trait VstSample: Copy + Default {
    fn symbolic() -> i32;
    fn set_planes(bus: &mut AudioBusBuffers, planes: *mut *mut Self);
}
macro_rules! sample_type {
    ($t:ty, $symbol:ident, $field:ident) => {
        impl VstSample for $t {
            #[allow(clippy::unnecessary_cast)]
            fn symbolic() -> i32 {
                SymbolicSampleSizes_::$symbol as i32
            }
            fn set_planes(bus: &mut AudioBusBuffers, planes: *mut *mut Self) {
                bus.__field0.$field = planes;
            }
        }
    };
}
sample_type!(f32, kSample32, channelBuffers32);
sample_type!(f64, kSample64, channelBuffers64);

struct State<T> {
    processor: ComPtr<IAudioProcessor>,
    input: [Vec<T>; 2],
    output: [Vec<T>; 2],
    _in_ptrs: Box<[*mut T; 2]>,
    _out_ptrs: Box<[*mut T; 2]>,
    in_bus: Box<AudioBusBuffers>,
    out_bus: Box<AudioBusBuffers>,
    context: Box<ProcessContextNative>,
    changes: ComWrapper<Changes>,
    out_changes: ComWrapper<Changes>,
    empty: ComWrapper<EmptyEvents>,
    commands: rtrb::Consumer<Command>,
    has_input: bool,
}
type ProcessContextNative = vst3::Steinberg::Vst::ProcessContext;
impl<T: VstSample> State<T> {
    #[allow(clippy::unnecessary_cast)] // VST3 sample-size constants differ in signedness across target ABIs.
    fn new(
        processor: ComPtr<IAudioProcessor>,
        spec: ProcessSpec,
        has_input: bool,
        commands: rtrb::Consumer<Command>,
    ) -> Result<Self> {
        let mut input = [
            vec![T::default(); spec.max_block_frames],
            vec![T::default(); spec.max_block_frames],
        ];
        let mut output = input.clone();
        let mut in_ptrs = Box::new([input[0].as_mut_ptr(), input[1].as_mut_ptr()]);
        let mut out_ptrs = Box::new([output[0].as_mut_ptr(), output[1].as_mut_ptr()]);
        let mut in_bus: Box<AudioBusBuffers> = Box::new(unsafe { std::mem::zeroed() });
        let mut out_bus = in_bus.clone();
        in_bus.numChannels = 2;
        out_bus.numChannels = 2;
        T::set_planes(&mut in_bus, in_ptrs.as_mut_ptr());
        T::set_planes(&mut out_bus, out_ptrs.as_mut_ptr());
        let mut context: Box<ProcessContextNative> = Box::new(unsafe { std::mem::zeroed() });
        context.sampleRate = spec.sample_rate;
        Ok(Self {
            processor,
            input,
            output,
            _in_ptrs: in_ptrs,
            _out_ptrs: out_ptrs,
            in_bus,
            out_bus,
            context,
            changes: ComWrapper::new(Changes::new()),
            out_changes: ComWrapper::new(Changes::new()),
            empty: ComWrapper::new(EmptyEvents),
            commands,
            has_input,
        })
    }
    #[allow(clippy::unnecessary_cast)] // VST3 flags differ in integer signedness across target ABIs.
    fn process(&mut self, data: &mut [T], ctx: &CoreContext<'_>) -> bool {
        let n = data.len() / 2;
        if n == 0 {
            return true;
        }
        self.changes.clear();
        self.out_changes.clear();
        for _ in 0..QUEUES {
            let Ok(c) = self.commands.pop() else {
                break;
            };
            if !self.changes.push(c.id, c.value) {
                return false;
            }
        }
        for (i, f) in data.as_chunks::<2>().0.iter().enumerate() {
            self.input[0][i] = f[0];
            self.input[1][i] = f[1];
        }
        self.output[0][..n].fill(T::default());
        self.output[1][..n].fill(T::default());
        self.in_bus.silenceFlags = 0;
        self.out_bus.silenceFlags = 0;
        self.context.state = (ProcessContext_::StatesAndFlags_::kTempoValid
            | ProcessContext_::StatesAndFlags_::kProjectTimeMusicValid
            | if ctx.transport.playing {
                ProcessContext_::StatesAndFlags_::kPlaying
            } else {
                0
            }) as u32;
        self.context.tempo = ctx.transport.tempo;
        self.context.projectTimeSamples = ctx.transport.sample_position.min(i64::MAX as u64) as i64;
        self.context.projectTimeMusic =
            ctx.transport.sample_position as f64 / self.context.sampleRate * ctx.transport.tempo
                / 60.;
        let mut packet: ProcessData = unsafe { std::mem::zeroed() };
        packet.processMode = ProcessModes_::kRealtime as i32;
        packet.symbolicSampleSize = T::symbolic();
        packet.numSamples = n as i32;
        packet.numInputs = i32::from(self.has_input);
        packet.numOutputs = 1;
        packet.inputs = if self.has_input {
            &mut *self.in_bus
        } else {
            ptr::null_mut()
        };
        packet.outputs = &mut *self.out_bus;
        packet.processContext = &mut *self.context;
        packet.inputParameterChanges = self
            .changes
            .as_com_ref::<IParameterChanges>()
            .expect("interface")
            .as_ptr();
        packet.outputParameterChanges = self
            .out_changes
            .as_com_ref::<IParameterChanges>()
            .expect("interface")
            .as_ptr();
        packet.inputEvents = self
            .empty
            .as_com_ref::<IEventList>()
            .expect("interface")
            .as_ptr();
        packet.outputEvents = packet.inputEvents;
        if unsafe { self.processor.process(&mut packet) } != kResultOk {
            return false;
        }
        for (i, f) in data.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            f[0] = self.output[0][i];
            f[1] = self.output[1][i];
        }
        true
    }
}
enum NativeState {
    F32(State<f32>),
    F64(State<f64>),
}

pub(super) struct Shared {
    busy: AtomicBool,
    stop: AtomicBool,
    state: UnsafeCell<Option<NativeState>>,
    pub errors: AtomicU64,
}
// SAFETY: only the successful busy-CAS holder accesses native state. Owner revokes
// after setting stop and waits for an in-flight callback before taking/dropping state.
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}
struct Guard<'a>(&'a AtomicBool);
impl Drop for Guard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl Shared {
    pub(super) fn revoke(&self) {
        self.stop.store(true, Ordering::Release);
        while self
            .busy
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            std::thread::yield_now();
        }
        let _g = Guard(&self.busy);
        unsafe {
            (*self.state.get()).take();
        }
    }
}
// Both endpoints share the same callback gate, revocation and state validation.
macro_rules! direct_endpoint {
    ($endpoint:ident, $processor:ident, $buffer:ident, $variant:ident) => {
        pub struct $endpoint {
            shared: Arc<Shared>,
            spec: ProcessSpec,
            latency: usize,
        }
        impl $processor for $endpoint {
            fn prepare(&mut self, s: ProcessSpec) -> std::result::Result<(), Error> {
                s.validate()?;
                if s.channels != 2
                    || s.sample_rate != self.spec.sample_rate
                    || s.max_block_frames > self.spec.max_block_frames
                {
                    return Err(Error::InvalidSpec);
                }
                Ok(())
            }
            fn reset(&mut self) {
                self.shared.stop.store(true, Ordering::Release);
            }
            fn latency_frames(&self) -> usize {
                self.latency
            }
            fn process(&mut self, b: &mut $buffer<'_>, ctx: &CoreContext<'_>) {
                if self
                    .shared
                    .busy
                    .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                    .is_err()
                {
                    b.clear();
                    return;
                }
                let _g = Guard(&self.shared.busy);
                if self.shared.stop.load(Ordering::Acquire)
                    || b.channels() != 2
                    || b.frames() > self.spec.max_block_frames
                    || !ctx.events.is_empty()
                    || !ctx.transport.tempo.is_finite()
                    || !(1.0..=1000.0).contains(&ctx.transport.tempo)
                {
                    b.clear();
                    self.shared.errors.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                let state = unsafe { &mut *self.shared.state.get() };
                let processed = match state {
                    Some(NativeState::$variant(s)) => s.process(b.samples_mut(), ctx),
                    _ => false,
                };
                if !processed {
                    b.clear();
                    self.shared.errors.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        impl Drop for $endpoint {
            fn drop(&mut self) {
                self.shared.stop.store(true, Ordering::Release);
            }
        }
    };
}
direct_endpoint!(DirectProcessor, AudioProcessor, AudioBuffer, F32);
direct_endpoint!(DirectProcessor64, AudioProcessor64, AudioBuffer64, F64);

impl Instance {
    /// Prepared direct mode: audio/normalized controls; MIDI/editor integration is not active in this first direct path.
    pub fn start_direct(&mut self) -> Result<DirectProcessor> {
        let (shared, latency) = self.start_direct_native(false)?;
        Ok(DirectProcessor {
            shared,
            spec: self.spec,
            latency,
        })
    }

    /// Start a native f64 direct lease. Unsupported devices fail negotiation.
    pub fn start_direct64(&mut self) -> Result<DirectProcessor64> {
        let (shared, latency) = self.start_direct_native(true)?;
        Ok(DirectProcessor64 {
            shared,
            spec: self.spec,
            latency,
        })
    }

    fn start_direct_native(&mut self, wide: bool) -> Result<(Arc<Shared>, usize)> {
        self.stop()?;
        self.close_editor();
        if self.isolated
            || self.layout.outputs.len() != 1
            || self.layout.outputs[0].channel_count != 2
            || self.layout.inputs.len() > 1
            || self
                .layout
                .inputs
                .first()
                .is_some_and(|b| b.channel_count != 2)
        {
            return Err("use a trusted native stereo VST3 layout for direct processing".into());
        }
        let started = (|| {
            let mut p = self.lock()?;
            p.service_run_loop();
            p.service_host_requests().map_err(err)?;
            p.set_direct_sample_size(Some(wide)).map_err(err)?;
            p.start_processing().map_err(err)?;
            p.service_host_requests().map_err(err)?;
            let processor = unsafe { p.direct_audio_processor() }
                .ok_or("backend cannot provide native direct processor")?;
            Ok::<_, HostError>((
                processor,
                p.latency_samples() as usize,
                !self.layout.inputs.is_empty(),
            ))
        })();
        let (processor, latency, has_input) = match started {
            Ok(value) => value,
            Err(error) => {
                if let Ok(mut p) = self.lock() {
                    let _ = p.stop_processing();
                    let _ = p.set_direct_sample_size(None);
                }
                return Err(error);
            }
        };
        let (tx, rx) = rtrb::RingBuffer::new(QUEUES);
        let state = if wide {
            NativeState::F64(State::new(processor, self.spec, has_input, rx)?)
        } else {
            NativeState::F32(State::new(processor, self.spec, has_input, rx)?)
        };
        let shared = Arc::new(Shared {
            busy: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            state: UnsafeCell::new(Some(state)),
            errors: AtomicU64::new(0),
        });
        self.direct = Some(shared.clone());
        self.direct_commands = Some(tx);
        Ok((shared, latency))
    }

    pub fn direct_errors(&self) -> u64 {
        self.direct
            .as_ref()
            .map_or(0, |s| s.errors.load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parameter_pool_is_bounded_and_reuses_slots() {
        let changes = Changes::new();
        for id in 0..QUEUES as u32 {
            assert!(changes.push(id, 0.5));
        }
        assert!(!changes.push(1000, 0.5));
        assert!(changes.push(0, 0.75));
        assert_eq!(changes.queues[0].point.get(), Some((0, 0.75)));
        changes.clear();
        assert_eq!(changes.used.get(), 0);
        assert!(changes.push(999, 0.25));
        assert_eq!(changes.queues[0].id.get(), 999);
    }
    #[test]
    fn revoked_callback_remains_silent_without_allocation() {
        let shared = Arc::new(Shared {
            busy: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            state: UnsafeCell::new(None),
            errors: AtomicU64::new(0),
        });
        let mut p = DirectProcessor {
            shared: shared.clone(),
            spec: ProcessSpec {
                sample_rate: 48000.,
                max_block_frames: 256,
                channels: 2,
            },
            latency: 0,
        };
        shared.revoke();
        let mut data = [1.; 512];
        let ctx = CoreContext {
            events: &[],
            transport: Default::default(),
        };
        let count = allocation_counter::measure(|| {
            p.process(&mut AudioBuffer::new(&mut data, 2).unwrap(), &ctx)
        });
        assert_eq!(count.count_total, 0);
        assert_eq!(data, [0.; 512]);
    }
}
