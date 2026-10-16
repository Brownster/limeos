//! Retained physical inputs for one mount, never complete consumer authority.
use crate::{
    ContainerDependencyEvidence, DeviceNumber, Failure, Result, StorageInventoryEvidence,
    configuration::{StorageConfigurationEvidence, StoragePlanningConfiguration},
    inventory, namespace, path,
};
use limeos_domain::{
    DirectoryIdentity, StorageBlockDevice, StorageDevice, StorageFilesystem, StorageInventory,
    TargetEvidence,
};
use rustix::{
    fd::{BorrowedFd, OwnedFd},
    fs::{Mode, OFlags, Statx},
};
use sha2::{Digest, Sha256};
use std::{
    cell::Cell,
    collections::BTreeSet,
    fs::File,
    os::{fd::AsFd, unix::fs::FileExt},
    path::Path,
    time::{Duration, Instant},
};

const WINDOW: Duration = Duration::from_secs(4);
const MAX_IMAGE: u64 = 2 * 1024 * 1024;
const MAX_PHYSICAL: usize = 8;
const IMAGE: &str = "/usr/bin/mount";
mod host_consumers;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Object {
    device: (u32, u32),
    inode: u64,
    mount: u64,
    mode: u16,
    uid: u32,
    gid: u32,
    links: u32,
    size: u64,
    changed: (i64, u32),
    modified: (i64, u32),
}
impl From<Statx> for Object {
    fn from(s: Statx) -> Self {
        Self {
            device: (s.stx_dev_major, s.stx_dev_minor),
            inode: s.stx_ino,
            mount: s.stx_mnt_id,
            mode: s.stx_mode,
            uid: s.stx_uid,
            gid: s.stx_gid,
            links: s.stx_nlink,
            size: s.stx_size,
            changed: (s.stx_ctime.tv_sec, s.stx_ctime.tv_nsec),
            modified: (s.stx_mtime.tv_sec, s.stx_mtime.tv_nsec),
        }
    }
}

/// Current namespace and root must equal independently opened PID 1 anchors.
/// No setns, caller namespace ID or systemd setting substitutes for these handles.
struct Host {
    current_namespace: OwnedFd,
    host_namespace: OwnedFd,
    current_root: OwnedFd,
    host_root: OwnedFd,
    namespace_inode: u64,
    root_mount: u64,
}
impl Host {
    fn capture() -> Result<Self> {
        if !rustix::process::geteuid().is_root() {
            return Err(Failure::Unavailable);
        }
        let status = crate::bounded_read(Path::new("/proc/self/status"), 16384)?;
        let mut capabilities = status.lines().filter_map(|s| s.strip_prefix("CapEff:"));
        let effective = capabilities
            .next()
            .and_then(|s| u64::from_str_radix(s.trim(), 16).ok())
            .ok_or(Failure::Unavailable)?;
        // CAP_DAC_READ_SEARCH, CAP_SYS_PTRACE and CAP_SYS_ADMIN. Existing
        // readers lack these privileges; they cannot stand in for this role.
        let required = (1_u64 << 2) | (1_u64 << 19) | (1_u64 << 21);
        if capabilities.next().is_some() || effective & required != required {
            return Err(Failure::Unavailable);
        }
        let open = |name, flags| {
            rustix::fs::open(name, flags | OFlags::CLOEXEC, Mode::empty())
                .map_err(|_| Failure::WrongNamespace)
        };
        let current_namespace = open("/proc/self/ns/mnt", OFlags::RDONLY)?;
        let host_namespace = open("/proc/1/ns/mnt", OFlags::RDONLY)?;
        let a = rustix::fs::fstat(&current_namespace).map_err(|_| Failure::Unavailable)?;
        let b = rustix::fs::fstat(&host_namespace).map_err(|_| Failure::Unavailable)?;
        if (a.st_dev, a.st_ino) != (b.st_dev, b.st_ino) || a.st_ino == 0 {
            return Err(Failure::WrongNamespace);
        }
        let current_root = path::open(Path::new("/"), true, true)?;
        let host_root = open("/proc/1/root", OFlags::PATH | OFlags::DIRECTORY)?;
        let root = path::stat(&current_root)?;
        let host = path::stat(&host_root)?;
        if (
            root.stx_dev_major,
            root.stx_dev_minor,
            root.stx_ino,
            root.stx_mnt_id,
        ) != (
            host.stx_dev_major,
            host.stx_dev_minor,
            host.stx_ino,
            host.stx_mnt_id,
        ) || namespace()? != root.stx_mnt_id
        {
            return Err(Failure::WrongNamespace);
        }
        Ok(Self {
            current_namespace,
            host_namespace,
            current_root,
            host_root,
            namespace_inode: a.st_ino,
            root_mount: root.stx_mnt_id,
        })
    }
    fn revalidate(&self) -> Result<()> {
        let fresh = Self::capture()?;
        for (held, new) in [
            (&self.current_namespace, &fresh.current_namespace),
            (&self.host_namespace, &fresh.host_namespace),
            (&self.current_root, &fresh.current_root),
            (&self.host_root, &fresh.host_root),
        ] {
            let a = rustix::fs::fstat(held).map_err(|_| Failure::Unavailable)?;
            let b = rustix::fs::fstat(new).map_err(|_| Failure::Unavailable)?;
            if (a.st_dev, a.st_ino) != (b.st_dev, b.st_ino) {
                return Err(Failure::WrongNamespace);
            }
        }
        if fresh.namespace_inode != self.namespace_inode || fresh.root_mount != self.root_mount {
            return Err(Failure::WrongNamespace);
        }
        Ok(())
    }
}

