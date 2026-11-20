//! Authenticated GET-only Engine observations; no retained daemon/process authority.
pub use super::read_admission::EngineReadProfile;
use super::{
    Docker, StatusCode, Value,
    compose_evidence::{self, ComposeContainerMembership, ComposeOwnership, EngineComposeSnapshot},
    health_evidence::{self, ContainerHealth, EngineHealthSnapshot},
    parse_api_version,
    read_admission::{Admission, Json, Scope},
    storage::Refusal,
};
use limeos_domain::deployment::discovery::{OmissionReason, OmittedContainer};
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

#[cfg(feature = "deferred")]
/// Stable declared facts for a review identity; original observation age stays separate.
#[derive(Serialize)]
pub(crate) struct EngineReviewFacts<'a> {
    version: u16,
    endpoint: &'a Path,
    policy: &'a EnginePeerPolicy,
    socket: &'a SocketIdentity,
    declaration_version: u16,
    engine_id: &'a str,
    containers: &'a [limeos_domain::ContainerStorageConsumer],
    running: &'a [EngineRunningProcess],
}

/// Only authenticated complete collection constructs this owner. Reports cannot
/// reconstruct its socket descriptor, original lifetime or fresh observations.
pub struct EngineStorageProcessEvidence {
    snapshot: EngineStorageProcessSnapshot,
    // Charged projection data drops before its payload reservation.
    compose: Option<compose_evidence::Capture>,
    health: Option<health_evidence::Capture>,
    authenticated: AuthenticatedEngine,
    lifetime: EvidenceLifetime,
    admission: Option<Admission>,
    _scope: Option<Scope>,
    _launch: Option<Scope>,
}
impl EngineStorageProcessEvidence {
    #[cfg(feature = "deferred")]
    pub(crate) fn review_facts(&self) -> EngineReviewFacts<'_> {
        EngineReviewFacts {
            version: self.snapshot.version,
            endpoint: &self.snapshot.endpoint,
            policy: &self.snapshot.policy,
            socket: &self.snapshot.socket,
            declaration_version: self.snapshot.declarations.version,
            engine_id: &self.snapshot.declarations.engine_id,
            containers: &self.snapshot.declarations.containers,
            running: &self.snapshot.running,
        }
    }
    pub fn snapshot(&self) -> &EngineStorageProcessSnapshot {
        &self.snapshot
    }
    pub fn declarations(&self) -> &ContainerStorageInventory {
        &self.snapshot.declarations
    }
    pub fn running(&self) -> &[EngineRunningProcess] {
        &self.snapshot.running
    }
    pub(super) fn compose_snapshot(&self) -> Option<&EngineComposeSnapshot> {
        self.compose.as_ref().map(|capture| &capture.snapshot)
    }
    pub(super) fn health_snapshot(&self) -> Option<&EngineHealthSnapshot> {
        self.health.as_ref().map(|capture| &capture.snapshot)
    }
    /// Fresh GETs compare complete facts; they do not extend the original age,
    /// authenticate a daemon PID, retain container processes or authorize effects.
    pub async fn revalidate(&self) -> Result<()> {
        let validation = self
            .compose
            .as_ref()
            .map(compose_evidence::Capture::begin)
            .transpose()?;
        let health_validation = self
            .health
            .as_ref()
            .map(health_evidence::Capture::begin)
            .transpose()?;
        let result = async {
            self.lifetime.check()?;
            let fresh = collect(
                &self.authenticated,
                &self.lifetime,
                self.admission.as_ref(),
                if self.compose.is_some() {
                    Projection::Compose
                } else if self.health.is_some() {
                    Projection::Health
                } else {
                    Projection::Storage
                },
            )
            .await?;
            self.lifetime.check()?;
            if fresh.declarations.engine_id != self.snapshot.declarations.engine_id
                || fresh.declarations.containers != self.snapshot.declarations.containers
                || fresh.running != self.snapshot.running
                || fresh.compose.as_ref() != self.compose_snapshot()
                || fresh.health.as_ref() != self.health_snapshot()
            {
                return Err(conflict());
            }
            self.authenticated.revalidate_socket()?;
            self.lifetime.check()
        }
        .await;
        let result = if let Some(validation) = health_validation {
            validation.finish(result)
        } else {
            result
        };
        if let Some(validation) = validation {
            validation.finish(result)
        } else {
            result
        }
    }
}

