//! Fresh storage evidence and protected root operator target preparation.
mod fstab;
mod inventory;
mod mounts;
mod path;
mod sources;
mod swaps;
mod targets;
mod topology;
pub use fstab::planned_fstab;
pub use inventory::inventory;
use limeos_domain::{StorageFilesystem, StorageMountWaitPlan};
use serde::Serialize;
pub use sources::{
    ContainerSourceEvidence, ContainerSourceIdentity, ContainerSourceKind, ContainerSourceMount,
    ContainerSourcePath, ContainerSourceSnapshot, inspect_container_sources,
};
use std::{
    collections::BTreeMap,
    io::Read,
    os::fd::AsRawFd,
    path::Path,
    time::{Duration, Instant},
};
pub use targets::{
    PreparationState, TargetPreparationPlan, TargetPreparationReceipt, prepare_targets,
    read_contract, read_protected_json, read_target_plan, reconcile_target_plan, reconcile_targets,
    target_plan, target_receipt, target_snapshot,
};
use tokio::io::AsyncReadExt;
pub use topology::DeviceNumber;

type Result<T> = std::result::Result<T, Failure>;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    InvalidPlan,
    UnsafePath,
    WrongNamespace,
    BootDevice,
    ActiveSwap,
    Conflict,
    StateNotDurable,
    IdentityMismatch,
    AmbiguousIdentity,
    NotReady,
    Unavailable,
    TimedOut,
}
impl Failure {
    pub fn message(self) -> &'static str {
        match self {
            Self::InvalidPlan => "Invalid storage plan; the file was preserved.",
            Self::UnsafePath => {
                "Storage paths must have root-controlled components without symlinks."
            }
            Self::WrongNamespace => "Storage verification requires the host mount namespace.",
            Self::BootDevice => {
                "The assignment shares a backing device with the root or boot filesystem."
            }
            Self::ActiveSwap => "The assignment shares a backing device with active swap.",
            Self::Conflict => "Storage evidence changed or a resource requires reconciliation.",
            Self::StateNotDurable => "Storage preparation requires durable protected receipts.",
            Self::IdentityMismatch => {
                "The mounted filesystem, UUID or serial differs from the assignment."
            }
            Self::AmbiguousIdentity => "The filesystem UUID occurs on multiple devices.",
            Self::NotReady => "An assigned device mount is not ready.",
            Self::Unavailable => "Fresh device or mount evidence is unavailable.",
            Self::TimedOut => "The storage readiness deadline expired.",
        }
    }
}
#[derive(Debug, Serialize)]
pub struct VerifiedMount {
    pub id: String,
    pub filesystem_uuid: String,
    pub device: DeviceNumber,
    pub mountpoint: String,
    pub mount_id: u64,
}
#[derive(Debug, Serialize)]
pub struct Verification {
    pub host_root_mount_id: u64,
    pub mounts: Vec<VerifiedMount>,
}

pub fn read_plan(file: &Path) -> Result<StorageMountWaitPlan> {
    path::plan(file)
}
fn namespace() -> Result<u64> {
    // Reading PID 1's namespace symlink requires ptrace capability even for
    // UID 0 with an empty capability set. Its mount table is readable instead.
    // Cloning a mount namespace allocates distinct kernel mount IDs; equal
    // device numbers/paths alone would not prove the executor's host view.
    let current = mounts::parse(&bounded_read(
        Path::new("/proc/self/mountinfo"),
        1024 * 1024,
    )?)?;
    let host = mounts::parse(&bounded_read(Path::new("/proc/1/mountinfo"), 1024 * 1024)?)?;
    host_view(&current, &host)
}
fn host_view(current: &[mounts::Mount], host: &[mounts::Mount]) -> Result<u64> {
    let root_id = |table: &[mounts::Mount]| -> Result<u64> {
        let roots: Vec<_> = table.iter().filter(|m| m.path == "/").collect();
        if roots.len() != 1 || roots[0].id == 0 {
            return Err(Failure::Unavailable);
        }
        Ok(roots[0].id)
    };
    let current = root_id(current)?;
    let host = root_id(host)?;
    if current != host {
        return Err(Failure::WrongNamespace);
    }
    Ok(host)
}
fn bounded_read(path: &Path, limit: u64) -> Result<String> {
    let mut text = String::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(limit + 1).read_to_string(&mut text))
        .map_err(|_| Failure::Unavailable)?;
    if text.len() as u64 > limit {
        return Err(Failure::Unavailable);
    }
    Ok(text)
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Filesystem {
    uuid: String,
    kind: String,
}

