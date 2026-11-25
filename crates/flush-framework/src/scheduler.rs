//! Background jobs on the async-std runtime: one-shot delays and fixed
//! intervals, cancellable by handle or all at once.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Cancels its job when [`JobHandle::cancel`] is called. Dropping the handle
/// leaves the job running.
#[derive(Debug, Clone)]
pub struct JobHandle {
    cancelled: Arc<AtomicBool>,
}

impl JobHandle {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Default, Clone)]
pub struct Scheduler {
    jobs: Arc<Mutex<Vec<JobHandle>>>,
    paused: Arc<AtomicBool>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self::default()
    }

    fn register(&self) -> JobHandle {
        let handle = JobHandle {
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        jobs.retain(|j| !j.is_cancelled());
        jobs.push(handle.clone());
        handle
    }

    /// Runs `job` once after `delay`, unless cancelled first.
    pub fn after<F, Fut>(&self, delay: Duration, job: F) -> JobHandle
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let handle = self.register();
        let flag = handle.clone();
        let paused = self.paused.clone();
        async_std::task::spawn(async move {
            async_std::task::sleep(delay).await;
            if !flag.is_cancelled() && !paused.load(Ordering::SeqCst) {
                job().await;
            }
            flag.cancel();
        });
        handle
    }

    /// Runs `job` every `period` until cancelled. The first run happens
    /// after one full period.
    pub fn every<F, Fut>(&self, period: Duration, job: F) -> JobHandle
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let handle = self.register();
        let flag = handle.clone();
        let paused = self.paused.clone();
        async_std::task::spawn(async move {
            loop {
                async_std::task::sleep(period).await;
                if flag.is_cancelled() {
                    break;
                }
                if paused.load(Ordering::SeqCst) {
                    continue;
                }
                job().await;
            }
        });
        handle
    }

    /// Jobs registered and not yet cancelled or finished.
    pub fn active(&self) -> usize {
        let jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        jobs.iter().filter(|j| !j.is_cancelled()).count()
    }

    /// While paused, due jobs are skipped (a one-shot job that comes due is
    /// dropped). A sharded bot pauses the scheduler on processes that should
    /// not run jobs.
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    pub fn cancel_all(&self) {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        for job in jobs.drain(..) {
            job.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    #[async_std::test]
    async fn interval_runs_until_cancelled() {
        let scheduler = Scheduler::new();
        let count = Arc::new(AtomicU32::new(0));
        let c = count.clone();
        let handle = scheduler.every(Duration::from_millis(10), move || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
            }
        });
        async_std::task::sleep(Duration::from_millis(80)).await;
        handle.cancel();
        let seen = count.load(Ordering::SeqCst);
        assert!(seen >= 2, "ran {seen} times");
        async_std::task::sleep(Duration::from_millis(40)).await;
        assert!(count.load(Ordering::SeqCst) <= seen + 1);
        assert_eq!(scheduler.active(), 0);
    }

    #[async_std::test]
    async fn cancelled_one_shot_never_runs() {
        let scheduler = Scheduler::new();
        let ran = Arc::new(AtomicBool::new(false));
        let r = ran.clone();
        let handle = scheduler.after(Duration::from_millis(20), move || async move {
            r.store(true, Ordering::SeqCst);
        });
        handle.cancel();
        async_std::task::sleep(Duration::from_millis(50)).await;
        assert!(!ran.load(Ordering::SeqCst));
    }
}
