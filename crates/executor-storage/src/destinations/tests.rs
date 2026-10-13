use super::*;
use crate::processes::{ContainerProcessNamespace, ContainerProcessRoot, RunningContainerProcess};
use limeos_domain::ContainerStorageConsumer;
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    os::unix::fs::{PermissionsExt, symlink},
};
use tempfile::TempDir;

fn limits() -> DestinationLimits {
    DestinationLimits {
        max_destinations: MAX_DESTINATIONS,
        max_retained_fds: MAX_DESTINATIONS,
        max_report_bytes: MAX_REPORT,
    }
}
fn device() -> DeviceNumber {
    DeviceNumber { major: 8, minor: 1 }
}
struct MappedFs {
    mounts: BTreeMap<u64, u64>,
    device: Cell<DeviceNumber>,
    attributes: Cell<usize>,
    wrong_mount: Cell<bool>,
}
impl Fs for MappedFs {
    fn mount_id(&self, fd: &OwnedFd) -> Result<u64> {
        if self.wrong_mount.get() {
            return Ok(9999);
        }
        let stat = rustix::fs::fstat(fd).map_err(|_| Failure::Unavailable)?;
        Ok(*self.mounts.get(&stat.st_ino).unwrap_or(&700))
    }
    fn identity(&self, fd: &OwnedFd) -> Result<DestinationIdentity> {
        self.attributes.set(self.attributes.get() + 1);
        let mut identity = Linux.identity(fd)?;
        // Real inode/type/mode/UID/GID/link checks, explicitly synthetic
        // namespace mount IDs and block devices. No public owner is forged.
        identity.mount_id = self.mount_id(fd)?;
        identity.device = self.device.get();
        Ok(identity)
    }
}
struct Fixture {
    tree: TempDir,
    root: OwnedFd,
    declarations: ContainerStorageInventory,
    processes: RunningContainerProcessSnapshot,
    sources: ContainerSourceSnapshot,
    physical: ContainerDependencySnapshot,
    mounts: Vec<ContainerProcessMount>,
    fs: MappedFs,
}
impl Fixture {
    fn new(file: bool) -> Self {
        Self::with_count(1, file)
    }
    fn with_count(count: usize, file: bool) -> Self {
        let tree = tempfile::tempdir().unwrap();
        let root =
            rustix::fs::open(tree.path(), OFlags::PATH | OFlags::DIRECTORY, Mode::empty()).unwrap();
        let container = ContainerSnapshot {
            resource: format!("container:{}", "a".repeat(64)),
            image: format!("sha256:{}", "b".repeat(64)),
            started_at: "2026-10-06T00:00:00Z".into(),
            running: true,
        };
        let mut declarations = ContainerStorageInventory {
            version: 1,
            engine_id: "fixture-engine".into(),
            observed_at: sources::now().unwrap(),
            containers: vec![ContainerStorageConsumer {
                container: container.clone(),
                mounts: vec![],
            }],
        };
        let mut sources = ContainerSourceSnapshot {
            version: 1,
            inventory_digest: String::new(),
            host_root_mount_id: 10,
            mounts_digest: "c".repeat(64),
            sources: vec![],
            mounts: vec![ContainerSourceMount {
                mount_id: 101,
                device: device(),
                filesystem_root: "/".into(),
                mountpoint: "/mnt/storage".into(),
                filesystem: "ext4".into(),
            }],
        };
        let mut table = Vec::new();
        let mut fs_map = BTreeMap::new();
        for i in 0..count {
            let destination = format!("/data{i}");
            let path = tree.path().join(destination.trim_start_matches('/'));
            if file {
                fs::write(&path, b"contents are never read").unwrap();
            } else {
                fs::create_dir(&path).unwrap();
            }
            let fd = rustix::fs::open(&path, OFlags::PATH, Mode::empty()).unwrap();
            let identity = Linux.identity(&fd).unwrap();
            let id = 700 + i as u64;
            fs_map.insert(identity.inode, id);
            let source_path = format!("/mnt/storage/Media{i}");
            declarations.containers[0]
                .mounts
                .push(ContainerStorageMount {
                    source: ContainerStorageSource::Bind {
                        path: source_path.clone(),
                    },
                    destination: destination.clone(),
                    writable: true,
                });
            sources.sources.push(ContainerSourcePath {
                container: container.resource.clone(),
                destination: destination.clone(),
                declared_path: source_path.clone(),
                resolved_path: source_path,
                identity: ContainerSourceIdentity {
                    device: device(),
                    mount_id: 101,
                    inode: identity.inode,
                    kind: identity.kind,
                },
                potential_mount_ids: vec![101],
            });
            table.push(ContainerProcessMount {
                mount_id: id,
                parent_mount_id: 600,
                device: device(),
                filesystem_root: format!("/Media{i}"),
                mountpoint: destination,
                filesystem: "ext4".into(),
                writable: true,
            });
        }
        let processes = RunningContainerProcessSnapshot {
            version: 1,
            declarations_digest: String::new(),
            bindings_digest: "d".repeat(64),
            host_boot_id: "11111111-1111-1111-1111-111111111111".into(),
            host_root_mount_id: 10,
            host_mount_table_digest: sources.mounts_digest.clone(),
            declarations_observed_at: declarations.observed_at,
            processes: vec![RunningContainerProcess {
                container,
                pid: 200,
                start_ticks: 1000,
                uids: [1000; 4],
                gids: [1000; 4],
                namespace: ContainerProcessNamespace {
                    device: DeviceNumber { major: 0, minor: 4 },
                    inode: 2000,
                },
                root: ContainerProcessRoot {
                    device: device(),
                    inode: 3000,
                    mount_id: 600,
                    mode: 0o040755,
                },
                mount_table_digest: "e".repeat(64),
                mount_table_bytes: 100,
                mount_count: table.len(),
            }],
        };
        let physical = ContainerDependencySnapshot {
            version: 1,
            contract_digest: "f".repeat(64),
            container_inventory_digest: String::new(),
            storage_inventory_digest: "a".repeat(64),
            source_snapshot_digest: String::new(),
            host_root_mount_id: 10,
            mounts_digest: sources.mounts_digest.clone(),
            dependencies: vec![],
        };
        let mut f = Self {
            tree,
            root,
            declarations,
            processes,
            sources,
            physical,
            mounts: table,
            fs: MappedFs {
                mounts: fs_map,
                device: Cell::new(device()),
                attributes: Cell::new(0),
                wrong_mount: Cell::new(false),
            },
        };
        f.rehash();
        f
    }
    fn rehash(&mut self) {
        let digest = processes::digest_json(&self.declarations, INPUT_BYTES).unwrap();
        self.processes.declarations_digest = digest.clone();
        self.sources.inventory_digest = digest.clone();
        self.physical.container_inventory_digest = digest;
        self.physical.source_snapshot_digest =
            processes::digest_json(&self.sources, INPUT_BYTES).unwrap();
    }
    fn select(&self, limits: DestinationLimits) -> Result<Vec<Selection<'_>>> {
        select(
            &self.declarations,
            &self.processes,
            &self.sources,
            &self.physical,
            limits,
            |_| Some(&self.mounts),
            |_| Some(&self.root),
        )
    }
    fn retain(&self) -> Result<Vec<Pinned<'_>>> {
        let deadline = Instant::now() + WORK;
        retain(
            self.select(limits())?,
            &self.fs,
            deadline,
            deadline,
            &|| false,
        )
    }
    fn path(&self, i: usize) -> std::path::PathBuf {
        self.tree.path().join(format!("data{i}"))
    }
    fn check(&self, pinned: &[Pinned<'_>]) -> Result<()> {
        let deadline = Instant::now() + WORK;
        revalidate_paths(pinned, &self.fs, deadline, deadline, &|| false)
    }
    fn report(
        &self,
        pinned: &[Pinned<'_>],
        limits: DestinationLimits,
    ) -> Result<ContainerDestinationSnapshot> {
        snapshot(
            pinned,
            &self.declarations,
            &self.processes,
            &self.physical,
            limits,
        )
    }
}
fn refusal<T>(result: Result<T>, expected: Failure) {
    assert!(
        matches!(result, Err(e) if e == expected),
        "unexpected refusal kind"
    );
}

#[test]
fn directory_and_file_destinations_bind_same_objects_with_independent_mount_ids() {
    for file in [false, true] {
        let f = Fixture::new(file);
        let pinned = f.retain().unwrap();
        let report = f.report(&pinned, limits()).unwrap();
        assert_eq!(report.destinations.len(), 1);
        let row = &report.destinations[0];
        assert_eq!(row.host_source.mount_id, 101);
        assert_eq!(row.identity.mount_id, 700);
        assert_eq!(row.host_source.inode, row.identity.inode);
        assert_eq!(
            row.identity.kind,
            if file {
                ContainerSourceKind::RegularFile
            } else {
                ContainerSourceKind::Directory
            }
        );
        assert_eq!(report.declarations_observed_at, f.declarations.observed_at);
        assert_eq!(
            report.physical_snapshot_digest,
            processes::digest_json(&f.physical, INPUT_BYTES).unwrap()
        );
        f.check(&pinned).unwrap();
    }
}
#[test]
fn named_volume_identity_and_read_only_state_survive_composition() {
    let mut f = Fixture::new(false);
    f.declarations.containers[0].mounts[0].source = ContainerStorageSource::Volume {
        name: "MediaVolume".into(),
        driver: "local".into(),
        path: "/mnt/storage/Media0".into(),
    };
    f.declarations.containers[0].mounts[0].writable = false;
    f.mounts[0].writable = false;
    f.rehash();
    let pinned = f.retain().unwrap();
    let report = f.report(&pinned, limits()).unwrap();
    assert_eq!(
        report.destinations[0].source,
        f.declarations.containers[0].mounts[0].source
    );
    assert!(!report.destinations[0].writable);
}
#[test]
fn different_inode_on_the_same_device_is_not_a_matching_destination() {
    let mut f = Fixture::new(false);
    f.sources.sources[0].identity.inode += 100;
    f.rehash();
    refusal(f.retain(), Failure::IdentityMismatch);
}
#[test]
fn coincident_host_and_process_mount_ids_do_not_replace_inode_proof() {
    let mut f = Fixture::new(false);
    f.sources.sources[0].identity.mount_id = 700;
    f.sources.sources[0].potential_mount_ids = vec![700];
    f.sources.mounts[0].mount_id = 700;
    f.sources.sources[0].identity.inode += 100;
    f.rehash();
    refusal(f.retain(), Failure::IdentityMismatch);
}
#[test]
fn fdinfo_mismatch_refuses_before_inode_attribute_reads() {
    let f = Fixture::new(false);
    f.fs.wrong_mount.set(true);
    refusal(f.retain(), Failure::IdentityMismatch);
    assert_eq!(f.fs.attributes.get(), 0);
}
#[test]
fn wrong_source_kind_refuses_and_special_files_are_not_read() {
    let mut f = Fixture::new(false);
    f.sources.sources[0].identity.kind = ContainerSourceKind::RegularFile;
    f.rehash();
    refusal(f.retain(), Failure::IdentityMismatch);
    for directory in [false, true] {
        let f = Fixture::new(directory); // `true` creates a file.
        if directory {
            fs::remove_file(f.path(0)).unwrap();
        } else {
            fs::remove_dir(f.path(0)).unwrap();
        }
        rustix::fs::mknodat(
            rustix::fs::CWD,
            f.path(0),
            rustix::fs::FileType::Fifo,
            Mode::from_raw_mode(0o600),
            0,
        )
        .unwrap();
        refusal(f.retain(), Failure::UnsafePath);
    }
}
#[test]
fn alias_mount_roots_are_reconstructed_with_literal_component_boundaries() {
    let mut f = Fixture::new(false);
    f.sources.mounts[0].mountpoint = "/Alias".into();
    f.sources.mounts[0].filesystem_root = "/Subtree".into();
    f.sources.sources[0].resolved_path = "/Alias/Media0".into();
    f.mounts[0].filesystem_root = "/Subtree/Media0".into();
    f.rehash();
    f.retain().unwrap();
    f.sources.mounts[0].mountpoint = "/Ali".into();
    f.rehash();
    refusal(f.select(limits()), Failure::IdentityMismatch);
}
#[test]
fn root_mount_and_exact_mount_source_paths_keep_one_canonical_separator() {
    let mut f = Fixture::new(false);
    f.sources.mounts[0].mountpoint = "/".into();
    f.sources.sources[0].resolved_path = "/Media0".into();
    f.rehash();
    f.retain().unwrap();
    f.sources.mounts[0].mountpoint = "/Media0".into();
    f.sources.mounts[0].filesystem_root = "/Media0".into();
    f.rehash();
    f.retain().unwrap();
    f.sources.mounts[0].filesystem_root = "/".into();
    f.mounts[0].filesystem_root = "/".into();
    f.rehash();
    f.retain().unwrap();
}
#[test]
fn case_wrong_filesystem_root_device_and_write_state_refuse() {
    for kind in 0..5 {
        let mut f = Fixture::new(false);
        match kind {
            0 => f.mounts[0].filesystem_root = "/media0".into(),
            1 => f.mounts[0].filesystem_root = "/Other".into(),
            2 => f.mounts[0].device.minor += 1,
            3 => f.mounts[0].writable = false,
            _ => f.mounts[0].filesystem = "xfs".into(),
        }
        refusal(f.select(limits()), Failure::IdentityMismatch);
    }
}
#[test]
fn source_and_process_nested_mounts_and_stacked_selected_mounts_refuse() {
    for kind in 0..3 {
        let mut f = Fixture::new(false);
        match kind {
            0 => f.sources.sources[0].potential_mount_ids.push(102),
            1 => {
                let mut nested = f.mounts[0].clone();
                nested.mount_id += 1;
                nested.mountpoint.push_str("/nested");
                f.mounts.push(nested);
            }
            _ => {
                let mut stacked = f.mounts[0].clone();
                stacked.mount_id += 1;
                f.mounts.push(stacked);
            }
        }
        f.rehash();
        refusal(f.select(limits()), Failure::Unavailable);
    }
}
#[test]
fn nearby_mounts_do_not_become_nested_consumers_by_string_prefix() {
    let mut f = Fixture::new(false);
    let mut other = f.mounts[0].clone();
    other.mount_id += 1;
    other.mountpoint = "/data0-other".into();
    f.mounts.push(other);
    f.retain().unwrap();
}
#[test]
fn unknown_remote_fuse_and_btrfs_sources_refuse_without_destination_reads() {
    for filesystem in ["btrfs", "overlay", "nfs", "fuse.mergerfs", "unknown"] {
        let mut f = Fixture::new(false);
        f.sources.mounts[0].filesystem = filesystem.into();
        f.mounts[0].filesystem = filesystem.into();
        f.rehash();
        refusal(f.select(limits()), Failure::Unavailable);
        assert_eq!(f.fs.attributes.get(), 0);
    }
}
#[test]
fn unsupported_zero_major_block_filesystems_refuse() {
    let mut f = Fixture::new(false);
    f.sources.mounts[0].device.major = 0;
    f.sources.sources[0].identity.device.major = 0;
    f.mounts[0].device.major = 0;
    f.rehash();
    refusal(f.select(limits()), Failure::Unavailable);
}
#[test]
fn explicit_tmpfs_pairs_use_object_identity_without_a_block_uuid_claim() {
    let mut f = Fixture::new(false);
    let number = DeviceNumber {
        major: 0,
        minor: 99,
    };
    f.sources.mounts[0].device = number;
    f.sources.mounts[0].filesystem = "tmpfs".into();
    f.sources.sources[0].identity.device = number;
    f.mounts[0].device = number;
    f.mounts[0].filesystem = "tmpfs".into();
    f.fs.device.set(number);
    f.rehash();
    let pinned = f.retain().unwrap();
    f.check(&pinned).unwrap();
    assert!(f.physical.dependencies.is_empty());
}
#[test]
fn expired_and_future_declarations_refuse_before_destination_access() {
    for offset in [-6, 1] {
        let mut f = Fixture::new(false);
        f.declarations.observed_at = sources::now().unwrap() + offset;
        f.processes.declarations_observed_at = f.declarations.observed_at;
        f.rehash();
        refusal(f.select(limits()), Failure::Conflict);
        assert_eq!(f.fs.attributes.get(), 0);
    }
}
#[test]
fn absent_literal_destination_and_symlink_destination_refuse() {
    let mut f = Fixture::new(false);
    f.mounts[0].mountpoint = "/Data0".into();
    refusal(f.select(limits()), Failure::NotReady);
    let f = Fixture::new(false);
    fs::rename(f.path(0), f.tree.path().join("other")).unwrap();
    symlink("other", f.path(0)).unwrap();
    refusal(f.retain(), Failure::UnsafePath);
}
#[test]
fn ancestor_symlinks_cannot_redirect_a_destination() {
    let mut f = Fixture::new(false);
    fs::create_dir(f.tree.path().join("actual")).unwrap();
    fs::rename(f.path(0), f.tree.path().join("actual/data0")).unwrap();
    symlink("actual", f.tree.path().join("link")).unwrap();
    f.declarations.containers[0].mounts[0].destination = "/link/data0".into();
    f.sources.sources[0].destination = "/link/data0".into();
    f.mounts[0].mountpoint = "/link/data0".into();
    f.rehash();
    refusal(f.retain(), Failure::UnsafePath);
}
#[test]
fn replacing_unlinking_or_changing_mode_invalidates_retained_destinations() {
    for kind in 0..3 {
        let f = Fixture::new(true);
        let pinned = f.retain().unwrap();
        match kind {
            0 => {
                fs::rename(f.path(0), f.tree.path().join("old")).unwrap();
                fs::write(f.path(0), b"replacement").unwrap();
            }
            1 => fs::remove_file(f.path(0)).unwrap(),
            _ => fs::set_permissions(f.path(0), fs::Permissions::from_mode(0o400)).unwrap(),
        }
        assert!(f.check(&pinned).is_err());
    }
}
#[test]
fn file_data_is_never_read_and_mutable_contents_do_not_change_object_identity() {
    let f = Fixture::new(true);
    let pinned = f.retain().unwrap();
    let fd = rustix::io::dup(&pinned[0].fd).unwrap();
    assert!(fs::File::from(fd).read_to_end(&mut Vec::new()).is_err());
    fs::write(f.path(0), b"new mutable file contents").unwrap();
    f.check(&pinned).unwrap();
}
#[test]
fn complete_declaration_source_and_table_identity_mismatches_refuse() {
    for kind in 0..7 {
        let mut f = Fixture::new(false);
        match kind {
            0 => f.processes.declarations_digest = "wrong".into(),
            1 => f.sources.inventory_digest = "wrong".into(),
            2 => f.physical.container_inventory_digest = "wrong".into(),
            3 => f.processes.host_root_mount_id += 1,
            4 => f.processes.host_mount_table_digest = "wrong".into(),
            5 => f.physical.source_snapshot_digest = "wrong".into(),
            _ => f.processes.declarations_observed_at -= 1,
        }
        refusal(f.select(limits()), Failure::IdentityMismatch);
    }
}
#[test]
fn missing_duplicate_or_wrong_source_and_process_membership_refuse() {
    for kind in 0..5 {
        let mut f = Fixture::new(false);
        match kind {
            0 => f.sources.sources.clear(),
            1 => f.sources.sources.push(f.sources.sources[0].clone()),
            2 => f.sources.sources[0].declared_path.push_str("/wrong"),
            3 => f.processes.processes.clear(),
            _ => f.processes.processes[0].container.image = format!("sha256:{}", "c".repeat(64)),
        }
        f.rehash();
        refusal(f.select(limits()), Failure::IdentityMismatch);
    }
}
#[test]
fn stopped_and_mountless_consumers_remain_explicit_without_destination_opens() {
    for running in [false, true] {
        let mut f = Fixture::with_count(if running { 0 } else { 1 }, false);
        f.declarations.containers[0].container.running = running;
        if !running {
            f.processes.processes.clear();
            f.sources.sources[0].potential_mount_ids.push(102);
        }
        f.rehash();
        let pinned = f.retain().unwrap();
        assert!(pinned.is_empty());
        let report = f.report(&pinned, limits()).unwrap();
        assert_eq!(report.uninspected_stopped.len(), usize::from(!running));
        assert_eq!(f.fs.attributes.get(), 0);
    }
}
#[test]
fn exact_destination_and_descriptor_limits_pass_then_whole_excess_refuses() {
    let f = Fixture::with_count(MAX_DESTINATIONS as usize, false);
    let pinned = f.retain().unwrap();
    assert_eq!(pinned.len(), MAX_DESTINATIONS as usize);
    f.report(&pinned, limits()).unwrap();
    refusal(
        f.select(DestinationLimits {
            max_destinations: MAX_DESTINATIONS - 1,
            ..limits()
        }),
        Failure::Unavailable,
    );
    refusal(
        f.select(DestinationLimits {
            max_retained_fds: MAX_DESTINATIONS - 1,
            ..limits()
        }),
        Failure::Unavailable,
    );
    let f = Fixture::with_count(MAX_DESTINATIONS as usize + 1, false);
    refusal(f.select(limits()), Failure::Unavailable);
}
#[test]
fn report_bound_is_counted_before_cloning_and_preserves_exact_wire_size() {
    let f = Fixture::new(false);
    let pinned = f.retain().unwrap();
    let mut exact = limits();
    for _ in 0..4 {
        exact.max_report_bytes = serde_json::to_vec(&f.report(&pinned, exact).unwrap())
            .unwrap()
            .len() as u32;
    }
    let report = f.report(&pinned, exact).unwrap();
    assert_eq!(
        serde_json::to_vec(&report).unwrap().len(),
        exact.max_report_bytes as usize
    );
    refusal(
        f.report(
            &pinned,
            DestinationLimits {
                max_report_bytes: exact.max_report_bytes - 1,
                ..exact
            },
        ),
        Failure::Unavailable,
    );
}
#[test]
fn invalid_limits_cancellation_and_expired_work_refuse() {
    for bad in [
        DestinationLimits {
            max_destinations: 0,
            ..limits()
        },
        DestinationLimits {
            max_destinations: MAX_DESTINATIONS + 1,
            ..limits()
        },
        DestinationLimits {
            max_retained_fds: 0,
            ..limits()
        },
        DestinationLimits {
            max_retained_fds: MAX_DESTINATIONS + 1,
            ..limits()
        },
        DestinationLimits {
            max_report_bytes: 0,
            ..limits()
        },
        DestinationLimits {
            max_report_bytes: MAX_REPORT + 1,
            ..limits()
        },
    ] {
        refusal(bad.validate(), Failure::InvalidPlan);
    }
    let f = Fixture::new(false);
    let deadline = Instant::now() + WORK;
    refusal(
        retain(
            f.select(limits()).unwrap(),
            &f.fs,
            deadline,
            deadline,
            &|| true,
        ),
        Failure::Conflict,
    );
    let past = Instant::now() - Duration::from_secs(1);
    refusal(
        retain(f.select(limits()).unwrap(), &f.fs, past, deadline, &|| {
            false
        }),
        Failure::TimedOut,
    );
    let pinned = f.retain().unwrap();
    refusal(
        revalidate_paths(&pinned, &f.fs, deadline, past, &|| false),
        Failure::TimedOut,
    );
}
#[test]
fn failed_reentrant_or_cancelled_validation_never_restores_ready_state() {
    let state = Validation(Cell::new(State::Ready));
    state.begin().unwrap();
    refusal(state.begin(), Failure::Conflict);
    refusal(state.finish(Ok(())), Failure::Conflict);
    refusal(state.begin(), Failure::Conflict);
    let state = Validation(Cell::new(State::Ready));
    state.begin().unwrap();
    // A dropped async future leaves Validating; the next attempt refuses.
    refusal(state.begin(), Failure::Conflict);
    let state = Validation(Cell::new(State::Ready));
    state.begin().unwrap();
    refusal(state.finish(Err(Failure::TimedOut)), Failure::TimedOut);
    refusal(state.begin(), Failure::Conflict);
}
#[test]
fn ordering_is_stable_and_report_rows_preserve_full_container_source_facts() {
    let mut f = Fixture::with_count(3, false);
    f.declarations.containers[0].mounts.reverse();
    f.sources.sources.reverse();
    f.rehash();
    let pinned = f.retain().unwrap();
    let report = f.report(&pinned, limits()).unwrap();
    assert_eq!(
        report
            .destinations
            .iter()
            .map(|r| r.destination.as_str())
            .collect::<Vec<_>>(),
        ["/data0", "/data1", "/data2"]
    );
    for row in &report.destinations {
        assert_eq!(row.container, f.declarations.containers[0].container);
    }
}
