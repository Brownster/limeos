//! Same-object running destinations linked to retained physical source evidence.
//! No Engine reacquisition, namespace entry, data read or operation authority.
use crate::{
    ContainerDependencyEvidence, ContainerDependencySnapshot, ContainerSourceIdentity,
    ContainerSourceKind, ContainerSourceMount, ContainerSourcePath, ContainerSourceSnapshot,
    DeviceNumber, Failure, Result,
    processes::{
        self, ContainerProcessMount, RunningContainerProcessEvidence,
        RunningContainerProcessSnapshot,
    },
    sources,
};
use limeos_domain::{
    ContainerSnapshot, ContainerStorageInventory, ContainerStorageMount, ContainerStorageSource,
    container_storage_path,
};
use rustix::{
    fd::OwnedFd,
    fs::{AtFlags, Mode, OFlags, ResolveFlags, StatxFlags},
};
use serde::{
    Deserialize, Serialize,
    ser::{SerializeSeq, Serializer},
};
use std::{
    cell::Cell,
    io::{self, Write},
    time::{Duration, Instant},
};

#[cfg(test)]
mod tests;
const MAX_DESTINATIONS: u32 = 32;
const MAX_REPORT: u32 = 32 * 1024;
const INPUT_BYTES: usize = 64 * 1024;
const WORK: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DestinationLimits {
    pub max_destinations: u32,
    pub max_retained_fds: u32,
    pub max_report_bytes: u32,
}
impl DestinationLimits {
    fn validate(self) -> Result<()> {
        if self.max_destinations == 0
            || self.max_destinations > MAX_DESTINATIONS
            || self.max_retained_fds == 0
            || self.max_retained_fds > MAX_DESTINATIONS
            || self.max_report_bytes == 0
            || self.max_report_bytes > MAX_REPORT
        {
            return Err(Failure::InvalidPlan);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DestinationIdentity {
    pub device: DeviceNumber,
    pub mount_id: u64,
    pub inode: u64,
    pub kind: ContainerSourceKind,
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContainerDestinationObservation {
    pub container: ContainerSnapshot,
    pub source: ContainerStorageSource,
    pub destination: String,
    pub writable: bool,
    pub host_source: ContainerSourceIdentity,
    pub process_mount: ContainerProcessMount,
    pub identity: DestinationIdentity,
}
/// Serialize-only facts cannot reconstruct retained destination descriptors.
/// ```compile_fail
/// use limeos_executor_storage::destinations::ContainerDestinationSnapshot;
/// let forged: ContainerDestinationSnapshot = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug, Serialize)]
pub struct ContainerDestinationSnapshot {
    pub version: u16,
    pub declarations_observed_at: i64,
    pub declarations_digest: String,
    pub process_snapshot_digest: String,
    pub physical_snapshot_digest: String,
    pub host_root_mount_id: u64,
    pub limits: DestinationLimits,
    pub uninspected_stopped: Vec<ContainerSnapshot>,
    pub destinations: Vec<ContainerDestinationObservation>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Ready,
    Validating,
    Poisoned,
}
struct Validation(Cell<State>);
impl Validation {
    fn begin(&self) -> Result<()> {
        if self.0.replace(State::Validating) != State::Ready {
            self.0.set(State::Poisoned);
            return Err(Failure::Conflict);
        }
        Ok(())
    }
    fn finish(&self, result: Result<()>) -> Result<()> {
        if result.is_ok() && self.0.get() == State::Validating {
            self.0.set(State::Ready);
            Ok(())
        } else {
            self.0.set(State::Poisoned);
            result.and(Err(Failure::Conflict))
        }
    }
}
struct Selection<'a> {
    container: &'a ContainerSnapshot,
    declared: &'a ContainerStorageMount,
    source: &'a ContainerSourcePath,
    process_mount: &'a ContainerProcessMount,
    root: &'a OwnedFd,
}
struct Pinned<'a> {
    selection: Selection<'a>,
    fd: OwnedFd,
    identity: DestinationIdentity,
}

/// The owner borrows both genuine parents; reports cannot create it.
/// ```compile_fail
/// use limeos_executor_storage::destinations::ContainerDestinationEvidence;
/// let forged: ContainerDestinationEvidence<'static> = serde_json::from_str("{}").unwrap();
/// ```
pub struct ContainerDestinationEvidence<'a> {
    physical: &'a ContainerDependencyEvidence,
    processes: &'a RunningContainerProcessEvidence,
    pinned: Vec<Pinned<'a>>,
    snapshot: ContainerDestinationSnapshot,
    expires: Instant,
    validation: Validation,
}
impl ContainerDestinationEvidence<'_> {
    pub fn snapshot(&self) -> &ContainerDestinationSnapshot {
        &self.snapshot
    }
    /// Includes stopped consumers and conservative shared-backing relationships.
    pub fn physical_snapshot(&self) -> &ContainerDependencySnapshot {
        self.physical.snapshot()
    }
    pub fn retained_descriptor_count(&self) -> usize {
        self.pinned.len()
    }
    /// Checks original parents and names without renewing any original lease.
    pub async fn revalidate(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        self.validation.begin()?;
        let deadline = Instant::now() + WORK;
        let result = tokio::time::timeout(WORK, async {
            poll(self.expires, deadline, cancelled)?;
            self.processes.revalidate()?;
            self.physical.revalidate().await?;
            revalidate_paths(&self.pinned, &Linux, self.expires, deadline, cancelled)?;
            self.physical.revalidate().await?;
            self.processes.revalidate()?;
            poll(self.expires, deadline, cancelled)
        })
        .await
        .map_err(|_| Failure::TimedOut)
        .and_then(|r| r);
        self.validation.finish(result)
    }
}

