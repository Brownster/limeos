//! Fresh storage-reader responses. Only this private owner carries local freshness.
use limeos_contracts::ExecutorOwner;
use limeos_domain::{Error, ErrorCode, Result, automation::ContainerHealthCohort};
use limeos_executor_protocol::{Receipt, Request};
use rustix::fs::{Mode, OFlags, ResolveFlags};
use std::{
    fs::File,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{net::UnixStream, sync::Semaphore};
type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;
/// The longest the 1 Hz background health read holds core's storage read
/// gate.
pub(crate) const WINDOW: Duration = Duration::from_secs(4);
const _: () = assert!(WINDOW.as_secs() < limeos_domain::CONTAINER_STORAGE_MAX_AGE_SECONDS as u64);
const _: () =
    assert!(WINDOW.as_secs() < limeos_domain::automation::CONTAINER_HEALTH_SECONDS as u64);

#[derive(Clone)]
pub struct HealthReader {
    socket: PathBuf,
    gate: Arc<Semaphore>,
    clock: Clock,
    /// The reader's socket must be owned by neither root nor core, and its
    /// peer must be that owner (`limeos_contracts::ExecutorOwner`).
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
#[derive(PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
    uid: u32,
    mode: u32,
}
struct Socket {
    file: File,
    identity: Identity,
}
fn identity(file: &File, owner: ExecutorOwner) -> Result<Identity> {
    let meta = file.metadata().map_err(|_| Error(ErrorCode::Unavailable))?;
    owner.socket_owner(&meta)?;
    if meta.mode() & 0o002 != 0 || meta.nlink() != 1 {
        return Err(Error(ErrorCode::Forbidden));
    }
    Ok(Identity {
        device: meta.dev(),
        inode: meta.ino(),
        uid: meta.uid(),
        mode: meta.mode(),
    })
}
impl Socket {
    fn open(path: &Path, owner: ExecutorOwner) -> Result<Self> {
        if !path.is_absolute() || path.as_os_str().len() > 512 {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let file = File::from(
            rustix::fs::openat2(
                rustix::fs::CWD,
                path,
                OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
                ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
            )
            .map_err(|_| Error(ErrorCode::Unavailable))?,
        );
        let identity = identity(&file, owner)?;
        Ok(Self { file, identity })
    }
    fn revalidate(&self, path: &Path, owner: ExecutorOwner) -> Result<()> {
        if identity(&self.file, owner)? != self.identity
            || Socket::open(path, owner)?.identity != self.identity
        {
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
            let mut stream = UnixStream::connect(&self.socket)
                .await
                .map_err(|_| Error(ErrorCode::Unavailable))?;
            socket.revalidate(&self.socket, owner)?;
            let peer = stream
                .peer_cred()
                .map_err(|_| Error(ErrorCode::Forbidden))?
                .uid();
            owner.check_peer(socket.identity.uid, peer)?;
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
    } = receipt;
    if version != limeos_contracts::VERSION
        || !ready
        || error.is_some()
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
    container_health.ok_or(Error(ErrorCode::Unavailable))
}

/// Core as tests model it: running under a UID other than the test
/// process's, whose sockets therefore stand in for an executor's.
#[cfg(test)]
pub(crate) fn test_owner() -> ExecutorOwner {
    ExecutorOwner::new(rustix::process::geteuid().as_raw().wrapping_add(1))
}

#[cfg(test)]
mod tests;
