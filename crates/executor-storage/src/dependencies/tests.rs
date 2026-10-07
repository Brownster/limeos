use super::*;
use crate::{
    ContainerSourceIdentity, ContainerSourceKind, ContainerSourceMount, ContainerSourcePath,
};
use limeos_domain::{StorageFilesystem, StorageObservedDevice, StorageObservedMount};

fn n(minor: u32) -> DeviceNumber {
    DeviceNumber { major: 8, minor }
}
struct Facts {
    contract: StorageContract,
    storage: StorageInventory,
    topology: Topology,
    table: Vec<Mount>,
    sources: ContainerSourceSnapshot,
}
impl Facts {
    fn new() -> Self {
        let fixtures: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/storage-contracts.json"
        ))
        .unwrap();
        let contract = serde_json::from_value(fixtures["cases"][0]["contract"].clone()).unwrap();
        let table = vec![
            Mount {
                id: 21,
                device: n(32),
                root: "/".into(),
                path: "/".into(),
                filesystem: "ext4".into(),
                writable: true,
            },
            Mount {
                id: 31,
                device: n(1),
                root: "/".into(),
                path: "/mnt/storage".into(),
                filesystem: "ext4".into(),
                writable: true,
            },
            Mount {
                id: 32,
                device: n(1),
                root: "/media".into(),
                path: "/alias Space".into(),
                filesystem: "ext4".into(),
                writable: true,
            },
            Mount {
                id: 33,
                device: n(2),
                root: "/".into(),
                path: "/sibling".into(),
                filesystem: "ext4".into(),
                writable: true,
            },
            Mount {
                id: 34,
                device: n(17),
                root: "/".into(),
                path: "/independent".into(),
                filesystem: "ext4".into(),
                writable: true,
            },
            Mount {
                id: 35,
                device: DeviceNumber { major: 0, minor: 1 },
                root: "/".into(),
                path: "/memory".into(),
                filesystem: "tmpfs".into(),
                writable: true,
            },
        ];
        let topology = Topology(
            [
                (0, None),
                (1, Some(0)),
                (2, Some(0)),
                (16, None),
                (17, Some(16)),
                (32, None),
            ]
            .into_iter()
            .map(|(minor, parent)| {
                (
                    n(minor),
                    crate::topology::Node {
                        name: format!("fixture{minor}"),
                        parents: parent.map(n).into_iter().collect(),
                        serial: None,
                    },
                )
            })
            .collect(),
        );
        let devices = [
            (32, "root", true),
            (1, "uuid-storage", false),
            (2, "sibling", false),
            (17, "independent", false),
        ]
        .into_iter()
        .map(|(minor, uuid, boot)| StorageObservedDevice {
            device: crate::inventory::wire(n(minor)),
            filesystem_uuid: uuid.into(),
            filesystem: "ext4".into(),
            serial: None,
            boot_backing: boot,
            in_use_as_swap: false,
            mounts: table
                .iter()
                .filter(|m| m.device == n(minor))
                .map(|m| StorageObservedMount {
                    mountpoint: m.path.clone(),
                    mount_id: m.id,
                    filesystem_root: m.root.clone(),
                    filesystem: m.filesystem.clone(),
                    writable: m.writable,
                })
                .collect(),
        })
        .collect();
        let storage = StorageInventory {
            host_root_mount_id: 21,
            topology_digest: "a".repeat(64),
            mounts_digest: digest(&table).unwrap(),
            fstab_digest: "b".repeat(64),
            fstab_entries: vec![],
            devices,
        };
        let sources = ContainerSourceSnapshot {
            version: 1,
            inventory_digest: "c".repeat(64),
            host_root_mount_id: 21,
            mounts_digest: "d".repeat(64),
            sources: vec![],
            mounts: vec![],
        };
        Self {
            contract,
            storage,
            topology,
            table,
            sources,
        }
    }
    fn add_source(
        &mut self,
        identity_mount: u64,
        candidates: &[u64],
        declared: &str,
        resolved: &str,
    ) {
        for id in candidates {
            if !self.sources.mounts.iter().any(|m| m.mount_id == *id) {
                let m = self.table.iter().find(|m| m.id == *id).unwrap();
                self.sources.mounts.push(ContainerSourceMount {
                    mount_id: m.id,
                    device: m.device,
                    filesystem_root: m.root.clone(),
                    mountpoint: m.path.clone(),
                    filesystem: m.filesystem.clone(),
                });
            }
        }
        let device = self
            .table
            .iter()
            .find(|m| m.id == identity_mount)
            .unwrap()
            .device;
        let index = self.sources.sources.len();
        self.sources.sources.push(ContainerSourcePath {
            container: format!("container:{:064x}", index / 64 + 1),
            destination: format!("/source{index}"),
            declared_path: declared.into(),
            resolved_path: resolved.into(),
            identity: ContainerSourceIdentity {
                device,
                mount_id: identity_mount,
                inode: 100 + index as u64,
                kind: ContainerSourceKind::Directory,
            },
            potential_mount_ids: candidates.to_vec(),
        });
    }
    fn bind(&self) -> Result<ContainerDependencySnapshot> {
        bind(
            &self.contract,
            &self.storage,
            &self.topology,
            &self.table,
            &"d".repeat(64),
            &self.sources,
        )
    }
    fn assigned(&mut self) -> &mut StorageObservedDevice {
        self.storage
            .devices
            .iter_mut()
            .find(|d| d.filesystem_uuid == "uuid-storage")
            .unwrap()
    }
    fn shared_pool(&mut self) {
        self.contract.profile = limeos_domain::StorageProfile::ProtectedPool;
        let template = self.contract.devices[0].clone();
        self.contract.devices.clear();
        // Fifteen distinct partition filesystems sharing one physical disk.
        for minor in 1..=15 {
            let mut assignment = template.clone();
            assignment.id = format!("partition{minor}");
            assignment.filesystem_uuid = format!("partition-{minor}");
            assignment.mountpoint = format!("/mnt/partition{minor}");
            assignment.role = if minor == 15 {
                limeos_domain::StorageRole::Parity
            } else {
                limeos_domain::StorageRole::Data
            };
            self.contract.devices.push(assignment);
            self.topology
                .0
                .entry(n(minor))
                .or_insert_with(|| crate::topology::Node {
                    name: format!("fixture{minor}"),
                    parents: BTreeSet::from([n(0)]),
                    serial: None,
                });
            if let Some(d) = self
                .storage
                .devices
                .iter_mut()
                .find(|d| d.device == crate::inventory::wire(n(minor)))
            {
                d.filesystem_uuid = format!("partition-{minor}");
            } else {
                self.storage.devices.push(StorageObservedDevice {
                    device: crate::inventory::wire(n(minor)),
                    filesystem_uuid: format!("partition-{minor}"),
                    filesystem: "ext4".into(),
                    serial: None,
                    boot_backing: false,
                    in_use_as_swap: false,
                    mounts: vec![],
                });
            }
        }
        self.contract.validate().unwrap();
        self.storage.validate().unwrap();
    }
}

