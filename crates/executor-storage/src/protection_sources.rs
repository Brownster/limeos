//! Retained SnapRAID path facts; no process execution or operation authority.
use crate::{Failure, StorageInventoryEvidence, mounts::Mount};
use limeos_domain::{
    PoolsConfig, ProtectionPathKind, ProtectionPathObservation, ProtectionPathState,
    ProtectionSourceEvidence, SnapraidConfig, SnapraidPreview, StorageBlockDevice, StorageContract,
    StorageFilesystem, StorageInventory,
};
use rustix::{
    fd::OwnedFd,
    fs::{AtFlags, Mode, OFlags, ResolveFlags, Statx, StatxFlags},
};
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fmt,
    io::{self, Write},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(test)]
mod tests;
const MAX_PATHS: u32 = 128;
const MAX_FDS: u32 = 128;
const MAX_BYTES: u32 = 64 * 1024;
const LIFETIME: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceLimits {
    pub max_paths: u32,
    pub max_retained_fds: u32,
    pub max_report_bytes: u32,
}
#[derive(Debug)]
pub enum SourceError {
    Storage(Failure),
    InvalidLimits,
    InvalidPlan,
    Unsupported,
    UnsafePath,
    Changed,
    Limit,
    Allocation,
    Cancelled,
    TimedOut,
    Io(io::Error),
}
impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "protection source evidence refused: {self:?}")
    }
}
impl Error for SourceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}
impl From<Failure> for SourceError {
    fn from(value: Failure) -> Self {
        Self::Storage(value)
    }
}
type Result<T> = std::result::Result<T, SourceError>;
fn poll(expires: Instant, cancelled: &dyn Fn() -> bool) -> Result<()> {
    if cancelled() {
        Err(SourceError::Cancelled)
    } else if Instant::now() >= expires {
        Err(SourceError::TimedOut)
    } else {
        Ok(())
    }
}
fn now() -> Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SourceError::Changed)?
        .as_secs()
        .try_into()
        .map_err(|_| SourceError::Changed)
}
fn limits_valid(limits: SourceLimits) -> Result<()> {
    if limits.max_paths == 0
        || limits.max_paths > MAX_PATHS
        || limits.max_retained_fds == 0
        || limits.max_retained_fds > MAX_FDS
        || limits.max_report_bytes == 0
        || limits.max_report_bytes > MAX_BYTES
    {
        Err(SourceError::InvalidLimits)
    } else {
        Ok(())
    }
}
// Existing identity helpers hash strings; count first without allocating a
// serialized input. Feed the bounded canonical body only after that preflight.
struct Count {
    count: usize,
    max: usize,
    refused: bool,
}
impl Write for Count {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self
            .count
            .checked_add(bytes.len())
            .filter(|n| *n <= self.max)
        {
            Some(n) => {
                self.count = n;
                Ok(bytes.len())
            }
            None => {
                self.refused = true;
                Err(io::Error::other("bounded serialization refused"))
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn bounded_bytes<T: Serialize>(value: &T, max: usize) -> Result<Vec<u8>> {
    let mut count = Count {
        count: 0,
        max,
        refused: false,
    };
    let result = serde_json::to_writer(&mut count, value);
    if count.refused {
        return Err(SourceError::Limit);
    }
    result.map_err(|_| SourceError::InvalidPlan)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(count.count)
        .map_err(|_| SourceError::Allocation)?;
    serde_json::to_writer(&mut output, value).map_err(|_| SourceError::InvalidPlan)?;
    if output.len() != count.count {
        return Err(SourceError::Changed);
    }
    Ok(output)
}
fn digest<T: Serialize>(value: &T, max: usize) -> Result<String> {
    let body = bounded_bytes(value, max)?;
    let text = std::str::from_utf8(&body).map_err(|_| SourceError::InvalidPlan)?;
    Ok(limeos_identity::digest(text))
}
fn configuration_identity(
    contract: &StorageContract,
    pools: &PoolsConfig,
    config: &SnapraidConfig,
    limits: SourceLimits,
) -> Result<String> {
    limits_valid(limits)?;
    digest(
        &(
            "limeos.protection-source.configuration.v1",
            contract,
            pools,
            config,
            limits,
        ),
        MAX_BYTES as usize,
    )
}
fn beneath(path: &str, parent: &str) -> bool {
    path == parent
        || path
            .strip_prefix(parent)
            .is_some_and(|rest| rest.starts_with('/'))
}
fn expected_mounts(
    preview: &SnapraidPreview,
    inventory: &StorageInventory,
    table: &[Mount],
) -> Result<Vec<(StorageBlockDevice, u64)>> {
    inventory.validate().map_err(|_| SourceError::InvalidPlan)?;
    let mut result = Vec::new();
    result
        .try_reserve_exact(preview.requirements.len())
        .map_err(|_| SourceError::Allocation)?;
    for requirement in &preview.requirements {
        let mut matches = inventory
            .devices
            .iter()
            .filter(|d| d.filesystem_uuid == requirement.expected.filesystem_uuid);
        let device = matches.next().ok_or(SourceError::Changed)?;
        if matches.next().is_some() {
            return Err(SourceError::Changed);
        }
        let filesystem = match requirement.expected.filesystem {
            StorageFilesystem::Ext2 => "ext2",
            StorageFilesystem::Ext3 => "ext3",
            StorageFilesystem::Ext4 => "ext4",
            StorageFilesystem::Xfs => "xfs",
            _ => return Err(SourceError::Unsupported),
        };
        if device.boot_backing
            || device.in_use_as_swap
            || device.filesystem != filesystem
            || requirement
                .expected
                .serial
                .as_ref()
                .is_some_and(|s| device.serial.as_ref() != Some(s))
        {
            return Err(SourceError::Changed);
        }
        let root = &requirement.expected.mountpoint;
        if !beneath(&requirement.path, root) {
            return Err(SourceError::InvalidPlan);
        }
        let mut views = inventory
            .devices
            .iter()
            .flat_map(|d| d.mounts.iter().map(move |m| (d, m)))
            .filter(|(_, m)| &m.mountpoint == root);
        let (mounted, view) = views.next().ok_or(SourceError::Changed)?;
        if views.next().is_some()
            || mounted.device != device.device
            || view.filesystem_root != "/"
            || view.filesystem != filesystem
            || !view.writable
            || view.mount_id == inventory.host_root_mount_id
        {
            return Err(SourceError::Changed);
        }
        let mut raw = table.iter().filter(|m| &m.path == root);
        let raw_view = raw.next().ok_or(SourceError::Changed)?;
        if raw.next().is_some()
            || raw_view.id != view.mount_id
            || raw_view.root != "/"
            || raw_view.device.major != device.device.major
            || raw_view.device.minor != device.device.minor
            || raw_view.filesystem != filesystem
            || !raw_view.writable
            || table
                .iter()
                .any(|m| &m.path != root && beneath(&m.path, root))
        {
            return Err(SourceError::Changed);
        }
        result.push((device.device, view.mount_id));
    }
    Ok(result)
}

fn same(a: &Statx, b: &Statx) -> bool {
    (
        a.stx_dev_major,
        a.stx_dev_minor,
        a.stx_mnt_id,
        a.stx_ino,
        a.stx_mode,
        a.stx_uid,
        a.stx_gid,
        a.stx_nlink,
        a.stx_size,
        a.stx_attributes,
    ) == (
        b.stx_dev_major,
        b.stx_dev_minor,
        b.stx_mnt_id,
        b.stx_ino,
        b.stx_mode,
        b.stx_uid,
        b.stx_gid,
        b.stx_nlink,
        b.stx_size,
        b.stx_attributes,
    ) && a.stx_mtime.tv_sec == b.stx_mtime.tv_sec
        && a.stx_mtime.tv_nsec == b.stx_mtime.tv_nsec
        && a.stx_ctime.tv_sec == b.stx_ctime.tv_sec
        && a.stx_ctime.tv_nsec == b.stx_ctime.tv_nsec
        && a.stx_attributes_mask == b.stx_attributes_mask
}
trait Fs {
    fn stat(&self, fd: &OwnedFd, _: &str) -> Result<Statx> {
        let stat = rustix::fs::statx(
            fd,
            "",
            AtFlags::EMPTY_PATH,
            StatxFlags::BASIC_STATS | StatxFlags::MNT_ID,
        )
        .map_err(|e| SourceError::Io(e.into()))?;
        if stat.stx_mask & (StatxFlags::BASIC_STATS | StatxFlags::MNT_ID).bits()
            != (StatxFlags::BASIC_STATS | StatxFlags::MNT_ID).bits()
            || stat.stx_mnt_id == 0
        {
            return Err(SourceError::Unsupported);
        }
        Ok(stat)
    }
}
struct Kernel;
impl Fs for Kernel {}
struct Pinned {
    path: String,
    parent_path: String,
    parent: OwnedFd,
    parent_stat: Statx,
    name: String,
    object: Option<OwnedFd>,
    stat: Option<Statx>,
    state: ProtectionPathState,
}
struct Paths {
    root: OwnedFd,
    root_stat: Statx,
    pinned: Vec<Pinned>,
    observations: Vec<ProtectionPathObservation>,
    expires: Instant,
}
fn open_directory(root: &OwnedFd, path: &str) -> Result<OwnedFd> {
    rustix::fs::openat2(
        root,
        path,
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::IN_ROOT | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|_| SourceError::UnsafePath)
}
fn leaf(parent: &OwnedFd, name: &str) -> Result<Option<OwnedFd>> {
    match rustix::fs::openat(
        parent,
        name,
        OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => Ok(Some(fd)),
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(_) => Err(SourceError::UnsafePath),
    }
}
fn checked_path(path: &str) -> Result<(&str, &str)> {
    if !path.starts_with('/')
        || path.len() > 1024
        || path.chars().any(char::is_control)
        || path[1..].split('/').count() > 32
        || path[1..]
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == ".." || p.len() > 255)
    {
        return Err(SourceError::InvalidPlan);
    }
    let (parent, name) = path.rsplit_once('/').ok_or(SourceError::InvalidPlan)?;
    Ok((if parent.is_empty() { "/" } else { parent }, name))
}
struct MountBindings<'a> {
    paths: &'a [(StorageBlockDevice, u64)],
    host_root: u64,
}
fn admit_paths(preview: &SnapraidPreview, limits: SourceLimits) -> Result<usize> {
    limits_valid(limits)?;
    if preview.requirements.is_empty() {
        return Err(SourceError::InvalidPlan);
    }
    if preview.requirements.len() > limits.max_paths as usize {
        return Err(SourceError::Limit);
    }
    for requirement in &preview.requirements {
        checked_path(&requirement.path)?;
    }
    let unique = preview
        .requirements
        .iter()
        .enumerate()
        .filter(|(i, r)| !preview.requirements[..*i].iter().any(|p| p.path == r.path))
        .count();
    if unique
        .checked_mul(2)
        .and_then(|n| n.checked_add(1))
        .is_none_or(|n| n > limits.max_retained_fds as usize)
    {
        return Err(SourceError::Limit);
    }
    Ok(unique)
}
fn collect_paths<F: Fs>(
    root: OwnedFd,
    preview: &SnapraidPreview,
    bindings: MountBindings<'_>,
    limits: SourceLimits,
    expires: Instant,
    cancelled: &dyn Fn() -> bool,
    fs: &F,
) -> Result<Paths> {
    let MountBindings {
        paths: expected,
        host_root,
    } = bindings;
    let unique = admit_paths(preview, limits)?;
    if expected.len() != preview.requirements.len() {
        return Err(SourceError::InvalidPlan);
    }
    let root_stat = fs.stat(&root, "/")?;
    if root_stat.stx_mnt_id != host_root || root_stat.stx_mode & 0o170000 != 0o040000 {
        return Err(SourceError::Changed);
    }
    let mut pinned: Vec<Pinned> = Vec::new();
    let mut observations = Vec::new();
    pinned
        .try_reserve_exact(unique)
        .map_err(|_| SourceError::Allocation)?;
    observations
        .try_reserve_exact(expected.len())
        .map_err(|_| SourceError::Allocation)?;
    for (required, (device, mount_id)) in preview.requirements.iter().zip(expected) {
        poll(expires, cancelled)?;
        let directory = matches!(
            required.kind,
            ProtectionPathKind::Source | ProtectionPathKind::Data
        );
        let item = if let Some(index) = pinned.iter().position(|p| p.path == required.path) {
            &pinned[index]
        } else {
            let (parent_path, name) = checked_path(&required.path)?;
            let parent = open_directory(&root, parent_path)?;
            let parent_stat = fs.stat(&parent, parent_path)?;
            let object = leaf(&parent, name)?;
            let stat = object
                .as_ref()
                .map(|fd| fs.stat(fd, &required.path))
                .transpose()?;
            let state = match &stat {
                Some(s) if s.stx_mode & 0o170000 == 0o040000 => ProtectionPathState::Directory,
                Some(s) if s.stx_mode & 0o170000 == 0o100000 && s.stx_nlink == 1 => {
                    ProtectionPathState::RegularFile
                }
                None => ProtectionPathState::AbsentFile,
                _ => return Err(SourceError::UnsafePath),
            };
            if !directory
                && (parent_stat.stx_dev_major != device.major
                    || parent_stat.stx_dev_minor != device.minor
                    || parent_stat.stx_mnt_id != *mount_id)
            {
                return Err(SourceError::Changed);
            }
            pinned.push(Pinned {
                path: required.path.clone(),
                parent_path: parent_path.into(),
                parent,
                parent_stat,
                name: name.into(),
                object,
                stat,
                state,
            });
            pinned.last().ok_or(SourceError::Changed)?
        };
        if (directory && item.state != ProtectionPathState::Directory)
            || (!directory
                && !matches!(
                    item.state,
                    ProtectionPathState::RegularFile | ProtectionPathState::AbsentFile
                ))
        {
            return Err(SourceError::UnsafePath);
        }
        let stat = item.stat.as_ref().unwrap_or(&item.parent_stat);
        if stat.stx_dev_major != device.major
            || stat.stx_dev_minor != device.minor
            || stat.stx_mnt_id != *mount_id
        {
            return Err(SourceError::Changed);
        }
        observations.push(ProtectionPathObservation {
            path: required.path.clone(),
            state: item.state,
            device: *device,
            mount_id: *mount_id,
        });
    }
    let paths = Paths {
        root,
        root_stat,
        pinned,
        observations,
        expires,
    };
    paths.revalidate(cancelled, fs)?;
    Ok(paths)
}
impl Paths {
    fn revalidate<F: Fs>(&self, cancelled: &dyn Fn() -> bool, fs: &F) -> Result<()> {
        poll(self.expires, cancelled)?;
        if !same(&self.root_stat, &fs.stat(&self.root, "/")?) {
            return Err(SourceError::Changed);
        }
        for pinned in &self.pinned {
            poll(self.expires, cancelled)?;
            let parent = open_directory(&self.root, &pinned.parent_path)?;
            if !same(&pinned.parent_stat, &fs.stat(&parent, &pinned.parent_path)?)
                || !same(
                    &pinned.parent_stat,
                    &fs.stat(&pinned.parent, &pinned.parent_path)?,
                )
            {
                return Err(SourceError::Changed);
            }
            let again = leaf(&parent, &pinned.name)?;
            let fresh = again
                .as_ref()
                .map(|fd| fs.stat(fd, &pinned.path))
                .transpose()?;
            let held = pinned
                .object
                .as_ref()
                .map(|fd| fs.stat(fd, &pinned.path))
                .transpose()?;
            match (&pinned.stat, fresh, held) {
                (None, None, None) => {}
                (Some(original), Some(fresh), Some(held))
                    if same(original, &fresh) && same(original, &held) => {}
                _ => return Err(SourceError::Changed),
            }
        }
        poll(self.expires, cancelled)
    }
}

#[derive(Debug, Serialize)]
pub struct ProtectionSnapshot {
    pub version: u16,
    pub configuration_identity: String,
    pub sources: ProtectionSourceEvidence,
}
/// Only actual collection creates an owner; JSON never creates descriptors.
/// ```compile_fail
/// use limeos_executor_storage::protection_sources::ProtectionSources;
/// let forged: ProtectionSources<'static> = serde_json::from_str("{}").unwrap();
/// ```
pub struct ProtectionSources<'a> {
    inventory: &'a StorageInventoryEvidence,
    paths: Paths,
    snapshot: ProtectionSnapshot,
    limits: SourceLimits,
}
impl ProtectionSources<'_> {
    pub fn snapshot(&self) -> &ProtectionSnapshot {
        &self.snapshot
    }
    pub fn retained_descriptor_count(&self) -> usize {
        1 + self
            .paths
            .pinned
            .iter()
            .map(|p| 1 + usize::from(p.object.is_some()))
            .sum::<usize>()
    }
    pub async fn revalidate(
        &self,
        contract: &StorageContract,
        pools: &PoolsConfig,
        config: &SnapraidConfig,
        limits: SourceLimits,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<()> {
        poll(self.paths.expires, cancelled)?;
        if limits != self.limits
            || configuration_identity(contract, pools, config, limits)?
                != self.snapshot.configuration_identity
        {
            return Err(SourceError::Changed);
        }
        self.inventory.revalidate().await?;
        self.paths.revalidate(cancelled, &Kernel)?;
        self.inventory.revalidate().await?;
        let age = now()?
            .checked_sub(self.snapshot.sources.observed_at)
            .ok_or(SourceError::Changed)?;
        if !(0..=limeos_domain::PROTECTION_EVIDENCE_MAX_AGE_SECONDS).contains(&age) {
            return Err(SourceError::TimedOut);
        }
        poll(self.paths.expires, cancelled)
    }
}
/// The result cannot outlive the genuine raw inventory it borrows.
/// ```compile_fail
/// use limeos_executor_storage::{StorageInventoryEvidence, protection_sources::{
///   inspect_protection_sources, ProtectionSources, SourceLimits}};
/// use limeos_domain::{StorageContract, PoolsConfig, SnapraidConfig};
/// async fn leak(i: &StorageInventoryEvidence, c: &StorageContract, p: &PoolsConfig,
///   s: &SnapraidConfig, l: SourceLimits) -> ProtectionSources<'static> {
///   inspect_protection_sources(i, c, p, s, l, &|| false).await.unwrap()
/// }
/// ```
pub async fn inspect_protection_sources<'a>(
    inventory: &'a StorageInventoryEvidence,
    contract: &StorageContract,
    pools: &PoolsConfig,
    config: &SnapraidConfig,
    limits: SourceLimits,
    cancelled: &dyn Fn() -> bool,
) -> Result<ProtectionSources<'a>> {
    let expires = Instant::now() + LIFETIME;
    let observed_at = now()?;
    poll(expires, cancelled)?;
    let configuration_identity = configuration_identity(contract, pools, config, limits)?;
    bounded_bytes(inventory.snapshot(), MAX_BYTES as usize)?;
    let preview = config
        .plan(pools, contract)
        .map_err(|_| SourceError::InvalidPlan)?;
    admit_paths(&preview, limits)?;
    if !rustix::process::geteuid().is_root() {
        return Err(SourceError::Storage(Failure::Unavailable));
    }
    inventory.revalidate().await?;
    let expected = expected_mounts(&preview, inventory.snapshot(), &inventory.table)?;
    let root = rustix::fs::open(
        "/",
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| SourceError::Io(e.into()))?;
    let paths = collect_paths(
        root,
        &preview,
        MountBindings {
            paths: &expected,
            host_root: inventory.snapshot().host_root_mount_id,
        },
        limits,
        expires,
        cancelled,
        &Kernel,
    )?;
    let snapshot = ProtectionSnapshot {
        version: 1,
        configuration_identity,
        sources: ProtectionSourceEvidence {
            observed_at,
            inventory: inventory.snapshot().clone(),
            paths: paths.observations.clone(),
        },
    };
    bounded_bytes(&snapshot, limits.max_report_bytes as usize)?;
    let owner = ProtectionSources {
        inventory,
        paths,
        snapshot,
        limits,
    };
    owner
        .revalidate(contract, pools, config, limits, cancelled)
        .await?;
    Ok(owner)
}