/// Only a successful protected parent collection can supply this boundary.
/// ```compile_fail
/// use limeos_executor_storage::{ContainerDependencyEvidence, processes::RunningContainerProcessEvidence,
///   destinations::{ContainerDestinationEvidence, DestinationLimits, inspect_container_destinations}};
/// async fn leak(p: &ContainerDependencyEvidence, r: &RunningContainerProcessEvidence,
///   l: DestinationLimits) -> ContainerDestinationEvidence<'static> {
///   inspect_container_destinations(p, r, l, &|| false).await.unwrap()
/// }
/// ```
pub async fn inspect_container_destinations<'a>(
    physical: &'a ContainerDependencyEvidence,
    processes: &'a RunningContainerProcessEvidence,
    limits: DestinationLimits,
    cancelled: &dyn Fn() -> bool,
) -> Result<ContainerDestinationEvidence<'a>> {
    limits.validate()?;
    let expires = Instant::now() + WORK;
    let deadline = expires;
    let evidence = tokio::time::timeout(WORK, async {
        poll(expires, deadline, cancelled)?;
        // Admission precedes this stage's descriptor opens and parent re-probes.
        let selections = select(
            processes.declarations(),
            processes.snapshot(),
            physical.sources().snapshot(),
            physical.snapshot(),
            limits,
            |resource| processes.mounts(resource),
            |resource| processes.root_descriptor(resource),
        )?;
        processes.revalidate()?;
        physical.revalidate().await?;
        let pinned = retain(selections, &Linux, expires, deadline, cancelled)?;
        let snapshot = snapshot(
            &pinned,
            processes.declarations(),
            processes.snapshot(),
            physical.snapshot(),
            limits,
        )?;
        let evidence = ContainerDestinationEvidence {
            physical,
            processes,
            pinned,
            snapshot,
            expires,
            validation: Validation(Cell::new(State::Ready)),
        };
        evidence.revalidate(cancelled).await?;
        poll(expires, deadline, cancelled)?;
        Ok(evidence)
    })
    .await
    .map_err(|_| Failure::TimedOut)??;
    Ok(evidence)
}

fn poll(expires: Instant, deadline: Instant, cancelled: &dyn Fn() -> bool) -> Result<()> {
    if cancelled() {
        Err(Failure::Conflict)
    } else if Instant::now() >= expires || Instant::now() >= deadline {
        Err(Failure::TimedOut)
    } else {
        Ok(())
    }
}
fn local(filesystem: &str, device: DeviceNumber) -> bool {
    matches!(
        filesystem,
        "ext2" | "ext3" | "ext4" | "xfs" | "vfat" | "exfat" | "ntfs3"
    ) && device.major != 0
        || filesystem == "tmpfs" && device.major == 0
}
fn beneath(path: &str, root: &str) -> bool {
    root == "/" || path == root || path.strip_prefix(root).is_some_and(|s| s.starts_with('/'))
}
fn filesystem_root(source: &ContainerSourcePath, mount: &ContainerSourceMount) -> Result<String> {
    if !container_storage_path(&source.resolved_path)
        || !container_storage_path(&mount.mountpoint)
        || !container_storage_path(&mount.filesystem_root)
        || !beneath(&source.resolved_path, &mount.mountpoint)
    {
        return Err(Failure::IdentityMismatch);
    }
    let suffix = if mount.mountpoint == "/" {
        source.resolved_path.trim_start_matches('/')
    } else {
        source
            .resolved_path
            .strip_prefix(&mount.mountpoint)
            .ok_or(Failure::IdentityMismatch)?
            .trim_start_matches('/')
    };
    let prefix = mount.filesystem_root.trim_end_matches('/');
    let length = prefix.len() + usize::from(!suffix.is_empty()) + suffix.len();
    if length > 512 {
        return Err(Failure::Unavailable);
    }
    let mut root = String::new();
    root.try_reserve_exact(length.max(1))
        .map_err(|_| Failure::Unavailable)?;
    root.push_str(prefix);
    if !suffix.is_empty() {
        root.push('/');
        root.push_str(suffix);
    }
    if root.is_empty() {
        root.push('/');
    }
    Ok(root)
}

