//! Retained host source descriptors, not complete physical dependency authority.
use crate::{Failure, Result, bounded_read, host_view, mounts, topology::DeviceNumber};
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
        table_matches(
            &read_mount_text()?,
            &self.table,
            &self.snapshot.mounts_digest,
        )?;
        let table = &self.table;
        if host_view(table, &read_host_table()?)? != self.snapshot.host_root_mount_id
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
                let again = self.context.open(name, table, deadline)?;
                if self.context.inspect(&again, table)?
                    != (source.identity.clone(), source.resolved_path.clone())
                {
                    return Err(Failure::Conflict);
                }
            }
        }
        table_matches(
            &read_mount_text()?,
            &self.table,
            &self.snapshot.mounts_digest,
        )?;
        if host_view(&self.table, &read_host_table()?)? != self.snapshot.host_root_mount_id {
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
    inventory.validate(now()?).map_err(domain_failure)?;
    if !rustix::process::geteuid().is_root() {
        return Err(Failure::Unavailable);
    }
    let deadline = Instant::now() + Duration::from_secs(WORK_SECONDS);
    let table = read_table()?;
    let ns = host_view(&table, &read_host_table()?)?;
    collect(inventory, Context::new()?, table, ns, deadline)
}

fn collect(
    inventory: &ContainerStorageInventory,
    context: Context,
    table: Vec<mounts::Mount>,
    ns: u64,
    deadline: Instant,
) -> Result<ContainerSourceEvidence> {
    let observed_at = now()?;
    inventory.validate(observed_at).map_err(domain_failure)?;
    let expires = Instant::now()
        + Duration::from_secs(
            (CONTAINER_STORAGE_MAX_AGE_SECONDS - (observed_at - inventory.observed_at)) as u64,
        );
    let mounts_digest = limeos_identity::digest(&read_mount_text()?);
    if descriptor_mount_id(&context.root)? != ns
        || serde_json::to_vec(inventory)
            .map_err(|_| Failure::Unavailable)?
            .len()
            > 64 * 1024
    {
        return Err(Failure::Unavailable);
    }
    let mut pinned = Vec::new();
    let mut selected = BTreeSet::<u64>::new();
    let mut references = 0usize;
    for consumer in &inventory.containers {
        for source in &consumer.mounts {
            check_deadline(deadline)?;
            let fd = context.open(source.source.path(), &table, deadline)?;
            let (identity, resolved_path) = context.inspect(&fd, &table)?;
            let potential_mount_ids = candidates(&identity, &resolved_path, &table)?;
            references += potential_mount_ids.len();
            if references > MAX_MOUNT_REFERENCES {
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
            inventory_digest: digest(inventory)?,
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
    };
    if serde_json::to_vec(evidence.snapshot())
        .map_err(|_| Failure::Unavailable)?
        .len()
        > MAX_REPORT_BYTES
    {
        return Err(Failure::Unavailable);
    }
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
    fn open(&self, name: &str, table: &[mounts::Mount], deadline: Instant) -> Result<OwnedFd> {
        if !container_storage_path(name) {
            return Err(Failure::InvalidPlan);
        }
        let mut pending: VecDeque<_> = name.split('/').map(str::to_owned).collect();
        let mut resolved = Vec::<String>::new();
        let mut links = 0;
        let mut steps = 0;
        while let Some(part) = pending.pop_front() {
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
                for component in target.split('/').rev() {
                    pending.push_front(component.into());
                }
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

fn candidates(
    identity: &ContainerSourceIdentity,
    resolved: &str,
    table: &[mounts::Mount],
) -> Result<Vec<u64>> {
    if !table.iter().any(|m| m.id == identity.mount_id) {
        return Err(Failure::Conflict);
    }
    let ids: BTreeSet<_> = table
        .iter()
        .filter(|m| {
            m.id == identity.mount_id
                || (identity.kind == ContainerSourceKind::Directory && beneath(&m.path, resolved))
        })
        .map(|m| m.id)
        .collect();
    if ids.len() > MAX_MOUNT_REFERENCES {
        return Err(Failure::Unavailable);
    }
    Ok(ids.into_iter().collect())
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
fn descriptor_mount_id(fd: &OwnedFd) -> Result<u64> {
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
fn read_table() -> Result<Vec<mounts::Mount>> {
    let table = mounts::parse(&read_mount_text()?)?;
    if table
        .iter()
        .any(|m| m.id == 0 || !container_storage_path(&m.path) || !container_storage_path(&m.root))
    {
        return Err(Failure::Unavailable);
    }
    Ok(table)
}
fn read_mount_text() -> Result<String> {
    bounded_read(Path::new("/proc/self/mountinfo"), 1024 * 1024)
}
fn table_matches(text: &str, table: &[mounts::Mount], expected_digest: &str) -> Result<()> {
    if limeos_identity::digest(text) != expected_digest || mounts::parse(text)? != table {
        Err(Failure::Conflict)
    } else {
        Ok(())
    }
}
fn read_host_table() -> Result<Vec<mounts::Mount>> {
    mounts::parse(&bounded_read(Path::new("/proc/1/mountinfo"), 1024 * 1024)?)
}
fn check_deadline(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        Err(Failure::TimedOut)
    } else {
        Ok(())
    }
}
fn now() -> Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .ok_or(Failure::Unavailable)
}
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
