//! The storage reader's own health, as it answers `Health`: whether a
//! storage read it abandoned at its deadline is still stuck (on a failing
//! disk, in the kernel). The reader keeps serving and refuses further
//! inventories; core lists it as degraded until the read returns.
//!
//! Also whether core trusts the reader's socket at all: a reader whose
//! socket core refuses (`limeos_contracts::ExecutorOwner`: run as root by a
//! drop-in, say, or bound somewhere else) is listed as degraded with code
//! `forbidden` and logged once, so it is not mistaken for a dormant reader.
use limeos_contracts::{
    DegradedDetail, DegradedSubsystem, ReaderState, ReaderStatus, SocketRefusal, Subsystem, VERSION,
};
use limeos_domain::ErrorCode;
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

/// The latest attempt.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Latest {
    /// No socket, or no well-formed answer: a dormant or busy reader, so
    /// nothing is listed.
    #[default]
    Silent,
    /// Core took the reader's answer.
    Accepted(Option<StrandedReads>),
    /// A socket is there and core refuses it, since then.
    Refused(SocketRefusal, i64),
}

/// The latest answer, shared with core's health response.
#[derive(Clone, Default)]
pub(crate) struct Status(Arc<Mutex<Latest>>);
impl Status {
    /// The reader as a degraded subsystem, if its latest answer reported a
    /// stuck read, or core refuses its socket. An unreachable reader (it is
    /// dormant until an operator starts it) is not listed: nothing is known
    /// to be wrong with it.
    pub(crate) fn degraded(&self) -> Option<DegradedSubsystem> {
        let detail = match &*self.0.lock().unwrap_or_else(PoisonError::into_inner) {
            Latest::Accepted(Some(stranded)) => DegradedDetail {
                code: ErrorCode::Unavailable,
                since: stranded.since,
                count: stranded.count,
            },
            Latest::Refused(_, since) => DegradedDetail {
                code: ErrorCode::Forbidden,
                since: *since,
                count: 0,
            },
            _ => return None,
        };
        Some(DegradedSubsystem {
            subsystem: Subsystem::StorageReader,
            detail: Some(detail),
        })
    }
    fn set(&self, value: Latest) {
        let mut current = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let stranded = |l: &Latest| matches!(l, Latest::Accepted(Some(_)));
        match (&*current, &value) {
            (Latest::Refused(before, since), Latest::Refused(now, _)) => {
                // Still refused: keep when it began; log a changed reason.
                if before != now {
                    tracing::warn!(reason = %now, "core still refuses the storage reader's socket, now because: {now}");
                }
                *current = Latest::Refused(*now, *since);
                return;
            }
            (_, Latest::Refused(reason, _)) => tracing::warn!(
                reason = %reason,
                "core refuses the storage reader's socket and takes no storage, stack or container-health answer from it: {reason}. The reader must run as its own account (limeos-storage-reader) from its runtime directory; check for a drop-in with systemctl cat limeos-storage-reader"
            ),
            (Latest::Refused(..), _) => {
                tracing::info!("core accepts the storage reader's socket again")
            }
            _ => {}
        }
        if stranded(&value) && !stranded(&current) {
            tracing::warn!(
                "the storage reader has a storage read stuck past its deadline; storage inventory is refused until it returns"
            );
        } else if !stranded(&value) && stranded(&current) {
            tracing::info!("the storage reader's stuck read returned");
        }
        *current = value;
    }
}

