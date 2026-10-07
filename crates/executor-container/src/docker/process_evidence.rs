//! Authenticated GET-only Engine observations; no retained daemon/process authority.
use super::{
    Docker, StatusCode, Value, parse_api_version,
    storage::{parse_storage_container, parse_storage_container_ids as container_ids},
};
use limeos_domain::{
    CONTAINER_STORAGE_MAX_AGE_SECONDS, CONTAINER_STORAGE_MAX_TOTAL_MOUNTS, ContainerSnapshot,
    ContainerStorageConsumer, ContainerStorageInventory, Error, ErrorCode, Result,
};
use rustix::fs::{Mode, OFlags, ResolveFlags};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    fs::File,
    io::Write,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{net::UnixStream, time::Instant};

const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_REPORT_BYTES: usize = 48 * 1024;
const COLLECTION_SECONDS: u64 = 4;
const REQUEST_SECONDS: u64 = 2;

fn unavailable() -> Error {
    Error(ErrorCode::Unavailable)
}
fn conflict() -> Error {
    Error(ErrorCode::Conflict)
}

/// Only trusted package/runtime configuration chooses the endpoint and policy.
/// Rootless Engine and public caller-selected UIDs are deliberately unsupported.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct EnginePeerPolicy {
    socket_uid: u32,
    peer_uid: u32,
}
impl EnginePeerPolicy {
    pub fn root() -> Self {
        Self {
            socket_uid: 0,
            peer_uid: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EngineRunningProcess {
    container: ContainerSnapshot,
    pid: u32,
}
impl EngineRunningProcess {
    pub fn container(&self) -> &ContainerSnapshot {
        &self.container
    }
    pub fn pid(&self) -> u32 {
        self.pid
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct SocketIdentity {
    device: u64,
    inode: u64,
    uid: u32,
    mode: u32,
}
#[derive(Debug, Serialize)]
pub struct EngineStorageProcessSnapshot {
    version: u16,
    endpoint: PathBuf,
    policy: EnginePeerPolicy,
    socket: SocketIdentity,
    declarations: ContainerStorageInventory,
    running: Vec<EngineRunningProcess>,
}

/// Only authenticated complete collection constructs this owner. Reports cannot
/// reconstruct its socket descriptor, original lifetime or fresh observations.
pub struct EngineStorageProcessEvidence {
    snapshot: EngineStorageProcessSnapshot,
    authenticated: AuthenticatedEngine,
    lifetime: EvidenceLifetime,
}
impl EngineStorageProcessEvidence {
    pub fn snapshot(&self) -> &EngineStorageProcessSnapshot {
        &self.snapshot
    }
    pub fn declarations(&self) -> &ContainerStorageInventory {
        &self.snapshot.declarations
    }
    pub fn running(&self) -> &[EngineRunningProcess] {
        &self.snapshot.running
    }
    /// Fresh GETs compare complete facts; they do not extend the original age,
    /// authenticate a daemon PID, retain container processes or authorize effects.
    pub async fn revalidate(&self) -> Result<()> {
        self.lifetime.check()?;
        let fresh = collect(&self.authenticated, &self.lifetime).await?;
        self.lifetime.check()?;
        if fresh.declarations.engine_id != self.snapshot.declarations.engine_id
            || fresh.declarations.containers != self.snapshot.declarations.containers
            || fresh.running != self.snapshot.running
        {
            return Err(conflict());
        }
        self.authenticated.revalidate_socket()?;
        self.lifetime.check()
    }
}

impl Docker {
    /// Library-only trusted in-process seam. The runtime must supply a protected,
    /// package-selected absolute socket path, host context and bounded admission.
    pub async fn inspect_storage_processes(
        &self,
        policy: EnginePeerPolicy,
    ) -> Result<EngineStorageProcessEvidence> {
        inspect(self, policy, TimeSource::default()).await
    }
}
async fn inspect(
    docker: &Docker,
    policy: EnginePeerPolicy,
    clock: TimeSource,
) -> Result<EngineStorageProcessEvidence> {
    let lifetime = EvidenceLifetime::new(clock)?;
    let authenticated = AuthenticatedEngine::open(docker.clone(), policy)?;
    let collected = collect(&authenticated, &lifetime).await?;
    let snapshot = EngineStorageProcessSnapshot {
        version: 1,
        endpoint: docker.socket.clone(),
        policy,
        socket: authenticated.socket.identity.clone(),
        declarations: collected.declarations,
        running: collected.running,
    };
    check_report(&snapshot)?;
    lifetime.check()?;
    Ok(EngineStorageProcessEvidence {
        snapshot,
        authenticated,
        lifetime,
    })
}

struct SocketEvidence {
    descriptor: File,
    identity: SocketIdentity,
}
impl SocketEvidence {
    fn open(path: &Path, policy: EnginePeerPolicy) -> Result<Self> {
        if !path.is_absolute() || path.as_os_str().len() > 512 {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let descriptor: File = rustix::fs::openat2(
            rustix::fs::CWD,
            path,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(|_| unavailable())?
        .into();
        let identity = socket_identity(&descriptor, policy)?;
        Ok(Self {
            descriptor,
            identity,
        })
    }
}
fn socket_identity(descriptor: &File, policy: EnginePeerPolicy) -> Result<SocketIdentity> {
    let metadata = descriptor.metadata().map_err(|_| unavailable())?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != policy.socket_uid
        || metadata.mode() & 0o002 != 0
        || metadata.nlink() != 1
    {
        return Err(Error(ErrorCode::Forbidden));
    }
    Ok(SocketIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        uid: metadata.uid(),
        mode: metadata.mode(),
    })
}

pub(super) struct AuthenticatedEngine {
    docker: Docker,
    policy: EnginePeerPolicy,
    socket: SocketEvidence,
}
impl AuthenticatedEngine {
    fn open(docker: Docker, policy: EnginePeerPolicy) -> Result<Self> {
        let socket = SocketEvidence::open(&docker.socket, policy)?;
        Ok(Self {
            docker,
            policy,
            socket,
        })
    }
    pub(super) fn authenticate(&self, stream: &UnixStream) -> Result<()> {
        let peer = stream
            .peer_cred()
            .map_err(|_| Error(ErrorCode::Forbidden))?;
        if peer.uid() != self.policy.peer_uid {
            return Err(Error(ErrorCode::Forbidden));
        }
        // Socket activation means peer.pid() is not independent daemon identity.
        Ok(())
    }
    pub(super) fn revalidate_socket(&self) -> Result<()> {
        let retained = socket_identity(&self.socket.descriptor, self.policy).map_err(|error| {
            if error.0 == ErrorCode::Forbidden {
                conflict()
            } else {
                error
            }
        })?;
        if retained != self.socket.identity
            || SocketEvidence::open(&self.docker.socket, self.policy)?.identity
                != self.socket.identity
        {
            return Err(conflict());
        }
        Ok(())
    }
    async fn json(
        &self,
        path: String,
        limit: usize,
        budget: &ResponseBudget,
        lifetime: &EvidenceLifetime,
    ) -> Result<Value> {
        lifetime.check()?;
        let value = tokio::time::timeout(Duration::from_secs(REQUEST_SECONDS), async {
            let (status, bytes, _) = self
                .docker
                .request_clipped_inner("GET", path, limit, false, Some((self, budget)))
                .await?;
            if status != StatusCode::OK {
                return Err(unavailable());
            }
            serde_json::from_slice(&bytes).map_err(|_| unavailable())
        })
        .await
        .map_err(|_| unavailable())??;
        lifetime.check()?;
        Ok(value)
    }
}

pub(super) struct ResponseBudget(AtomicUsize);
impl ResponseBudget {
    fn new() -> Self {
        Self(AtomicUsize::new(MAX_RESPONSE_BYTES))
    }
    pub(super) fn consume(&self, bytes: usize) -> Result<()> {
        self.0
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| {
                remaining.checked_sub(bytes)
            })
            .map(|_| ())
            .map_err(|_| unavailable())
    }
}

#[derive(Clone, Default)]
struct TimeSource {
    #[cfg(test)]
    wall: Option<std::sync::Arc<dyn Fn() -> Result<i64> + Send + Sync>>,
}
impl TimeSource {
    fn now(&self) -> Result<i64> {
        #[cfg(test)]
        if let Some(wall) = &self.wall {
            return wall();
        }
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| unavailable())?
            .as_secs()
            .try_into()
            .map_err(|_| unavailable())
    }
}
struct EvidenceLifetime {
    clock: TimeSource,
    observed_at: i64,
    expires: Instant,
}
impl EvidenceLifetime {
    fn new(clock: TimeSource) -> Result<Self> {
        let observed_at = clock.now()?;
        if observed_at < 0 {
            return Err(unavailable());
        }
        Ok(Self {
            clock,
            observed_at,
            expires: Instant::now() + Duration::from_secs(CONTAINER_STORAGE_MAX_AGE_SECONDS as u64),
        })
    }
    fn check(&self) -> Result<()> {
        let now = self.clock.now()?;
        if now < self.observed_at
            || now.saturating_sub(self.observed_at) > CONTAINER_STORAGE_MAX_AGE_SECONDS
            || Instant::now() >= self.expires
        {
            return Err(conflict());
        }
        Ok(())
    }
}

struct Collected {
    declarations: ContainerStorageInventory,
    running: Vec<EngineRunningProcess>,
}
async fn collect(
    authenticated: &AuthenticatedEngine,
    lifetime: &EvidenceLifetime,
) -> Result<Collected> {
    lifetime.check()?;
    let deadline = lifetime
        .expires
        .min(Instant::now() + Duration::from_secs(COLLECTION_SECONDS));
    tokio::time::timeout_at(deadline, async {
        let budget = ResponseBudget::new();
        let get = |path, limit| authenticated.json(path, limit, &budget, lifetime);
        let version = parse_api_version(&get("/version".into(), 8192).await?)?;
        let engine_identity = engine_id(&get(format!("/{version}/info"), 128 * 1024).await?)?;
        let list_path = format!("/{version}/containers/json?all=1");
        let ids = container_ids(&get(list_path.clone(), 512 * 1024).await?)?;
        let mut containers = Vec::with_capacity(ids.len());
        let mut pids = Vec::with_capacity(ids.len());
        let mut total_mounts = 0;
        let mut running_pids = BTreeSet::new();
        for id in &ids {
            let value = get(format!("/{version}/containers/{id}/json"), 512 * 1024).await?;
            let consumer = parse_storage_container(&value, id)?;
            let pid = container_pid(&value, &consumer)?;
            if pid != 0 && !running_pids.insert(pid) {
                return Err(unavailable());
            }
            total_mounts += consumer.mounts.len();
            if total_mounts > CONTAINER_STORAGE_MAX_TOTAL_MOUNTS {
                return Err(unavailable());
            }
            containers.push(consumer);
            pids.push(pid);
        }
        let declarations = ContainerStorageInventory {
            version: 1,
            engine_id: engine_identity,
            observed_at: lifetime.observed_at,
            containers,
        };
        declarations
            .validate(lifetime.clock.now()?)
            .map_err(|_| unavailable())?;
        for ((id, before), &pid) in ids.iter().zip(&declarations.containers).zip(&pids) {
            let value = get(format!("/{version}/containers/{id}/json"), 512 * 1024).await?;
            let after = parse_storage_container(&value, id)?;
            if after != *before || container_pid(&value, &after)? != pid {
                return Err(conflict());
            }
        }
        if container_ids(&get(list_path, 512 * 1024).await?)? != ids
            || engine_id(&get(format!("/{version}/info"), 128 * 1024).await?)?
                != declarations.engine_id
        {
            return Err(conflict());
        }
        declarations
            .validate(lifetime.clock.now()?)
            .map_err(|_| unavailable())?;
        lifetime.check()?;
        let running = declarations
            .containers
            .iter()
            .zip(pids)
            .filter(|(consumer, _)| consumer.container.running)
            .map(|(consumer, pid)| EngineRunningProcess {
                container: consumer.container.clone(),
                pid,
            })
            .collect();
        Ok(Collected {
            declarations,
            running,
        })
    })
    .await
    .map_err(|_| {
        if Instant::now() >= lifetime.expires {
            conflict()
        } else {
            unavailable()
        }
    })?
}
fn engine_id(value: &Value) -> Result<String> {
    Ok(value["ID"].as_str().ok_or_else(unavailable)?.into())
}
fn container_pid(value: &Value, consumer: &ContainerStorageConsumer) -> Result<u32> {
    let pid = value["State"]["Pid"].as_u64().ok_or_else(unavailable)?;
    if (consumer.container.running && !(2..=i32::MAX as u64).contains(&pid))
        || (!consumer.container.running && pid != 0)
    {
        return Err(unavailable());
    }
    Ok(pid as u32)
}
fn check_report(snapshot: &EngineStorageProcessSnapshot) -> Result<()> {
    struct Size(usize);
    impl Write for Size {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .filter(|&size| size <= MAX_REPORT_BYTES)
                .ok_or_else(|| std::io::Error::other("bounded evidence report exceeded"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Size(0), snapshot).map_err(|_| unavailable())
}

#[cfg(test)]
mod tests;
