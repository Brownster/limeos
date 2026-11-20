//! Restarts a failed background task after a capped, jittered wait instead of
//! ending core. Only startup errors (configuration, store, bind) stop the
//! process; a task that fails later is reported in `Health::degraded` until a
//! successful pass clears it.
//!
//! What this contains is a task that returns: an error, or an unexpected
//! `Ok`. It does not contain a panic. Release builds use `panic = "abort"`,
//! so a panic anywhere, in a supervised task or not, aborts the whole
//! daemon at once, with no unwinding and nothing here running; systemd then
//! restarts core (`Restart=on-failure`). A task that hangs without returning
//! is not seen here either.
use limeos_contracts::{DegradedDetail, DegradedSubsystem, Subsystem};
use limeos_domain::{Error, ErrorCode, Result};
use std::{
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Failures further apart than this start a new run, so the wait and the log
/// throttle start again from the beginning.
const STREAK_RESET: Duration = Duration::from_secs(600);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Backoff {
    pub(crate) base: Duration,
    pub(crate) cap: Duration,
}
impl Backoff {
    /// 1 s after the first failure, doubling to at most a minute.
    pub(crate) const STANDARD: Self = Self {
        base: Duration::from_secs(1),
        cap: Duration::from_secs(60),
    };
    /// The wait before restart `failures` of a run: `base` doubled per earlier
    /// failure, capped, then a random point in its upper half so restarts of
    /// several tasks do not stay in step.
    pub(crate) fn delay(&self, failures: u32, entropy: u64) -> Duration {
        let doubled = self
            .base
            .saturating_mul(1 << failures.saturating_sub(1).min(16))
            .min(self.cap);
        let half = doubled / 2;
        let span = u64::try_from(half.as_millis()).unwrap_or(u64::MAX);
        half + Duration::from_millis(entropy % span.saturating_add(1))
    }
}
fn entropy() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    // Randomly keyed per process; no generator dependency is needed for jitter.
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    );
    hasher.finish()
}

#[derive(Default)]
struct Record {
    current: Option<DegradedSubsystem>,
    streak: u32,
    last_failure: Option<Instant>,
    last_code: Option<ErrorCode>,
    logged: bool,
}
/// Health of one supervised subsystem, shared with the status surface.
#[derive(Clone, Default)]
pub(crate) struct Health(Arc<Mutex<Record>>);
/// What a failure changed, for the caller's log line.
struct Failure {
    failures: u32,
    log: bool,
}
impl Health {
    fn failed(&self, subsystem: Subsystem, code: ErrorCode, at: i64) -> Failure {
        let Ok(mut record) = self.0.lock() else {
            return Failure {
                failures: 1,
                log: true,
            };
        };
        let now = Instant::now();
        if record
            .last_failure
            .is_none_or(|last| now.duration_since(last) >= STREAK_RESET)
        {
            record.streak = 0;
        }
        record.streak = record.streak.saturating_add(1);
        record.last_failure = Some(now);
        let changed = record.last_code != Some(code);
        record.last_code = Some(code);
        let since = record
            .current
            .as_ref()
            .and_then(|c| c.detail.as_ref())
            .filter(|d| d.code == code)
            .map_or(at, |d| d.since);
        // Repeats are logged at failures 1, 2, 4, 8, ... of a run, so a
        // persistent error costs a few lines a day, not one per restart.
        let log = changed || record.streak.is_power_of_two();
        record.logged = log;
        record.current = Some(DegradedSubsystem {
            subsystem,
            detail: Some(DegradedDetail {
                code,
                since,
                count: record.streak,
            }),
        });
        Failure {
            failures: record.streak,
            log,
        }
    }
    /// A successful pass: clears the degraded state. Returns whether the
    /// failure it clears was logged, so the recovery is logged to match.
    pub(crate) fn recovered(&self) -> bool {
        let Ok(mut record) = self.0.lock() else {
            return false;
        };
        let logged = record.current.take().is_some() && record.logged;
        record.logged = false;
        logged
    }
    pub(crate) fn degraded(&self) -> Option<DegradedSubsystem> {
        self.0.lock().ok().and_then(|record| record.current.clone())
    }
}

/// Runs `start` again after each failure, forever. A run that returns, with an
/// error or without one, is a failure: these tasks are meant never to end.
/// A panic is not a return: it aborts the process (`panic = "abort"`).
pub(crate) async fn supervise<F, Fut>(
    subsystem: Subsystem,
    health: Health,
    backoff: Backoff,
    mut start: F,
) where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<()>>,
{
    loop {
        let error = match start().await {
            Ok(()) => Error(ErrorCode::Unavailable),
            Err(error) => error,
        };
        let failure = health.failed(subsystem, error.0, super::now());
        let wait = backoff.delay(failure.failures, entropy());
        if failure.log {
            tracing::error!(
                ?subsystem,
                code = ?error.0,
                failures = failure.failures,
                retry_ms = u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
                "background task failed; core keeps serving and restarts it"
            );
        }
        tokio::time::sleep(wait).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_to_its_cap_and_jitters_within_the_upper_half() {
        let backoff = Backoff::STANDARD;
        for (failures, full) in [(1, 1), (2, 2), (3, 4), (6, 32), (7, 60), (40, 60)] {
            let full = Duration::from_secs(full);
            for entropy in [0, 1, 7_919, u64::MAX] {
                let wait = backoff.delay(failures, entropy);
                assert!(wait >= full / 2 && wait <= full, "{failures} {wait:?}");
            }
        }
        assert_eq!(backoff.delay(0, 0), Duration::from_millis(500));
    }

    #[test]
    fn repeated_failures_log_logarithmically_and_recovery_clears_the_state() {
        let health = Health::default();
        let logged: Vec<_> = (1..=64)
            .map(|at| {
                health
                    .failed(
                        Subsystem::ObservationRuntime,
                        ErrorCode::StateNotDurable,
                        at,
                    )
                    .log
            })
            .collect();
        assert_eq!(logged.iter().filter(|log| **log).count(), 7);
        let current = health.degraded().unwrap().detail.unwrap();
        assert_eq!((current.since, current.count), (1, 64));
        // A different error is always logged and starts a new `since`.
        let changed = health.failed(Subsystem::ObservationRuntime, ErrorCode::InvalidInput, 70);
        assert!(changed.log);
        assert_eq!(health.degraded().unwrap().detail.unwrap().since, 70);
        assert!(health.recovered());
        assert_eq!(health.degraded(), None);
        assert!(!health.recovered());
        // The run continues after one good pass, so a flapping task keeps
        // its longer wait and quiet log, and its recovery is not logged either.
        let next = health.failed(Subsystem::ObservationRuntime, ErrorCode::InvalidInput, 80);
        assert_eq!(next.failures, 66);
        assert!(!next.log);
        assert_eq!(health.degraded().unwrap().detail.unwrap().since, 80);
        assert!(!health.recovered());
    }
}