async fn probe(fd: &rustix::fd::OwnedFd) -> Result<Option<Filesystem>> {
    // Probe through the retained descriptor, not a mutable /dev alias or udev cache.
    let descriptor = format!("/proc/{}/fd/{}", std::process::id(), fd.as_raw_fd());
    let mut child = tokio::process::Command::new("/usr/sbin/blkid")
        .args([
            "-p",
            "-o",
            "export",
            "-s",
            "UUID",
            "-s",
            "TYPE",
            &descriptor,
        ])
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| Failure::Unavailable)?;
    tokio::time::timeout(Duration::from_secs(3), async {
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .ok_or(Failure::Unavailable)?
            .take(8193)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| Failure::Unavailable)?;
        if bytes.len() > 8192 {
            return Err(Failure::Unavailable);
        }
        let status = child.wait().await.map_err(|_| Failure::Unavailable)?;
        if status.code() == Some(2) {
            return Ok(None);
        } // No filesystem signature.
        if !status.success() {
            return Err(Failure::Unavailable);
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| Failure::Unavailable)?;
        let field = |key: &str| -> Result<Option<String>> {
            let values: Vec<_> = text.lines().filter_map(|l| l.strip_prefix(key)).collect();
            if values.len() > 1 {
                return Err(Failure::Unavailable);
            }
            Ok(values.first().map(|s| s.to_string()))
        };
        let Some(uuid) = field("UUID=")? else {
            return Ok(None);
        };
        let kind = field("TYPE=")?.ok_or(Failure::Unavailable)?;
        if uuid.is_empty()
            || uuid.len() > 128
            || !uuid
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            || kind.is_empty()
            || kind.len() > 64
        {
            return Err(Failure::Unavailable);
        }
        Ok(Some(Filesystem { uuid, kind }))
    })
    .await
    .map_err(|_| Failure::Unavailable)?
}

fn filesystem_matches(expected: StorageFilesystem, actual: &str) -> bool {
    match expected {
        StorageFilesystem::Ext2 => actual == "ext2",
        StorageFilesystem::Ext3 => actual == "ext3",
        StorageFilesystem::Ext4 => actual == "ext4",
        StorageFilesystem::Exfat => actual == "exfat",
        StorageFilesystem::Vfat => actual == "vfat",
        StorageFilesystem::Xfs => actual == "xfs",
        StorageFilesystem::Ntfs => matches!(actual, "ntfs" | "ntfs3"),
        // Btrfs needs a complete multi-device backing graph before acceptance.
        StorageFilesystem::Btrfs => false,
    }
}

