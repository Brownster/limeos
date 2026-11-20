//! Synchronous device and filesystem reads, run off the service's async
//! thread so a deadline can abandon them. A read stuck in the kernel (a
//! failing disk can leave it in uninterruptible sleep) cannot be cancelled;
//! its thread is left to finish and counted, and once `limit` are stranded
//! the service refuses further reads and reports itself unhealthy, which
//! withholds its watchdog keep-alive so systemd restarts it.
use std::{
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicUsize, Ordering},
    },
    time::Duration,
};

const RUNNING: u8 = 0;
const DONE: u8 = 1;
const ABANDONED: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Too many earlier reads are still stuck; nothing was started.
    Stranded,
    /// The read did not finish by its deadline and was abandoned.
    Deadline,
    /// No thread could be started, or the read ended without an answer.
    Unavailable,
}

pub struct BlockingReads {
    limit: usize,
    stranded: Arc<AtomicUsize>,
}
/// Marks a job finished when the read returns, or unwinds in a test.
struct Finished {
    state: Arc<AtomicU8>,
    stranded: Arc<AtomicUsize>,
}
impl Drop for Finished {
    fn drop(&mut self) {
        if self.state.swap(DONE, Ordering::AcqRel) == ABANDONED {
            self.stranded.fetch_sub(1, Ordering::AcqRel);
        }
    }
}
/// Abandons the job if its waiter goes away first: on the deadline, or when
/// the request itself is dropped.
struct Waiting {
    state: Arc<AtomicU8>,
    stranded: Arc<AtomicUsize>,
}
impl Drop for Waiting {
    fn drop(&mut self) {
        if self.state.load(Ordering::Acquire) == DONE {
            return;
        }
        // Counted before it is marked, so the thread's decrement can never
        // run first and wrap the counter.
        self.stranded.fetch_add(1, Ordering::AcqRel);
        if self
            .state
            .compare_exchange(RUNNING, ABANDONED, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            self.stranded.fetch_sub(1, Ordering::AcqRel);
        }
    }
}
impl BlockingReads {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            stranded: Arc::default(),
        }
    }
    /// Reads that missed their deadline and have not returned yet.
    pub fn stranded(&self) -> usize {
        self.stranded.load(Ordering::Acquire)
    }
    pub fn healthy(&self) -> bool {
        self.stranded() < self.limit
    }
    /// Runs `work` on its own thread and waits at most `deadline` for it.
    pub async fn run<T: Send + 'static>(
        &self,
        name: &str,
        deadline: Duration,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, Refusal> {
        if !self.healthy() {
            return Err(Refusal::Stranded);
        }
        let state = Arc::new(AtomicU8::new(RUNNING));
        let finished = Finished {
            state: state.clone(),
            stranded: self.stranded.clone(),
        };
        let (sender, receiver) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let value = work();
                drop(finished);
                let _ = sender.send(value);
            })
            .map_err(|_| Refusal::Unavailable)?;
        let _waiting = Waiting {
            state,
            stranded: self.stranded.clone(),
        };
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

    #[tokio::test]
    async fn a_read_that_never_returns_is_abandoned_counted_and_released_when_it_ends() {
        let reads = BlockingReads::new(1);
        assert_eq!(
            reads.run("quick", Duration::from_secs(5), || 7).await,
            Ok(7)
        );
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
        assert_eq!(reads.stranded(), 1);
        assert!(!reads.healthy());
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
        tokio::time::timeout(Duration::from_secs(5), async {
            while !reads.healthy() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(reads.stranded(), 0);
        assert_eq!(
            reads.run("again", Duration::from_secs(5), || 8).await,
            Ok(8)
        );
    }

    #[tokio::test]
    async fn a_dropped_request_still_counts_its_stuck_read() {
        let reads = BlockingReads::new(2);
        let (release, blocked) = std::sync::mpsc::channel::<()>();
        let request = reads.run("dropped", Duration::from_secs(60), move || blocked.recv());
        assert!(
            tokio::time::timeout(Duration::from_millis(20), request)
                .await
                .is_err()
        );
        assert_eq!(reads.stranded(), 1);
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while reads.stranded() != 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }
}