impl EngineStorageProcessEvidence {
    /// For a display projection (Compose or health) only. A fresh collection
    /// must still name the same Engine; the resources whose facts differ from
    /// the original, or that the fresh read left out, are returned so a reader
    /// shows them as changing. Evidence for an effect uses `revalidate`.
    pub async fn revalidate_display(&self) -> Result<Vec<String>> {
        let projection = if self.compose.is_some() {
            Projection::Compose
        } else if self.health.is_some() {
            Projection::Health
        } else {
            return Err(unavailable());
        };
        let validation = self
            .compose
            .as_ref()
            .map(compose_evidence::Capture::begin)
            .transpose()?;
        let health_validation = self
            .health
            .as_ref()
            .map(health_evidence::Capture::begin)
            .transpose()?;
        let result = async {
            self.lifetime.check()?;
            let fresh = collect(
                &self.authenticated,
                &self.lifetime,
                self.admission.as_ref(),
                projection,
            )
            .await?;
            self.lifetime.check()?;
            if fresh.declarations.engine_id != self.snapshot.declarations.engine_id {
                return Err(conflict());
            }
            let mut changed = Vec::new();
            for consumer in &self.snapshot.declarations.containers {
                let resource = &consumer.container.resource;
                let running = |list: &[EngineRunningProcess]| {
                    list.iter()
                        .find(|r| r.container.resource == *resource)
                        .map(|r| r.pid)
                };
                let same = fresh
                    .declarations
                    .containers
                    .iter()
                    .find(|c| c.container.resource == *resource)
                    == Some(consumer)
                    && running(&fresh.running) == running(&self.snapshot.running)
                    && match (self.compose_snapshot(), &fresh.compose) {
                        (Some(old), Some(new)) => {
                            let find = |s: &EngineComposeSnapshot| {
                                s.containers
                                    .iter()
                                    .find(|m| m.container.resource == *resource)
                                    .cloned()
                            };
                            find(old) == find(new)
                        }
                        _ => true,
                    }
                    && match (self.health_snapshot(), &fresh.health) {
                        (Some(old), Some(new)) => {
                            let find = |s: &EngineHealthSnapshot| {
                                s.containers.iter().position(|m| m.resource == *resource)
                            };
                            match (find(old), find(new)) {
                                (Some(a), Some(b)) => {
                                    old.containers.get(a) == new.containers.get(b)
                                }
                                _ => false,
                            }
                        }
                        _ => true,
                    };
                if !same {
                    changed.push(resource.clone());
                }
            }
            self.authenticated.revalidate_socket()?;
            self.lifetime.check()?;
            Ok(changed)
        }
        .await;
        let outcome = result.as_ref().map(|_| ()).map_err(|e| *e);
        let outcome = if let Some(validation) = health_validation {
            validation.finish(outcome)
        } else {
            outcome
        };
        let outcome = if let Some(validation) = validation {
            validation.finish(outcome)
        } else {
            outcome
        };
        outcome.and(result)
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
    inspect_selected(docker, policy, clock, admission, false).await
}
pub(super) async fn inspect_compose(
    docker: &Docker,
    policy: EnginePeerPolicy,
) -> Result<EngineStorageProcessEvidence> {
    inspect_selected(
        docker,
        policy,
        TimeSource::default(),
        Some(Admission::new(EngineReadProfile::EngineWideV1)),
        true,
    )
    .await
}
async fn inspect_selected(
    docker: &Docker,
    policy: EnginePeerPolicy,
    clock: TimeSource,
    admission: Option<Admission>,
    capture_compose: bool,
) -> Result<EngineStorageProcessEvidence> {
    inspect_projection(
        docker,
        policy,
        clock,
        admission,
        if capture_compose {
            Projection::Compose
        } else {
            Projection::Storage
        },
    )
    .await
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Projection {
    Storage,
    Compose,
    Health,
}
pub(super) async fn inspect_health(
    docker: &Docker,
    policy: EnginePeerPolicy,
) -> Result<EngineStorageProcessEvidence> {
    inspect_projection(
        docker,
        policy,
        TimeSource::default(),
        Some(Admission::new(EngineReadProfile::CombinedV1)),
        Projection::Health,
    )
    .await
}
async fn inspect_projection(
    docker: &Docker,
    policy: EnginePeerPolicy,
    clock: TimeSource,
    admission: Option<Admission>,
    projection: Projection,
) -> Result<EngineStorageProcessEvidence> {
    if projection != Projection::Storage && admission.is_none() {
        return Err(unavailable());
    }
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
    let collected = collect(&authenticated, &lifetime, admission.as_ref(), projection).await?;
    let snapshot = EngineStorageProcessSnapshot {
        version: 1,
        endpoint: docker.socket.clone(),
        policy,
        socket: authenticated.socket.identity.clone(),
        declarations: collected.declarations,
        running: collected.running,
    };
    let report_limit = admission
        .as_ref()
        .map_or(MAX_REPORT_BYTES, |a| a.limits().report);
    check_report(&snapshot, report_limit)?;
    if let Some(compose) = &collected.compose {
        #[derive(Serialize)]
        struct Report<'a> {
            storage: &'a EngineStorageProcessSnapshot,
            compose: &'a EngineComposeSnapshot,
        }
        check_report(
            &Report {
                storage: &snapshot,
                compose,
            },
            report_limit,
        )?;
    }
    if let Some(health) = &collected.health {
        #[derive(Serialize)]
        struct Report<'a> {
            storage: &'a EngineStorageProcessSnapshot,
            health: &'a EngineHealthSnapshot,
        }
        check_report(
            &Report {
                storage: &snapshot,
                health,
            },
            report_limit,
        )?;
    }
    lifetime.check()?;
    Ok(EngineStorageProcessEvidence {
        snapshot,
        authenticated,
        lifetime,
        admission,
        _scope: collected._scope,
        _launch: launch,
        compose: collected.compose.map(compose_evidence::Capture::new),
        health: collected.health.map(health_evidence::Capture::new),
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
        unique: bool,
    ) -> Result<Json> {
        lifetime.check()?;
        if let Some(admission) = admission {
            admission.ensure_body()?;
        }
        let list = path.ends_with("/containers/json?all=1");
        let limit = match admission {
            Some(a) if list => limit.min(a.limits().list_response),
            Some(a) => limit.min(a.limits().response),
            None => limit,
        };
        // Reserve body/decoder/HTTP scratch before the transport or serde can grow.
        let scope = admission.map(|a| a.response_scope(limit)).transpose()?;
        let value = tokio::time::timeout(Duration::from_secs(REQUEST_SECONDS), async {
            let (status, bytes) = self
                .docker
                .request_inner("GET", path, limit, Some((self, budget)))
                .await?;
            if status != StatusCode::OK {
                return Err(unavailable());
            }
            match scope {
                Some(scope) if unique => Json::decode_unique(&bytes, scope, list),
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

impl AuthenticatedEngine {
    /// The Engine-wide listing for a display projection: every row decoded for
    /// its identity, state and name only, so more containers than the profile
    /// admits can be ordered and truncated rather than refused.
    async fn listing(
        &self,
        path: String,
        budget: &ResponseBudget,
        lifetime: &EvidenceLifetime,
        admission: &Admission,
    ) -> Result<Vec<super::storage::Listed>> {
        lifetime.check()?;
        admission.ensure_body()?;
        let limits = admission.limits();
        // Body Vec growth overlap plus the selected rows, and HTTP scratch.
        let mut scope = admission.scope();
        scope.charge(4 * limits.listing + 64 * 1024)?;
        let rows = tokio::time::timeout(Duration::from_secs(REQUEST_SECONDS), async {
            let (status, bytes) = self
                .docker
                .request_inner("GET", path, limits.listing, Some((self, budget)))
                .await?;
            if status != StatusCode::OK {
                return Err(unavailable());
            }
            super::storage::parse_listing(&bytes, limits.listed)
        })
        .await
        .map_err(|_| unavailable())??;
        lifetime.check()?;
        Ok(rows)
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
    compose: Option<EngineComposeSnapshot>,
    health: Option<EngineHealthSnapshot>,
    _scope: Option<Scope>,
}
async fn collect(
    authenticated: &AuthenticatedEngine,
    lifetime: &EvidenceLifetime,
    admission: Option<&Admission>,
    projection: Projection,
) -> Result<Collected> {
    let capture_compose = projection == Projection::Compose;
    let capture_health = projection == Projection::Health;
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
        let get = |path, limit| {
            authenticated.json(
                path,
                limit,
                &budget,
                lifetime,
                admission,
                projection != Projection::Storage,
            )
        };
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
        let listing = Listing {
            authenticated,
            budget: &budget,
            lifetime,
            admission,
            projection,
            path: &list_path,
        };
        let (ids, truncated) = listing.ids(scope.as_mut()).await?;
        // Storage evidence refuses on any odd or changing container. A display
        // projection leaves that one container out with a reason instead.
        let tolerant = projection != Projection::Storage;
        let mut kept: Vec<&String> = Vec::new();
        let mut containers = Vec::new();
        let mut pids = Vec::new();
        let mut members = Vec::new();
        let mut health_members = Vec::new();
        let mut left_out = LeftOut::default();
        if let Some(scope) = &mut scope {
            scope.reserve(&mut kept, ids.len())?;
            scope.reserve(&mut containers, ids.len())?;
            scope.reserve(&mut pids, ids.len())?;
            if capture_compose {
                scope.reserve(&mut members, ids.len())?;
            }
            if capture_health {
                scope.reserve(&mut health_members, ids.len())?;
            }
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
                if running_count > admission.map_or(0, |a| a.limits().running) {
                    return Err(unavailable());
                }
            }
            let admitted = admit(
                &value,
                id,
                projection,
                admission
                    .map_or(0, |a| a.limits().mounts_total)
                    .saturating_sub(total_mounts),
                scope.as_mut(),
            );
            let admitted = match admitted {
                Ok(admitted) => {
                    if tolerant && !single_valid(&admitted.consumer, &engine_identity, lifetime)? {
                        Err(Refusal::Unsupported)
                    } else {
                        Ok(admitted)
                    }
                }
                other => other,
            };
            let Admitted {
                consumer,
                pid,
                member,
                health,
            } = match admitted {
                Ok(admitted) => admitted,
                Err(Refusal::Unsupported) if tolerant => {
                    let scope = scope.as_mut().ok_or_else(unavailable)?;
                    let project = if capture_compose {
                        compose_evidence::declared_project(&value, scope)?
                    } else {
                        None
                    };
                    let resource = format!("container:{id}");
                    left_out.add(
                        projection,
                        &resource,
                        project,
                        OmissionReason::Unsupported,
                        None,
                        scope,
                    )?;
                    continue;
                }
                Err(refusal) => return Err(refusal.strict()),
            };
            if pid != 0 {
                if let Some(scope) = &mut scope {
                    scope.charge(32)?;
                }
            }
            if pid != 0 && !running_pids.insert(pid) {
                if !tolerant {
                    return Err(unavailable());
                }
                // A PID reused by a restarting container: changing, not stable.
                let scope = scope.as_mut().ok_or_else(unavailable)?;
                let project = member.as_ref().and_then(project_of);
                left_out.add(
                    projection,
                    &consumer.container.resource,
                    project,
                    OmissionReason::Changing,
                    health.as_ref(),
                    scope,
                )?;
                continue;
            }
            total_mounts += consumer.mounts.len();
            if total_mounts > CONTAINER_STORAGE_MAX_TOTAL_MOUNTS {
                return Err(unavailable());
            }
            kept.push(id);
            containers.push(consumer);
            pids.push(pid);
            if let Some(member) = member {
                members.push(member);
            }
            if let Some(health) = health {
                health_members.push(health);
            }
        }
        if capture_compose {
            // Two members naming one project with different directories or
            // files: that project is unsupported, the others stay.
            let names: Vec<String> = compose_evidence::conflicting(&members)
                .into_iter()
                .map(str::to_owned)
                .collect();
            let mut index = 0;
            while let Some(member) = members.get(index) {
                let Some(project) = project_of(member).filter(|p| names.contains(p)) else {
                    index += 1;
                    continue;
                };
                let resource = member.container.resource.clone();
                remove_at(
                    index,
                    &mut kept,
                    &mut containers,
                    &mut pids,
                    &mut members,
                    &mut health_members,
                );
                left_out.add(
                    projection,
                    &resource,
                    Some(project),
                    OmissionReason::Unsupported,
                    None,
                    scope.as_mut().ok_or_else(unavailable)?,
                )?;
            }
        }
        // The second pass compares each container with its first.
        let mut changing = Vec::new();
        if let Some(scope) = &mut scope {
            scope.reserve(&mut changing, kept.len())?;
        }
        for (index, id) in kept.iter().enumerate() {
            let value = get(format!("/{version}/containers/{id}/json"), 512 * 1024).await?;
            let mut comparison = admission.map(Admission::scope);
            let same = unchanged(
                &value,
                id,
                Before {
                    consumer: containers.get(index).ok_or_else(unavailable)?,
                    pid: pids.get(index).copied().ok_or_else(unavailable)?,
                    member: members.get(index),
                    health: health_members.get(index),
                },
                projection,
                admission.map_or(0, |a| a.limits().mounts_each),
                comparison.as_mut(),
            );
            match same {
                Ok(true) => {}
                Ok(false) | Err(Refusal::Unsupported) if tolerant => changing.push(index),
                Ok(false) => return Err(conflict()),
                Err(refusal) => return Err(refusal.strict()),
            }
        }
        for index in changing.into_iter().rev() {
            let resource = containers
                .get(index)
                .map(|c| c.container.resource.clone())
                .ok_or_else(unavailable)?;
            let (member, known) = remove_at(
                index,
                &mut kept,
                &mut containers,
                &mut pids,
                &mut members,
                &mut health_members,
            );
            left_out.add(
                projection,
                &resource,
                member.as_ref().and_then(project_of),
                OmissionReason::Changing,
                known.as_ref(),
                scope.as_mut().ok_or_else(unavailable)?,
            )?;
        }
        let mut comparison = admission.map(Admission::scope);
        if listing.ids(comparison.as_mut()).await? != (ids.clone(), truncated) {
            return Err(conflict());
        }
        drop(comparison);
        let declarations = ContainerStorageInventory {
            version: 1,
            engine_id: engine_identity,
            observed_at: lifetime.observed_at,
            containers,
        };
        let mut comparison = admission.map(Admission::scope);
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
        let compose = if capture_compose {
            Some(EngineComposeSnapshot {
                version: 1,
                engine_id: scope
                    .as_mut()
                    .ok_or_else(unavailable)?
                    .string(&declarations.engine_id)?,
                containers_observed_at: lifetime.observed_at,
                containers: members,
                truncated: u16::try_from(truncated).map_err(|_| unavailable())?,
                omitted: left_out.compose,
            })
        } else {
            None
        };
        let health = if capture_health {
            Some(EngineHealthSnapshot {
                version: 1,
                engine_id: scope
                    .as_mut()
                    .ok_or_else(unavailable)?
                    .string(&declarations.engine_id)?,
                containers_observed_at: lifetime.observed_at,
                complete: health_evidence::complete(&health_members)
                    && health_evidence::complete(&left_out.health),
                containers: {
                    let mut all = health_members;
                    if let Some(scope) = &mut scope {
                        scope.reserve(&mut all, left_out.health.len())?;
                    }
                    all.extend(left_out.health);
                    all.sort_by(|a, b| a.resource.cmp(&b.resource));
                    all
                },
            })
        } else {
            None
        };
        Ok(Collected {
            declarations,
            running,
            _scope: scope,
            compose,
            health,
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
/// The Engine-wide container IDs of one collection pass.
struct Listing<'a> {
    authenticated: &'a AuthenticatedEngine,
    budget: &'a ResponseBudget,
    lifetime: &'a EvidenceLifetime,
    admission: Option<&'a Admission>,
    projection: Projection,
    path: &'a str,
}
impl Listing<'_> {
    /// Storage evidence refuses an Engine over its bound. The Compose display
    /// projection keeps the first containers, running ones first, and counts
    /// the rest.
    async fn ids(&self, scope: Option<&mut Scope>) -> Result<(Vec<String>, usize)> {
        if self.projection == Projection::Compose {
            let admission = self.admission.ok_or_else(unavailable)?;
            let rows = self
                .authenticated
                .listing(self.path.into(), self.budget, self.lifetime, admission)
                .await?;
            return super::storage::select_listed(
                rows,
                admission.limits().consumers,
                scope.ok_or_else(unavailable)?,
            );
        }
        let list = self
            .authenticated
            .json(
                self.path.into(),
                512 * 1024,
                self.budget,
                self.lifetime,
                self.admission,
                self.projection != Projection::Storage,
            )
            .await?;
        let ids = super::storage::parse_storage_container_ids_with_admission(&list, scope)?;
        Ok((ids, 0))
    }
}
/// One container's first-pass facts in the selected projection.
struct Admitted {
    consumer: ContainerStorageConsumer,
    pid: u32,
    member: Option<ComposeContainerMembership>,
    health: Option<ContainerHealth>,
}
fn admit(
    value: &Value,
    id: &str,
    projection: Projection,
    mounts_left: usize,
    mut scope: Option<&mut Scope>,
) -> std::result::Result<Admitted, Refusal> {
    let consumer = super::storage::parse_storage_container_with_admission(
        value,
        id,
        mounts_left,
        scope.as_deref_mut(),
    )?;
    let pid = container_pid(value, &consumer).map_err(|_| Refusal::Unsupported)?;
    let member = if projection == Projection::Compose {
        Some(compose_evidence::parse(
            value,
            &consumer.container,
            scope.as_deref_mut().ok_or_else(unavailable)?,
        )?)
    } else {
        None
    };
    let health = if projection == Projection::Health {
        Some(health_evidence::parse(
            value,
            &consumer.container,
            scope.ok_or_else(unavailable)?,
        )?)
    } else {
        None
    };
    Ok(Admitted {
        consumer,
        pid,
        member,
        health,
    })
}
/// One consumer alone passes the inventory's own validation. A shape it
/// refuses (`InvalidInput`) is unsupported; any other refusal is not per item.
fn single_valid(
    consumer: &ContainerStorageConsumer,
    engine_id: &str,
    lifetime: &EvidenceLifetime,
) -> Result<bool> {
    let single = ContainerStorageInventory {
        version: 1,
        engine_id: engine_id.into(),
        observed_at: lifetime.observed_at,
        containers: vec![consumer.clone()],
    };
    match single.validate(lifetime.clock.now()?) {
        Ok(()) => Ok(true),
        Err(Error(ErrorCode::InvalidInput)) => Ok(false),
        Err(_) => Err(unavailable()),
    }
}
/// The first-pass facts of one kept container.
struct Before<'a> {
    consumer: &'a ContainerStorageConsumer,
    pid: u32,
    member: Option<&'a ComposeContainerMembership>,
    health: Option<&'a ContainerHealth>,
}
/// Whether the second inspection repeats the first in every selected fact.
fn unchanged(
    value: &Value,
    id: &str,
    before: Before<'_>,
    projection: Projection,
    mounts_each: usize,
    mut comparison: Option<&mut Scope>,
) -> std::result::Result<bool, Refusal> {
    let after = super::storage::parse_storage_container_with_admission(
        value,
        id,
        mounts_each,
        comparison.as_deref_mut(),
    )?;
    if after != *before.consumer
        || container_pid(value, &after).map_err(|_| Refusal::Unsupported)? != before.pid
    {
        return Ok(false);
    }
    if projection == Projection::Compose
        && Some(&compose_evidence::parse(
            value,
            &after.container,
            comparison.as_deref_mut().ok_or_else(unavailable)?,
        )?) != before.member
    {
        return Ok(false);
    }
    if projection == Projection::Health
        && Some(&health_evidence::parse(
            value,
            &after.container,
            comparison.ok_or_else(unavailable)?,
        )?) != before.health
    {
        return Ok(false);
    }
    Ok(true)
}
fn project_of(member: &ComposeContainerMembership) -> Option<String> {
    match &member.ownership {
        ComposeOwnership::Compose { project, .. } => Some(project.name.clone()),
        ComposeOwnership::NonCompose => None,
    }
}
/// Drop one kept container from every parallel record, returning its
/// projection facts.
fn remove_at(
    index: usize,
    kept: &mut Vec<&String>,
    containers: &mut Vec<ContainerStorageConsumer>,
    pids: &mut Vec<u32>,
    members: &mut Vec<ComposeContainerMembership>,
    health: &mut Vec<ContainerHealth>,
) -> (Option<ComposeContainerMembership>, Option<ContainerHealth>) {
    if index < kept.len() {
        kept.remove(index);
    }
    if index < containers.len() {
        containers.remove(index);
    }
    if index < pids.len() {
        pids.remove(index);
    }
    (
        (index < members.len()).then(|| members.remove(index)),
        (index < health.len()).then(|| health.remove(index)),
    )
}
/// Containers a display projection left out, with their reason.
#[derive(Default)]
struct LeftOut {
    compose: Vec<OmittedContainer>,
    health: Vec<ContainerHealth>,
}
impl LeftOut {
    fn add(
        &mut self,
        projection: Projection,
        resource: &str,
        project: Option<String>,
        reason: OmissionReason,
        known: Option<&ContainerHealth>,
        scope: &mut Scope,
    ) -> Result<()> {
        match projection {
            Projection::Compose => {
                scope.reserve(&mut self.compose, 1)?;
                self.compose
                    .push(compose_evidence::omitted(resource, project, reason, scope)?);
            }
            Projection::Health => {
                scope.reserve(&mut self.health, 1)?;
                self.health
                    .push(health_evidence::unavailable(resource, known, scope)?);
            }
            Projection::Storage => return Err(unavailable()),
        }
        Ok(())
    }
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
fn check_report(snapshot: &impl Serialize, limit: usize) -> Result<()> {
    struct Size(usize, usize);
    impl Write for Size {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .filter(|&size| size <= self.1)
                .ok_or_else(|| std::io::Error::other("bounded evidence report exceeded"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Size(0, limit), snapshot).map_err(|_| unavailable())
}

#[cfg(test)]
mod tests;
