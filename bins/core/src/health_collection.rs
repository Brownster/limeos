//! Fresh root-reader responses. Only this private owner carries local freshness.
use limeos_domain::{Error, ErrorCode, Result, automation::ContainerHealthCohort};
use limeos_executor_protocol::{Receipt, Request};
use rustix::fs::{Mode, OFlags, ResolveFlags};
use std::{
    fs::File,
    os::unix::fs::{FileTypeExt, MetadataExt},
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
    #[cfg(test)]
    test_socket_uid: Option<u32>,
    #[cfg(test)]
    test_peer_uid: Option<u32>,
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
fn identity(file: &File, uid: u32) -> Result<Identity> {
    let meta = file.metadata().map_err(|_| Error(ErrorCode::Unavailable))?;
    if !meta.file_type().is_socket()
        || meta.uid() != uid
        || meta.mode() & 0o002 != 0
        || meta.nlink() != 1
    {
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
    fn open(path: &Path, uid: u32) -> Result<Self> {
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
        let identity = identity(&file, uid)?;
        Ok(Self { file, identity })
    }
    fn revalidate(&self, path: &Path, uid: u32) -> Result<()> {
        if identity(&self.file, uid)? != self.identity
            || Socket::open(path, uid)?.identity != self.identity
        {
            return Err(Error(ErrorCode::Conflict));
        }
        Ok(())
    }
}
impl HealthReader {
    #[cfg(test)]
    pub(crate) fn for_test(socket: PathBuf, gate: Arc<Semaphore>, clock: Clock) -> Self {
        let uid = rustix::process::geteuid().as_raw();
        let mut reader = Self::new(socket, gate);
        reader.clock = clock;
        reader.test_socket_uid = Some(uid);
        reader.test_peer_uid = Some(uid);
        reader
    }
    /// Trusted package/configuration path and the existing Core read gate, never HTTP input.
    pub fn new(socket: PathBuf, gate: Arc<Semaphore>) -> Self {
        Self {
            socket,
            gate,
            clock: Arc::new(super::now),
            #[cfg(test)]
            test_socket_uid: None,
            #[cfg(test)]
            test_peer_uid: None,
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
        let socket_uid = 0;
        let peer_uid = 0;
        #[cfg(test)]
        let socket_uid = self.test_socket_uid.unwrap_or(socket_uid);
        #[cfg(test)]
        let peer_uid = self.test_peer_uid.unwrap_or(peer_uid);
        let cohort = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
            let socket = Socket::open(&self.socket, socket_uid)?;
            live(deadline)?;
            let mut stream = UnixStream::connect(&self.socket)
                .await
                .map_err(|_| Error(ErrorCode::Unavailable))?;
            socket.revalidate(&self.socket, socket_uid)?;
            if stream
                .peer_cred()
                .map_err(|_| Error(ErrorCode::Forbidden))?
                .uid()
                != peer_uid
            {
                return Err(Error(ErrorCode::Forbidden));
            }
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
            socket.revalidate(&self.socket, socket_uid)?;
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

#[cfg(test)]
mod tests;