// Reports are private inputs here; the public entry point supplies genuine owners.
#[allow(clippy::too_many_arguments)]
fn select<'a>(
    declarations: &'a ContainerStorageInventory,
    processes: &RunningContainerProcessSnapshot,
    sources: &'a ContainerSourceSnapshot,
    physical: &ContainerDependencySnapshot,
    limits: DestinationLimits,
    mounts: impl Fn(&str) -> Option<&'a [ContainerProcessMount]>,
    roots: impl Fn(&str) -> Option<&'a OwnedFd>,
) -> Result<Vec<Selection<'a>>> {
    limits.validate()?;
    declarations.validate(sources::now()?).map_err(|e| {
        if e.0 == limeos_domain::ErrorCode::Conflict {
            Failure::Conflict
        } else {
            Failure::InvalidPlan
        }
    })?;
    let count: usize = declarations
        .containers
        .iter()
        .filter(|c| c.container.running)
        .map(|c| c.mounts.len())
        .sum();
    if count > limits.max_destinations as usize || count > limits.max_retained_fds as usize {
        return Err(Failure::Unavailable);
    }
    let digest = processes::digest_json(declarations, INPUT_BYTES)?;
    if digest != processes.declarations_digest
        || digest != sources.inventory_digest
        || digest != physical.container_inventory_digest
        || processes.declarations_observed_at != declarations.observed_at
        || sources.host_root_mount_id != physical.host_root_mount_id
        || processes.host_root_mount_id != physical.host_root_mount_id
        || sources.mounts_digest != physical.mounts_digest
        || processes.host_mount_table_digest != physical.mounts_digest
        || processes::digest_json(sources, INPUT_BYTES)? != physical.source_snapshot_digest
        || sources.sources.len()
            != declarations
                .containers
                .iter()
                .map(|c| c.mounts.len())
                .sum::<usize>()
        || processes.processes.len()
            != declarations
                .containers
                .iter()
                .filter(|c| c.container.running)
                .count()
    {
        return Err(Failure::IdentityMismatch);
    }
    // Preflight parent snapshot serialization before destination reads/clones.
    processes::digest_json(processes, INPUT_BYTES)?;
    processes::digest_json(physical, INPUT_BYTES)?;
    let mut selected = Vec::new();
    selected
        .try_reserve_exact(count)
        .map_err(|_| Failure::Unavailable)?;
    for consumer in &declarations.containers {
        if consumer.container.running
            && processes
                .processes
                .iter()
                .filter(|p| p.container == consumer.container)
                .count()
                != 1
        {
            return Err(Failure::IdentityMismatch);
        }
        for declared in &consumer.mounts {
            let mut paths = sources.sources.iter().filter(|s| {
                s.container == consumer.container.resource && s.destination == declared.destination
            });
            let source = paths.next().ok_or(Failure::IdentityMismatch)?;
            if paths.next().is_some() || source.declared_path != declared.source.path() {
                return Err(Failure::IdentityMismatch);
            }
            if !consumer.container.running {
                continue;
            }
            if source.potential_mount_ids.as_slice() != [source.identity.mount_id] {
                return Err(Failure::Unavailable);
            }
            let mut host_rows = sources
                .mounts
                .iter()
                .filter(|m| m.mount_id == source.identity.mount_id);
            let host = host_rows.next().ok_or(Failure::IdentityMismatch)?;
            if host_rows.next().is_some()
                || host.device != source.identity.device
                || !local(&host.filesystem, host.device)
            {
                return Err(Failure::Unavailable);
            }
            let table = mounts(&consumer.container.resource).ok_or(Failure::IdentityMismatch)?;
            let mut rows = table
                .iter()
                .filter(|m| m.mountpoint == declared.destination);
            let row = rows.next().ok_or(Failure::NotReady)?;
            if rows.next().is_some()
                || table.iter().any(|m| {
                    m.mountpoint != declared.destination
                        && beneath(&m.mountpoint, &declared.destination)
                })
            {
                return Err(Failure::Unavailable);
            }
            if row.device != host.device
                || row.filesystem != host.filesystem
                || !local(&row.filesystem, row.device)
                || row.writable != declared.writable
                || row.filesystem_root != filesystem_root(source, host)?
            {
                return Err(Failure::IdentityMismatch);
            }
            selected.push(Selection {
                container: &consumer.container,
                declared,
                source,
                process_mount: row,
                root: roots(&consumer.container.resource).ok_or(Failure::IdentityMismatch)?,
            });
        }
    }
    selected.sort_by(|a, b| {
        (&a.container.resource, &a.declared.destination)
            .cmp(&(&b.container.resource, &b.declared.destination))
    });
    Ok(selected)
}

