//! Background jobs on the async-std runtime: one-shot delays and fixed
//! intervals, cancellable by handle or all at once.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Cancels its job when [`JobHandle::cancel`] is called. Dropping the handle
/// leaves the job running. A cancelled job wakes immediately instead of
/// sleeping until its original deadline.
#[derive(Debug, Clone)]
pub struct JobHandle {
    cancelled: Arc<AtomicBool>,
    wake: async_std::channel::Sender<()>,
}

impl JobHandle {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let _ = self.wake.try_send(());
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

    fn register(&self) -> (JobHandle, async_std::channel::Receiver<()>) {
        let (wake, wake_rx) = async_std::channel::bounded(1);
        let handle = JobHandle {
            cancelled: Arc::new(AtomicBool::new(false)),
            wake,
        };
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        jobs.retain(|j| !j.is_cancelled());
        jobs.push(handle.clone());
        (handle, wake_rx)
    }

    /// Runs `job` once after `delay`, unless cancelled first.
    pub fn after<F, Fut>(&self, delay: Duration, job: F) -> JobHandle
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let (handle, wake) = self.register();
        let flag = handle.clone();
        let paused = self.paused.clone();
        async_std::task::spawn(async move {
            let _ = async_std::future::timeout(delay, wake.recv()).await;
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
        let (handle, wake) = self.register();
        let flag = handle.clone();
        let paused = self.paused.clone();
        async_std::task::spawn(async move {
            loop {
                let _ = async_std::future::timeout(period, wake.recv()).await;
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

    /// Runs `job` on a cron schedule in `timezone` (IANA name, e.g.
    /// `"America/Bahia"` or `"UTC"`), with `RunOnce` missed-run policy.
    /// The first run happens on the next matching instant.
    pub fn cron<F, Fut>(
        &self,
        expression: &str,
        timezone: &str,
        job: F,
    ) -> Result<JobHandle, crate::cron::CronError>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.cron_with_policy(
            expression,
            timezone,
            crate::cron::MissedRunPolicy::RunOnce,
            job,
        )
    }

    /// [`Scheduler::cron`] with an explicit [`MissedRunPolicy`]: `Skip`
    /// drops an occurrence more than 60s late, `RunOnce` fires it on wake.
    /// Both re-arm on the next matching instant.
    pub fn cron_with_policy<F, Fut>(
        &self,
        expression: &str,
        timezone: &str,
        missed_run: crate::cron::MissedRunPolicy,
        job: F,
    ) -> Result<JobHandle, crate::cron::CronError>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        use crate::cron::{next_cron, MissedRunPolicy};
        let now_ms = || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0)
        };
        let expression = expression.to_string();
        crate::cron::parse_cron(&expression)?;
        let timezone = timezone.to_string();
        crate::cron::assert_timezone(&timezone)?;
        // A schedule with no matching instant is refused at registration,
        // not silently inside the background task.
        let mut due = next_cron(&expression, &timezone, now_ms())?;
        let (handle, wake) = self.register();
        let flag = handle.clone();
        let paused = self.paused.clone();
        async_std::task::spawn(async move {
            let mut after;
            loop {
                let delay = (due - now_ms()).max(0) as u64;
                let _ = async_std::future::timeout(Duration::from_millis(delay), wake.recv()).await;
                if flag.is_cancelled() {
                    break;
                }
                let now = now_ms();
                if now < due {
                    // Woke before the deadline (e.g. the clock moved back):
                    // keep waiting for the same occurrence.
                    continue;
                }
                if paused.load(Ordering::SeqCst) {
                    after = now;
                } else if missed_run == MissedRunPolicy::Skip && due + 60_000 <= now {
                    // Mirrors Yuzuha: skip to the next slot after this minute.
                    after = (now / 60_000) * 60_000 - 1;
                } else {
                    job().await;
                    after = now_ms();
                }
                due = match next_cron(&expression, &timezone, after) {
                    Ok(due) => due,
                    Err(e) => {
                        log::warn!("flush: cron job ended: {e}");
                        break;
                    }
                };
            }
            flag.cancel();
        });
        Ok(handle)
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

    #[async_std::test]
    async fn cancelled_cron_job_drops_its_closure_promptly() {
        struct OnDrop(Arc<AtomicBool>);
        impl Drop for OnDrop {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let scheduler = Scheduler::new();
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = OnDrop(dropped.clone());
        // The next occurrence can be months away; the job must not hold its
        // resources until then once cancelled.
        let handle = scheduler
            .cron("59 23 31 12 *", "UTC", move || {
                let _guard = &guard;
                async move {}
            })
            .unwrap();
        async_std::task::sleep(Duration::from_millis(50)).await;
        handle.cancel();
        async_std::task::sleep(Duration::from_millis(200)).await;
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(scheduler.active(), 0);
    }

    #[async_std::test]
    async fn cron_validates_expression_and_timezone() {
        let scheduler = Scheduler::new();
        assert!(scheduler.cron("not cron", "UTC", || async move {}).is_err());
        assert!(scheduler
            .cron("* * * * *", "Not/AZone", || async move {})
            .is_err());
        let handle = scheduler.cron("* * * * *", "UTC", || async move {}).unwrap();
        assert_eq!(scheduler.active(), 1);
        handle.cancel();
        assert_eq!(scheduler.active(), 0);
    }
}
