//! A shared, bounded worker for reloadable convolution processors.
use crate::stereo_convolution::{
    convolution_pair, ConvolutionController, ConvolutionMode, ReloadableConvolution, StereoImpulse,
};
use soniclaw_core::{Error, ProcessSpec};
use std::{
    sync::{mpsc, Arc, Mutex},
    thread::{self, JoinHandle},
};

/// Submission never blocks. The impulse is returned intact when it was not queued.
#[derive(Debug)]
pub enum SubmitError<T> {
    Full(T),
    Closed(T),
    WrongQueue(T),
}

impl<T: std::fmt::Debug> std::fmt::Display for SubmitError<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Full(_) => "convolution update queue is full",
            Self::Closed(_) => "convolution update worker has stopped",
            Self::WrongQueue(_) => "convolution target belongs to another queue",
        })
    }
}

impl<T: std::fmt::Debug> std::error::Error for SubmitError<T> {}

/// Completion is observed on a control thread. A prepared replacement may still
/// be waiting in the processor's audio-side queue when this ticket resolves.
pub struct UpdateTicket<T>(mpsc::Receiver<T>);

impl<T> UpdateTicket<T> {
    pub fn try_result(&self) -> Result<T, mpsc::TryRecvError> {
        self.0.try_recv()
    }

    pub fn wait(self) -> Result<T, mpsc::RecvError> {
        self.0.recv()
    }
}

/// Control handle associated with one processor and one queue.
pub struct SharedConvolution {
    owner: Arc<()>,
    controller: Arc<Mutex<ConvolutionController>>,
}

enum Job {
    Replace(
        Arc<Mutex<ConvolutionController>>,
        StereoImpulse,
        mpsc::Sender<Result<(), Error>>,
    ),
    Collect(Arc<Mutex<ConvolutionController>>, mpsc::Sender<usize>),
}

/// The queue and all submission and completion methods belong on control threads.
pub struct ConvolutionUpdateQueue {
    owner: Arc<()>,
    sender: Option<mpsc::SyncSender<Job>>,
    worker: Option<JoinHandle<()>>,
}

impl ConvolutionUpdateQueue {
    pub fn new(capacity: usize) -> std::io::Result<Self> {
        if !(1..=1024).contains(&capacity) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "queue capacity must be 1–1024",
            ));
        }
        let (sender, receiver) = mpsc::sync_channel(capacity);
        let worker = thread::Builder::new()
            .name("soniclaw-convolution-updates".into())
            .spawn(move || {
                for job in receiver {
                    match job {
                        Job::Replace(controller, impulse, result) => {
                            let outcome = controller
                                .lock()
                                .expect("one worker owns controller")
                                .replace(impulse);
                            let _ = result.send(outcome);
                        }
                        Job::Collect(controller, result) => {
                            let count = controller
                                .lock()
                                .expect("one worker owns controller")
                                .collect_retired();
                            let _ = result.send(count);
                        }
                    }
                }
            })?;
        Ok(Self {
            owner: Arc::new(()),
            sender: Some(sender),
            worker: Some(worker),
        })
    }

    /// Initial kernel creation is synchronous on the control thread.
    pub fn create(
        &self,
        impulse: StereoImpulse,
        mode: ConvolutionMode,
        extra_latency: usize,
        fade_frames: usize,
        spec: ProcessSpec,
    ) -> Result<(SharedConvolution, ReloadableConvolution), Error> {
        let (controller, audio) =
            convolution_pair(impulse, mode, extra_latency, fade_frames, spec)?;
        Ok((
            SharedConvolution {
                owner: self.owner.clone(),
                controller: Arc::new(Mutex::new(controller)),
            },
            audio,
        ))
    }

    pub fn try_replace(
        &self,
        target: &SharedConvolution,
        impulse: StereoImpulse,
    ) -> Result<UpdateTicket<Result<(), Error>>, SubmitError<StereoImpulse>> {
        if !Arc::ptr_eq(&target.owner, &self.owner) {
            return Err(SubmitError::WrongQueue(impulse));
        }
        let (tx, rx) = mpsc::channel();
        let job = Job::Replace(target.controller.clone(), impulse, tx);
        match self.sender.as_ref().expect("queue is open").try_send(job) {
            Ok(()) => Ok(UpdateTicket(rx)),
            Err(mpsc::TrySendError::Full(Job::Replace(_, impulse, _))) => {
                Err(SubmitError::Full(impulse))
            }
            Err(mpsc::TrySendError::Disconnected(Job::Replace(_, impulse, _))) => {
                Err(SubmitError::Closed(impulse))
            }
            Err(_) => unreachable!("returned the submitted replacement"),
        }
    }

    /// Explicitly reclaim retired kernels when no further replacement is pending.
    pub fn try_collect(
        &self,
        target: &SharedConvolution,
    ) -> Result<UpdateTicket<usize>, SubmitError<()>> {
        if !Arc::ptr_eq(&target.owner, &self.owner) {
            return Err(SubmitError::WrongQueue(()));
        }
        let (tx, rx) = mpsc::channel();
        self.sender
            .as_ref()
            .expect("queue is open")
            .try_send(Job::Collect(target.controller.clone(), tx))
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => SubmitError::Full(()),
                mpsc::TrySendError::Disconnected(_) => SubmitError::Closed(()),
            })?;
        Ok(UpdateTicket(rx))
    }
}

