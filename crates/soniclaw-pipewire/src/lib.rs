//! Native Linux PipeWire DSP filter backend with explicit named ports and links.
use serde::Serialize;
use soniclaw_core::*;
use std::sync::{
    atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering},
    Arc,
};
pub type PipeWireError = Box<dyn std::error::Error + Send + Sync>;
#[derive(Clone, Debug)]
pub struct Config {
    pub name: String,
    pub remote: String,
    pub sample_rate: u32,
    pub channels: usize,
    pub max_block_frames: usize,
    pub requested_quantum: u32,
}
impl Config {
    fn spec(&self) -> Result<ProcessSpec, PipeWireError> {
        if self.name.is_empty()
            || self.name.len() > 128
            || self.remote.is_empty()
            || self.remote.len() > 128
            || !(8000..=192000).contains(&self.sample_rate)
            || !(1..=8).contains(&self.channels)
            || !(1..=1024).contains(&self.max_block_frames)
            || !(16..=8192).contains(&self.requested_quantum)
            || self.name.contains('\0')
            || self.remote.contains('\0')
        {
            return Err("invalid PipeWire configuration".into());
        }
        Ok(ProcessSpec {
            sample_rate: self.sample_rate as f64,
            channels: self.channels,
            max_block_frames: self.max_block_frames,
        })
    }
}
#[derive(Default)]
pub struct Telemetry {
    callbacks: AtomicU64,
    frames: AtomicU64,
    quantum: AtomicU32,
    rate: AtomicU32,
    clock_nsec: AtomicU64,
    delay: AtomicI64,
    xruns: AtomicU64,
    rate_mismatches: AtomicU64,
    max_callback_nsec: AtomicU64,
    overruns: AtomicU64,
    input_peak: AtomicU32,
    output_peak: AtomicU32,
}
#[derive(Debug, Clone, Serialize)]
pub struct Timing {
    pub callbacks: u64,
    pub frames: u64,
    pub quantum: u32,
    pub sample_rate: u32,
    pub clock_nsec: u64,
    pub driver_delay_frames: i64,
    pub xrun_recovery_cycles: u64,
    pub rate_mismatches: u64,
    pub max_callback_nsec: u64,
    pub callback_overruns: u64,
    pub input_peak: f32,
    pub output_peak: f32,
}
impl Telemetry {
    pub fn snapshot(&self) -> Timing {
        Timing {
            callbacks: self.callbacks.load(Ordering::Relaxed),
            frames: self.frames.load(Ordering::Relaxed),
            quantum: self.quantum.load(Ordering::Relaxed),
            sample_rate: self.rate.load(Ordering::Relaxed),
            clock_nsec: self.clock_nsec.load(Ordering::Relaxed),
            driver_delay_frames: self.delay.load(Ordering::Relaxed),
            xrun_recovery_cycles: self.xruns.load(Ordering::Relaxed),
            rate_mismatches: self.rate_mismatches.load(Ordering::Relaxed),
            max_callback_nsec: self.max_callback_nsec.load(Ordering::Relaxed),
            callback_overruns: self.overruns.load(Ordering::Relaxed),
            input_peak: f32::from_bits(self.input_peak.load(Ordering::Relaxed)),
            output_peak: f32::from_bits(self.output_peak.load(Ordering::Relaxed)),
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Object {
    pub id: u32,
    pub kind: String,
    pub node: u32,
    pub output: bool,
    pub name: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub node_id: u32,
    pub state: i32,
    pub error: String,
    pub registry_overflows: u32,
    pub objects: Vec<Object>,
}
#[cfg(target_os = "linux")]
mod native {
    use super::*;
    use std::{
        ffi::{c_char, c_void, CStr, CString},
        ptr::NonNull,
    };
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Info {
        id: u32,
        kind: u32,
        node: u32,
        direction: u32,
        name: [c_char; 256],
    }
    unsafe extern "C" {
        fn sl_pw_init();
        fn sl_pw_create(
            name: *const c_char,
            remote: *const c_char,
            channels: u32,
            rate: u32,
            quantum: u32,
            latency: u32,
            callback: unsafe extern "C" fn(
                *mut c_void,
                *const *const f32,
                *const *mut f32,
                u32,
                u32,
                u64,
                i64,
                u32,
            ),
            user: *mut c_void,
        ) -> *mut c_void;
        fn sl_pw_destroy(pointer: *mut c_void);
        fn sl_pw_snapshot(
            pointer: *mut c_void,
            out: *mut Info,
            capacity: u32,
            node: *mut u32,
            state: *mut i32,
            overflows: *mut u32,
            error: *mut c_char,
        ) -> u32;
        fn sl_pw_link(pointer: *mut c_void, output: u32, input: u32) -> i32;
        fn sl_pw_unlink(pointer: *mut c_void, slot: u32) -> i32;
    }
    struct Callback {
        processor: Box<dyn AudioProcessor>,
        pcm: Vec<f32>,
        spec: ProcessSpec,
        sample: u64,
        telemetry: Arc<Telemetry>,
    }
    // SAFETY: C owns non-overlapping port buffers for this synchronous invocation; context remains
    // alive until native destruction joins its loops. No mutable Rust access occurs elsewhere.
    unsafe extern "C" fn process(
        user: *mut c_void,
        inputs: *const *const f32,
        outputs: *const *mut f32,
        frames: u32,
        rate: u32,
        nsec: u64,
        delay: i64,
        flags: u32,
    ) {
        unsafe {
            let state = &mut *user.cast::<Callback>();
            let count = frames as usize;
            let channels = state.spec.channels;
            let ins = std::slice::from_raw_parts(inputs, channels);
            let outs = std::slice::from_raw_parts(outputs, channels);
            let started = std::time::Instant::now();
            let stats = &state.telemetry;
            stats.callbacks.fetch_add(1, Ordering::Relaxed);
            stats.frames.fetch_add(frames as u64, Ordering::Relaxed);
            stats.quantum.store(frames, Ordering::Relaxed);
            stats.rate.store(rate, Ordering::Relaxed);
            stats.clock_nsec.store(nsec, Ordering::Relaxed);
            stats.delay.store(delay, Ordering::Relaxed);
            if flags & 2 != 0 {
                stats.xruns.fetch_add(1, Ordering::Relaxed);
            }
            if rate as f64 != state.spec.sample_rate {
                for out in outs {
                    if !out.is_null() {
                        std::slice::from_raw_parts_mut(*out, count).fill(0.);
                    }
                }
                stats.rate_mismatches.fetch_add(1, Ordering::Relaxed);
                return;
            }
            let mut offset = 0;
            let mut input_peak = 0f32;
            let mut output_peak = 0f32;
            while offset < count {
                let n = (count - offset).min(state.spec.max_block_frames);
                for f in 0..n {
                    for (c, p) in ins.iter().enumerate() {
                        let v = if p.is_null() {
                            0.
                        } else {
                            *(*p).add(offset + f)
                        };
                        let v = if v.is_finite() { v } else { 0. };
                        input_peak = input_peak.max(v.abs());
                        state.pcm[f * channels + c] = v;
                    }
                }
                state.processor.process(
                    &mut AudioBuffer::new(&mut state.pcm[..n * channels], channels)
                        .expect("prepared"),
                    &ProcessContext {
                        events: &[],
                        transport: Transport {
                            sample_position: state.sample,
                            tempo: 120.,
                            playing: true,
                        },
                    },
                );
                state.sample += n as u64;
                for f in 0..n {
                    for (c, p) in outs.iter().enumerate() {
                        let v = state.pcm[f * channels + c];
                        let v = if v.is_finite() { v } else { 0. };
                        output_peak = output_peak.max(v.abs());
                        if !p.is_null() {
                            *(*p).add(offset + f) = v;
                        }
                    }
                }
                offset += n;
            }
            stats
                .input_peak
                .fetch_max(input_peak.to_bits(), Ordering::Relaxed);
            stats
                .output_peak
                .fetch_max(output_peak.to_bits(), Ordering::Relaxed);
            let elapsed = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
            stats
                .max_callback_nsec
                .fetch_max(elapsed, Ordering::Relaxed);
            if elapsed > frames as u64 * 1_000_000_000 / rate as u64 {
                stats.overruns.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    /// Control-thread owner; drop disconnects ports/links and joins loops before releasing DSP.
    pub struct Host {
        pointer: NonNull<c_void>,
        _callback: Box<Callback>,
        telemetry: Arc<Telemetry>,
        _main: std::marker::PhantomData<std::rc::Rc<()>>,
    }
    impl Host {
        pub fn start(
            config: Config,
            mut processor: Box<dyn AudioProcessor>,
        ) -> Result<Self, PipeWireError> {
            let spec = config.spec()?;
            processor.prepare(spec)?;
            let latency = u32::try_from(processor.latency_frames())?;
            let mut callback = Box::new(Callback {
                processor,
                pcm: vec![0.; spec.max_block_frames * spec.channels],
                spec,
                sample: 0,
                telemetry: Arc::new(Telemetry::default()),
            });
            let name = CString::new(config.name)?;
            let remote = CString::new(config.remote)?;
            static INIT: std::sync::Once = std::sync::Once::new();
            INIT.call_once(|| unsafe {
                sl_pw_init();
            });
            let telemetry = callback.telemetry.clone();
            let pointer = unsafe {
                sl_pw_create(
                    name.as_ptr(),
                    remote.as_ptr(),
                    spec.channels as u32,
                    config.sample_rate,
                    config.requested_quantum,
                    latency,
                    process,
                    (&mut *callback as *mut Callback).cast(),
                )
            };
            Ok(Self {
                pointer: NonNull::new(pointer).ok_or("PipeWire connection/setup failed")?,
                _callback: callback,
                telemetry,
                _main: std::marker::PhantomData,
            })
        }
        /// Wait for asynchronous server registration on a control thread only.
        pub fn wait_ready(&self, timeout: std::time::Duration) -> Result<Snapshot, PipeWireError> {
            if timeout > std::time::Duration::from_secs(10) {
                return Err("ready timeout exceeds ten seconds".into());
            }
            let deadline = std::time::Instant::now() + timeout;
            loop {
                let snapshot = self.snapshot();
                if !snapshot.error.is_empty() || snapshot.state < 0 {
                    return Err(format!("PipeWire registration failed: {}", snapshot.error).into());
                }
                if snapshot.state >= 2 && snapshot.node_id != u32::MAX {
                    return Ok(snapshot);
                }
                if std::time::Instant::now() >= deadline {
                    return Err("PipeWire registration deadline expired".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        pub fn telemetry(&self) -> Arc<Telemetry> {
            self.telemetry.clone()
        }
        pub fn snapshot(&self) -> Snapshot {
            let mut objects = [Info {
                id: 0,
                kind: 0,
                node: 0,
                direction: 0,
                name: [0; 256],
            }; 512];
            let (mut node, mut state, mut overflows) = (0, 0, 0);
            let mut error = [0; 256];
            let n = unsafe {
                sl_pw_snapshot(
                    self.pointer.as_ptr(),
                    objects.as_mut_ptr(),
                    512,
                    &mut node,
                    &mut state,
                    &mut overflows,
                    error.as_mut_ptr(),
                )
            };
            Snapshot {
                node_id: node,
                state,
                error: unsafe { CStr::from_ptr(error.as_ptr()) }
                    .to_string_lossy()
                    .into_owned(),
                registry_overflows: overflows,
                objects: objects[..n as usize]
                    .iter()
                    .map(|o| Object {
                        id: o.id,
                        kind: match o.kind {
                            1 => "node",
                            2 => "port",
                            _ => "link",
                        }
                        .into(),
                        node: o.node,
                        output: o.direction != 0,
                        name: unsafe { CStr::from_ptr(o.name.as_ptr()) }
                            .to_string_lossy()
                            .into_owned(),
                    })
                    .collect(),
            }
        }
        /// Creates a non-lingering link by global port IDs. Return is a local link handle;
        /// inspect snapshots/errors to observe asynchronous server acceptance.
        pub fn link(&mut self, output: u32, input: u32) -> Result<u32, PipeWireError> {
            let snapshot = self.snapshot();
            if !snapshot
                .objects
                .iter()
                .any(|o| o.id == output && o.kind == "port" && o.output)
                || !snapshot
                    .objects
                    .iter()
                    .any(|o| o.id == input && o.kind == "port" && !o.output)
            {
                return Err("ports missing or direction mismatch".into());
            }
            let slot = unsafe { sl_pw_link(self.pointer.as_ptr(), output, input) };
            if slot < 0 {
                Err("link capacity/allocation failure".into())
            } else {
                Ok(slot as u32)
            }
        }
        pub fn unlink(&mut self, handle: u32) -> Result<(), PipeWireError> {
            if unsafe { sl_pw_unlink(self.pointer.as_ptr(), handle) } < 0 {
                Err("unknown link handle".into())
            } else {
                Ok(())
            }
        }
    }
    impl Drop for Host {
        fn drop(&mut self) {
            unsafe {
                sl_pw_destroy(self.pointer.as_ptr());
            }
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        struct Gain;
        impl AudioProcessor for Gain {
            fn prepare(&mut self, _: ProcessSpec) -> Result<(), Error> {
                Ok(())
            }
            fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
                for s in b.samples_mut() {
                    *s *= 0.5;
                }
            }
            fn reset(&mut self) {}
        }
        #[test]
        fn planar_callback_splits_without_allocation_and_rejects_wrong_rate() {
            let spec = ProcessSpec {
                sample_rate: 48000.,
                max_block_frames: 64,
                channels: 2,
            };
            let mut state = Callback {
                processor: Box::new(Gain),
                pcm: vec![0.; 128],
                spec,
                sample: 0,
                telemetry: Arc::new(Telemetry::default()),
            };
            let left = [1.; 191];
            let right = [0.5; 191];
            let ins = [left.as_ptr(), right.as_ptr()];
            let (mut l, mut r) = ([0.; 191], [0.; 191]);
            let outs = [l.as_mut_ptr(), r.as_mut_ptr()];
            let measured = allocation_counter::measure(|| unsafe {
                process(
                    (&mut state as *mut Callback).cast(),
                    ins.as_ptr(),
                    outs.as_ptr(),
                    191,
                    48000,
                    123,
                    5,
                    0,
                )
            });
            assert_eq!(measured.count_total, 0);
            assert_eq!(l, [0.5; 191]);
            assert_eq!(r, [0.25; 191]);
            assert_eq!(state.sample, 191);
            unsafe {
                process(
                    (&mut state as *mut Callback).cast(),
                    ins.as_ptr(),
                    outs.as_ptr(),
                    191,
                    44100,
                    456,
                    0,
                    2,
                )
            }
            assert_eq!(l, [0.; 191]);
            let stats = state.telemetry.snapshot();
            assert_eq!(stats.rate_mismatches, 1);
            assert_eq!(stats.xrun_recovery_cycles, 1);
        }
    }
}
#[cfg(target_os = "linux")]
pub use native::Host;
#[cfg(not(target_os = "linux"))]
pub struct Host;
#[cfg(not(target_os = "linux"))]
impl Host {
    pub fn start(
        config: Config,
        _processor: Box<dyn AudioProcessor>,
    ) -> Result<Self, PipeWireError> {
        config.spec()?;
        Err("PipeWire backend is Linux-only".into())
    }
}
