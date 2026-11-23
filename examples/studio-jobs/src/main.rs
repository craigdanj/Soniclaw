use soniclaw::studio::{
    jobs::{Limits, Priority, Scheduler, Ticket},
    project::ProjectError,
};
fn wait<T: Send + 'static>(
    t: &mut Ticket<T>,
    epoch: u64,
) -> Result<T, soniclaw::studio::jobs::Error> {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(v) = t.poll(epoch) {
            return v;
        }
        assert!(std::time::Instant::now() < until);
        std::thread::yield_now();
    }
}
fn main() -> Result<(), ProjectError> {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: studio-jobs NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&root)?;
    let scheduler = Scheduler::new(Limits {
        workers: 1,
        retained_jobs: 4,
        reserved_bytes: 1024 * 1024,
    })?;
    let mut job = scheduler.submit("analysis", 1, Priority::Interactive, 800000, |c| {
        c.progress(100, 100);
        Ok(1234u32)
    })?;
    assert!(scheduler
        .submit("over-budget", 1, Priority::Normal, 800000, |_| Ok(()))
        .is_err());
    assert_eq!(wait(&mut job, 1)?, 1234);
    let mut stale = scheduler.submit("old-project", 2, Priority::Normal, 100, |_| Ok(123))?;
    assert!(wait(&mut stale, 3).is_err());
    let mut cancelled = scheduler.submit("cancel", 3, Priority::Normal, 100, |c| {
        while !c.cancelled() {
            std::thread::yield_now();
        }
        Ok(())
    })?;
    cancelled.cancel();
    assert!(wait(&mut cancelled, 3).is_err());
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"typed_result":1234,"over_budget_rejected":true,"stale_epoch_rejected":true,"cooperative_cancelled":true,"progress":job.progress(),"remaining_reserved":scheduler.stats().reserved_bytes}),
        )?,
    )?;
    Ok(())
}