#[test]
fn alias_and_symlink_sources_bind_uuid_without_lexical_overlap() {
    let mut f = Facts::new();
    f.add_source(32, &[32], "/different/link", "/alias Space/Media");
    let result = f.bind().unwrap();
    let dependency = &result.dependencies[0];
    assert_eq!(dependency.mountpoint, "/mnt/storage");
    assert_eq!(dependency.consumers.len(), 1);
    assert_eq!(
        dependency.consumers[0].matches,
        [ContainerFilesystemMatch {
            mount_id: 32,
            filesystem_uuid: "uuid-storage".into(),
            relation: ContainerFilesystemRelation::SameFilesystem,
        }]
    );
    assert_eq!(result.contract_digest, digest(&f.contract).unwrap());
    assert_eq!(result.storage_inventory_digest, digest(&f.storage).unwrap());
    assert_eq!(result.source_snapshot_digest, digest(&f.sources).unwrap());
    assert_eq!(
        result.container_inventory_digest,
        f.sources.inventory_digest
    );
    assert_ne!(result.mounts_digest, f.storage.mounts_digest);
}
#[test]
fn sibling_partitions_are_conservative_shared_backing_consumers() {
    let mut f = Facts::new();
    f.add_source(33, &[33], "/sibling/media", "/sibling/media");
    let result = f.bind().unwrap();
    assert_eq!(
        result.dependencies[0].consumers[0].matches,
        [ContainerFilesystemMatch {
            mount_id: 33,
            filesystem_uuid: "sibling".into(),
            relation: ContainerFilesystemRelation::SharedBacking,
        }]
    );
}
#[test]
fn independent_disks_do_not_match_even_with_overlapping_declared_names() {
    let mut f = Facts::new();
    f.add_source(34, &[34], "/mnt/storage/media", "/independent/media");
    assert!(f.bind().unwrap().dependencies[0].consumers.is_empty());
}
#[test]
fn root_ancestor_candidate_includes_nested_storage_without_matching_boot_disk() {
    let mut f = Facts::new();
    f.add_source(21, &[21, 31, 32, 34], "/", "/");
    let result = f.bind().unwrap();
    assert_eq!(
        result.dependencies[0].consumers[0]
            .matches
            .iter()
            .map(|m| m.mount_id)
            .collect::<Vec<_>>(),
        [31, 32]
    );
}
#[test]
fn tmpfs_has_no_block_backing_but_its_nested_candidates_still_count() {
    let mut f = Facts::new();
    f.table.iter_mut().find(|m| m.id == 31).unwrap().path = "/memory/nested".into();
    f.storage.mounts_digest = digest(&f.table).unwrap();
    f.add_source(35, &[35, 31], "/memory", "/memory");
    assert_eq!(
        f.bind().unwrap().dependencies[0].consumers[0].matches[0].mount_id,
        31
    );
    f.sources.sources[0].potential_mount_ids = vec![35];
    assert!(f.bind().unwrap().dependencies[0].consumers.is_empty());
}
#[test]
fn duplicate_uuid_on_unmounted_device_refuses_the_whole_report() {
    let mut f = Facts::new();
    let mut clone = f.assigned().clone();
    clone.device = crate::inventory::wire(n(17));
    clone.mounts.clear();
    f.storage.devices.retain(|d| d.device != clone.device);
    f.storage.devices.push(clone);
    assert_eq!(f.bind(), Err(Failure::AmbiguousIdentity));
}
#[test]
fn missing_uuid_and_unprobed_source_refuse_instead_of_returning_empty() {
    let mut f = Facts::new();
    f.contract.devices[0].filesystem_uuid = "not-present".into();
    assert_eq!(f.bind(), Err(Failure::NotReady));
    f.contract.devices[0].filesystem_uuid = "uuid-storage".into();
    f.add_source(34, &[34], "/independent", "/independent");
    f.storage
        .devices
        .retain(|d| d.device != crate::inventory::wire(n(17)));
    assert_eq!(f.bind(), Err(Failure::Unavailable));
}
#[test]
fn raw_mount_digest_namespace_and_selected_table_digest_must_all_match() {
    let mut f = Facts::new();
    f.sources.mounts_digest = "e".repeat(64);
    assert_eq!(f.bind(), Err(Failure::IdentityMismatch));
    f.sources.mounts_digest = "d".repeat(64);
    f.sources.host_root_mount_id += 1;
    assert_eq!(f.bind(), Err(Failure::IdentityMismatch));
    f.sources.host_root_mount_id -= 1;
    f.table[0].writable = false;
    assert_eq!(f.bind(), Err(Failure::IdentityMismatch));
}
#[test]
fn altered_alias_root_device_type_and_missing_mount_ids_are_refused() {
    let mut f = Facts::new();
    f.add_source(32, &[32], "/alias Space", "/alias Space");
    let original = f.sources.clone();
    for changed in 0..4 {
        f.sources = original.clone();
        match changed {
            0 => f.sources.mounts[0].filesystem_root = "/other".into(),
            1 => f.sources.mounts[0].device = n(17),
            2 => f.sources.mounts[0].filesystem = "xfs".into(),
            _ => f.sources.mounts[0].mount_id = 999,
        }
        assert_eq!(f.bind(), Err(Failure::IdentityMismatch));
    }
    f.sources = original;
    f.sources.sources[0].potential_mount_ids.push(999);
    assert_eq!(f.bind(), Err(Failure::Unavailable));
}
#[test]
fn unsupported_or_misidentified_nested_filesystem_refuses_complete_read() {
    for filesystem in ["btrfs", "autofs", "fuse.mergerfs", "nfs", "overlay", "xfs"] {
        let mut f = Facts::new();
        f.table.iter_mut().find(|m| m.id == 34).unwrap().filesystem = filesystem.into();
        f.storage.mounts_digest = digest(&f.table).unwrap();
        f.add_source(31, &[31, 34], "/mnt/storage", "/mnt/storage");
        assert_eq!(f.bind(), Err(Failure::Unavailable), "{filesystem}");
    }
}
#[test]
fn assignment_boot_swap_serial_and_filesystem_mismatch_are_refused() {
    let mut f = Facts::new();
    f.assigned().boot_backing = true;
    assert_eq!(f.bind(), Err(Failure::BootDevice));
    f.assigned().boot_backing = false;
    f.assigned().in_use_as_swap = true;
    assert_eq!(f.bind(), Err(Failure::ActiveSwap));
    f.assigned().in_use_as_swap = false;
    f.contract.devices[0].serial = Some("expected".into());
    assert_eq!(f.bind(), Err(Failure::IdentityMismatch));
    f.assigned().serial = Some("expected".into());
    f.bind().unwrap();
    f.contract.devices[0].filesystem = StorageFilesystem::Xfs;
    assert_eq!(f.bind(), Err(Failure::IdentityMismatch));
    f.contract.devices[0].filesystem = StorageFilesystem::Btrfs;
    f.assigned().filesystem = "btrfs".into();
    assert_eq!(f.bind(), Err(Failure::IdentityMismatch));
}
#[test]
fn ntfs_kernel_driver_matches_direct_probe_but_tmpfs_requires_nonblock_device() {
    assert!(mounted_filesystem_matches("ntfs", "ntfs3"));
    assert!(mounted_filesystem_matches("ntfs3", "ntfs3"));
    assert!(!mounted_filesystem_matches("ext4", "ntfs3"));
    let mut f = Facts::new();
    f.table.iter_mut().find(|m| m.id == 35).unwrap().device = n(17);
    f.storage.mounts_digest = digest(&f.table).unwrap();
    f.add_source(35, &[35], "/memory", "/memory");
    assert_eq!(f.bind(), Err(Failure::Unavailable));
}
#[test]
fn inconsistent_source_identity_and_repeated_candidates_are_refused() {
    let mut f = Facts::new();
    f.add_source(31, &[31], "/mnt/storage", "/mnt/storage");
    f.sources.sources[0].identity.device = n(17);
    assert_eq!(f.bind(), Err(Failure::IdentityMismatch));
    f.sources.sources[0].identity.device = n(1);
    f.sources.sources[0].potential_mount_ids = vec![32];
    assert_eq!(f.bind(), Err(Failure::IdentityMismatch));
    f.sources.sources[0].potential_mount_ids = vec![31, 31];
    assert_eq!(f.bind(), Err(Failure::IdentityMismatch));
}
#[test]
fn device_missing_from_topology_is_unknown_not_an_independent_disk() {
    let mut f = Facts::new();
    f.topology.0.remove(&n(17));
    assert_eq!(f.bind(), Err(Failure::Unavailable));
}
#[test]
fn dependency_match_limit_refuses_instead_of_clipping() {
    let mut f = Facts::new();
    f.shared_pool();
    for _ in 0..35 {
        f.add_source(32, &[32], "/alias Space", "/alias Space");
    }
    assert_eq!(f.sources.sources.len() * f.contract.devices.len(), 525);
    assert!(serde_json::to_vec(&f.sources).unwrap().len() < MAX_REPORT_BYTES);
    assert_eq!(f.bind(), Err(Failure::Unavailable));
}
#[test]
fn serialized_report_limit_refuses_below_the_match_count_limit() {
    let mut f = Facts::new();
    f.shared_pool();
    for _ in 0..32 {
        f.add_source(32, &[32], "/alias Space", "/alias Space");
    }
    assert_eq!(f.sources.sources.len() * f.contract.devices.len(), 480);
    assert!(serde_json::to_vec(&f.sources).unwrap().len() < MAX_REPORT_BYTES);
    assert_eq!(f.bind(), Err(Failure::Unavailable));
}
#[test]
fn empty_sources_remain_explicit_with_all_contract_and_evidence_digests() {
    let f = Facts::new();
    let result = f.bind().unwrap();
    assert_eq!(result.dependencies.len(), 1);
    assert!(result.dependencies[0].consumers.is_empty());
    assert_eq!(
        result.container_inventory_digest,
        f.sources.inventory_digest
    );
    assert_eq!(result.contract_digest, digest(&f.contract).unwrap());
    assert_eq!(check_deadline(Instant::now()), Err(Failure::TimedOut));
}

#[tokio::test]
async fn invalid_or_expired_input_is_refused_before_privileged_collection() {
    let f = Facts::new();
    let mut containers = ContainerStorageInventory {
        version: 1,
        engine_id: "fixture".into(),
        observed_at: 0,
        containers: vec![],
    };
    assert!(matches!(
        inspect_container_dependencies(&f.contract, &containers).await,
        Err(Failure::Conflict)
    ));
    containers.observed_at = crate::sources::now().unwrap();
    containers.version = 99;
    assert!(matches!(
        inspect_container_dependencies(&f.contract, &containers).await,
        Err(Failure::InvalidPlan)
    ));
    containers.version = 1;
    if !rustix::process::geteuid().is_root() {
        assert!(matches!(
            inspect_container_dependencies(&f.contract, &containers).await,
            Err(Failure::Unavailable)
        ));
    }
}
