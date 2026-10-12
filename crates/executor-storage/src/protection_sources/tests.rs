use super::*;
use limeos_domain::{StorageObservedDevice, StorageObservedMount};
use std::{cell::Cell, fs, os::unix::fs::symlink};
use tempfile::TempDir;

fn limits() -> SourceLimits {
    SourceLimits {
        max_paths: 128,
        max_retained_fds: 128,
        max_report_bytes: 65536,
    }
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(include_str!(
        "../../../../tests/fixtures/pool-protection/layouts.json"
    ))
    .unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
struct Fixture {
    tree: TempDir,
    preview: SnapraidPreview,
    inventory: StorageInventory,
    table: Vec<Mount>,
}
impl Fixture {
    fn new() -> Self {
        Self::from_case(&cases()[2])
    }
    fn from_case(case: &serde_json::Value) -> Self {
        let tree = tempfile::tempdir().unwrap();
        let config: SnapraidConfig = serde_json::from_value(case["snapraid"].clone()).unwrap();
        let pools: PoolsConfig = serde_json::from_value(case["pools"].clone()).unwrap();
        let contract: StorageContract = serde_json::from_value(case["contract"].clone()).unwrap();
        let preview = config.plan(&pools, &contract).unwrap();
        let devices: Vec<_> = contract
            .devices
            .iter()
            .enumerate()
            .map(|(i, d)| StorageObservedDevice {
                device: StorageBlockDevice {
                    major: 8,
                    minor: i as u32 + 1,
                },
                filesystem_uuid: d.filesystem_uuid.clone(),
                filesystem: "ext4".into(),
                serial: d.serial.clone(),
                boot_backing: false,
                in_use_as_swap: false,
                mounts: vec![StorageObservedMount {
                    mountpoint: d.mountpoint.clone(),
                    mount_id: i as u64 + 10,
                    filesystem_root: "/".into(),
                    filesystem: "ext4".into(),
                    writable: true,
                }],
            })
            .collect();
        let table = devices
            .iter()
            .flat_map(|d| {
                d.mounts.iter().map(move |m| Mount {
                    id: m.mount_id,
                    device: crate::DeviceNumber {
                        major: d.device.major,
                        minor: d.device.minor,
                    },
                    root: m.filesystem_root.clone(),
                    path: m.mountpoint.clone(),
                    filesystem: m.filesystem.clone(),
                    writable: m.writable,
                })
            })
            .collect();
        for r in &preview.requirements {
            let path = tree.path().join(r.path.trim_start_matches('/'));
            if matches!(
                r.kind,
                ProtectionPathKind::Source | ProtectionPathKind::Data
            ) {
                fs::create_dir_all(path).unwrap();
            }
        }
        let inventory = StorageInventory {
            host_root_mount_id: 1,
            topology_digest: "a".repeat(64),
            mounts_digest: "b".repeat(64),
            fstab_digest: "c".repeat(64),
            fstab_entries: vec![],
            devices,
        };
        Self {
            tree,
            preview,
            inventory,
            table,
        }
    }
    fn root(&self) -> OwnedFd {
        rustix::fs::open(
            self.tree.path(),
            OFlags::PATH | OFlags::DIRECTORY,
            Mode::empty(),
        )
        .unwrap()
    }
    fn path(&self, label: &str) -> std::path::PathBuf {
        self.tree.path().join(label.trim_start_matches('/'))
    }
    fn expected(&self) -> Vec<(StorageBlockDevice, u64)> {
        expected_mounts(&self.preview, &self.inventory, &self.table).unwrap()
    }
    fn collect(&self) -> Result<Paths> {
        collect_paths(
            self.root(),
            &self.preview,
            MountBindings {
                paths: &self.expected(),
                host_root: 1,
            },
            limits(),
            Instant::now() + LIFETIME,
            &|| false,
            self,
        )
    }
    fn first_file(&self) -> &str {
        &self
            .preview
            .requirements
            .iter()
            .find(|r| {
                matches!(
                    r.kind,
                    ProtectionPathKind::Content | ProtectionPathKind::Parity
                )
            })
            .unwrap()
            .path
    }
}
impl Fs for Fixture {
    fn stat(&self, fd: &OwnedFd, label: &str) -> Result<Statx> {
        // Real descriptor identities/types/timestamps with explicitly synthetic
        // mount/device projections. This cannot construct public raw inventory.
        let mut stat = Kernel.stat(fd, label)?;
        stat.stx_mnt_id = 1;
        stat.stx_dev_major = 8;
        stat.stx_dev_minor = 200;
        if let Some(mount) = self
            .table
            .iter()
            .filter(|m| beneath(label, &m.path))
            .max_by_key(|m| m.path.len())
        {
            stat.stx_mnt_id = mount.id;
            stat.stx_dev_major = mount.device.major;
            stat.stx_dev_minor = mount.device.minor;
        }
        Ok(stat)
    }
}

#[test]
fn frozen_layouts_preserve_order_and_multiparity_requirements() {
    for case in cases() {
        let f = Fixture::from_case(&case);
        if f.preview.requirements.is_empty() {
            assert!(
                collect_paths(
                    f.root(),
                    &f.preview,
                    MountBindings {
                        paths: &[],
                        host_root: 1
                    },
                    limits(),
                    Instant::now() + LIFETIME,
                    &|| false,
                    &f
                )
                .is_err()
            );
            continue;
        }
        let paths = f.collect().unwrap();
        assert_eq!(paths.observations.len(), f.preview.requirements.len());
        for (row, required) in paths.observations.iter().zip(&f.preview.requirements) {
            assert_eq!(row.path, required.path);
            assert_eq!(
                row.state,
                if matches!(
                    required.kind,
                    ProtectionPathKind::Source | ProtectionPathKind::Data
                ) {
                    ProtectionPathState::Directory
                } else {
                    ProtectionPathState::AbsentFile
                }
            );
        }
        assert!(paths.pinned.len() < paths.observations.len());
        paths.revalidate(&|| false, &f).unwrap();
        eprintln!(
            "SnapRAID fixture {}: rows={}, distinct_paths={}, retained_fds={}",
            case["name"],
            paths.observations.len(),
            paths.pinned.len(),
            1 + paths
                .pinned
                .iter()
                .map(|p| 1 + usize::from(p.object.is_some()))
                .sum::<usize>()
        );
    }
}

#[test]
fn regular_content_files_and_absent_leaves_are_both_pinned() {
    let f = Fixture::new();
    fs::write(f.path(f.first_file()), b"content checkpoint").unwrap();
    let paths = f.collect().unwrap();
    assert!(
        paths
            .observations
            .iter()
            .any(|p| p.state == ProtectionPathState::RegularFile)
    );
    assert!(
        paths
            .observations
            .iter()
            .any(|p| p.state == ProtectionPathState::AbsentFile)
    );
    paths.revalidate(&|| false, &f).unwrap();
    assert_eq!(
        fs::read(f.path(f.first_file())).unwrap(),
        b"content checkpoint"
    );
}

#[test]
fn source_directory_missing_or_wrong_type_refuses_the_whole_read() {
    for file in [false, true] {
        let f = Fixture::new();
        let path = f.path(&f.preview.requirements[0].path);
        fs::remove_dir(&path).unwrap();
        if file {
            fs::write(&path, b"wrong source").unwrap();
        }
        assert!(f.collect().is_err());
        if file {
            assert_eq!(fs::read(path).unwrap(), b"wrong source");
        }
    }
}

#[test]
fn symlink_directory_fifo_and_hardlinked_leaves_refuse_without_blocking() {
    for choice in 0..4 {
        let f = Fixture::new();
        let path = f.path(f.first_file());
        match choice {
            0 => symlink("/etc/passwd", &path).unwrap(),
            1 => fs::create_dir(&path).unwrap(),
            2 => {
                let parent = rustix::fs::open(
                    path.parent().unwrap(),
                    OFlags::PATH | OFlags::DIRECTORY,
                    Mode::empty(),
                )
                .unwrap();
                rustix::fs::mknodat(
                    &parent,
                    path.file_name().unwrap(),
                    rustix::fs::FileType::Fifo,
                    Mode::from_raw_mode(0o600),
                    0,
                )
                .unwrap();
            }
            _ => {
                fs::write(&path, b"hardlinked").unwrap();
                fs::hard_link(&path, f.tree.path().join("alias")).unwrap();
            }
        }
        let start = Instant::now();
        assert!(matches!(f.collect(), Err(SourceError::UnsafePath)));
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(fs::symlink_metadata(path).is_ok());
    }
}

#[test]
fn ancestor_symlink_and_escaping_paths_never_follow_targets() {
    let mut f = Fixture::new();
    fs::rename(f.tree.path().join("mnt"), f.tree.path().join("held-mnt")).unwrap();
    symlink("held-mnt", f.tree.path().join("mnt")).unwrap();
    assert!(matches!(f.collect(), Err(SourceError::UnsafePath)));
    for invalid in [
        "/mnt/../escape",
        "/mnt//disk",
        "/mnt/disk/",
        "relative",
        "/mnt/disk\n",
    ] {
        assert!(checked_path(invalid).is_err());
    }
    let expected = f.expected();
    f.preview.requirements[0].path = "/../../etc/passwd".into();
    assert!(matches!(
        collect_paths(
            f.root(),
            &f.preview,
            MountBindings {
                paths: &expected,
                host_root: 1
            },
            limits(),
            Instant::now() + LIFETIME,
            &|| false,
            &f
        ),
        Err(SourceError::InvalidPlan)
    ));
}

#[test]
fn every_uuid_mount_backing_and_raw_table_mismatch_refuses() {
    for choice in 0..12 {
        let mut f = Fixture::new();
        match choice {
            0 => f.inventory.devices[0].boot_backing = true,
            1 => f.inventory.devices[0].in_use_as_swap = true,
            2 => f.inventory.devices[0].filesystem_uuid = "replaced".into(),
            3 => f.inventory.devices.push(f.inventory.devices[0].clone()),
            4 => f.inventory.devices[0].mounts[0].writable = false,
            5 => f.inventory.devices[0].mounts[0].filesystem_root = "/subtree".into(),
            6 => f.inventory.devices[0].mounts[0].mount_id = 1,
            7 => f.inventory.devices[0].filesystem = "btrfs".into(),
            8 => f.table[0].id += 200,
            9 => f.table[0].root = "/subtree".into(),
            10 => f.table[0].writable = false,
            _ => {
                let mut nested = f.table[0].clone();
                nested.id += 200;
                nested.path.push_str("/foreign");
                f.table.push(nested);
            }
        }
        assert!(
            expected_mounts(&f.preview, &f.inventory, &f.table).is_err(),
            "case {choice}"
        );
    }
}

#[test]
fn wrong_descriptor_device_mount_and_root_identity_refuse() {
    let f = Fixture::new();
    let expected = f.expected();
    for choice in 0..3 {
        let mut facts = expected.clone();
        let host = if choice == 0 { 2 } else { 1 };
        if choice == 1 {
            facts[0].0.minor += 1;
        }
        if choice == 2 {
            facts[0].1 += 1;
        }
        assert!(matches!(
            collect_paths(
                f.root(),
                &f.preview,
                MountBindings {
                    paths: &facts,
                    host_root: host
                },
                limits(),
                Instant::now() + LIFETIME,
                &|| false,
                &f
            ),
            Err(SourceError::Changed)
        ));
    }
}

#[test]
fn metadata_write_unlink_replacement_and_absent_appearance_refuse_revalidation() {
    for choice in 0..4 {
        let f = Fixture::new();
        let path = f.path(f.first_file());
        if choice != 3 {
            fs::write(&path, b"original").unwrap();
        }
        let paths = f.collect().unwrap();
        match choice {
            0 => fs::write(&path, b"changed").unwrap(),
            1 => fs::remove_file(&path).unwrap(),
            2 => {
                fs::rename(&path, f.tree.path().join("held")).unwrap();
                fs::write(&path, b"original").unwrap();
            }
            _ => fs::write(&path, b"appeared").unwrap(),
        }
        assert!(matches!(
            paths.revalidate(&|| false, &f),
            Err(SourceError::Changed)
        ));
    }
}

#[test]
fn retained_parent_name_replacement_is_not_redirected() {
    let f = Fixture::new();
    let paths = f.collect().unwrap();
    let root = f.path(&f.preview.requirements[0].path);
    fs::rename(&root, f.tree.path().join("held")).unwrap();
    fs::create_dir(&root).unwrap();
    assert!(matches!(
        paths.revalidate(&|| false, &f),
        Err(SourceError::Changed)
    ));
    assert!(root.is_dir());
    assert!(f.tree.path().join("held").is_dir());
}

#[test]
fn cancellation_and_original_deadline_are_not_renewed() {
    let f = Fixture::new();
    let expected = f.expected();
    assert!(matches!(
        collect_paths(
            f.root(),
            &f.preview,
            MountBindings {
                paths: &expected,
                host_root: 1
            },
            limits(),
            Instant::now() + LIFETIME,
            &|| true,
            &f
        ),
        Err(SourceError::Cancelled)
    ));
    let calls = Cell::new(0);
    assert!(matches!(
        collect_paths(
            f.root(),
            &f.preview,
            MountBindings {
                paths: &expected,
                host_root: 1
            },
            limits(),
            Instant::now() + LIFETIME,
            &|| {
                calls.set(calls.get() + 1);
                calls.get() > 3
            },
            &f
        ),
        Err(SourceError::Cancelled)
    ));
    let mut paths = f.collect().unwrap();
    let original = paths.expires;
    paths.revalidate(&|| false, &f).unwrap();
    assert_eq!(paths.expires, original);
    paths.expires = Instant::now() - Duration::from_millis(1);
    assert!(matches!(
        paths.revalidate(&|| false, &f),
        Err(SourceError::TimedOut)
    ));
    assert!(matches!(
        paths.revalidate(&|| true, &f),
        Err(SourceError::Cancelled)
    ));
}

#[test]
fn bounds_refuse_and_configuration_identity_covers_complete_current_facts() {
    let f = Fixture::new();
    let case = &cases()[2];
    let mut config: SnapraidConfig = serde_json::from_value(case["snapraid"].clone()).unwrap();
    let pools: PoolsConfig = serde_json::from_value(case["pools"].clone()).unwrap();
    let contract: StorageContract = serde_json::from_value(case["contract"].clone()).unwrap();
    let identity = configuration_identity(&contract, &pools, &config, limits()).unwrap();
    config.thresholds.delete_threshold += 1;
    assert_ne!(
        identity,
        configuration_identity(&contract, &pools, &config, limits()).unwrap()
    );
    for invalid in [
        SourceLimits {
            max_paths: 0,
            ..limits()
        },
        SourceLimits {
            max_retained_fds: 129,
            ..limits()
        },
        SourceLimits {
            max_report_bytes: 65537,
            ..limits()
        },
    ] {
        assert!(matches!(
            limits_valid(invalid),
            Err(SourceError::InvalidLimits)
        ));
    }
    for small in [
        SourceLimits {
            max_paths: 1,
            ..limits()
        },
        SourceLimits {
            max_retained_fds: 2,
            ..limits()
        },
    ] {
        assert!(matches!(
            collect_paths(
                f.root(),
                &f.preview,
                MountBindings {
                    paths: &f.expected(),
                    host_root: 1
                },
                small,
                Instant::now() + LIFETIME,
                &|| false,
                &f
            ),
            Err(SourceError::Limit)
        ));
    }
    assert!(matches!(
        bounded_bytes(&"abcdef", 2),
        Err(SourceError::Limit)
    ));
    assert!(matches!(
        bounded_bytes(&vec!["x".repeat(1024); 128], 65536),
        Err(SourceError::Limit)
    ));
    let encoded = bounded_bytes(&f.inventory, 65536).unwrap();
    assert_eq!(
        serde_json::from_slice::<StorageInventory>(&encoded).unwrap(),
        f.inventory
    );
}

#[test]
fn owned_descriptor_cost_and_drop_close_without_filesystem_writes() {
    let f = Fixture::new();
    let before = fs::read_dir("/proc/self/fd").unwrap().count();
    let paths = f.collect().unwrap();
    let added = fs::read_dir("/proc/self/fd").unwrap().count() - before;
    let expected = 1 + paths
        .pinned
        .iter()
        .map(|p| 1 + usize::from(p.object.is_some()))
        .sum::<usize>();
    assert_eq!(added, expected);
    assert!(added <= 128);
    drop(paths);
    assert_eq!(fs::read_dir("/proc/self/fd").unwrap().count(), before);
    for r in &f.preview.requirements {
        if matches!(
            r.kind,
            ProtectionPathKind::Content | ProtectionPathKind::Parity
        ) {
            assert!(!f.path(&r.path).exists());
        }
    }
}

#[test]
fn changed_mount_mapping_and_metadata_permissions_refuse() {
    use std::os::unix::fs::PermissionsExt;
    let mut f = Fixture::new();
    let paths = f.collect().unwrap();
    f.table[0].id += 100;
    assert!(matches!(
        paths.revalidate(&|| false, &f),
        Err(SourceError::Changed)
    ));
    let f = Fixture::new();
    let path = f.path(f.first_file());
    fs::write(&path, b"content").unwrap();
    let paths = f.collect().unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(matches!(
        paths.revalidate(&|| false, &f),
        Err(SourceError::Changed)
    ));
}

#[test]
fn cancelled_and_midcollection_io_failure_close_all_owned_handles() {
    struct Broken<'a> {
        fixture: &'a Fixture,
        calls: Cell<usize>,
    }
    impl Fs for Broken<'_> {
        fn stat(&self, fd: &OwnedFd, label: &str) -> Result<Statx> {
            self.calls.set(self.calls.get() + 1);
            if self.calls.get() == 8 {
                return Err(SourceError::Io(io::Error::from_raw_os_error(5)));
            }
            self.fixture.stat(fd, label)
        }
    }
    let f = Fixture::new();
    let expected = f.expected();
    let before = fs::read_dir("/proc/self/fd").unwrap().count();
    let broken = Broken {
        fixture: &f,
        calls: Cell::new(0),
    };
    assert!(
        matches!(collect_paths(f.root(), &f.preview, MountBindings { paths: &expected, host_root: 1 },
        limits(), Instant::now()+LIFETIME, &|| false, &broken), Err(SourceError::Io(e)) if e.raw_os_error()==Some(5))
    );
    assert_eq!(fs::read_dir("/proc/self/fd").unwrap().count(), before);
    assert!(matches!(
        collect_paths(
            f.root(),
            &f.preview,
            MountBindings {
                paths: &expected,
                host_root: 1
            },
            limits(),
            Instant::now() + LIFETIME,
            &|| true,
            &f
        ),
        Err(SourceError::Cancelled)
    ));
    assert_eq!(fs::read_dir("/proc/self/fd").unwrap().count(), before);
}

#[test]
fn limits_records_are_closed_and_report_ceiling_is_exact() {
    for invalid in [
        r#"{"max_paths":1,"max_retained_fds":3,"max_report_bytes":100,"extra":0}"#,
        r#"{"max_paths":1,"max_paths":2,"max_retained_fds":3,"max_report_bytes":100}"#,
    ] {
        assert!(serde_json::from_str::<SourceLimits>(invalid).is_err());
    }
    let f = Fixture::new();
    let snapshot = ProtectionSnapshot {
        version: 1,
        configuration_identity: "a".repeat(64),
        sources: ProtectionSourceEvidence {
            observed_at: 100,
            inventory: f.inventory.clone(),
            paths: f.collect().unwrap().observations,
        },
    };
    let exact = serde_json::to_vec(&snapshot).unwrap();
    assert_eq!(bounded_bytes(&snapshot, exact.len()).unwrap(), exact);
    assert!(matches!(
        bounded_bytes(&snapshot, exact.len() - 1),
        Err(SourceError::Limit)
    ));
}
