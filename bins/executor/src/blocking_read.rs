//! Synchronous device and filesystem reads, run off the service's async
//! thread so a deadline can abandon them. A read stuck in the kernel (a
//! failing disk can leave it in uninterruptible sleep) cannot be cancelled,
//! and restarting the service would not free it either: the thread cannot
//! exit, so it stays in the unit's cgroup (counting against `TasksMax`)
//! whatever happens to the process. Its thread is left to finish and
//! counted; once `limit` are stranded the service refuses further reads
//! (so stuck threads stay bounded) and keeps serving everything else,
//! reporting the stranded reads, and since when, through its health answer.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Too many earlier reads are still stuck; nothing was started.
    Stranded,
    /// The read did not finish by its deadline and was abandoned.
    Deadline,
    /// No thread could be started, or the read ended without an answer.
    Unavailable,
}

/// Reads still stuck after their deadline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stranded {
    pub count: usize,
    /// When the oldest was abandoned (Unix seconds).
    pub since: i64,
}

/// Each started read, by id: `None` while its request still waits for it,
/// the Unix time it was abandoned once the request gave up. A read leaves
/// the map when it returns.
type Jobs = Arc<Mutex<BTreeMap<u64, Option<i64>>>>;

pub struct BlockingReads {
    limit: usize,
    jobs: Jobs,
    next: Mutex<u64>,
    clock: fn() -> i64,
}
/// Removes a job when its read returns, or unwinds in a test.
struct Finished {
    id: u64,
    jobs: Jobs,
}
impl Drop for Finished {
    fn drop(&mut self) {
        self.jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
    }
}
/// Abandons the job if its waiter goes away first: on the deadline, or when
/// the request itself is dropped.
struct Waiting {
    id: u64,
    jobs: Jobs,
    clock: fn() -> i64,
}
impl Drop for Waiting {
    fn drop(&mut self) {
        let mut jobs = self.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        // Absent: the read already returned.
        if let Some(abandoned) = jobs.get_mut(&self.id) {
            abandoned.get_or_insert_with(self.clock);
        }
    }
}
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}
impl BlockingReads {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            jobs: Arc::default(),
            next: Mutex::new(0),
            clock: unix_now,
        }
    }
    /// Reads that missed their deadline and have not returned yet.
    pub fn stranded(&self) -> Option<Stranded> {
        let jobs = self.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        let abandoned = jobs.values().flatten();
        Some(Stranded {
            count: abandoned.clone().count(),
            since: *abandoned.min()?,
        })
    }
    /// Whether another read may start.
    pub fn accepting(&self) -> bool {
        self.stranded()
            .is_none_or(|stranded| stranded.count < self.limit)
    }
    /// Runs `work` on its own thread and waits at most `deadline` for it.
    pub async fn run<T: Send + 'static>(
        &self,
        name: &str,
        deadline: Duration,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, Refusal> {
        if !self.accepting() {
            return Err(Refusal::Stranded);
        }
        let id = {
            let mut next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
            *next += 1;
            *next
        };
        self.jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, None);
        let finished = Finished {
            id,
            jobs: self.jobs.clone(),
        };
        let waiting = Waiting {
            id,
            jobs: self.jobs.clone(),
            clock: self.clock,
        };
        let (sender, receiver) = tokio::sync::oneshot::channel();
        if std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let value = work();
                drop(finished);
                let _ = sender.send(value);
            })
            .is_err()
        {
            // The closure, and with it `finished`, was dropped unrun.
            return Err(Refusal::Unavailable);
        }
        let _waiting = waiting;
        match tokio::time::timeout(deadline, receiver).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_)) => Err(Refusal::Unavailable),
            Err(_) => Err(Refusal::Deadline),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    async fn until(condition: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_read_that_never_returns_is_abandoned_counted_with_its_time_and_released_when_it_ends()
     {
        let mut reads = BlockingReads::new(1);
        reads.clock = || 1_700_000_000;
        assert_eq!(
            reads.run("quick", Duration::from_secs(5), || 7).await,
            Ok(7)
        );
        assert_eq!(reads.stranded(), None);
        let (release, blocked) = std::sync::mpsc::channel::<()>();
        let started = std::time::Instant::now();
        assert_eq!(
            reads
                .run("stuck", Duration::from_millis(50), move || blocked.recv())
                .await
                .map(|_| ()),
            Err(Refusal::Deadline)
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(
            reads.stranded(),
            Some(Stranded {
                count: 1,
                since: 1_700_000_000
            })
        );
        assert!(!reads.accepting());
        // At the bound no further thread is started.
        let ran = Arc::new(AtomicUsize::new(0));
        let counted = ran.clone();
        assert_eq!(
            reads
                .run("refused", Duration::from_secs(5), move || {
                    counted.fetch_add(1, Ordering::SeqCst)
                })
                .await,
            Err(Refusal::Stranded)
        );
        assert_eq!(ran.load(Ordering::SeqCst), 0);
        release.send(()).unwrap();
        until(|| reads.accepting()).await;
        assert_eq!(reads.stranded(), None);
        assert_eq!(
            reads.run("again", Duration::from_secs(5), || 8).await,
            Ok(8)
        );
    }

    #[tokio::test]
    async fn a_dropped_request_still_counts_its_stuck_read_and_the_oldest_time_is_reported() {
        let mut reads = BlockingReads::new(2);
        reads.clock = || 100;
        let (release, blocked) = std::sync::mpsc::channel::<()>();
        let request = reads.run("dropped", Duration::from_secs(60), move || blocked.recv());
        assert!(
            tokio::time::timeout(Duration::from_millis(20), request)
                .await
                .is_err()
        );
        reads.clock = || 200;
        let (_keep, second) = std::sync::mpsc::channel::<()>();
        assert_eq!(
            reads
                .run("second", Duration::from_millis(20), move || second.recv())
                .await
                .map(|_| ()),
            Err(Refusal::Deadline)
        );
        assert_eq!(
            reads.stranded(),
            Some(Stranded {
                count: 2,
                since: 100
            })
        );
        release.send(()).unwrap();
        until(|| reads.stranded().is_some_and(|s| s.count == 1)).await;
        assert_eq!(reads.stranded().unwrap().since, 200);
    }
}