#[derive(Clone)]
pub(crate) struct Watcher {
    socket: PathBuf,
    /// The reader runs as its own account; a socket the rule refuses
    /// (`limeos_contracts::ExecutorOwner`) is never asked anything.
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
    /// One question; the entry always describes the latest attempt. What it
    /// found, as `limeosctl reader-status` reports it.
    pub(crate) async fn check(&self) -> ReaderStatus {
        let latest = match tokio::time::timeout(DEADLINE, self.ask()).await {
            Ok(Ok(stranded)) => Latest::Accepted(stranded),
            Ok(Err(Some(refusal))) => Latest::Refused(refusal, super::now()),
            Ok(Err(None)) | Err(_) => Latest::Silent,
        };
        let status = match &latest {
            Latest::Accepted(_) => ReaderStatus {
                state: ReaderState::Accepted,
                reason: None,
            },
            Latest::Refused(refusal, _) => ReaderStatus {
                state: ReaderState::Refused,
                reason: Some(refusal.message().into()),
            },
            Latest::Silent => ReaderStatus {
                state: if self.owner.open(&self.socket).err() == Some(SocketRefusal::Missing) {
                    ReaderState::Unreachable
                } else {
                    ReaderState::Unanswered
                },
                reason: None,
            },
        };
        self.status.set(latest);
        status
    }
    /// The reader's answer; `Err(Some(_))` for a socket the rule refuses,
    /// `Err(None)` for no socket or no well-formed answer.
    async fn ask(&self) -> std::result::Result<Option<StrandedReads>, Option<SocketRefusal>> {
        let mut stream = match self.owner.connect_explained(&self.socket).await {
            Ok(stream) => stream,
            Err(SocketRefusal::Missing) => return Err(None),
            Err(refusal) => return Err(Some(refusal)),
        };
        limeos_contracts::write_frame(&mut stream, &Request::Health { version: VERSION })
            .await
            .map_err(|_| None)?;
        let receipt: Receipt = limeos_contracts::read_frame(&mut stream)
            .await
            .map_err(|_| None)?;
        if receipt.version != VERSION || !receipt.ready || receipt.error.is_some() {
            return Err(None);
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
        let socket = crate::health_collection::test_socket(dir.path());
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
    async fn an_unreachable_reader_is_not_listed_and_a_refused_one_is_listed_forbidden() {
        let dir = tempfile::tempdir().unwrap();
        let socket = crate::health_collection::test_socket(dir.path());
        let status = Status::default();
        status.set(Latest::Accepted(Some(StrandedReads { count: 1, since: 1 })));
        Watcher::for_test(socket.clone(), status.clone())
            .check()
            .await;
        assert_eq!(status.degraded(), None);
        let answer = Arc::new(Mutex::new(Some(StrandedReads { count: 1, since: 1 })));
        reader(UnixListener::bind(&socket).unwrap(), answer);
        // Under the production rule this socket is outside root's /run, and
        // as core's own account it is core's: never taken for the reader's.
        // Its stuck read is not listed; the refusal is, with code forbidden.
        let uid = rustix::process::geteuid().as_raw();
        for owner in [
            limeos_contracts::ExecutorOwner::with_anchor_owner(uid, uid),
            crate::storage::executor_owner(),
        ] {
            let watcher = Watcher {
                socket: socket.clone(),
                owner,
                status: status.clone(),
            };
            let probe = watcher.check().await;
            assert_eq!(probe.state, ReaderState::Refused);
            assert!(probe.reason.is_some());
            let listed = status.degraded().unwrap();
            assert_eq!(listed.subsystem, Subsystem::StorageReader);
            assert_eq!(listed.detail.unwrap().code, ErrorCode::Forbidden);
        }
    }

    #[tokio::test]
    async fn a_refused_reader_is_listed_from_when_it_was_first_refused_and_cleared_once_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let socket = crate::health_collection::test_socket(dir.path());
        let answer = Arc::new(Mutex::new(None));
        reader(UnixListener::bind(&socket).unwrap(), answer);
        let status = Status::default();
        let uid = rustix::process::geteuid().as_raw();
        let refused = Watcher {
            socket: socket.clone(),
            owner: limeos_contracts::ExecutorOwner::with_anchor_owner(uid, uid),
            status: status.clone(),
        };
        refused.check().await;
        let since = status.degraded().unwrap().detail.unwrap().since;
        status.set(Latest::Refused(SocketRefusal::WrongOwner, since + 50));
        assert_eq!(status.degraded().unwrap().detail.unwrap().since, since);
        let accepted = Watcher::for_test(socket.clone(), status.clone())
            .check()
            .await;
        assert_eq!(accepted.state, ReaderState::Accepted);
        assert_eq!(status.degraded(), None);
        // A reader that is not there at all is unreachable, and not listed.
        std::fs::remove_file(&socket).unwrap();
        let gone = Watcher::for_test(socket, status.clone()).check().await;
        assert_eq!(gone.state, ReaderState::Unreachable);
        assert_eq!(status.degraded(), None);
    }
}
