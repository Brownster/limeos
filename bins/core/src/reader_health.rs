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
    DegradedDetail, DegradedSubsystem, ReadResult, ReaderRead, ReaderReadOutcome, ReaderState,
    ReaderStatus, SocketRefusal, Subsystem, VERSION,
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
    /// Records the latest attempt and logs only changes worth a line, which
    /// it returns (for tests). A refused reader stays refused, dated from the
    /// first refusal, until core accepts it: a check that finds no socket or
    /// gets no answer says nothing new about it, so a root reader restarting
    /// under `Restart=on-failure` logs one warning, not one per restart.
    fn set(&self, value: Latest) -> Option<Logged> {
        let mut current = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let stranded = |l: &Latest| matches!(l, Latest::Accepted(Some(_)));
        let mut logged = None;
        match (&*current, &value) {
            (Latest::Refused(..), Latest::Silent) => return None,
            (Latest::Refused(before, since), Latest::Refused(now, _)) => {
                let since = *since;
                if before != now {
                    tracing::warn!(reason = %now, "core still refuses the storage reader's socket, now because: {now}");
                    logged = Some(Logged::StillRefused);
                }
                *current = Latest::Refused(*now, since);
                return logged;
            }
            (_, Latest::Refused(reason, _)) => {
                tracing::warn!(
                    reason = %reason,
                    "core refuses the storage reader's socket and takes no storage, stack or container-health answer from it: {reason}. The reader must run as its own account (limeos-storage-reader) from its runtime directory; check for a drop-in with systemctl cat limeos-storage-reader"
                );
                logged = Some(Logged::Refused);
            }
            (Latest::Refused(..), Latest::Accepted(_)) => {
                tracing::info!("core accepts the storage reader's socket again");
                logged = Some(Logged::AcceptedAgain);
            }
            _ => {}
        }
        if stranded(&value) && !stranded(&current) {
            tracing::warn!(
                "the storage reader has a storage read stuck past its deadline; storage inventory is refused until it returns"
            );
            logged = logged.or(Some(Logged::Stuck));
        } else if !stranded(&value) && stranded(&current) {
            tracing::info!("the storage reader's stuck read returned");
            logged = logged.or(Some(Logged::Returned));
        }
        *current = value;
        logged
    }
}
/// The line `Status::set` logged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Logged {
    Refused,
    StillRefused,
    AcceptedAgain,
    Stuck,
    Returned,
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
        let asked = tokio::time::timeout(DEADLINE, self.ask())
            .await
            .unwrap_or(Asked::NoAnswer("no well-formed answer within the deadline"));
        let (latest, state, reason) = match asked {
            Asked::Answered(stranded) => (Latest::Accepted(stranded), ReaderState::Accepted, None),
            Asked::Refused(refusal) => (
                Latest::Refused(refusal, super::now()),
                ReaderState::Refused,
                Some(refusal.message().to_owned()),
            ),
            Asked::NoSocket => (Latest::Silent, ReaderState::Unreachable, None),
            Asked::NoAnswer(why) => (
                Latest::Silent,
                ReaderState::Unanswered,
                Some(why.to_owned()),
            ),
        };
        self.status.set(latest);
        ReaderStatus {
            state,
            reason,
            read: None,
        }
    }
    async fn ask(&self) -> Asked {
        let mut stream = match self.owner.connect_explained(&self.socket).await {
            Ok(stream) => stream,
            Err(SocketRefusal::Missing) => return Asked::NoSocket,
            Err(SocketRefusal::Failed) => return Asked::NoAnswer(SocketRefusal::Failed.message()),
            Err(refusal) => return Asked::Refused(refusal),
        };
        let unanswered = |_| Asked::NoAnswer("the reader did not answer its health request");
        if let Err(error) =
            limeos_contracts::write_frame(&mut stream, &Request::Health { version: VERSION }).await
        {
            return unanswered(error);
        }
        let receipt: Receipt = match limeos_contracts::read_frame(&mut stream).await {
            Ok(receipt) => receipt,
            Err(error) => return unanswered(error),
        };
        if receipt.version != VERSION || !receipt.ready || receipt.error.is_some() {
            return Asked::NoAnswer("the reader refused or failed its health request");
        }
        Asked::Answered(receipt.stranded_reads)
    }
}