/// Only genuine protected configuration and physical collection create this owner.
/// The aggregate executor separately authenticates the complete consumer inputs.
pub struct MountTarget {
    configuration: StorageConfigurationEvidence,
    physical: ContainerDependencyEvidence,
    selected: StorageDevice,
    target: TargetEvidence,
    physical_devices: Vec<StorageBlockDevice>,
    source: OwnedFd,
    source_number: DeviceNumber,
    parent: OwnedFd,
    directory: OwnedFd,
    program: OwnedFd,
    program_object: Object,
    image_digest: String,
    host: Host,
    consumers: host_consumers::HostConsumers,
    expires: Instant,
    poisoned: Cell<bool>,
}

pub async fn inspect(
    configuration: StorageConfigurationEvidence,
    physical: ContainerDependencyEvidence,
    target_id: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<MountTarget> {
    let expires = Instant::now() + WINDOW;
    configuration.revalidate(cancelled)?;
    physical.revalidate().await?;
    let host = Host::capture()?;
    let contract = &configuration.configuration().contract;
    contract.validate().map_err(|_| Failure::InvalidPlan)?;
    let selected = contract
        .devices
        .iter()
        .find(|d| d.id == target_id)
        .ok_or(Failure::InvalidPlan)?
        .clone();
    if selected.filesystem != StorageFilesystem::Ext4 {
        return Err(Failure::InvalidPlan);
    }
    let storage = physical.retained_inventory();
    let (source_number, physical_devices) = selected_disk(storage, &selected)?;
    no_fstab_consumer(storage.snapshot(), &selected, source_number)?;
    limeos_domain::storage_managed_fstab(contract, 10, storage.snapshot())
        .map_err(|_| Failure::InvalidPlan)?;
    let source = path::open(&storage.topology.path(source_number)?, false, false)?;
    source_identity(&source, source_number)?;
    let (parent, directory, target) = prepared_target(&selected, host.root_mount)?;
    let program = path::open(Path::new(IMAGE), false, true)?;
    let program_object = image_object(&program)?;
    let image_digest = hash_image(&program, &program_object)?;
    let consumers = host_consumers::HostConsumers::capture(
        &selected,
        &physical_devices,
        target.existing.as_ref().ok_or(Failure::NotReady)?,
        cancelled,
    )?;
    let owner = MountTarget {
        configuration,
        physical,
        selected,
        target,
        physical_devices,
        source,
        source_number,
        parent,
        directory,
        program,
        program_object,
        image_digest,
        host,
        consumers,
        expires,
        poisoned: Cell::new(false),
    };
    owner.revalidate(cancelled).await?;
    Ok(owner)
}

impl MountTarget {
    pub fn selected(&self) -> &StorageDevice {
        &self.selected
    }
    pub fn inventory(&self) -> &StorageInventory {
        self.physical.retained_inventory().snapshot()
    }
    pub fn target(&self) -> &TargetEvidence {
        &self.target
    }
    pub fn physical_devices(&self) -> &[StorageBlockDevice] {
        &self.physical_devices
    }
    pub fn configuration(&self) -> &StoragePlanningConfiguration {
        self.configuration.configuration()
    }
    pub fn physical_evidence(&self) -> &ContainerDependencyEvidence {
        &self.physical
    }
    pub fn source_snapshot(&self) -> &crate::ContainerSourceSnapshot {
        self.physical.sources().snapshot()
    }
    pub fn configuration_snapshot(&self) -> &crate::configuration::StorageConfigurationSnapshot {
        self.configuration.snapshot()
    }
    pub fn namespace_inode(&self) -> u64 {
        self.host.namespace_inode
    }
    pub fn image_digest(&self) -> &str {
        &self.image_digest
    }
    pub fn host_consumer_digest(&self) -> &str {
        self.consumers.digest()
    }
    pub(crate) fn source_fd(&self) -> BorrowedFd<'_> {
        self.source.as_fd()
    }
    pub(crate) fn target_fd(&self) -> BorrowedFd<'_> {
        self.directory.as_fd()
    }
    pub(crate) fn program_fd(&self) -> BorrowedFd<'_> {
        self.program.as_fd()
    }
    pub fn invocation(&self) -> Result<crate::effects::mounts::MountInvocation<'_>> {
        crate::effects::mounts::MountInvocation::from_retained(self)
    }
    fn poll(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        if self.poisoned.get() || cancelled() || Instant::now() >= self.expires {
            self.poisoned.set(true);
            return Err(Failure::TimedOut);
        }
        Ok(())
    }
    pub async fn revalidate(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        let result = self.check(cancelled).await;
        if result.is_err() {
            self.poisoned.set(true);
        }
        result
    }
    async fn check(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        self.poll(cancelled)?;
        self.configuration.revalidate(cancelled)?;
        self.host.revalidate()?;
        self.consumers.revalidate(
            &self.selected,
            &self.physical_devices,
            self.target.existing.as_ref().ok_or(Failure::NotReady)?,
            cancelled,
        )?;
        self.physical.revalidate().await?;
        self.poll(cancelled)?;
        let (number, devices) = selected_disk(self.physical.retained_inventory(), &self.selected)?;
        no_fstab_consumer(self.inventory(), &self.selected, number)?;
        if number != self.source_number || devices != self.physical_devices {
            return Err(Failure::IdentityMismatch);
        }
        source_identity(&self.source, number)?;
        let (_, _, target) = prepared_target(&self.selected, self.host.root_mount)?;
        if target != self.target
            || directory_identity(&self.parent)? != self.target.parent
            || Some(directory_identity(&self.directory)?) != self.target.existing
            || image_object(&self.program)? != self.program_object
        {
            return Err(Failure::IdentityMismatch);
        }
        empty(&self.directory)?;
        let fresh_image = path::open(Path::new(IMAGE), false, true)?;
        if image_object(&fresh_image)? != self.program_object
            || hash_image(&self.program, &self.program_object)? != self.image_digest
        {
            return Err(Failure::IdentityMismatch);
        }
        self.host.revalidate()?;
        self.consumers.revalidate(
            &self.selected,
            &self.physical_devices,
            self.target.existing.as_ref().ok_or(Failure::NotReady)?,
            cancelled,
        )?;
        self.configuration.revalidate(cancelled)?;
        self.poll(cancelled)
    }
}

