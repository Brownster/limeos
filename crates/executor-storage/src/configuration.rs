//! Retained declared planning configuration; no physical or operation authority.
use crate::{Failure, Result, StorageReadProfile, namespace, namespace_with_profile, path};
use limeos_domain::{PoolsConfig, SnapraidConfig, StorageContract, shares::SambaSharesConfig};
use rustix::{
    fd::OwnedFd,
    fs::{Mode, OFlags, Statx},
};
use serde::{Deserialize, Serialize};
use std::{
    cell::Cell,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

mod admission;
#[cfg(test)]
mod tests;
pub use admission::ConfigurationReadProfile;

const MAX_FILE_BYTES: u32 = 64 * 1024;
const MAX_TOTAL_BYTES: u32 = 128 * 1024;
const MAX_FDS: u32 = 65;
const MAX_REPORT_BYTES: u32 = 16 * 1024;
const MAX_COMPONENTS: usize = 16;
const LIFETIME: Duration = Duration::from_secs(4);
const WORK: Duration = Duration::from_secs(2);

/// All four paths are required. Missing files never imply an empty configuration.
pub struct ConfigurationPaths<'a> {
    pub contract: &'a Path,
    pub pools: &'a Path,
    pub protection: &'a Path,
    pub shares: &'a Path,
}
impl ConfigurationPaths<'_> {
    fn files(&self) -> [&Path; 4] {
        [self.contract, self.pools, self.protection, self.shares]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationLimits {
    pub max_file_bytes: u32,
    pub max_total_bytes: u32,
    pub max_retained_fds: u32,
    pub max_report_bytes: u32,
}
impl ConfigurationLimits {
    fn validate(self) -> Result<()> {
        if self.max_file_bytes == 0
            || self.max_file_bytes > MAX_FILE_BYTES
            || self.max_total_bytes == 0
            || self.max_total_bytes > MAX_TOTAL_BYTES
            || self.max_retained_fds == 0
            || self.max_retained_fds > MAX_FDS
            || self.max_report_bytes == 0
            || self.max_report_bytes > MAX_REPORT_BYTES
        {
            return Err(Failure::InvalidPlan);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationRole {
    Contract,
    Pools,
    Protection,
    Shares,
}
const ROLES: [ConfigurationRole; 4] = [
    ConfigurationRole::Contract,
    ConfigurationRole::Pools,
    ConfigurationRole::Protection,
    ConfigurationRole::Shares,
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConfigurationObjectIdentity {
    pub major: u32,
    pub minor: u32,
    pub mount_id: u64,
    pub inode: u64,
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
}
impl From<&Statx> for ConfigurationObjectIdentity {
    fn from(s: &Statx) -> Self {
        Self {
            major: s.stx_dev_major,
            minor: s.stx_dev_minor,
            mount_id: s.stx_mnt_id,
            inode: s.stx_ino,
            mode: s.stx_mode,
            uid: s.stx_uid,
            gid: s.stx_gid,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConfigurationFileIdentity {
    pub object: ConfigurationObjectIdentity,
    pub links: u32,
    pub size: u64,
    pub modified_seconds: i64,
    pub modified_nanoseconds: u32,
    pub changed_seconds: i64,
    pub changed_nanoseconds: u32,
}
impl From<&Statx> for ConfigurationFileIdentity {
    fn from(s: &Statx) -> Self {
        Self {
            object: s.into(),
            links: s.stx_nlink,
            size: s.stx_size,
            modified_seconds: s.stx_mtime.tv_sec,
            modified_nanoseconds: s.stx_mtime.tv_nsec,
            changed_seconds: s.stx_ctime.tv_sec,
            changed_nanoseconds: s.stx_ctime.tv_nsec,
        }
    }
}
#[derive(Debug, Serialize)]
pub struct ConfigurationFileObservation {
    pub role: ConfigurationRole,
    pub path: PathBuf,
    pub ancestors: Vec<ConfigurationObjectIdentity>,
    pub identity: ConfigurationFileIdentity,
    pub bytes_digest: String,
}
/// Metadata reports cannot recreate retained evidence.
/// ```compile_fail
/// use limeos_executor_storage::configuration::StorageConfigurationSnapshot;
/// let forged: StorageConfigurationSnapshot = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug, Serialize)]
pub struct StorageConfigurationSnapshot {
    pub version: u16,
    pub observed_at: i64,
    pub limits: ConfigurationLimits,
    pub host_root: ConfigurationObjectIdentity,
    pub configuration_digest: String,
    pub files: [ConfigurationFileObservation; 4],
}
/// Checked typed declarations, suitable as inputs to the configured graph.
#[derive(Debug, Serialize)]
pub struct StoragePlanningConfiguration {
    pub contract: StorageContract,
    pub pools: PoolsConfig,
    pub protection: SnapraidConfig,
    pub shares: SambaSharesConfig,
}

struct Ancestor {
    name: String,
    fd: OwnedFd,
    identity: ConfigurationObjectIdentity,
}
struct File {
    ancestors: Vec<Ancestor>,
    name: String,
    fd: OwnedFd,
    bytes: Vec<u8>,
    observation: ConfigurationFileObservation,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ValidationState {
    Ready,
    Validating,
    Poisoned,
}
/// Only protected collection constructs an owner. Its original lease never renews.
/// ```compile_fail
/// use limeos_executor_storage::configuration::StorageConfigurationEvidence;
/// let forged: StorageConfigurationEvidence = serde_json::from_str("{}").unwrap();
/// ```
pub struct StorageConfigurationEvidence {
    root: OwnedFd,
    files: [File; 4],
    configuration: StoragePlanningConfiguration,
    snapshot: StorageConfigurationSnapshot,
    expires: Instant,
    state: Cell<ValidationState>,
    owner: u32,
    profile: Option<ConfigurationReadProfile>,
}
impl StorageConfigurationEvidence {
    /// Borrowed declarations alone establish neither currentness nor authority.
    pub fn configuration(&self) -> &StoragePlanningConfiguration {
        &self.configuration
    }
    pub fn snapshot(&self) -> &StorageConfigurationSnapshot {
        &self.snapshot
    }
    pub fn retained_descriptor_count(&self) -> usize {
        1 + self
            .files
            .iter()
            .map(|f| f.ancestors.len() + 1)
            .sum::<usize>()
    }
    /// Any failure permanently invalidates this owner; acquire a new one to retry.
    pub fn revalidate(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        self.checked(|| {
            let deadline = Instant::now() + WORK;
            poll(self.expires, deadline, cancelled)?;
            let fresh_root = host_root(self.profile)?;
            self.validate_from(&fresh_root, now()?, deadline, cancelled)?;
            let final_root = host_root(self.profile)?;
            if directory(&final_root, 0)? != self.snapshot.host_root {
                return Err(Failure::Conflict);
            }
            age(self.snapshot.observed_at, now()?)?;
            poll(self.expires, deadline, cancelled)
        })
    }
    fn checked(&self, operation: impl FnOnce() -> Result<()>) -> Result<()> {
        if self.state.get() != ValidationState::Ready {
            self.state.set(ValidationState::Poisoned);
            return Err(Failure::Conflict);
        }
        self.state.set(ValidationState::Validating);
        let result = operation();
        if result.is_ok() && self.state.get() == ValidationState::Validating {
            self.state.set(ValidationState::Ready);
            Ok(())
        } else {
            self.state.set(ValidationState::Poisoned);
            result.and(Err(Failure::Conflict))
        }
    }
    fn validate_from(
        &self,
        fresh_root: &OwnedFd,
        at: i64,
        deadline: Instant,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<()> {
        poll(self.expires, deadline, cancelled)?;
        age(self.snapshot.observed_at, at)?;
        if directory(&self.root, self.owner)? != self.snapshot.host_root
            || directory(fresh_root, self.owner)? != self.snapshot.host_root
        {
            return Err(Failure::Conflict);
        }
        for file in &self.files {
            poll(self.expires, deadline, cancelled)?;
            let mut fresh = None;
            for ancestor in &file.ancestors {
                poll(self.expires, deadline, cancelled)?;
                let fd = open_directory(fresh.as_ref().unwrap_or(fresh_root), &ancestor.name)?;
                if directory(&ancestor.fd, self.owner)? != ancestor.identity
                    || directory(&fd, self.owner)? != ancestor.identity
                {
                    return Err(Failure::Conflict);
                }
                fresh = Some(fd);
            }
            let (again, identity, bytes) = read_file(
                fresh.as_ref().unwrap_or(fresh_root),
                &file.name,
                self.owner,
                self.snapshot.limits.max_file_bytes,
            )?;
            if identity != file.observation.identity
                || bytes != file.bytes
                || checked_read(&file.fd, self.owner, self.snapshot.limits.max_file_bytes)?.1
                    != file.bytes
                || file_identity(&file.fd, self.owner, self.snapshot.limits.max_file_bytes)?
                    != identity
                || file_identity(&again, self.owner, self.snapshot.limits.max_file_bytes)?
                    != identity
            {
                return Err(Failure::Conflict);
            }
        }
        poll(self.expires, deadline, cancelled)
    }
}

fn now() -> Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Failure::Conflict)?
        .as_secs()
        .try_into()
        .map_err(|_| Failure::Conflict)
}
fn age(issued: i64, at: i64) -> Result<()> {
    if at.checked_sub(issued).is_some_and(|n| (0..=4).contains(&n)) {
        Ok(())
    } else {
        Err(Failure::TimedOut)
    }
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
fn directory(fd: &OwnedFd, owner: u32) -> Result<ConfigurationObjectIdentity> {
    let stat = path::stat(fd)?;
    if stat.stx_mode & 0o170000 != 0o040000
        || stat.stx_uid != owner
        || stat.stx_mode & 0o022 != 0
        || stat.stx_nlink == 0
    {
        return Err(Failure::UnsafePath);
    }
    Ok((&stat).into())
}
fn host_root(profile: Option<ConfigurationReadProfile>) -> Result<OwnedFd> {
    if !rustix::process::geteuid().is_root() {
        return Err(Failure::Unavailable);
    }
    let view = || {
        if profile.is_some() {
            namespace_with_profile(StorageReadProfile::CombinedV1)
        } else {
            namespace()
        }
    };
    let mount_id = view()?;
    let root = rustix::fs::open(
        "/",
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| Failure::Unavailable)?;
    if directory(&root, 0)?.mount_id != mount_id || view()? != mount_id {
        return Err(Failure::WrongNamespace);
    }
    Ok(root)
}
fn open_directory(parent: &OwnedFd, name: &str) -> Result<OwnedFd> {
    rustix::fs::openat(
        parent,
        name,
        OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| Failure::UnsafePath)
}
fn file_identity(fd: &OwnedFd, owner: u32, limit: u32) -> Result<ConfigurationFileIdentity> {
    let stat = path::stat(fd)?;
    if stat.stx_mode & 0o170000 != 0o100000
        || stat.stx_uid != owner
        || stat.stx_mode & 0o022 != 0
        || stat.stx_nlink != 1
    {
        return Err(Failure::UnsafePath);
    }
    if stat.stx_size > u64::from(limit) {
        return Err(Failure::Unavailable);
    }
    Ok((&stat).into())
}
fn checked_read(
    fd: &OwnedFd,
    owner: u32,
    limit: u32,
) -> Result<(ConfigurationFileIdentity, Vec<u8>)> {
    let before = file_identity(fd, owner, limit)?;
    // Allocate only the admitted size plus a growth sentinel, rather than
    // retaining four maximum-size buffers for small configuration files.
    let capacity = before.size as usize + 1;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity)
        .map_err(|_| Failure::Unavailable)?;
    bytes.resize(capacity, 0);
    let mut length = 0;
    while length < bytes.len() {
        match rustix::io::pread(fd, &mut bytes[length..], length as u64) {
            Ok(0) => break,
            Ok(n) => length += n,
            Err(_) => return Err(Failure::Unavailable),
        }
    }
    if length > limit as usize {
        return Err(Failure::Unavailable);
    }
    bytes.truncate(length);
    if length as u64 != before.size || file_identity(fd, owner, limit)? != before {
        return Err(Failure::Conflict);
    }
    Ok((before, bytes))
}
fn read_file(
    parent: &OwnedFd,
    name: &str,
    owner: u32,
    limit: u32,
) -> Result<(OwnedFd, ConfigurationFileIdentity, Vec<u8>)> {
    let classified = rustix::fs::openat(
        parent,
        name,
        OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| Failure::NotReady)?;
    let expected = file_identity(&classified, owner, limit)?;
    let fd = rustix::fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| Failure::UnsafePath)?;
    if file_identity(&fd, owner, limit)? != expected {
        return Err(Failure::Conflict);
    }
    let (identity, bytes) = checked_read(&fd, owner, limit)?;
    if identity != expected || file_identity(&classified, owner, limit)? != expected {
        return Err(Failure::Conflict);
    }
    Ok((fd, identity, bytes))
}

fn admit<'a>(
    paths: &'a ConfigurationPaths<'_>,
    limits: ConfigurationLimits,
) -> Result<[Vec<&'a str>; 4]> {
    limits.validate()?;
    let mut count = 1usize;
    let mut parts: [Vec<&str>; 4] = std::array::from_fn(|_| Vec::new());
    for (i, path) in paths.files().iter().enumerate() {
        let text = path.to_str().ok_or(Failure::InvalidPlan)?;
        if !text.starts_with('/')
            || !(2..=512).contains(&text.len())
            || text.chars().any(char::is_control)
        {
            return Err(Failure::InvalidPlan);
        }
        let depth = text[1..].split('/').count();
        if depth > MAX_COMPONENTS
            || text[1..]
                .split('/')
                .any(|s| s.is_empty() || matches!(s, "." | "..") || s.len() > 255)
        {
            return Err(Failure::InvalidPlan);
        }
        if paths.files()[..i].contains(path) {
            return Err(Failure::AmbiguousIdentity);
        }
        count += depth;
        if count > limits.max_retained_fds as usize {
            return Err(Failure::Unavailable);
        }
        parts[i]
            .try_reserve_exact(depth)
            .map_err(|_| Failure::Unavailable)?;
        parts[i].extend(text[1..].split('/'));
    }
    Ok(parts)
}

struct Count {
    bytes: usize,
    max: usize,
}
impl Write for Count {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .filter(|n| *n <= self.max)
            .ok_or_else(|| io::Error::other("configuration report limit"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn byte_count(value: &impl Serialize, max: usize) -> Result<usize> {
    let mut count = Count { bytes: 0, max };
    serde_json::to_writer(&mut count, value).map_err(|_| Failure::Unavailable)?;
    Ok(count.bytes)
}
fn configuration_digest(config: &StoragePlanningConfiguration, max: usize) -> Result<String> {
    let value = ("limeos.storage-planning-configuration.v1", config);
    let count = byte_count(&value, max)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(count)
        .map_err(|_| Failure::Unavailable)?;
    serde_json::to_writer(&mut bytes, &value).map_err(|_| Failure::Unavailable)?;
    Ok(limeos_identity::digest(
        std::str::from_utf8(&bytes).map_err(|_| Failure::InvalidPlan)?,
    ))
}
fn parse(files: &[File; 4]) -> Result<StoragePlanningConfiguration> {
    let config = StoragePlanningConfiguration {
        contract: serde_json::from_slice(&files[0].bytes).map_err(|_| Failure::InvalidPlan)?,
        pools: serde_json::from_slice(&files[1].bytes).map_err(|_| Failure::InvalidPlan)?,
        protection: serde_json::from_slice(&files[2].bytes).map_err(|_| Failure::InvalidPlan)?,
        shares: serde_json::from_slice(&files[3].bytes).map_err(|_| Failure::InvalidPlan)?,
    };
    config
        .contract
        .validate()
        .map_err(|_| Failure::InvalidPlan)?;
    config
        .pools
        .plan(&config.contract)
        .map_err(|_| Failure::InvalidPlan)?;
    config
        .protection
        .plan(&config.pools, &config.contract)
        .map_err(|_| Failure::InvalidPlan)?;
    config
        .shares
        .plan(&config.contract, &config.pools)
        .map_err(|_| Failure::InvalidPlan)?;
    Ok(config)
}

/// Collect only declared configuration. Independent worker supervision is still required.
pub fn inspect_storage_configuration(
    paths: ConfigurationPaths<'_>,
    limits: ConfigurationLimits,
    cancelled: &dyn Fn() -> bool,
) -> Result<StorageConfigurationEvidence> {
    inspect(paths, limits, None, cancelled)
}
/// Closed trusted configuration admission for the later combined worker.
pub fn inspect_storage_configuration_with_profile(
    paths: ConfigurationPaths<'_>,
    profile: ConfigurationReadProfile,
    cancelled: &dyn Fn() -> bool,
) -> Result<StorageConfigurationEvidence> {
    inspect(paths, profile.limits(), Some(profile), cancelled)
}
fn inspect(
    paths: ConfigurationPaths<'_>,
    limits: ConfigurationLimits,
    profile: Option<ConfigurationReadProfile>,
    cancelled: &dyn Fn() -> bool,
) -> Result<StorageConfigurationEvidence> {
    let parts = admit(&paths, limits)?;
    let expires = Instant::now() + LIFETIME;
    let deadline = Instant::now() + WORK;
    let at = now()?;
    poll(expires, deadline, cancelled)?;
    let root = host_root(profile)?;
    let evidence = collect(
        root,
        &paths,
        parts,
        CollectionBudget {
            owner: 0,
            limits,
            at,
            expires,
            deadline,
            profile,
        },
        cancelled,
    )?;
    evidence.revalidate(cancelled)?;
    Ok(evidence)
}

struct CollectionBudget {
    owner: u32,
    limits: ConfigurationLimits,
    at: i64,
    expires: Instant,
    deadline: Instant,
    profile: Option<ConfigurationReadProfile>,
}
fn collect(
    root: OwnedFd,
    paths: &ConfigurationPaths<'_>,
    parts: [Vec<&str>; 4],
    budget: CollectionBudget,
    cancelled: &dyn Fn() -> bool,
) -> Result<StorageConfigurationEvidence> {
    let CollectionBudget {
        owner,
        limits,
        at,
        expires,
        deadline,
        profile,
    } = budget;
    poll(expires, deadline, cancelled)?;
    let root_identity = directory(&root, owner)?;
    let mut files: Vec<File> = Vec::new();
    files
        .try_reserve_exact(4)
        .map_err(|_| Failure::Unavailable)?;
    let mut total_bytes = 0usize;
    for (i, components) in parts.iter().enumerate() {
        let mut ancestors: Vec<Ancestor> = Vec::new();
        ancestors
            .try_reserve_exact(components.len() - 1)
            .map_err(|_| Failure::Unavailable)?;
        for name in &components[..components.len() - 1] {
            poll(expires, deadline, cancelled)?;
            let parent = ancestors.last().map(|a| &a.fd).unwrap_or(&root);
            let fd = open_directory(parent, name)?;
            let identity = directory(&fd, owner)?;
            ancestors.push(Ancestor {
                name: (*name).into(),
                fd,
                identity,
            });
        }
        poll(expires, deadline, cancelled)?;
        let parent = ancestors.last().map(|a| &a.fd).unwrap_or(&root);
        let name = components.last().ok_or(Failure::InvalidPlan)?;
        let remaining = limits.max_total_bytes as usize - total_bytes;
        let (fd, identity, bytes) = read_file(
            parent,
            name,
            owner,
            limits.max_file_bytes.min(remaining as u32),
        )?;
        if files.iter().any(|f| {
            f.observation.identity.object.major == identity.object.major
                && f.observation.identity.object.minor == identity.object.minor
                && f.observation.identity.object.inode == identity.object.inode
        }) {
            return Err(Failure::AmbiguousIdentity);
        }
        total_bytes += bytes.len();
        let observation = ConfigurationFileObservation {
            role: ROLES[i],
            path: paths.files()[i].into(),
            ancestors: ancestors.iter().map(|a| a.identity.clone()).collect(),
            identity,
            bytes_digest: limeos_identity::digest(
                std::str::from_utf8(&bytes).map_err(|_| Failure::InvalidPlan)?,
            ),
        };
        files.push(File {
            ancestors,
            name: (*name).into(),
            fd,
            bytes,
            observation,
        });
    }
    let files: [File; 4] = files.try_into().map_err(|_| Failure::InvalidPlan)?;
    poll(expires, deadline, cancelled)?;
    if profile.is_some() {
        let mut shape = admission::Shape::new();
        for file in &files {
            poll(expires, deadline, cancelled)?;
            shape.inspect(&file.bytes)?;
        }
        poll(expires, deadline, cancelled)?;
    }
    let configuration = parse(&files)?;
    let configuration_digest =
        configuration_digest(&configuration, limits.max_total_bytes as usize + 128)?;
    let snapshot = StorageConfigurationSnapshot {
        version: 1,
        observed_at: at,
        limits,
        host_root: root_identity,
        configuration_digest,
        files: std::array::from_fn(|i| ConfigurationFileObservation {
            role: files[i].observation.role,
            path: files[i].observation.path.clone(),
            ancestors: files[i].observation.ancestors.clone(),
            identity: files[i].observation.identity.clone(),
            bytes_digest: files[i].observation.bytes_digest.clone(),
        }),
    };
    byte_count(&snapshot, limits.max_report_bytes as usize)?;
    let evidence = StorageConfigurationEvidence {
        root,
        files,
        configuration,
        snapshot,
        expires,
        state: Cell::new(ValidationState::Ready),
        owner,
        profile,
    };
    evidence.checked(|| evidence.validate_from(&evidence.root, at, deadline, cancelled))?;
    Ok(evidence)
}
