//! Bounded shared control/worker scheduler. Never call from an audio callback.
use std::{
    any::Any,
    collections::{BTreeMap, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, Mutex, Weak,
    },
    thread::JoinHandle,
};
type Value = Box<dyn Any + Send>;
type Work = Box<dyn FnOnce(Context) -> Result<Value, String> + Send>;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Full,
    Cancelled,
    Stale,
    Failed(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "background job: {self:?}")
    }
}
impl std::error::Error for Error {}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Interactive,
    Normal,
    Maintenance,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub workers: usize,
    pub retained_jobs: usize,
    pub reserved_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            workers: 2,
            retained_jobs: 16,
            reserved_bytes: 256 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Stats {
    pub queued: usize,
    pub active: usize,
    pub retained: usize,
    pub reserved_bytes: usize,
}
struct State {
    queue: VecDeque<Task>,
    stop: bool,
    active: usize,
    count: usize,
    bytes: usize,
    latest: BTreeMap<String, (Weak<AtomicBool>, Weak<Reservation>)>,
}
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    limits: Limits,
}
struct Reservation {
    shared: Weak<Shared>,
    bytes: usize,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some(s) = self.shared.upgrade() {
            let mut g = s.state.lock().unwrap_or_else(|e| e.into_inner());
            g.count -= 1;
            g.bytes -= self.bytes;
        }
    }
}
#[derive(Clone)]
pub struct Context {
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
}
impl Context {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }
    pub fn progress(&self, done: u64, total: u64) {
        self.total.store(total, Ordering::Relaxed);
        self.done.store(done.min(total), Ordering::Release);
    }
}
struct Cell {
    result: Mutex<Option<Result<Value, Error>>>,
}
struct Task {
    priority: Priority,
    work: Work,
    context: Context,
    cell: Arc<Cell>,
    _permit: Arc<Reservation>,
}
pub struct Ticket<T> {
    cell: Arc<Cell>,
    context: Context,
    epoch: u64,
    permit: Option<Arc<Reservation>>,
    _type: std::marker::PhantomData<T>,
}
impl<T: Send + 'static> Ticket<T> {
    pub fn cancel(&self) {
        self.context.cancel.store(true, Ordering::Release);
    }
    pub fn progress(&self) -> (u64, u64) {
        (
            self.context.done.load(Ordering::Acquire),
            self.context.total.load(Ordering::Relaxed),
        )
    }
    pub fn poll(&mut self, current_epoch: u64) -> Option<Result<T, Error>> {
        let result = self
            .cell
            .result
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()?;
        self.permit.take();
        if current_epoch != self.epoch {
            return Some(Err(Error::Stale));
        }
        Some(result.and_then(|v| {
            v.downcast::<T>()
                .map(|v| *v)
                .map_err(|_| Error::Failed("result type mismatch".into()))
        }))
    }
}
impl<T> Drop for Ticket<T> {
    fn drop(&mut self) {
        self.context.cancel.store(true, Ordering::Release);
    }
}
pub struct Scheduler {
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
}
impl Scheduler {
    pub fn new(limits: Limits) -> Result<Self, Error> {
        if !(1..=4).contains(&limits.workers)
            || !(1..=64).contains(&limits.retained_jobs)
            || !(1024 * 1024..=1024 * 1024 * 1024).contains(&limits.reserved_bytes)
        {
            return Err(Error::Invalid);
        }
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                queue: VecDeque::new(),
                stop: false,
                active: 0,
                count: 0,
                bytes: 0,
                latest: BTreeMap::new(),
            }),
            wake: Condvar::new(),
            limits,
        });
        let mut pool = Self {
            shared,
            workers: vec![],
        };
        for i in 0..limits.workers {
            let s = pool.shared.clone();
            let worker = std::thread::Builder::new()
                .name(format!("soniclaw-job-{i}"))
                .spawn(move || loop {
                    let task = {
                        let mut g = s.state.lock().unwrap_or_else(|e| e.into_inner());
                        while g.queue.is_empty() && !g.stop {
                            g = s.wake.wait(g).unwrap_or_else(|e| e.into_inner());
                        }
                        if g.stop {
                            return;
                        }
                        let index = g
                            .queue
                            .iter()
                            .enumerate()
                            .min_by_key(|(_, t)| t.priority)
                            .unwrap()
                            .0;
                        g.active += 1;
                        g.queue.remove(index).unwrap()
                    };
                    let result = if task.context.cancelled() {
                        Err(Error::Cancelled)
                    } else {
                        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            (task.work)(task.context.clone())
                        })) {
                            Ok(r) => r.map_err(Error::Failed),
                            Err(_) => Err(Error::Failed("worker panicked".into())),
                        }
                    };
                    let result = if task.context.cancelled() {
                        Err(Error::Cancelled)
                    } else {
                        result
                    };
                    *task.cell.result.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
                    {
                        let mut g = s.state.lock().unwrap_or_else(|e| e.into_inner());
                        g.active -= 1;
                    }
                })
                .map_err(|e| Error::Failed(e.to_string()))?;
            pool.workers.push(worker);
        }
        Ok(pool)
    }
    /// Reservation includes queued, active and unconsumed result handles; estimates must include captures/results.
    pub fn submit<T: Send + 'static>(
        &self,
        key: &str,
        epoch: u64,
        priority: Priority,
        reserved_bytes: usize,
        work: impl FnOnce(Context) -> Result<T, String> + Send + 'static,
    ) -> Result<Ticket<T>, Error> {
        if key.is_empty() || key.len() > 128 || reserved_bytes == 0 {
            return Err(Error::Invalid);
        }
        let context = Context {
            cancel: Arc::new(AtomicBool::new(false)),
            done: Default::default(),
            total: Default::default(),
        };
        let cell = Arc::new(Cell {
            result: Mutex::new(None),
        });
        let mut g = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if g.stop
            || g.count >= self.shared.limits.retained_jobs
            || reserved_bytes > self.shared.limits.reserved_bytes.saturating_sub(g.bytes)
        {
            return Err(Error::Full);
        }
        g.latest.retain(|_, (_, permit)| permit.strong_count() > 0);
        if let Some(old) = g.latest.get(key).and_then(|(cancel, _)| cancel.upgrade()) {
            old.store(true, Ordering::Release);
        }
        g.count += 1;
        g.bytes += reserved_bytes;
        let permit = Arc::new(Reservation {
            shared: Arc::downgrade(&self.shared),
            bytes: reserved_bytes,
        });
        g.latest.insert(
            key.into(),
            (Arc::downgrade(&context.cancel), Arc::downgrade(&permit)),
        );
        g.queue.push_back(Task {
            priority,
            work: Box::new(move |c| work(c).map(|v| Box::new(v) as Value)),
            context: context.clone(),
            cell: cell.clone(),
            _permit: permit.clone(),
        });
        drop(g);
        self.shared.wake.notify_one();
        Ok(Ticket {
            cell,
            context,
            epoch,
            permit: Some(permit),
            _type: Default::default(),
        })
    }
    pub fn stats(&self) -> Stats {
        let g = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        Stats {
            queued: g.queue.len(),
            active: g.active,
            retained: g.count,
            reserved_bytes: g.bytes,
        }
    }
}
impl Drop for Scheduler {
    fn drop(&mut self) {
        let queued = {
            let mut g = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            g.stop = true;
            for c in g.latest.values().filter_map(|(cancel, _)| cancel.upgrade()) {
                c.store(true, Ordering::Release);
            }
            std::mem::take(&mut g.queue)
        };
        for task in &queued {
            *task.cell.result.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(Err(Error::Cancelled));
        }
        drop(queued);
        self.shared.wake.notify_all();
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn get<T: Send + 'static>(t: &mut Ticket<T>, epoch: u64) -> Result<T, Error> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(r) = t.poll(epoch) {
                return r;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
    #[test]
    fn capacity_progress_stale_cancellation_and_shutdown() {
        let pool = Scheduler::new(Limits {
            workers: 1,
            retained_jobs: 2,
            reserved_bytes: 1024 * 1024,
        })
        .unwrap();
        let mut a = pool
            .submit("first", 1, Priority::Normal, 600000, |c| {
                c.progress(2, 3);
                Ok(42u32)
            })
            .unwrap();
        assert!(matches!(
            pool.submit("too-big", 1, Priority::Normal, 500000, |_| Ok(())),
            Err(Error::Full)
        ));
        assert_eq!(get(&mut a, 1).unwrap(), 42);
        assert_eq!(a.progress(), (2, 3));
        let mut stale = pool
            .submit("stale", 3, Priority::Normal, 1000, |_| Ok(7))
            .unwrap();
        assert_eq!(get(&mut stale, 4), Err(Error::Stale));
        let mut waiting = pool
            .submit("block", 1, Priority::Normal, 1000, |c| {
                while !c.cancelled() {
                    std::thread::yield_now();
                }
                Ok(())
            })
            .unwrap();
        let mut queued = pool
            .submit("queued", 1, Priority::Normal, 1000, |_| Ok(1))
            .unwrap();
        drop(pool);
        assert_eq!(get(&mut waiting, 1), Err(Error::Cancelled));
        assert_eq!(get(&mut queued, 1), Err(Error::Cancelled));
    }
    #[test]
    fn priority_fifo_and_replacement() {
        let pool = Scheduler::new(Limits {
            workers: 1,
            ..Default::default()
        })
        .unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut block = pool
            .submit("block", 0, Priority::Normal, 1, move |_| {
                rx.recv().unwrap();
                Ok(())
            })
            .unwrap();
        while pool.stats().active == 0 {
            std::thread::yield_now();
        }
        let order = Arc::new(Mutex::new(vec![]));
        let o = order.clone();
        let mut low = pool
            .submit("low", 0, Priority::Maintenance, 1, move |_| {
                o.lock().unwrap().push(2);
                Ok(())
            })
            .unwrap();
        let o = order.clone();
        let mut high = pool
            .submit("high", 0, Priority::Interactive, 1, move |_| {
                o.lock().unwrap().push(1);
                Ok(())
            })
            .unwrap();
        let mut old = pool
            .submit("same", 1, Priority::Normal, 1, |_| Ok(1))
            .unwrap();
        let mut new = pool
            .submit("same", 2, Priority::Normal, 1, |_| Ok(2))
            .unwrap();
        tx.send(()).unwrap();
        get(&mut block, 0).unwrap();
        get(&mut high, 0).unwrap();
        get(&mut low, 0).unwrap();
        assert_eq!(*order.lock().unwrap(), [1, 2]);
        assert_eq!(get(&mut old, 1), Err(Error::Cancelled));
        assert_eq!(get(&mut new, 2).unwrap(), 2);
    }
    #[test]
    fn consumed_tickets_do_not_retain_unbounded_replacement_keys() {
        let pool = Scheduler::new(Limits {
            workers: 1,
            ..Default::default()
        })
        .unwrap();
        let mut tickets = vec![];
        for i in 0..128 {
            let mut ticket = pool
                .submit(&format!("finished-{i}"), 0, Priority::Normal, 1, |_| Ok(()))
                .unwrap();
            get(&mut ticket, 0).unwrap();
            tickets.push(ticket);
        }
        let keys = pool.shared.state.lock().unwrap().latest.len();
        assert!(
            keys <= 2,
            "consumed tickets must not keep replacement keys alive: {keys}"
        );
    }
}