fn selected_disk(
    storage: &StorageInventoryEvidence,
    selected: &StorageDevice,
) -> Result<(DeviceNumber, Vec<StorageBlockDevice>)> {
    let matches: Vec<_> = storage
        .snapshot()
        .devices
        .iter()
        .filter(|d| d.filesystem_uuid == selected.filesystem_uuid)
        .collect();
    if matches.len() != 1 {
        return Err(Failure::IdentityMismatch);
    }
    let actual = matches[0];
    admit_observation(actual, selected)?;
    let number = DeviceNumber {
        major: actual.device.major,
        minor: actual.device.minor,
    };
    let connected = backing_chain(&storage.topology, number)?;
    for n in &connected {
        if storage.snapshot().devices.iter().any(|d| {
            d.device.major == n.major
                && d.device.minor == n.minor
                && (d.boot_backing
                    || d.in_use_as_swap
                    || !d.mounts.is_empty()
                    || (d.device != actual.device))
        }) {
            return Err(Failure::Conflict);
        }
    }
    Ok((number, connected.into_iter().map(inventory::wire).collect()))
}
fn admit_observation(
    actual: &limeos_domain::StorageObservedDevice,
    selected: &StorageDevice,
) -> Result<()> {
    if actual.filesystem != "ext4"
        || !actual.mounts.is_empty()
        || actual.boot_backing
        || actual.in_use_as_swap
        || selected
            .serial
            .as_ref()
            .is_some_and(|s| Some(s) != actual.serial.as_ref())
    {
        return Err(Failure::Conflict);
    }
    Ok(())
}
fn backing_chain(
    topology: &crate::topology::Topology,
    number: DeviceNumber,
) -> Result<BTreeSet<DeviceNumber>> {
    let connected = topology.connected(&BTreeSet::from([number]))?;
    if connected.is_empty() || connected.len() > MAX_PHYSICAL {
        return Err(Failure::Unavailable);
    }
    // A1 admits one independent disk/partition chain. Sibling partitions,
    // holders, RAID/device-mapper and unknown aliases remain unsupported even
    // when they have no probed filesystem row.
    let mut chain = BTreeSet::new();
    let mut cursor = number;
    loop {
        if !chain.insert(cursor) || chain.len() > MAX_PHYSICAL {
            return Err(Failure::Conflict);
        }
        let node = topology.0.get(&cursor).ok_or(Failure::Unavailable)?;
        if node.parents.len() > 1 || node.name.starts_with("dm-") || node.name.starts_with("md") {
            return Err(Failure::Conflict);
        }
        match node.parents.first() {
            Some(parent) => cursor = *parent,
            None => break,
        }
    }
    if connected != chain {
        return Err(Failure::Conflict);
    }
    for n in &connected {
        let node = topology.0.get(n).ok_or(Failure::Unavailable)?;
        if node.name.starts_with("loop") || node.name.starts_with("zram") {
            return Err(Failure::Conflict);
        }
    }
    Ok(connected)
}
fn source_identity(fd: &OwnedFd, number: DeviceNumber) -> Result<()> {
    let s = path::stat(fd)?;
    if s.stx_mode & 0o170000 != 0o060000
        || (s.stx_rdev_major, s.stx_rdev_minor) != (number.major, number.minor)
    {
        return Err(Failure::IdentityMismatch);
    }
    Ok(())
}
fn no_fstab_consumer(
    storage: &StorageInventory,
    selected: &StorageDevice,
    number: DeviceNumber,
) -> Result<()> {
    let overlaps = |a: &str, b: &str| {
        a == b
            || a.strip_prefix(b).is_some_and(|s| s.starts_with('/'))
            || b.strip_prefix(a).is_some_and(|s| s.starts_with('/'))
    };
    if storage.fstab_entries.iter().any(|entry| {
        entry.filesystem_uuid.as_ref() == Some(&selected.filesystem_uuid)
            || entry
                .device
                .as_ref()
                .is_some_and(|d| (d.major, d.minor) == (number.major, number.minor))
            || (entry.mountpoint != "/" && overlaps(&entry.mountpoint, &selected.mountpoint))
    }) {
        return Err(Failure::Conflict);
    }
    Ok(())
}
fn directory_identity(fd: &OwnedFd) -> Result<DirectoryIdentity> {
    let s = path::stat(fd)?;
    if s.stx_mode & 0o170000 != 0o040000 || s.stx_uid != 0 || s.stx_mode & 0o022 != 0 {
        return Err(Failure::UnsafePath);
    }
    Ok(DirectoryIdentity {
        major: s.stx_dev_major,
        minor: s.stx_dev_minor,
        mount_id: s.stx_mnt_id,
        inode: s.stx_ino,
        mode: s.stx_mode,
        uid: s.stx_uid,
        gid: s.stx_gid,
    })
}
fn empty(fd: &OwnedFd) -> Result<()> {
    // O_PATH target handles cannot be passed to getdents. Reopen the same
    // directory for bounded enumeration without following its name again.
    let readable = rustix::fs::openat(
        fd,
        ".",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| Failure::Unavailable)?;
    for entry in rustix::fs::Dir::read_from(&readable).map_err(|_| Failure::Unavailable)? {
        let entry = entry.map_err(|_| Failure::Unavailable)?;
        if !matches!(entry.file_name().to_bytes(), b"." | b"..") {
            return Err(Failure::Conflict);
        }
    }
    Ok(())
}
fn prepared_target(
    selected: &StorageDevice,
    root_mount: u64,
) -> Result<(OwnedFd, OwnedFd, TargetEvidence)> {
    let name = Path::new(&selected.mountpoint);
    let parent = path::open(name.parent().ok_or(Failure::InvalidPlan)?, true, true)?;
    let directory = path::open(name, true, true)?;
    let parent_identity = directory_identity(&parent)?;
    let existing = directory_identity(&directory)?;
    if parent_identity.mount_id != root_mount || existing.mount_id != root_mount {
        return Err(Failure::WrongNamespace);
    }
    empty(&directory)?;
    Ok((
        parent,
        directory,
        TargetEvidence {
            mountpoint: selected.mountpoint.clone(),
            parent: parent_identity,
            existing: Some(existing),
        },
    ))
}
fn image_object(fd: &OwnedFd) -> Result<Object> {
    let s = path::stat(fd)?;
    if s.stx_mode & 0o170000 != 0o100000
        || s.stx_uid != 0
        || s.stx_mode & 0o022 != 0
        || s.stx_mode & 0o111 == 0
        || s.stx_nlink != 1
        || s.stx_size == 0
        || s.stx_size > MAX_IMAGE
    {
        return Err(Failure::UnsafePath);
    }
    Ok(s.into())
}
fn hash_image(fd: &OwnedFd, expected: &Object) -> Result<String> {
    let file = File::from(rustix::io::dup(fd).map_err(|_| Failure::Unavailable)?);
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 8192];
    let mut offset = 0;
    while offset < expected.size {
        let wanted = usize::try_from((expected.size - offset).min(buffer.len() as u64))
            .map_err(|_| Failure::Unavailable)?;
        let n = file
            .read_at(&mut buffer[..wanted], offset)
            .map_err(|_| Failure::Unavailable)?;
        if n == 0 {
            return Err(Failure::IdentityMismatch);
        }
        hash.update(&buffer[..n]);
        offset += n as u64;
    }
    if file
        .read_at(&mut buffer[..1], offset)
        .map_err(|_| Failure::Unavailable)?
        != 0
        || image_object(fd)? != *expected
    {
        return Err(Failure::IdentityMismatch);
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Fresh named-mount observation. This neither reuses the old empty-target
/// predicate nor creates, renews, or reconstructs a launch authority.
pub async fn inspect_mounted(
    configuration: StorageConfigurationEvidence,
    plan: &limeos_domain::StorageMountEffectPlan,
    cancelled: &dyn Fn() -> bool,
) -> Result<limeos_domain::StorageMountedSnapshot> {
    let expires = Instant::now() + WINDOW;
    let poll = || -> Result<()> {
        if cancelled() || Instant::now() >= expires {
            Err(Failure::TimedOut)
        } else {
            Ok(())
        }
    };
    plan.validate(plan.created_at)
        .map_err(|_| Failure::InvalidPlan)?;
    poll()?;
    configuration.revalidate(cancelled)?;
    if !configuration
        .configuration()
        .contract
        .devices
        .iter()
        .any(|d| d == &plan.device)
    {
        return Err(Failure::IdentityMismatch);
    }
    let host = Host::capture()?;
    if host.root_mount != plan.expected.host_root_mount_id {
        return Err(Failure::WrongNamespace);
    }
    let storage =
        crate::inspect_storage_inventory_with_profile(crate::StorageReadProfile::CombinedV1)
            .await?;
    poll()?;
    let mut matches = storage
        .snapshot()
        .devices
        .iter()
        .filter(|d| d.filesystem_uuid == plan.device.filesystem_uuid);
    let actual = matches.next().ok_or(Failure::IdentityMismatch)?;
    if matches.next().is_some()
        || actual.filesystem != "ext4"
        || actual.boot_backing
        || actual.in_use_as_swap
        || plan
            .device
            .serial
            .as_ref()
            .is_some_and(|serial| actual.serial.as_ref() != Some(serial))
        || actual.mounts.len() != 1
    {
        return Err(Failure::Conflict);
    }
    let number = DeviceNumber {
        major: actual.device.major,
        minor: actual.device.minor,
    };
    no_fstab_consumer(storage.snapshot(), &plan.device, number)?;
    let chain = backing_chain(&storage.topology, number)?;
    let physical: Vec<_> = chain.iter().copied().map(inventory::wire).collect();
    if physical != plan.physical_devices
        || storage.snapshot().topology_digest != plan.expected.topology_digest
        || storage.snapshot().fstab_digest != plan.expected.fstab_digest
        || storage.snapshot().devices.iter().any(|d| {
            d.device != actual.device
                && chain
                    .iter()
                    .any(|n| (n.major, n.minor) == (d.device.major, d.device.minor))
        })
    {
        return Err(Failure::IdentityMismatch);
    }
    let mounted = &actual.mounts[0];
    if mounted.mountpoint != plan.device.mountpoint
        || mounted.filesystem != "ext4"
        || mounted.filesystem_root != "/"
        || !mounted.writable
    {
        return Err(Failure::Conflict);
    }
    let parent = path::open(
        Path::new(&plan.device.mountpoint)
            .parent()
            .ok_or(Failure::InvalidPlan)?,
        true,
        true,
    )?;
    if directory_identity(&parent)? != plan.target.parent {
        return Err(Failure::IdentityMismatch);
    }
    let directory = path::open(Path::new(&plan.device.mountpoint), true, false)?;
    let target = mounted_directory_identity(&directory)?;
    if target.mount_id != mounted.mount_id
        || (target.major, target.minor) != (number.major, number.minor)
    {
        return Err(Failure::IdentityMismatch);
    }
    let text = crate::bounded_read(Path::new("/proc/self/mountinfo"), 64 * 1024)?;
    let rows = crate::mounts::parse_with_profile(&text, crate::StorageReadProfile::CombinedV1)?;
    if limeos_identity::digest(&text) != storage.raw_mounts_digest {
        return Err(Failure::IdentityMismatch);
    }
    let matching: Vec<_> = rows
        .iter()
        .filter(|m| m.path == plan.device.mountpoint)
        .collect();
    if matching.len() != 1
        || matching[0].id != target.mount_id
        || matching[0].device != number
        || matching[0].root != "/"
        || matching[0].filesystem != "ext4"
        || !matching[0].writable
    {
        return Err(Failure::Conflict);
    }
    let (nosuid, nodev) = mount_restrictions(&text, target.mount_id)?;
    if !nosuid || !nodev {
        return Err(Failure::Conflict);
    }
    storage.revalidate().await?;
    configuration.revalidate(cancelled)?;
    host.revalidate()?;
    let named = path::open(Path::new(&plan.device.mountpoint), true, false)?;
    if mounted_directory_identity(&named)? != target {
        return Err(Failure::IdentityMismatch);
    }
    poll()?;
    Ok(limeos_domain::StorageMountedSnapshot {
        filesystem_uuid: actual.filesystem_uuid.clone(),
        device: actual.device,
        mountpoint: mounted.mountpoint.clone(),
        filesystem: mounted.filesystem.clone(),
        filesystem_root: mounted.filesystem_root.clone(),
        writable: mounted.writable,
        nosuid,
        nodev,
        mount_id: mounted.mount_id,
        host_root_mount_id: host.root_mount,
        target,
    })
}
fn mount_restrictions(text: &str, id: u64) -> Result<(bool, bool)> {
    let mut found = None;
    for line in text.lines() {
        let (left, _) = line.split_once(" - ").ok_or(Failure::Unavailable)?;
        let fields = crate::mounts::fixed_fields::<6>(left, true)?;
        if fields[0].parse::<u64>().map_err(|_| Failure::Unavailable)? != id {
            continue;
        }
        if found.is_some() {
            return Err(Failure::AmbiguousIdentity);
        }
        let options: BTreeSet<_> = fields[5].split(',').collect();
        if options.contains("nosuid") && options.contains("suid")
            || options.contains("nodev") && options.contains("dev")
        {
            return Err(Failure::Conflict);
        }
        found = Some((options.contains("nosuid"), options.contains("nodev")));
    }
    found.ok_or(Failure::IdentityMismatch)
}

fn mounted_directory_identity(fd: &OwnedFd) -> Result<DirectoryIdentity> {
    let s = path::stat(fd)?;
    observed_directory(DirectoryIdentity {
        major: s.stx_dev_major,
        minor: s.stx_dev_minor,
        mount_id: s.stx_mnt_id,
        inode: s.stx_ino,
        mode: s.stx_mode,
        uid: s.stx_uid,
        gid: s.stx_gid,
    })
}
fn observed_directory(identity: DirectoryIdentity) -> Result<DirectoryIdentity> {
    if identity.mode & 0o170000 != 0o040000 {
        return Err(Failure::IdentityMismatch);
    }
    Ok(identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topology::{Node, Topology};
    use std::collections::BTreeMap;
    fn number(minor: u32) -> DeviceNumber {
        DeviceNumber { major: 8, minor }
    }
    fn disk(name: &str, parents: &[DeviceNumber]) -> Node {
        Node {
            name: name.into(),
            parents: parents.iter().copied().collect(),
            serial: None,
        }
    }
    #[test]
    fn one_separate_disk_chain_is_supported_but_siblings_and_loop_backing_refuse() {
        let mut topology = Topology(BTreeMap::from([
            (number(0), disk("sdb", &[])),
            (number(1), disk("sdb1", &[number(0)])),
        ]));
        assert_eq!(
            backing_chain(&topology, number(1)).unwrap(),
            BTreeSet::from([number(0), number(1)])
        );
        topology.0.insert(number(2), disk("sdb2", &[number(0)]));
        assert_eq!(backing_chain(&topology, number(1)), Err(Failure::Conflict));
        topology.0.remove(&number(2));
        topology.0.get_mut(&number(0)).unwrap().name = "loop0".into();
        assert_eq!(backing_chain(&topology, number(1)), Err(Failure::Conflict));
    }
    #[test]
    fn selected_boot_and_swap_devices_refuse_even_with_matching_uuid() {
        let selected = StorageDevice {
            id: "data".into(),
            role: limeos_domain::StorageRole::Data,
            filesystem_uuid: "uuid".into(),
            filesystem: StorageFilesystem::Ext4,
            mountpoint: "/mnt/data".into(),
            serial: None,
        };
        let mut actual = limeos_domain::StorageObservedDevice {
            device: inventory::wire(number(1)),
            filesystem_uuid: "uuid".into(),
            filesystem: "ext4".into(),
            serial: None,
            boot_backing: false,
            in_use_as_swap: false,
            mounts: Vec::new(),
        };
        assert_eq!(admit_observation(&actual, &selected), Ok(()));
        actual.boot_backing = true;
        assert_eq!(
            admit_observation(&actual, &selected),
            Err(Failure::Conflict)
        );
        actual.boot_backing = false;
        actual.in_use_as_swap = true;
        assert_eq!(
            admit_observation(&actual, &selected),
            Err(Failure::Conflict)
        );
    }
    #[test]
    fn fresh_mount_flags_are_read_independently_and_conflicts_refuse() {
        let row = |options: &str| format!("42 1 8:1 / /mnt/data {options} - ext4 /dev/sdb1 rw\n");
        assert_eq!(
            mount_restrictions(&row("rw,nosuid,nodev"), 42),
            Ok((true, true))
        );
        assert_eq!(mount_restrictions(&row("rw,nodev"), 42), Ok((false, true)));
        assert_eq!(mount_restrictions(&row("rw,nosuid"), 42), Ok((true, false)));
        assert_eq!(
            mount_restrictions(&row("rw,nosuid,suid,nodev"), 42),
            Err(Failure::Conflict)
        );
    }
    #[test]
    fn mounted_filesystem_root_preserves_existing_owner_and_mode() {
        let identity = DirectoryIdentity {
            major: 8,
            minor: 1,
            mount_id: 42,
            inode: 2,
            mode: 0o40777,
            uid: 1000,
            gid: 1000,
        };
        assert_eq!(observed_directory(identity.clone()), Ok(identity));
    }
}