trait Fs {
    fn mount_id(&self, fd: &OwnedFd) -> Result<u64>;
    fn identity(&self, fd: &OwnedFd) -> Result<DestinationIdentity>;
}
struct Linux;
impl Fs for Linux {
    fn mount_id(&self, fd: &OwnedFd) -> Result<u64> {
        sources::descriptor_mount_id(fd)
    }
    fn identity(&self, fd: &OwnedFd) -> Result<DestinationIdentity> {
        let mask = StatxFlags::TYPE
            | StatxFlags::MODE
            | StatxFlags::INO
            | StatxFlags::NLINK
            | StatxFlags::UID
            | StatxFlags::GID
            | StatxFlags::MNT_ID;
        let s = rustix::fs::statx(fd, "", AtFlags::EMPTY_PATH | AtFlags::STATX_DONT_SYNC, mask)
            .map_err(|_| Failure::Unavailable)?;
        if s.stx_mask & mask.bits() != mask.bits()
            || s.stx_ino == 0
            || s.stx_nlink == 0
            || s.stx_mnt_id == 0
        {
            return Err(Failure::Unavailable);
        }
        let kind = match s.stx_mode & 0o170000 {
            0o100000 => ContainerSourceKind::RegularFile,
            0o040000 => ContainerSourceKind::Directory,
            _ => return Err(Failure::UnsafePath),
        };
        Ok(DestinationIdentity {
            device: DeviceNumber {
                major: s.stx_dev_major,
                minor: s.stx_dev_minor,
            },
            mount_id: s.stx_mnt_id,
            inode: s.stx_ino,
            kind,
            mode: s.stx_mode,
            uid: s.stx_uid,
            gid: s.stx_gid,
        })
    }
}
fn open(selection: &Selection<'_>) -> Result<OwnedFd> {
    rustix::fs::openat2(
        selection.root,
        &selection.declared.destination,
        OFlags::PATH | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::IN_ROOT
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::CACHED,
    )
    .map_err(|e| match e {
        rustix::io::Errno::NOENT => Failure::NotReady,
        rustix::io::Errno::LOOP | rustix::io::Errno::XDEV => Failure::UnsafePath,
        _ => Failure::Unavailable,
    })
}
fn inspect(fd: &OwnedFd, selection: &Selection<'_>, fs: &impl Fs) -> Result<DestinationIdentity> {
    if fs.mount_id(fd)? != selection.process_mount.mount_id {
        return Err(Failure::IdentityMismatch);
    }
    let identity = fs.identity(fd)?;
    if identity.mount_id != selection.process_mount.mount_id
        || identity.device != selection.source.identity.device
        || identity.inode != selection.source.identity.inode
        || identity.kind != selection.source.identity.kind
    {
        return Err(Failure::IdentityMismatch);
    }
    Ok(identity)
}
fn retain<'a>(
    selected: Vec<Selection<'a>>,
    fs: &impl Fs,
    expires: Instant,
    deadline: Instant,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<Pinned<'a>>> {
    let mut pinned = Vec::new();
    pinned
        .try_reserve_exact(selected.len())
        .map_err(|_| Failure::Unavailable)?;
    for selection in selected {
        poll(expires, deadline, cancelled)?;
        let fd = open(&selection)?;
        let identity = inspect(&fd, &selection, fs)?;
        pinned.push(Pinned {
            selection,
            fd,
            identity,
        });
    }
    revalidate_paths(&pinned, fs, expires, deadline, cancelled)?;
    Ok(pinned)
}
fn revalidate_paths(
    pinned: &[Pinned<'_>],
    fs: &impl Fs,
    expires: Instant,
    deadline: Instant,
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    for path in pinned {
        poll(expires, deadline, cancelled)?;
        let fresh = open(&path.selection)?;
        if inspect(&path.fd, &path.selection, fs)? != path.identity
            || inspect(&fresh, &path.selection, fs)? != path.identity
        {
            return Err(Failure::Conflict);
        }
    }
    poll(expires, deadline, cancelled)
}

// Serialize borrowed rows before allocating report copies.
#[derive(Serialize)]
struct Row<'a> {
    container: &'a ContainerSnapshot,
    source: &'a ContainerStorageSource,
    destination: &'a str,
    writable: bool,
    host_source: &'a ContainerSourceIdentity,
    process_mount: &'a ContainerProcessMount,
    identity: &'a DestinationIdentity,
}
struct Rows<'a, 'b>(&'a [Pinned<'b>]);
impl Serialize for Rows<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.0.len()))?;
        for p in self.0 {
            seq.serialize_element(&Row {
                container: p.selection.container,
                source: &p.selection.declared.source,
                destination: &p.selection.declared.destination,
                writable: p.selection.declared.writable,
                host_source: &p.selection.source.identity,
                process_mount: p.selection.process_mount,
                identity: &p.identity,
            })?;
        }
        seq.end()
    }
}
struct Stopped<'a>(&'a ContainerStorageInventory);
impl Serialize for Stopped<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(
            self.0
                .containers
                .iter()
                .filter(|c| !c.container.running)
                .count(),
        ))?;
        for c in &self.0.containers {
            if !c.container.running {
                seq.serialize_element(&c.container)?;
            }
        }
        seq.end()
    }
}
#[derive(Serialize)]
struct Report<'a, 'b> {
    version: u16,
    declarations_observed_at: i64,
    declarations_digest: &'a str,
    process_snapshot_digest: &'a str,
    physical_snapshot_digest: &'a str,
    host_root_mount_id: u64,
    limits: DestinationLimits,
    uninspected_stopped: Stopped<'a>,
    destinations: Rows<'a, 'b>,
}
struct Count {
    count: usize,
    max: usize,
}
impl Write for Count {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.count = self
            .count
            .checked_add(bytes.len())
            .filter(|n| *n <= self.max)
            .ok_or_else(|| io::Error::other("destination report limit"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn snapshot(
    pinned: &[Pinned<'_>],
    declarations: &ContainerStorageInventory,
    processes: &RunningContainerProcessSnapshot,
    physical: &ContainerDependencySnapshot,
    limits: DestinationLimits,
) -> Result<ContainerDestinationSnapshot> {
    let process_digest = processes::digest_json(processes, INPUT_BYTES)?;
    let physical_digest = processes::digest_json(physical, INPUT_BYTES)?;
    let view = Report {
        version: 1,
        declarations_observed_at: declarations.observed_at,
        declarations_digest: &processes.declarations_digest,
        process_snapshot_digest: &process_digest,
        physical_snapshot_digest: &physical_digest,
        host_root_mount_id: physical.host_root_mount_id,
        limits,
        uninspected_stopped: Stopped(declarations),
        destinations: Rows(pinned),
    };
    let mut count = Count {
        count: 0,
        max: limits.max_report_bytes as usize,
    };
    serde_json::to_writer(&mut count, &view).map_err(|_| Failure::Unavailable)?;
    let mut destinations = Vec::new();
    destinations
        .try_reserve_exact(pinned.len())
        .map_err(|_| Failure::Unavailable)?;
    for p in pinned {
        destinations.push(ContainerDestinationObservation {
            container: p.selection.container.clone(),
            source: p.selection.declared.source.clone(),
            destination: p.selection.declared.destination.clone(),
            writable: p.selection.declared.writable,
            host_source: p.selection.source.identity.clone(),
            process_mount: p.selection.process_mount.clone(),
            identity: p.identity.clone(),
        });
    }
    let mut stopped = Vec::new();
    stopped
        .try_reserve_exact(
            declarations
                .containers
                .iter()
                .filter(|c| !c.container.running)
                .count(),
        )
        .map_err(|_| Failure::Unavailable)?;
    stopped.extend(
        declarations
            .containers
            .iter()
            .filter(|c| !c.container.running)
            .map(|c| c.container.clone()),
    );
    Ok(ContainerDestinationSnapshot {
        version: 1,
        declarations_observed_at: declarations.observed_at,
        declarations_digest: processes.declarations_digest.clone(),
        process_snapshot_digest: process_digest,
        physical_snapshot_digest: physical_digest,
        host_root_mount_id: physical.host_root_mount_id,
        limits,
        uninspected_stopped: stopped,
        destinations,
    })
}
