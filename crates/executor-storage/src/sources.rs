//! Retained host source descriptors, not complete physical dependency authority.
use crate::{
    Failure, Result, StorageReadProfile, bounded_read, host_view, mounts, read_admission,
    topology::DeviceNumber,
};
use limeos_domain::{
    CONTAINER_STORAGE_MAX_AGE_SECONDS, ContainerStorageInventory, container_storage_path,
};
use rustix::{
    fd::OwnedFd,
    fs::{AtFlags, Mode, OFlags, ResolveFlags, StatxFlags},
};
use serde::Serialize;
use std::{
    collections::{BTreeSet, VecDeque},
    os::fd::AsRawFd,
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(test)]
const MAX_MOUNT_REFERENCES: usize = 512;
const MAX_REPORT_BYTES: usize = 48 * 1024;
const WORK_SECONDS: u64 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerSourceKind {
    RegularFile,
    Directory,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContainerSourceIdentity {
    pub device: DeviceNumber,
    pub mount_id: u64,
    pub inode: u64,
    pub kind: ContainerSourceKind,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContainerSourcePath {
    pub container: String,
    pub destination: String,
    pub declared_path: String,
    pub resolved_path: String,
    pub identity: ContainerSourceIdentity,
    /// Conservative host-view candidates; running namespaces remain uninspected.
    pub potential_mount_ids: Vec<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContainerSourceMount {
    pub mount_id: u64,
    pub device: DeviceNumber,
    pub filesystem_root: String,
    pub mountpoint: String,
    pub filesystem: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContainerSourceSnapshot {
    pub version: u16,
    pub inventory_digest: String,
    pub host_root_mount_id: u64,
    pub mounts_digest: String,
    pub sources: Vec<ContainerSourcePath>,
    pub mounts: Vec<ContainerSourceMount>,
}

/// Only collection constructs this object. Reports cannot reconstruct its handles.
pub struct ContainerSourceEvidence {
    snapshot: ContainerSourceSnapshot,
    inventory: ContainerStorageInventory,
    table: Vec<mounts::Mount>,
    context: Context,
    retained: Vec<OwnedFd>,
    expires: Instant,
    profile: StorageReadProfile,
}
impl ContainerSourceEvidence {
    pub fn snapshot(&self) -> &ContainerSourceSnapshot {
        &self.snapshot
    }
    /// Does not reacquire Docker, bind UUIDs, inspect containers or authorize effects.
    pub fn revalidate(&self) -> Result<()> {
        if !rustix::process::geteuid().is_root() {
            return Err(Failure::Unavailable);
        }
        self.revalidate_inner(Instant::now() + Duration::from_secs(WORK_SECONDS))
    }
    fn revalidate_inner(&self, deadline: Instant) -> Result<()> {
        check_deadline(self.expires)?;
        self.inventory.validate(now()?).map_err(domain_failure)?;
        table_matches_with_profile(
            &read_mount_text_with_profile(self.profile)?,
            &self.table,
            &self.snapshot.mounts_digest,
            self.profile,
        )?;
        let table = &self.table;
        if host_view(table, &read_host_table_with_profile(self.profile)?)?
            != self.snapshot.host_root_mount_id
            || descriptor_mount_id(&self.context.root)? != self.snapshot.host_root_mount_id
        {
            return Err(Failure::Conflict);
        }
        for (source, fd) in self.snapshot.sources.iter().zip(&self.retained) {
            check_deadline(deadline)?;
            if self.context.inspect(fd, table)?
                != (source.identity.clone(), source.resolved_path.clone())
            {
                return Err(Failure::Conflict);
            }
            for name in [&source.declared_path, &source.resolved_path] {
                let again = self
                    .context
                    .open_with_profile(name, table, deadline, self.profile)?;
                if self.context.inspect(&again, table)?
                    != (source.identity.clone(), source.resolved_path.clone())
                {
                    return Err(Failure::Conflict);
                }
            }
        }
        table_matches_with_profile(
            &read_mount_text_with_profile(self.profile)?,
            &self.table,
            &self.snapshot.mounts_digest,
            self.profile,
        )?;
        if host_view(&self.table, &read_host_table_with_profile(self.profile)?)?
            != self.snapshot.host_root_mount_id
        {
            return Err(Failure::Conflict);
        }
        self.inventory.validate(now()?).map_err(domain_failure)?;
        check_deadline(self.expires)?;
        check_deadline(deadline)
    }
}

/// Read-only library boundary. The later service must enforce worker/process bounds.
pub fn inspect_container_sources(
    inventory: &ContainerStorageInventory,
) -> Result<ContainerSourceEvidence> {
    inspect_container_sources_with_profile(inventory, StorageReadProfile::Standard)
}
pub fn inspect_container_sources_with_profile(
    inventory: &ContainerStorageInventory,
    profile: StorageReadProfile,
) -> Result<ContainerSourceEvidence> {
    profile.admit_sources(inventory)?;
    inventory.validate(now()?).map_err(domain_failure)?;
    read_admission::json_limit(inventory, 64 * 1024)?;
    if !rustix::process::geteuid().is_root() {
        return Err(Failure::Unavailable);
    }
    let deadline = Instant::now() + Duration::from_secs(WORK_SECONDS);
    let table = read_table_with_profile(profile)?;
    let ns = host_view(&table, &read_host_table_with_profile(profile)?)?;
    collect_with_profile(inventory, Context::new()?, table, ns, deadline, profile)
}

#[cfg(test)]
fn collect(
    inventory: &ContainerStorageInventory,
    context: Context,
    table: Vec<mounts::Mount>,
    ns: u64,
    deadline: Instant,
) -> Result<ContainerSourceEvidence> {
    collect_with_profile(
        inventory,
        context,
        table,
        ns,
        deadline,
        StorageReadProfile::Standard,
    )
}
fn collect_with_profile(
    inventory: &ContainerStorageInventory,
    context: Context,
    table: Vec<mounts::Mount>,
    ns: u64,
    deadline: Instant,
    profile: StorageReadProfile,
) -> Result<ContainerSourceEvidence> {
    let count = profile.admit_sources(inventory)?;
    let observed_at = now()?;
    inventory.validate(observed_at).map_err(domain_failure)?;
    let expires = Instant::now()
        + Duration::from_secs(
            (CONTAINER_STORAGE_MAX_AGE_SECONDS - (observed_at - inventory.observed_at)) as u64,
        );
    let mounts_digest = limeos_identity::digest(&read_mount_text_with_profile(profile)?);
    read_admission::json_limit(inventory, 64 * 1024)?;
    if descriptor_mount_id(&context.root)? != ns {
        return Err(Failure::Unavailable);
    }
    let mut pinned = Vec::new();
    pinned
        .try_reserve_exact(count)
        .map_err(|_| Failure::Unavailable)?;
    let mut selected = BTreeSet::<u64>::new();
    let mut references = 0usize;
    for consumer in &inventory.containers {
        for source in &consumer.mounts {
            check_deadline(deadline)?;
            let fd = context.open_with_profile(source.source.path(), &table, deadline, profile)?;
            let (identity, resolved_path) = context.inspect(&fd, &table)?;
            let potential_mount_ids = candidates_with_limit(
                &identity,
                &resolved_path,
                &table,
                profile.references() - references,
            )?;
            references += potential_mount_ids.len();
            if references > profile.references() {
                return Err(Failure::Unavailable);
            }
            selected.extend(potential_mount_ids.iter().copied());
            pinned.push((
                ContainerSourcePath {
                    container: consumer.container.resource.clone(),
                    destination: source.destination.clone(),
                    declared_path: source.source.path().into(),
                    resolved_path,
                    identity,
                    potential_mount_ids,
                },
                fd,
            ));
        }
    }
    pinned.sort_by(|a, b| {
        (&a.0.container, &a.0.destination).cmp(&(&b.0.container, &b.0.destination))
    });
    let (sources, retained) = pinned.into_iter().unzip();
    let mut selected_mounts: Vec<_> = table
        .iter()
        .filter(|m| selected.contains(&m.id))
        .map(|m| ContainerSourceMount {
            mount_id: m.id,
            device: m.device,
            filesystem_root: m.root.clone(),
            mountpoint: m.path.clone(),
            filesystem: m.filesystem.clone(),
        })
        .collect();
    selected_mounts.sort_by_key(|m| m.mount_id);
    let evidence = ContainerSourceEvidence {
        snapshot: ContainerSourceSnapshot {
            version: 1,
            inventory_digest: profile.digest(inventory)?,
            host_root_mount_id: ns,
            mounts_digest,
            sources,
            mounts: selected_mounts,
        },
        inventory: inventory.clone(),
        table,
        context,
        retained,
        expires,
        profile,
    };
    read_admission::json_limit(evidence.snapshot(), MAX_REPORT_BYTES)?;
    evidence.revalidate_inner(deadline)?;
    Ok(evidence)
}

struct Context {
    root: OwnedFd,
    descriptors: OwnedFd,
}
impl Context {
    fn new() -> Result<Self> {
        let open = |p: &str| {
            rustix::fs::open(
                p,
                OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| Failure::Unavailable)
        };
        Ok(Self {
            root: open("/")?,
            descriptors: open(&format!("/proc/{}/fd", std::process::id()))?,
        })
    }
    #[cfg(test)]
    fn open(&self, name: &str, table: &[mounts::Mount], deadline: Instant) -> Result<OwnedFd> {
        self.open_with_profile(name, table, deadline, StorageReadProfile::Standard)
    }
    fn open_with_profile(
        &self,
        name: &str,
        table: &[mounts::Mount],
        deadline: Instant,
        profile: StorageReadProfile,
    ) -> Result<OwnedFd> {
        if !container_storage_path(name) {
            return Err(Failure::InvalidPlan);
        }
        let mut pending = Pending::new(profile);
        pending.prepend(name)?;
        let mut resolved = Vec::<String>::new();
        let mut links = 0;
        let mut steps = 0;
        while let Some(part) = pending.pop() {
            check_deadline(deadline)?;
            steps += 1;
            if steps > 512 {
                return Err(Failure::Unavailable);
            }
            match part.as_str() {
                "" | "." => continue,
                ".." => {
                    resolved.pop();
                    continue;
                }
                _ => {}
            }
            let path = format!("/{}/{}", resolved.join("/"), part);
            let path = if resolved.is_empty() {
                format!("/{part}")
            } else {
                path
            };
            let fd = self.cached_leaf(&path)?;
            let mount_id = descriptor_mount_id(&fd)?;
            let mount = table
                .iter()
                .find(|m| m.id == mount_id)
                .ok_or(Failure::Conflict)?;
            if !local_filesystem(&mount.filesystem) {
                return Err(Failure::Unavailable);
            }
            let s = rustix::fs::statx(
                &fd,
                "",
                AtFlags::EMPTY_PATH | AtFlags::STATX_DONT_SYNC,
                StatxFlags::TYPE | StatxFlags::MNT_ID,
            )
            .map_err(|_| Failure::Unavailable)?;
            let mask = StatxFlags::TYPE | StatxFlags::MNT_ID;
            if s.stx_mask & mask.bits() != mask.bits()
                || s.stx_mnt_id != mount_id
                || s.stx_dev_major != mount.device.major
                || s.stx_dev_minor != mount.device.minor
            {
                return Err(Failure::Conflict);
            }
            if s.stx_mode & 0o170000 == 0o120000 {
                links += 1;
                if links > 40 {
                    return Err(Failure::UnsafePath);
                }
                // Read the retained ordinary link itself, never its mutable name.
                // Local link metadata may update atime; no file data is opened.
                let mut buffer = [0u8; 513];
                let length = rustix::fs::readlinkat_raw(&fd, "", &mut buffer)
                    .map_err(|_| Failure::Unavailable)?;
                if length == 0 || length > 512 {
                    return Err(Failure::Unavailable);
                }
                let target =
                    std::str::from_utf8(&buffer[..length]).map_err(|_| Failure::UnsafePath)?;
                if target.chars().any(char::is_control) {
                    return Err(Failure::UnsafePath);
                }
                if target.starts_with('/') {
                    resolved.clear();
                }
                pending.prepend(target)?;
            } else {
                // A subsequent '.', '..' or empty trailing component also
                // requires a directory in the real VFS path walk.
                if !pending.is_empty() && s.stx_mode & 0o170000 != 0o040000 {
                    return Err(Failure::UnsafePath);
                }
                resolved.push(part);
                if pending.is_empty() {
                    return Ok(fd);
                }
            }
        }
        self.cached_leaf(&format!("/{}", resolved.join("/")))
    }
    fn cached_leaf(&self, name: &str) -> Result<OwnedFd> {
        if !container_storage_path(name) {
            return Err(Failure::Unavailable);
        }
        rustix::fs::openat2(
            &self.root,
            name,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
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
    fn inspect(
        &self,
        fd: &OwnedFd,
        table: &[mounts::Mount],
    ) -> Result<(ContainerSourceIdentity, String)> {
        // Proc's mount ID is kernel metadata. Reject remote/FUSE sources before
        // requesting inode attributes from a potentially stalled filesystem.
        let mount_id = descriptor_mount_id(fd)?;
        let mount = table
            .iter()
            .find(|m| m.id == mount_id)
            .ok_or(Failure::Conflict)?;
        if !local_filesystem(&mount.filesystem) {
            return Err(Failure::Unavailable);
        }
        let mask = StatxFlags::TYPE | StatxFlags::INO | StatxFlags::NLINK | StatxFlags::MNT_ID;
        let s = rustix::fs::statx(fd, "", AtFlags::EMPTY_PATH | AtFlags::STATX_DONT_SYNC, mask)
            .map_err(|_| Failure::Unavailable)?;
        if s.stx_mask & mask.bits() != mask.bits() || s.stx_ino == 0 || s.stx_nlink == 0 {
            return Err(Failure::Unavailable);
        }
        let kind = match s.stx_mode & 0o170000 {
            0o100000 => ContainerSourceKind::RegularFile,
            0o040000 => ContainerSourceKind::Directory,
            _ => return Err(Failure::UnsafePath),
        };
        let device = DeviceNumber {
            major: s.stx_dev_major,
            minor: s.stx_dev_minor,
        };
        if s.stx_mnt_id != mount_id || device != mount.device {
            return Err(Failure::Conflict);
        }
        let mut buffer = [0u8; 513];
        let length =
            rustix::fs::readlinkat_raw(&self.descriptors, fd.as_raw_fd().to_string(), &mut buffer)
                .map_err(|_| Failure::Unavailable)?;
        if length > 512 {
            return Err(Failure::Unavailable);
        }
        let resolved = std::str::from_utf8(&buffer[..length]).map_err(|_| Failure::Unavailable)?;
        if !container_storage_path(resolved)
            || resolved.ends_with(" (deleted)")
            || !beneath(resolved, &mount.path)
        {
            return Err(Failure::Conflict);
        }
        Ok((
            ContainerSourceIdentity {
                device,
                mount_id,
                inode: s.stx_ino,
                kind,
            },
            resolved.into(),
        ))
    }
}

struct Pending {
    values: VecDeque<String>,
    bytes: usize,
    profile: StorageReadProfile,
}
impl Pending {
    fn new(profile: StorageReadProfile) -> Self {
        Self {
            values: VecDeque::new(),
            bytes: 0,
            profile,
        }
    }
    fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    fn pop(&mut self) -> Option<String> {
        let part = self.values.pop_front()?;
        self.bytes -= part.capacity();
        Some(part)
    }
    fn prepend(&mut self, text: &str) -> Result<()> {
        let count = text.split('/').count();
        let bytes = text
            .split('/')
            .try_fold(self.bytes, |n, c| n.checked_add(c.len()))
            .ok_or(Failure::Unavailable)?;
        let total = self
            .values
            .len()
            .checked_add(count)
            .ok_or(Failure::Unavailable)?;
        if self.profile == StorageReadProfile::CombinedV1 && (total > 256 || bytes > 16 * 1024) {
            return Err(Failure::Unavailable);
        }
        self.values
            .try_reserve_exact(count)
            .map_err(|_| Failure::Unavailable)?;
        if self.profile == StorageReadProfile::CombinedV1 && self.values.capacity() > 256 {
            return Err(Failure::Unavailable);
        }
        for component in text.split('/').rev() {
            let value = read_admission::copy_text(component, usize::MAX)?;
            self.bytes = self
                .bytes
                .checked_add(value.capacity())
                .ok_or(Failure::Unavailable)?;
            if self.profile == StorageReadProfile::CombinedV1 && self.bytes > 16 * 1024 {
                return Err(Failure::Unavailable);
            }
            self.values.push_front(value);
        }
        Ok(())
    }
}

#[cfg(test)]
fn candidates(
    identity: &ContainerSourceIdentity,
    resolved: &str,
    table: &[mounts::Mount],
) -> Result<Vec<u64>> {
    candidates_with_limit(identity, resolved, table, MAX_MOUNT_REFERENCES)
}
fn candidates_with_limit(
    identity: &ContainerSourceIdentity,
    resolved: &str,
    table: &[mounts::Mount],
    limit: usize,
) -> Result<Vec<u64>> {
    if !table.iter().any(|m| m.id == identity.mount_id) {
        return Err(Failure::Conflict);
    }
    let selected = || {
        table
            .iter()
            .filter(|m| {
                m.id == identity.mount_id
                    || (identity.kind == ContainerSourceKind::Directory
                        && beneath(&m.path, resolved))
            })
            .map(|m| m.id)
    };
    let count = selected().count();
    if count > limit {
        return Err(Failure::Unavailable);
    }
    let mut ids = Vec::new();
    ids.try_reserve_exact(count)
        .map_err(|_| Failure::Unavailable)?;
    ids.extend(selected());
    ids.sort_unstable();
    ids.dedup();
    if ids.len() != count {
        return Err(Failure::Unavailable);
    }
    Ok(ids)
}
fn beneath(path: &str, root: &str) -> bool {
    root == "/" || path == root || path.strip_prefix(root).is_some_and(|s| s.starts_with('/'))
}
fn local_filesystem(fs: &str) -> bool {
    // Device-number observations are not UUID or multi-device backing proof.
    matches!(
        fs,
        "ext2" | "ext3" | "ext4" | "xfs" | "btrfs" | "vfat" | "exfat" | "ntfs3" | "tmpfs"
    )
}
pub(crate) fn descriptor_mount_id(fd: &OwnedFd) -> Result<u64> {
    mount_id(&bounded_read(
        Path::new(&format!(
            "/proc/{}/fdinfo/{}",
            std::process::id(),
            fd.as_raw_fd()
        )),
        4096,
    )?)
}
fn mount_id(text: &str) -> Result<u64> {
    let mut values = text
        .lines()
        .filter_map(|line| line.strip_prefix("mnt_id:"))
        .map(str::trim);
    let id: u64 = values
        .next()
        .ok_or(Failure::Unavailable)?
        .parse()
        .map_err(|_| Failure::Unavailable)?;
    if id == 0 || values.next().is_some() {
        return Err(Failure::Unavailable);
    }
    Ok(id)
}
#[cfg(test)]
fn read_table() -> Result<Vec<mounts::Mount>> {
    read_table_with_profile(StorageReadProfile::Standard)
}
fn read_table_with_profile(profile: StorageReadProfile) -> Result<Vec<mounts::Mount>> {
    let table = mounts::parse_with_profile(&read_mount_text_with_profile(profile)?, profile)?;
    if table
        .iter()
        .any(|m| m.id == 0 || !container_storage_path(&m.path) || !container_storage_path(&m.root))
    {
        return Err(Failure::Unavailable);
    }
    Ok(table)
}
fn read_mount_text_with_profile(profile: StorageReadProfile) -> Result<String> {
    profile.read(Path::new("/proc/self/mountinfo"), profile.table_bytes())
}
#[cfg(test)]
fn table_matches(text: &str, table: &[mounts::Mount], expected_digest: &str) -> Result<()> {
    table_matches_with_profile(text, table, expected_digest, StorageReadProfile::Standard)
}
fn table_matches_with_profile(
    text: &str,
    table: &[mounts::Mount],
    expected_digest: &str,
    profile: StorageReadProfile,
) -> Result<()> {
    if profile == StorageReadProfile::Standard && limeos_identity::digest(text) != expected_digest {
        return Err(Failure::Conflict);
    }
    let parsed = mounts::parse_with_profile(text, profile)?;
    if limeos_identity::digest(text) != expected_digest || parsed != table {
        Err(Failure::Conflict)
    } else {
        Ok(())
    }
}
#[cfg(test)]
fn read_host_table() -> Result<Vec<mounts::Mount>> {
    read_host_table_with_profile(StorageReadProfile::Standard)
}
fn read_host_table_with_profile(profile: StorageReadProfile) -> Result<Vec<mounts::Mount>> {
    mounts::parse_with_profile(
        &profile.read(Path::new("/proc/1/mountinfo"), profile.table_bytes())?,
        profile,
    )
}
fn check_deadline(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        Err(Failure::TimedOut)
    } else {
        Ok(())
    }
}
pub(crate) fn now() -> Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .ok_or(Failure::Unavailable)
}
#[cfg(test)]
fn digest<T: Serialize>(value: &T) -> Result<String> {
    Ok(limeos_identity::digest(
        &serde_json::to_string(value).map_err(|_| Failure::Unavailable)?,
    ))
}
fn domain_failure(e: limeos_domain::Error) -> Failure {
    if e.0 == limeos_domain::ErrorCode::Conflict {
        Failure::Conflict
    } else {
        Failure::InvalidPlan
    }
}

#[cfg(test)]
mod tests;