/// What one health question found.
enum Asked {
    Answered(Option<StrandedReads>),
    /// Nothing at the socket's path, or nothing listening: a dormant reader.
    NoSocket,
    /// A socket core may not trust or reach.
    Refused(SocketRefusal),
    /// Core could not try, or the reader gave no well-formed answer.
    NoAnswer(&'static str),
}

/// One read the reader serves, made now by core as it would for a user
/// (`limeosctl reader-status --read`), so a deployment check proves more
/// than the socket: a reader whose capability or Docker group a drop-in
/// took away still answers `Health`, but fails these.
pub(crate) async fn read_once(core: &crate::Core, read: ReaderRead) -> ReaderReadOutcome {
    let result = match read {
        ReaderRead::Stack => stack_once(core).await.map(|()| None),
        ReaderRead::Storage => core.probe_storage().await.map(|_| None).map_err(failure),
        ReaderRead::ContainerHealth => {
            return container_health_read(crate::health_collection::HealthReader::new(
                (*core.storage_socket).clone(),
                core.storage_readers.clone(),
            ))
            .await;
        }
    };
    outcome(read, result)
}
fn outcome(
    read: ReaderRead,
    result: std::result::Result<Option<String>, Failure>,
) -> ReaderReadOutcome {
    let (result, reason) = match result {
        Ok(note) => (ReadResult::Accepted, note),
        Err(Failure::NotGranted) => (ReadResult::NotGranted, None),
        Err(Failure::Failed(reason)) => (ReadResult::Failed, Some(reason)),
    };
    ReaderReadOutcome {
        read,
        result,
        reason,
    }
}
enum Failure {
    NotGranted,
    Failed(String),
}
/// A container-health read through the reader, as `read_once` makes it.
/// Accepted past the Engine bound, it says how many containers it left out;
/// refused by the reader's ceiling, it is not granted; failed, it gives the
/// reader's reason when the reader sent one.
pub(crate) async fn container_health_read(
    reader: crate::health_collection::HealthReader,
) -> ReaderReadOutcome {
    let result = match reader
        .read_explained(std::time::Instant::now() + crate::health_collection::WINDOW)
        .await
    {
        Ok(fresh) => Ok(truncation_note(fresh.cohort())),
        Err(refused) => Err(match (failure(refused.error), refused.reason) {
            (Failure::Failed(base), Some(reason)) => {
                Failure::Failed(format!("{base}: {}", reason.message()))
            }
            (other, _) => other,
        }),
    };
    outcome(ReaderRead::ContainerHealth, result)
}
fn truncation_note(cohort: &limeos_domain::automation::ContainerHealthCohort) -> Option<String> {
    let left_out = cohort.truncated();
    (left_out > 0).then(|| {
        format!(
            "truncated: {left_out} container(s) beyond the {}-container bound (stopped ones first) were not read and are not watched",
            limeos_domain::automation::CONTAINER_HEALTH_MEMBERS
        )
    })
}
/// A read's error: `Forbidden` from a reader whose socket core already
/// accepted is its ceiling saying no.
fn failure(error: limeos_domain::Error) -> Failure {
    match error.0 {
        ErrorCode::Forbidden => Failure::NotGranted,
        code => Failure::Failed(format!("the read failed: {code:?}")),
    }
}
/// The reader's latest stack scan, which must be fresh from both the
/// catalog and the Engine.
async fn stack_once(core: &crate::Core) -> std::result::Result<(), Failure> {
    use limeos_domain::deployment::discovery::SourceState;
    let receipt: Receipt = tokio::time::timeout(limeos_contracts::RPC_DEADLINE, async {
        let mut stream = crate::storage::executor_owner()
            .connect(&core.storage_socket)
            .await?;
        limeos_contracts::write_frame(&mut stream, &Request::StackInventory { version: VERSION })
            .await?;
        limeos_contracts::read_frame(&mut stream).await
    })
    .await
    .unwrap_or(Err(limeos_domain::Error(ErrorCode::Unavailable)))
    .map_err(failure)?;
    if let Some(code) = receipt.error {
        return Err(failure(limeos_domain::Error(code)));
    }
    let cohort = receipt
        .stack_cohort
        .ok_or_else(|| Failure::Failed("the reader has no stack scan yet".into()))?;
    let inventory = cohort
        .project(super::now())
        .map_err(|error| Failure::Failed(format!("the stack scan is not usable: {:?}", error.0)))?;
    if inventory.catalog.state != SourceState::Fresh || inventory.engine.state != SourceState::Fresh
    {
        return Err(Failure::Failed(format!(
            "the stack scan is not fresh: catalog {:?}, Engine {:?}",
            inventory.catalog.state, inventory.engine.state
        )));
    }
    Ok(())
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

    #[test]
    fn a_refused_reader_logs_once_until_accepted_and_silence_changes_nothing() {
        let status = Status::default();
        let refused = |since| Latest::Refused(SocketRefusal::WrongOwner, since);
        assert_eq!(status.set(refused(100)), Some(Logged::Refused));
        // A root reader restarting: no socket for a moment, then refused again.
        for at in [110, 120, 130] {
            assert_eq!(status.set(Latest::Silent), None);
            assert_eq!(status.set(refused(at)), None);
        }
        let listed = status.degraded().unwrap().detail.unwrap();
        assert_eq!((listed.code, listed.since), (ErrorCode::Forbidden, 100));
        // A different reason is worth a line; the date stays the first refusal's.
        assert_eq!(
            status.set(Latest::Refused(SocketRefusal::Denied, 140)),
            Some(Logged::StillRefused)
        );
        assert_eq!(status.degraded().unwrap().detail.unwrap().since, 100);
        // Only acceptance clears it, and says so.
        assert_eq!(
            status.set(Latest::Accepted(None)),
            Some(Logged::AcceptedAgain)
        );
        assert_eq!(status.degraded(), None);
        assert_eq!(status.set(Latest::Silent), None);
        assert_eq!(status.set(Latest::Accepted(None)), None);
        // A stuck read is logged when it starts and when it returns.
        let stuck = Latest::Accepted(Some(StrandedReads { count: 1, since: 5 }));
        assert_eq!(status.set(stuck.clone()), Some(Logged::Stuck));
        assert_eq!(status.set(stuck), None);
        assert_eq!(status.set(Latest::Accepted(None)), Some(Logged::Returned));
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
