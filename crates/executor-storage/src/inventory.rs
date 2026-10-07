//! Direct, bounded evidence for existing-filesystem guided assignments.
use crate::*;
use limeos_domain::{
    StorageBlockDevice, StorageInventory, StorageObservedDevice, StorageObservedMount,
};
use rustix::fd::OwnedFd;
use std::collections::BTreeSet;

pub(crate) fn digest<T: Serialize>(value: &T) -> Result<String> {
    let body = serde_json::to_string(value).map_err(|_| Failure::Unavailable)?;
    Ok(limeos_identity::digest(&body))
}
pub(crate) fn wire(n: DeviceNumber) -> StorageBlockDevice {
    StorageBlockDevice {
        major: n.major,
        minor: n.minor,
    }
}
fn serial(topology: &topology::Topology, number: DeviceNumber) -> Option<String> {
    let mut next = vec![number];
    let mut seen = BTreeSet::new();
    let mut found = BTreeSet::new();
    while let Some(n) = next.pop() {
        if !seen.insert(n) {
            continue;
        }
        let node = topology.0.get(&n)?;
        if let Some(s) = &node.serial {
            found.insert(s.clone());
        }
        next.extend(&node.parents);
    }
    if found.len() == 1 {
        found.pop_first()
    } else {
        None
    }
}

/// Construction and revalidation retain raw descriptors; the wire inventory
/// alone cannot establish fresh signatures or a stable kernel mount table.
pub struct StorageInventoryEvidence {
    snapshot: StorageInventory,
    pub(crate) topology: topology::Topology,
    pub(crate) table: Vec<mounts::Mount>,
    pub(crate) raw_mounts_digest: String,
    swaps: swaps::Snapshot,
    fstab: OwnedFd,
    handles: Vec<(OwnedFd, Option<Filesystem>)>,
    expires: Instant,
}
impl StorageInventoryEvidence {
    pub fn snapshot(&self) -> &StorageInventory {
        &self.snapshot
    }
    pub async fn revalidate(&self) -> Result<()> {
        if !rustix::process::geteuid().is_root() {
            return Err(Failure::Unavailable);
        }
        self.check_context()?;
        for (fd, expected) in &self.handles {
            self.check_age()?;
            let remaining = self.expires.saturating_duration_since(Instant::now());
            if &tokio::time::timeout(remaining, probe(fd))
                .await
                .map_err(|_| Failure::TimedOut)??
                != expected
            {
                return Err(Failure::IdentityMismatch);
            }
        }
        self.check_context()
    }
    fn check_age(&self) -> Result<()> {
        if Instant::now() >= self.expires {
            Err(Failure::TimedOut)
        } else {
            Ok(())
        }
    }
    fn check_context(&self) -> Result<()> {
        self.check_age()?;
        let text = bounded_read(Path::new("/proc/self/mountinfo"), 1024 * 1024)?;
        let (fresh_fstab, fstab_text) = fstab::read()?;
        if namespace()? != self.snapshot.host_root_mount_id
            || limeos_identity::digest(&text) != self.raw_mounts_digest
            || mounts::parse(&text)? != self.table
            || topology::Topology::collect()? != self.topology
            || swaps::Snapshot::collect()? != self.swaps
            || limeos_identity::digest(&fstab_text) != self.snapshot.fstab_digest
        {
            return Err(Failure::IdentityMismatch);
        }
        same_fstab(&self.fstab, &fresh_fstab)?;
        self.check_age()
    }
}

fn same_fstab(retained: &OwnedFd, fresh: &OwnedFd) -> Result<()> {
    let a = path::stat(retained)?;
    let b = path::stat(fresh)?;
    if a.stx_nlink != 1
        || b.stx_nlink != 1
        || a.stx_mode & 0o170000 != 0o100000
        || b.stx_mode & 0o170000 != 0o100000
        || (a.stx_dev_major, a.stx_dev_minor, a.stx_mnt_id, a.stx_ino)
            != (b.stx_dev_major, b.stx_dev_minor, b.stx_mnt_id, b.stx_ino)
    {
        Err(Failure::IdentityMismatch)
    } else {
        Ok(())
    }
}

/// The caller enforces a global deadline and single-flight admission. No cache,
/// udev alias, previous success or caller-supplied filesystem path is authority.
pub async fn inventory() -> Result<StorageInventory> {
    Ok(inspect_storage_inventory().await?.snapshot().clone())
}

