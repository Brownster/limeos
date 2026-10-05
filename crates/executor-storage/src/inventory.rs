//! Direct, bounded evidence for existing-filesystem guided assignments.
use crate::*;
use limeos_domain::{
    StorageBlockDevice, StorageInventory, StorageObservedDevice, StorageObservedMount,
};
use std::collections::BTreeSet;

fn digest<T: Serialize>(value: &T) -> Result<String> {
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

/// The caller enforces a global deadline and single-flight admission. No cache,
/// udev alias, previous success or caller-supplied filesystem path is authority.
pub async fn inventory() -> Result<StorageInventory> {
    if !rustix::process::geteuid().is_root() {
        return Err(Failure::Unavailable);
    }
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
        let fs = probe(&fd).await?;
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
    // Re-probe every retained raw descriptor: an unmounted filesystem can be
    // relabeled without changing sysfs, mountinfo or udev.
    for (fd, expected) in &handles {
        if &probe(fd).await? != expected {
            return Err(Failure::IdentityMismatch);
        }
    }
    if swaps::Snapshot::collect()? != swaps
        || namespace()? != ns
        || topology::Topology::collect()? != topology
        || mounts::parse(&bounded_read(
            Path::new("/proc/self/mountinfo"),
            1024 * 1024,
        )?)? != mounts
        || fstab::read()?.1 != fstab_text
        || path::stat(&fstab_handle)?.stx_nlink != 1
    {
        return Err(Failure::IdentityMismatch);
    }
    let topology_rows: Vec<_> = topology.0.iter().collect();
    let value = StorageInventory {
        host_root_mount_id: ns,
        topology_digest: digest(&(topology_rows, swaps))?,
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
    Ok(value)
}
