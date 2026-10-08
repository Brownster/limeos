//! Authenticated GET-only Engine observations; no retained daemon/process authority.
pub use super::read_admission::EngineReadProfile;
use super::{
    Docker, StatusCode, Value, parse_api_version,
    read_admission::{self, Admission, Json, Scope},
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
    admission: Option<Admission>,
    _scope: Option<Scope>,
    _launch: Option<Scope>,
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
        let fresh = collect(&self.authenticated, &self.lifetime, self.admission.as_ref()).await?;
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
    /// Closed admission for later bounded composition. No runtime/wire selection.
    pub async fn inspect_storage_processes_with_profile(
        &self,
        policy: EnginePeerPolicy,
        profile: EngineReadProfile,
    ) -> Result<EngineStorageProcessEvidence> {
        inspect_profile(
            self,
            policy,
            TimeSource::default(),
            Some(Admission::new(profile)),
        )
        .await
    }
}
async fn inspect(
    docker: &Docker,
    policy: EnginePeerPolicy,
    clock: TimeSource,
) -> Result<EngineStorageProcessEvidence> {
    inspect_profile(docker, policy, clock, None).await
}
async fn inspect_profile(
    docker: &Docker,
    policy: EnginePeerPolicy,
    clock: TimeSource,
    admission: Option<Admission>,
) -> Result<EngineStorageProcessEvidence> {
    let lifetime = EvidenceLifetime::new(clock)?;
    if admission.is_some()
        && (!docker.socket.is_absolute() || docker.socket.as_os_str().len() > 512)
    {
        return Err(Error(ErrorCode::InvalidInput));
    }
    let mut launch = admission.as_ref().map(Admission::scope);
    if let Some(scope) = &mut launch {
        scope.charge(8192)?;
    }
    let authenticated = AuthenticatedEngine::open(docker.clone(), policy)?;
    let collected = collect(&authenticated, &lifetime, admission.as_ref()).await?;
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
        admission,
        _scope: collected._scope,
        _launch: launch,
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
        admission: Option<&Admission>,
    ) -> Result<Json> {
        lifetime.check()?;
        if let Some(admission) = admission {
            admission.ensure_body()?;
        }
        let limit = if admission.is_some() {
            limit.min(read_admission::RESPONSE)
        } else {
            limit
        };
        // Reserve body/decoder/HTTP scratch before the transport or serde can grow.
        let scope = admission.map(|a| a.response_scope(limit)).transpose()?;
        let list = path.ends_with("/containers/json?all=1");
        let value = tokio::time::timeout(Duration::from_secs(REQUEST_SECONDS), async {
            let (status, bytes, _) = self
                .docker
                .request_clipped_inner("GET", path, limit, false, Some((self, budget)))
                .await?;
            if status != StatusCode::OK {
                return Err(unavailable());
            }
            match scope {
                Some(scope) => Json::decode(&bytes, scope, list),
                None => serde_json::from_slice(&bytes)
                    .map(Json::standard)
                    .map_err(|_| unavailable()),
            }
        })
        .await
        .map_err(|_| unavailable())??;
        lifetime.check()?;
        Ok(value)
    }
}