/// Retained read-only evidence. Synchronous reads still require process admission
/// and a process deadline at the later service boundary.
pub async fn inspect_storage_inventory() -> Result<StorageInventoryEvidence> {
    if !rustix::process::geteuid().is_root() {
        return Err(Failure::Unavailable);
    }
    let expires = Instant::now() + Duration::from_secs(5);
    let ns = namespace()?;
    let topology = topology::Topology::collect()?;
    let mount_text = bounded_read(Path::new("/proc/self/mountinfo"), 1024 * 1024)?;
    let mounts = mounts::parse(&mount_text)?;
    let boot = topology.connected(&mounts::protected(&mounts)?)?;
    let swaps = swaps::Snapshot::collect()?;
    let swap_backing = topology.connected(&swaps.devices)?;
    let (fstab_handle, fstab_text) = fstab::read()?;
    let fstab_entries = fstab::entries(&fstab_text, &topology)?;
    let mut devices = Vec::new();
    let mut handles = Vec::new();
    for (number, node) in &topology.0 {
        if ["loop", "ram", "zram", "sr"]
            .iter()
            .any(|p| node.name.starts_with(p))
        {
            continue;
        }
        let fd = path::open(&topology.path(*number)?, false, false)?;
        let stat = path::stat(&fd)?;
        if stat.stx_mode & 0o170000 != 0o060000
            || stat.stx_rdev_major != number.major
            || stat.stx_rdev_minor != number.minor
        {
            return Err(Failure::IdentityMismatch);
        }
        let fs = tokio::time::timeout(
            expires.saturating_duration_since(Instant::now()),
            probe(&fd),
        )
        .await
        .map_err(|_| Failure::TimedOut)??;
        if let Some(fs) = &fs {
            let mut selected: Vec<_> = mounts
                .iter()
                .filter(|m| m.device == *number)
                .map(|m| StorageObservedMount {
                    mountpoint: m.path.clone(),
                    mount_id: m.id,
                    filesystem_root: m.root.clone(),
                    filesystem: m.filesystem.clone(),
                    writable: m.writable,
                })
                .collect();
            selected.sort_by(|a, b| a.mount_id.cmp(&b.mount_id));
            devices.push(StorageObservedDevice {
                device: wire(*number),
                filesystem_uuid: fs.uuid.clone(),
                filesystem: fs.kind.clone(),
                serial: serial(&topology, *number),
                boot_backing: boot.contains(number),
                in_use_as_swap: swap_backing.contains(number),
                mounts: selected,
            });
        }
        handles.push((fd, fs));
    }
    let topology_rows: Vec<_> = topology.0.iter().collect();
    let value = StorageInventory {
        host_root_mount_id: ns,
        topology_digest: digest(&(topology_rows, &swaps))?,
        mounts_digest: digest(&mounts)?,
        fstab_digest: limeos_identity::digest(&fstab_text),
        fstab_entries,
        devices,
    };
    value.validate().map_err(|_| Failure::Unavailable)?;
    if serde_json::to_vec(&value)
        .map_err(|_| Failure::Unavailable)?
        .len()
        > 48 * 1024
    {
        return Err(Failure::Unavailable);
    }
    let evidence = StorageInventoryEvidence {
        snapshot: value,
        topology,
        table: mounts,
        raw_mounts_digest: limeos_identity::digest(&mount_text),
        swaps,
        fstab: fstab_handle,
        handles,
        expires,
    };
    evidence.revalidate().await?;
    Ok(evidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retained_fstab_identity_refuses_equal_bytes_at_a_replaced_name() {
        let dir = tempfile::tempdir().unwrap();
        let name = dir.path().join("fstab");
        std::fs::write(&name, b"# same bytes\n").unwrap();
        let open = || {
            rustix::fs::open(&name, rustix::fs::OFlags::RDONLY, rustix::fs::Mode::empty()).unwrap()
        };
        let retained = open();
        same_fstab(&retained, &open()).unwrap();
        std::fs::rename(&name, dir.path().join("old")).unwrap();
        std::fs::write(&name, b"# same bytes\n").unwrap();
        assert_eq!(
            same_fstab(&retained, &open()),
            Err(Failure::IdentityMismatch)
        );
    }
    #[test]
    fn retained_fstab_identity_refuses_unlinked_or_multiply_linked_files() {
        let dir = tempfile::tempdir().unwrap();
        let name = dir.path().join("fstab");
        std::fs::write(&name, b"# fixture\n").unwrap();
        let fd =
            rustix::fs::open(&name, rustix::fs::OFlags::RDONLY, rustix::fs::Mode::empty()).unwrap();
        std::fs::hard_link(&name, dir.path().join("alias")).unwrap();
        assert_eq!(same_fstab(&fd, &fd), Err(Failure::IdentityMismatch));
        std::fs::remove_file(dir.path().join("alias")).unwrap();
        same_fstab(&fd, &fd).unwrap();
        std::fs::remove_file(name).unwrap();
        assert_eq!(same_fstab(&fd, &fd), Err(Failure::IdentityMismatch));
    }
    #[tokio::test]
    async fn public_storage_collection_requires_root() {
        if !rustix::process::geteuid().is_root() {
            assert!(matches!(
                inspect_storage_inventory().await,
                Err(Failure::Unavailable)
            ));
        }
    }
}
