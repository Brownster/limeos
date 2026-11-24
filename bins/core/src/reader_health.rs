//! The storage reader's own health, as it answers `Health`: whether a
//! storage read it abandoned at its deadline is still stuck (on a failing
//! disk, in the kernel). The reader keeps serving and refuses further
//! inventories; core lists it as degraded until the read returns.
use limeos_contracts::{DegradedDetail, DegradedSubsystem, Subsystem, VERSION};
use limeos_domain::{Error, ErrorCode, Result};
use limeos_executor_protocol::{Receipt, Request, StrandedReads};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

/// How often core asks. The answer is cheap: it reads a counter.
const INTERVAL: Duration = Duration::from_secs(10);
/// One question and answer; the reader answers from its accept loop.
const DEADLINE: Duration = Duration::from_secs(2);

/// The latest answer, shared with core's health response.
#[derive(Clone, Default)]
pub(crate) struct Status(Arc<Mutex<Option<StrandedReads>>>);
impl Status {
    /// The reader as a degraded subsystem, if its latest answer reported a
    /// stuck read. An unreachable reader (it is dormant until an operator
    /// starts it) is not listed: nothing is known to be wrong with it.
    pub(crate) fn degraded(&self) -> Option<DegradedSubsystem> {
        let stranded = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()?;
        Some(DegradedSubsystem {
            subsystem: Subsystem::StorageReader,
            detail: Some(DegradedDetail {
                code: ErrorCode::Unavailable,
                since: stranded.since,
                count: stranded.count,
            }),
        })
    }
    fn set(&self, value: Option<StrandedReads>) {
        let mut current = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if value.is_some() && current.is_none() {
            tracing::warn!(
                "the storage reader has a storage read stuck past its deadline; storage inventory is refused until it returns"
            );
        } else if value.is_none() && current.is_some() {
            tracing::info!("the storage reader's stuck read returned");
        }
        *current = value;
    }
}

#[derive(Clone)]
pub(crate) struct Watcher {
    socket: PathBuf,
    /// The reader runs as its own account; a socket that account does not
    /// own and answer (`limeos_contracts::ExecutorOwner`) is ignored.
    owner: limeos_contracts::ExecutorOwner,
    status: Status,
}
impl Watcher {
    pub(crate) fn new(socket: PathBuf, status: Status) -> Self {
        Self {
            socket,
            owner: crate::storage::executor_owner(),
            status,
        }
    }
    #[cfg(test)]
    pub(crate) fn for_test(socket: PathBuf, status: Status) -> Self {
        Self {
            socket,
            owner: crate::health_collection::test_owner(),
            status,
        }
    }
    /// Asks now and every `INTERVAL`, forever.
    pub(crate) async fn run(self) {
        loop {
            self.check().await;
            tokio::time::sleep(INTERVAL).await;
        }
    }
    /// One question; whatever the reader cannot answer clears the entry, so
    /// it always describes the latest answer.
    pub(crate) async fn check(&self) {
        let answer = tokio::time::timeout(DEADLINE, self.ask())
            .await
            .unwrap_or(Err(Error(ErrorCode::Unavailable)));
        self.status.set(answer.ok().flatten());
    }
    async fn ask(&self) -> Result<Option<StrandedReads>> {
        let mut stream = self.owner.connect(&self.socket).await?;
        limeos_contracts::write_frame(&mut stream, &Request::Health { version: VERSION }).await?;
        let receipt: Receipt = limeos_contracts::read_frame(&mut stream).await?;
        if receipt.version != VERSION || !receipt.ready || receipt.error.is_some() {
            return Err(Error(ErrorCode::Unavailable));
        }
        Ok(receipt.stranded_reads)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::UnixListener;

    /// A reader that answers `Health` with `stranded`, once per connection.
    fn reader(listener: UnixListener, stranded: Arc<Mutex<Option<StrandedReads>>>) {
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let request: Request = limeos_contracts::read_frame(&mut stream).await.unwrap();
                assert!(matches!(request, Request::Health { version: VERSION }));
                let mut receipt = Receipt::empty();
                receipt.stranded_reads = stranded.lock().unwrap().clone();
                limeos_contracts::write_frame(&mut stream, &receipt)
                    .await
                    .unwrap();
            }
        });
    }

    #[tokio::test]
    async fn a_stuck_reader_read_is_listed_with_its_time_and_cleared_when_it_returns() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("reader.sock");
        let answer = Arc::new(Mutex::new(None));
        reader(UnixListener::bind(&socket).unwrap(), answer.clone());
        let status = Status::default();
        let watcher = Watcher::for_test(socket, status.clone());
        watcher.check().await;
        assert_eq!(status.degraded(), None);
        *answer.lock().unwrap() = Some(StrandedReads {
            count: 1,
            since: 1_700_000_000,
        });
        watcher.check().await;
        assert_eq!(
            status.degraded(),
            Some(DegradedSubsystem {
                subsystem: Subsystem::StorageReader,
                detail: Some(DegradedDetail {
                    code: ErrorCode::Unavailable,
                    since: 1_700_000_000,
                    count: 1,
                }),
            })
        );
        *answer.lock().unwrap() = None;
        watcher.check().await;
        assert_eq!(status.degraded(), None);
    }

    #[tokio::test]
    async fn an_unreachable_or_foreign_reader_is_not_listed() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("reader.sock");
        let status = Status::default();
        status.set(Some(StrandedReads { count: 1, since: 1 }));
        Watcher::for_test(socket.clone(), status.clone())
            .check()
            .await;
        assert_eq!(status.degraded(), None);
        let answer = Arc::new(Mutex::new(Some(StrandedReads { count: 1, since: 1 })));
        reader(UnixListener::bind(&socket).unwrap(), answer);
        // Under the production rule this test's user is core, so a socket it
        // serves is core's own and is not taken for the reader's.
        Watcher::new(socket, status.clone()).check().await;
        assert_eq!(status.degraded(), None);
    }
}