pub async fn verify(plan: &StorageMountWaitPlan) -> Result<Verification> {
    plan.validate().map_err(|_| Failure::InvalidPlan)?;
    if !rustix::process::geteuid().is_root() {
        return Err(Failure::Unavailable);
    }
    let ns = namespace()?;
    let topology = topology::Topology::collect()?;
    let mounts = mounts::parse(&bounded_read(
        Path::new("/proc/self/mountinfo"),
        1024 * 1024,
    )?)?;
    let excluded = topology.connected(&mounts::protected(&mounts)?)?;
    let swaps = swaps::Snapshot::collect()?;
    let swap_backing = topology.connected(&swaps.devices)?;
    let mut identities = BTreeMap::new();
    let mut handles = BTreeMap::new();
    // Probe all non-virtual block nodes, so duplicate UUIDs fail closed even
    // when the second disk is unmounted and the lsblk/udev cache is stale.
    for (number, node) in &topology.0 {
        if ["loop", "ram", "zram", "sr"]
            .iter()
            .any(|p| node.name.starts_with(p))
        {
            continue;
        }
        let fd = path::open(&topology.path(*number)?, false, false)?;
        let s = path::stat(&fd)?;
        if s.stx_mode & 0o170000 != 0o060000
            || s.stx_rdev_major != number.major
            || s.stx_rdev_minor != number.minor
        {
            return Err(Failure::IdentityMismatch);
        }
        if let Some(fs) = probe(&fd).await? {
            if identities.insert(fs.uuid.clone(), (*number, fs)).is_some() {
                return Err(Failure::AmbiguousIdentity);
            }
        }
        handles.insert(*number, fd);
    }
    let mut verified = Vec::new();
    let mut mount_handles = Vec::new();
    for assignment in &plan.devices {
        let (number, fs) = identities.get(&assignment.filesystem_uuid).ok_or_else(|| {
            if mounts.iter().any(|m| m.path == assignment.mountpoint) {
                Failure::IdentityMismatch
            } else {
                Failure::NotReady
            }
        })?;
        if excluded.contains(number) {
            return Err(Failure::BootDevice);
        }
        if swap_backing.contains(number) {
            return Err(Failure::ActiveSwap);
        }
        if !filesystem_matches(assignment.filesystem, &fs.kind)
            || assignment
                .serial
                .as_ref()
                .is_some_and(|s| !topology.serial_matches(*number, s))
        {
            return Err(Failure::IdentityMismatch);
        }
        let exact: Vec<_> = mounts
            .iter()
            .filter(|m| m.path == assignment.mountpoint)
            .collect();
        if exact.is_empty() {
            return Err(Failure::NotReady);
        }
        if exact.len() != 1 {
            return Err(Failure::IdentityMismatch);
        }
        let m = exact[0];
        if m.root != "/"
            || m.device != *number
            || !m.writable
            || !filesystem_matches(assignment.filesystem, &m.filesystem)
        {
            return Err(Failure::IdentityMismatch);
        }
        let fd = path::open(Path::new(&assignment.mountpoint), true, true)?;
        if probe(handles.get(number).ok_or(Failure::Unavailable)?)
            .await?
            .as_ref()
            != Some(fs)
        {
            return Err(Failure::IdentityMismatch);
        }
        let s = path::stat(&fd)?;
        if s.stx_mnt_id != m.id
            || s.stx_dev_major != number.major
            || s.stx_dev_minor != number.minor
        {
            return Err(Failure::IdentityMismatch);
        }
        mount_handles.push(fd);
        verified.push(VerifiedMount {
            id: assignment.id.clone(),
            filesystem_uuid: fs.uuid.clone(),
            device: *number,
            mountpoint: assignment.mountpoint.clone(),
            mount_id: m.id,
        });
    }
    // Detect hotplug, overmount and namespace changes during collection. Retain
    // all descriptors until these comparisons finish. Effects must re-verify.
    if swaps::Snapshot::collect()? != swaps
        || namespace()? != ns
        || topology::Topology::collect()? != topology
        || mounts::parse(&bounded_read(
            Path::new("/proc/self/mountinfo"),
            1024 * 1024,
        )?)? != mounts
    {
        return Err(Failure::IdentityMismatch);
    }
    Ok(Verification {
        host_root_mount_id: ns,
        mounts: verified,
    })
}

pub async fn wait(plan: &StorageMountWaitPlan) -> Result<Verification> {
    plan.validate().map_err(|_| Failure::InvalidPlan)?;
    let duration = Duration::from_secs(u64::from(plan.timeout_seconds));
    let deadline = Instant::now() + duration;
    tokio::time::timeout(duration, async {
        loop {
            match verify(plan).await {
                Err(Failure::NotReady) => {}
                result => return result,
            }
            if Instant::now() >= deadline {
                return Err(Failure::TimedOut);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .map_err(|_| Failure::TimedOut)?
}

#[cfg(test)]
mod tests;