pub(super) struct ResponseBudget(AtomicUsize, Option<Admission>);
impl ResponseBudget {
    fn new() -> Self {
        Self(AtomicUsize::new(MAX_RESPONSE_BYTES), None)
    }
    fn for_admission(admission: Option<&Admission>) -> Self {
        let mut budget = Self::new();
        budget.1 = admission.cloned();
        budget
    }
    pub(super) fn constrained(&self) -> bool {
        self.1.is_some()
    }
    pub(super) fn consume(&self, bytes: usize) -> Result<()> {
        if let Some(admission) = &self.1 {
            return admission.consume(bytes);
        }
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
    _scope: Option<Scope>,
}
async fn collect(
    authenticated: &AuthenticatedEngine,
    lifetime: &EvidenceLifetime,
    admission: Option<&Admission>,
) -> Result<Collected> {
    lifetime.check()?;
    let _collecting = admission.map(Admission::collect).transpose()?;
    let deadline = lifetime
        .expires
        .min(Instant::now() + Duration::from_secs(COLLECTION_SECONDS));
    tokio::time::timeout_at(deadline, async {
        let budget = ResponseBudget::for_admission(admission);
        let mut scope = admission.map(Admission::scope);
        // Closed16/32 validator sets, fixed version/URI formatting and small
        // request/error bookkeeping. This is a conservative counted allowance,
        // not individually fallible std allocations or total allocator heap.
        if let Some(scope) = &mut scope {
            scope.charge(8192)?;
        }
        let get = |path, limit| authenticated.json(path, limit, &budget, lifetime, admission);
        let version_json = get("/version".into(), 8192).await?;
        let version = parse_api_version(&version_json)?;
        drop(version_json);
        if let Some(scope) = &mut scope {
            scope.charge(version.len())?;
        }
        let info = get(format!("/{version}/info"), 128 * 1024).await?;
        let engine_identity = engine_id_with_scope(&info, scope.as_mut())?;
        drop(info);
        let list_path = format!("/{version}/containers/json?all=1");
        let list = get(list_path.clone(), 512 * 1024).await?;
        let ids =
            super::storage::parse_storage_container_ids_with_admission(&list, scope.as_mut())?;
        drop(list);
        let mut containers = Vec::new();
        let mut pids = Vec::new();
        if let Some(scope) = &mut scope {
            scope.reserve(&mut containers, ids.len())?;
            scope.reserve(&mut pids, ids.len())?;
        } else {
            containers.reserve(ids.len());
            pids.reserve(ids.len());
        }
        let mut total_mounts = 0;
        let mut running_pids = BTreeSet::new();
        let mut running_count = 0;
        for id in &ids {
            let value = get(format!("/{version}/containers/{id}/json"), 512 * 1024).await?;
            if admission.is_some() && value["State"]["Running"].as_bool() == Some(true) {
                running_count += 1;
                if running_count > read_admission::RUNNING {
                    return Err(unavailable());
                }
            }
            let consumer = super::storage::parse_storage_container_with_admission(
                &value,
                id,
                read_admission::MOUNTS.saturating_sub(total_mounts),
                scope.as_mut(),
            )?;
            let pid = container_pid(&value, &consumer)?;
            if pid != 0 {
                if let Some(scope) = &mut scope {
                    scope.charge(32)?;
                }
            }
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
            let mut comparison = admission.map(Admission::scope);
            let after = super::storage::parse_storage_container_with_admission(
                &value,
                id,
                read_admission::MOUNTS,
                comparison.as_mut(),
            )?;
            if after != *before || container_pid(&value, &after)? != pid {
                return Err(conflict());
            }
        }
        let mut comparison = admission.map(Admission::scope);
        let list_json = get(list_path, 512 * 1024).await?;
        if super::storage::parse_storage_container_ids_with_admission(
            &list_json,
            comparison.as_mut(),
        )? != ids
        {
            return Err(conflict());
        }
        drop(list_json);
        let info_json = get(format!("/{version}/info"), 128 * 1024).await?;
        if engine_id_with_scope(&info_json, comparison.as_mut())? != declarations.engine_id {
            return Err(conflict());
        }
        drop(info_json);
        declarations
            .validate(lifetime.clock.now()?)
            .map_err(|_| unavailable())?;
        lifetime.check()?;
        let mut running = Vec::new();
        for (consumer, pid) in declarations.containers.iter().zip(pids) {
            if !consumer.container.running {
                continue;
            }
            let container = if let Some(scope) = &mut scope {
                scope.reserve(&mut running, 1)?;
                ContainerSnapshot {
                    resource: scope.string(&consumer.container.resource)?,
                    image: scope.string(&consumer.container.image)?,
                    started_at: scope.string(&consumer.container.started_at)?,
                    running: consumer.container.running,
                }
            } else {
                consumer.container.clone()
            };
            running.push(EngineRunningProcess { container, pid });
        }
        Ok(Collected {
            declarations,
            running,
            _scope: scope,
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
fn engine_id_with_scope(value: &Value, scope: Option<&mut Scope>) -> Result<String> {
    match scope {
        Some(scope) => scope.string(value["ID"].as_str().ok_or_else(unavailable)?),
        None => engine_id(value),
    }
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
