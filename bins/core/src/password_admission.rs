//! Admission to the password workers and the comparison budget for login
//! names that have no account.
use limeos_domain::{Error, ErrorCode, Result};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// How long a password check waits for a busy worker, inside the 10-second
/// HTTP request deadline.
const WAIT: Duration = Duration::from_secs(2);
/// Checks that may wait for a worker at once; more are refused immediately.
const WAITERS: usize = 8;
/// Comparisons a minute for login names with no account, across all of them.
const UNKNOWN_NAMES: u32 = 120;
const MINUTE: Duration = Duration::from_secs(60);

pub(crate) struct PasswordWorkers {
    workers: Arc<Semaphore>,
    /// Workers plus waiters: bounds how many checks hold or wait for one.
    admitted: Arc<Semaphore>,
    wait: Duration,
    unknown_names: Mutex<(Option<Instant>, u32)>,
    unknown_limit: u32,
}
/// A worker held for one check, with its admission place.
pub(crate) struct Admission {
    _worker: OwnedSemaphorePermit,
    _admitted: OwnedSemaphorePermit,
}
impl PasswordWorkers {
    pub(crate) fn new(workers: usize) -> Self {
        Self::with_limits(workers, WAITERS, WAIT, UNKNOWN_NAMES)
    }
    pub(crate) fn with_limits(
        workers: usize,
        waiters: usize,
        wait: Duration,
        unknown_limit: u32,
    ) -> Self {
        Self {
            workers: Arc::new(Semaphore::new(workers)),
            admitted: Arc::new(Semaphore::new(workers + waiters)),
            wait,
            unknown_names: Mutex::new((None, 0)),
            unknown_limit,
        }
    }
    /// Take a worker, waiting in FIFO order (Tokio's semaphore is fair) for
    /// at most the wait time. Too many waiters, or the wait running out, is
    /// `Overloaded`; either way no comparison happened.
    pub(crate) async fn admit(&self) -> Result<Admission> {
        let admitted = self
            .admitted
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error(ErrorCode::Overloaded))?;
        let worker = tokio::time::timeout(self.wait, self.workers.clone().acquire_owned())
            .await
            .map_err(|_| Error(ErrorCode::Overloaded))?
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        Ok(Admission {
            _worker: worker,
            _admitted: admitted,
        })
    }
    /// One comparison for a login name with no account. Existing accounts
    /// never draw on this, so no traffic against other names can refuse
    /// them. Once it is spent for the minute, such a name is refused without
    /// a comparison: while someone floods the form, that reveals which names
    /// do not exist (recorded in the operations guide).
    pub(crate) fn take_unknown_name(&self, now: Instant) -> bool {
        let Ok(mut budget) = self.unknown_names.lock() else {
            return false;
        };
        if budget
            .0
            .is_none_or(|start| now.saturating_duration_since(start) >= MINUTE)
        {
            *budget = (Some(now), 0);
        }
        if budget.1 >= self.unknown_limit {
            return false;
        }
        budget.1 += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Short real waits: this crate's tests run without a paused clock.
    const TEST_WAIT: Duration = Duration::from_millis(200);

    #[tokio::test]
    async fn a_waiting_check_gets_the_worker_that_frees_up() {
        let workers = Arc::new(PasswordWorkers::with_limits(1, 2, TEST_WAIT, 1));
        let held = workers.admit().await.unwrap();
        let waiting = tokio::spawn({
            let workers = workers.clone();
            async move { workers.admit().await.map(|_| ()) }
        });
        tokio::time::sleep(TEST_WAIT / 4).await;
        assert!(!waiting.is_finished());
        drop(held);
        assert!(waiting.await.unwrap().is_ok());
    }
    #[tokio::test]
    async fn a_wait_with_no_free_worker_times_out_as_overloaded() {
        let workers = PasswordWorkers::with_limits(1, 2, TEST_WAIT, 1);
        let _held = workers.admit().await.unwrap();
        let started = Instant::now();
        assert_eq!(
            workers.admit().await.err(),
            Some(Error(ErrorCode::Overloaded))
        );
        assert!(started.elapsed() >= TEST_WAIT);
        // The timed-out waiter gave its place back.
        assert_eq!(workers.admitted.available_permits(), 2);
    }
    #[tokio::test]
    async fn waiters_are_bounded_and_served_in_order() {
        let workers = Arc::new(PasswordWorkers::with_limits(
            1,
            2,
            Duration::from_secs(5),
            1,
        ));
        let held = workers.admit().await.unwrap();
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut waiting = Vec::new();
        for i in 0..2 {
            let (queued, order) = (workers.clone(), order.clone());
            waiting.push(tokio::spawn(async move {
                let admission = queued.admit().await.unwrap();
                order.lock().unwrap().push(i);
                drop(admission);
            }));
            // Let this waiter queue before the next one.
            while workers.admitted.available_permits() > 2 - i - 1 {
                tokio::task::yield_now().await;
            }
        }
        // One worker busy, two waiting: a third check is refused at once.
        let started = Instant::now();
        assert_eq!(
            workers.admit().await.err(),
            Some(Error(ErrorCode::Overloaded))
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(held);
        for task in waiting {
            task.await.unwrap();
        }
        assert_eq!(*order.lock().unwrap(), [0, 1]);
    }
    #[test]
    fn names_with_no_account_share_a_per_minute_budget() {
        let workers = PasswordWorkers::with_limits(2, 8, WAIT, 3);
        let start = Instant::now();
        for _ in 0..3 {
            assert!(workers.take_unknown_name(start));
        }
        assert!(!workers.take_unknown_name(start + Duration::from_secs(59)));
        assert!(workers.take_unknown_name(start + MINUTE));
    }
}