impl Drop for ConvolutionUpdateQueue {
    /// Drains queued work and joins the worker. Drop from a control thread.
    fn drop(&mut self) {
        drop(self.sender.take());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soniclaw_core::{AudioBuffer, AudioProcessor, ProcessContext, Transport};

    #[test]
    fn shared_worker_preserves_per_target_crossfade_and_retirement() {
        let queue = ConvolutionUpdateQueue::new(8).unwrap();
        let spec = ProcessSpec {
            channels: 1,
            sample_rate: 48000.,
            max_block_frames: 8,
        };
        let ir = |gain| StereoImpulse::new(vec![gain], 1, 48000.).unwrap();
        let (a, mut x) = queue
            .create(ir(1.), ConvolutionMode::ZeroLatency, 0, 4, spec)
            .unwrap();
        let (b, mut y) = queue
            .create(ir(1.), ConvolutionMode::ZeroLatency, 0, 4, spec)
            .unwrap();
        let first = queue.try_replace(&a, ir(0.5)).unwrap();
        let second = queue.try_replace(&b, ir(0.25)).unwrap();
        first.wait().unwrap().unwrap();
        second.wait().unwrap().unwrap();
        let other = ConvolutionUpdateQueue::new(1).unwrap();
        assert!(matches!(
            other.try_replace(&a, ir(1.)),
            Err(SubmitError::WrongQueue(_))
        ));
        let ctx = ProcessContext {
            events: &[],
            transport: Transport::default(),
        };
        for (handle, engine, target) in [(&a, &mut x, 0.5), (&b, &mut y, 0.25)] {
            let mut samples = [1.; 8];
            engine.process(&mut AudioBuffer::new(&mut samples, 1).unwrap(), &ctx);
            assert_eq!(samples[0], 0.75 + target * 0.25);
            assert_eq!(samples[7], target);
            engine.process(&mut AudioBuffer::new(&mut samples, 1).unwrap(), &ctx);
            assert_eq!(queue.try_collect(handle).unwrap().wait().unwrap(), 1);
        }
    }

    #[test]
    fn full_worker_returns_impulse_and_drains_on_drop() {
        let queue = ConvolutionUpdateQueue::new(1).unwrap();
        let spec = ProcessSpec {
            channels: 1,
            sample_rate: 48000.,
            max_block_frames: 8,
        };
        let ir = || StereoImpulse::new(vec![1.], 1, 48000.).unwrap();
        let (control, _audio) = queue
            .create(ir(), ConvolutionMode::ZeroLatency, 0, 4, spec)
            .unwrap();
        let guard = control.controller.lock().unwrap();
        let first = queue.try_replace(&control, ir()).unwrap();
        let mut second = None;
        loop {
            let result = queue.try_replace(&control, ir());
            match result {
                Ok(ticket) => {
                    second = Some(ticket);
                }
                Err(SubmitError::Full(impulse)) if second.is_some() => {
                    assert_eq!(impulse.frames(), 1);
                    break;
                }
                Err(SubmitError::Full(_)) => thread::yield_now(),
                Err(e) => panic!("{e}"),
            }
        }
        drop(guard);
        drop(queue);
        first.wait().unwrap().unwrap();
        second.unwrap().wait().unwrap().unwrap();
    }
}
