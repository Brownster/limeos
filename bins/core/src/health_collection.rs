//! Fresh storage-reader responses. Only this private owner carries local freshness.
use limeos_contracts::ExecutorOwner;
use limeos_domain::{Error, ErrorCode, Result, automation::ContainerHealthCohort};
use limeos_executor_protocol::{HealthReadFailure, Receipt, Request};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;
type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;
/// The longest the 1 Hz background health read holds core's storage read
/// gate.
pub(crate) const WINDOW: Duration = Duration::from_secs(4);
const _: () = assert!(WINDOW.as_secs() < limeos_domain::CONTAINER_STORAGE_MAX_AGE_SECONDS as u64);
const _: () =
    assert!(WINDOW.as_secs() < limeos_domain::automation::CONTAINER_HEALTH_SECONDS as u64);
// The reader's worker window, counted from the request's arrival, ends well
// inside this one, so a slow read reaches core as the reader's reason rather
// than as core's own expiry.
const _: () = assert!(
    limeos_executor_protocol::CONTAINER_HEALTH_READER_WINDOW.as_millis() + 400
        <= WINDOW.as_millis()
);

#[derive(Clone)]
pub struct HealthReader {
    socket: PathBuf,
    gate: Arc<Semaphore>,
    clock: Clock,
    /// The rule the reader's socket must meet (`limeos_contracts::ExecutorOwner`):
    /// in its owner's directory under a root-owned one, owned by neither
    /// root nor core, answered by its owner.
    owner: ExecutorOwner,
}
#[derive(Debug)]
pub struct FreshContainerHealth {
    cohort: ContainerHealthCohort,
    deadline: Instant,
}
impl FreshContainerHealth {
    pub fn cohort(&self) -> &ContainerHealthCohort {
        &self.cohort
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    pub fn ensure_live(&self) -> Result<()> {
        live(self.deadline)
    }
}
fn live(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        Err(Error(ErrorCode::Expired))
    } else {
        Ok(())
    }
}
/// The reader's socket as the shared rule opened it, rechecked by name and by
/// descriptor around the exchange.
struct Socket(limeos_contracts::ExecutorSocket);
impl Socket {
    fn open(path: &Path, owner: ExecutorOwner) -> Result<Self> {
        owner
            .open(path)
            .map(Self)
            .map_err(limeos_contracts::SocketRefusal::error)
    }
    fn revalidate(&self, path: &Path, owner: ExecutorOwner) -> Result<()> {
        if !self.0.unchanged() || Socket::open(path, owner)?.0.identity != self.0.identity {
            return Err(Error(ErrorCode::Conflict));
        }
        Ok(())
    }
}
impl HealthReader {
    /// A reader whose socket this test process serves: core is taken to run
    /// under another UID, so the test's own socket is an executor's.
    #[cfg(test)]
    pub(crate) fn for_test(socket: PathBuf, gate: Arc<Semaphore>, clock: Clock) -> Self {
        let mut reader = Self::new(socket, gate);
        reader.clock = clock;
        reader.owner = test_owner();
        reader
    }
    /// Trusted package/configuration path and the existing Core read gate, never HTTP input.
    pub fn new(socket: PathBuf, gate: Arc<Semaphore>) -> Self {
        Self {
            socket,
            gate,
            clock: Arc::new(super::now),
            owner: crate::storage::executor_owner(),
        }
    }
    pub async fn read(&self, outer_deadline: Instant) -> Result<FreshContainerHealth> {
        self.read_explained(outer_deadline)
            .await
            .map_err(|refused| refused.error)
    }
    /// As `read`, with the reader's reason when it refused a granted read.
    pub(crate) async fn read_explained(
        &self,
        outer_deadline: Instant,
    ) -> std::result::Result<FreshContainerHealth, Refused> {
        let mut answered = Answered::default();
        let answer = self.exchange(outer_deadline, &mut answered).await;
        answer.map_err(|error| Refused {
            error,
            reason: answered.reason,
            by_reader: answered.error.is_some_and(|code| refusal(code) == error),
        })
    }
    async fn exchange(
        &self,
        outer_deadline: Instant,
        answered: &mut Answered,
    ) -> Result<FreshContainerHealth> {
        let deadline = outer_deadline.min(Instant::now() + WINDOW);
        live(deadline)?;
        // Background: only a free gate. A request queued for it goes first.
        let _permit = self
            .gate
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error(ErrorCode::Overloaded))?;
        let nonce = limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?;
        let requested = (self.clock)();
        let owner = self.owner;
        let cohort = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
            let socket = Socket::open(&self.socket, owner)?;
            live(deadline)?;
            // Connects to the opened inode and checks the kernel's peer.
            let mut stream = socket
                .0
                .connect()
                .await
                .map_err(limeos_contracts::SocketRefusal::error)?;
            socket.revalidate(&self.socket, owner)?;
            live(deadline)?;
            limeos_contracts::write_frame(
                &mut stream,
                &Request::ContainerHealth {
                    version: limeos_contracts::VERSION,
                    nonce: nonce.clone(),
                },
            )
            .await?;
            let receipt: Receipt = limeos_contracts::read_frame(&mut stream).await?;
            live(deadline)?;
            socket.revalidate(&self.socket, owner)?;
            answered.error = receipt.error;
            answered.reason = receipt.health_failure;
            let cohort = only_health(receipt)?;
            if cohort.nonce() != nonce {
                return Err(Error(ErrorCode::Conflict));
            }
            cohort.check_time(requested, (self.clock)())?;
            Ok(cohort)
        })
        .await
        .map_err(|_| Error(ErrorCode::Expired))??;
        live(deadline)?;
        Ok(FreshContainerHealth { cohort, deadline })
    }
}
/// What the reader's answer said, if one arrived.
#[derive(Default)]
struct Answered {
    error: Option<ErrorCode>,
    reason: Option<HealthReadFailure>,
}
/// A refused read: the code, the reader's reason if it gave one, and whether
/// the code is the reader's own answer (its ceiling's `Forbidden`, say)
/// rather than core's refusal to use the reader's socket or its answer.
#[derive(Debug)]
pub(crate) struct Refused {
    pub(crate) error: Error,
    pub(crate) reason: Option<HealthReadFailure>,
    pub(crate) by_reader: bool,
}
/// The reader's refusal as core treats it. Its ceiling's "no" (`Forbidden`)
/// stays `Forbidden`, which `limeosctl reader-status` reports as not granted
/// and the automation runtime retries like any unavailable read; a busy
/// reader stays `Overloaded`. Any other refusal is `Unavailable`.
fn refusal(code: ErrorCode) -> Error {
    Error(match code {
        ErrorCode::Forbidden | ErrorCode::Overloaded => code,
        _ => ErrorCode::Unavailable,
    })
}
fn only_health(receipt: Receipt) -> Result<ContainerHealthCohort> {
    let Receipt {
        container_health,
        stack_cohort,
        storage_mount_catalog,
        storage_mount_inspection,
        storage_mount,
        mounted_storage,
        version,
        ready,
        observations,
        inspection,
        restart,
        stack,
        stack_members,
        resolution,
        absence,
        closed,
        resolution_refusal,
        epoch,
        logs,
        error,
        budget_frees_at,
        storage,
        storage_targets,
        target_preparation,
        container_storage,
        stranded_reads,
        health_failure,
    } = receipt;
    // Only the cohort, or a refusal (`ready: false`, the error and at most
    // the reader's reason); anything else is a malformed answer.
    if version != limeos_contracts::VERSION
        || budget_frees_at.is_some()
        || stack_cohort.is_some()
        || storage_mount_catalog.is_some()
        || storage_mount_inspection.is_some()
        || storage_mount.is_some()
        || mounted_storage.is_some()
        || observations.is_some()
        || inspection.is_some()
        || restart.is_some()
        || stack.is_some()
        || stack_members.is_some()
        || resolution.is_some()
        || absence.is_some()
        || closed
        || resolution_refusal.is_some()
        || epoch.is_some()
        || logs.is_some()
        || storage.is_some()
        || storage_targets.is_some()
        || target_preparation.is_some()
        || container_storage.is_some()
        || stranded_reads.is_some()
    {
        return Err(Error(ErrorCode::Unavailable));
    }
    match (ready, error, container_health) {
        (false, Some(code), None) => Err(refusal(code)),
        (true, None, Some(cohort)) if health_failure.is_none() => Ok(cohort),
        _ => Err(Error(ErrorCode::Unavailable)),
    }
}

/// Core as tests model it: running under a UID other than the test
/// process's, whose sockets therefore stand in for an executor's, with the
/// test's own directories standing in for root's `/run`.
#[cfg(test)]
pub(crate) fn test_owner() -> ExecutorOwner {
    let uid = rustix::process::geteuid().as_raw();
    ExecutorOwner::with_anchor_owner(uid.wrapping_add(1), uid)
}
/// Where a test executor binds, laid out as packaged: `DIR/run` (0755, for
/// `/run`) and `DIR/run/reader` (0750, the unit's runtime directory).
#[cfg(test)]
pub(crate) fn test_socket(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let run = dir.join("run");
    let runtime = run.join("reader");
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o750)).unwrap();
    runtime.join("reader.sock")
}

#[cfg(test)]
mod tests;
