use super::*;
use limeos_domain::{
    ContainerSnapshot, ContainerStorageConsumer, ContainerStorageMount, ContainerStorageSource,
};
use std::{
    fs,
    io::Read,
    os::unix::fs::{PermissionsExt, symlink},
};

fn inventory(paths: &[&Path]) -> ContainerStorageInventory {
    ContainerStorageInventory {
        version: 1,
        engine_id: "fixture-engine".into(),
        observed_at: now().unwrap(),
        containers: vec![ContainerStorageConsumer {
            container: ContainerSnapshot {
                resource: format!("container:{}", "a".repeat(64)),
                image: format!("sha256:{}", "f".repeat(64)),
                started_at: "2026-10-06T00:00:00Z".into(),
                running: false,
            },
            mounts: paths
                .iter()
                .enumerate()
                .map(|(i, p)| ContainerStorageMount {
                    source: ContainerStorageSource::Bind {
                        path: p.to_str().unwrap().into(),
                    },
                    destination: format!("/data/{i}"),
                    writable: true,
                })
                .collect(),
        }],
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(WORK_SECONDS)
}
fn evidence(value: &ContainerStorageInventory) -> Result<ContainerSourceEvidence> {
    let table = read_table()?;
    let ns = host_view(&table, &read_host_table()?)?;
    collect(value, Context::new()?, table, ns, deadline())
}
fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("Media");
    fs::write(&file, b"private contents").unwrap();
    (dir, file)
}

#[test]
fn real_descriptors_bind_stopped_sources_without_reading_or_changing_contents() {
    let (dir, file) = fixture();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o0)).unwrap();
    let value = inventory(&[dir.path(), &file]);
    let result = evidence(&value).unwrap();
    assert_eq!(result.snapshot.inventory_digest, digest(&value).unwrap());
    assert_eq!(
        result.snapshot.sources[0].identity.kind,
        ContainerSourceKind::Directory
    );
    assert_eq!(
        result.snapshot.sources[1].identity.kind,
        ContainerSourceKind::RegularFile
    );
    assert_eq!(
        result.snapshot.sources[1].resolved_path,
        file.to_str().unwrap()
    );
    assert_eq!(
        result.snapshot.sources[1].potential_mount_ids,
        [result.snapshot.sources[1].identity.mount_id]
    );
    let fd = rustix::io::dup(&result.retained[1]).unwrap();
    assert!(fs::File::from(fd).read_to_end(&mut vec![]).is_err());
    result.revalidate_inner(deadline()).unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(fs::read(&file).unwrap(), b"private contents");
}
#[test]
fn ordinary_relative_and_absolute_links_resolve_the_same_kernel_identity() {
    let (dir, file) = fixture();
    let relative = dir.path().join("relative");
    let absolute = dir.path().join("absolute");
    symlink("Media", &relative).unwrap();
    symlink(&file, &absolute).unwrap();
    let result = evidence(&inventory(&[&file, &relative, &absolute])).unwrap();
    for source in &result.snapshot.sources {
        assert_eq!(source.resolved_path, file.to_str().unwrap());
        assert_eq!(source.identity, result.snapshot.sources[0].identity);
    }
    result.revalidate_inner(deadline()).unwrap();
}
#[test]
fn link_targets_preserve_kernel_directory_and_trailing_slash_semantics() {
    let (dir, _file) = fixture();
    fs::write(dir.path().join("other"), b"other").unwrap();
    for (name, target) in [
        ("dotdot", "Media/../other"),
        ("slash", "Media/"),
        ("dot", "Media/."),
    ] {
        let link = dir.path().join(name);
        symlink(target, &link).unwrap();
        assert!(fs::metadata(&link).is_err());
        assert!(evidence(&inventory(&[&link])).is_err(), "{target}");
    }
}
#[test]
fn linked_parent_and_normal_target_dotdot_follow_the_kernel_object() {
    use std::os::unix::fs::MetadataExt;
    let (dir, file) = fixture();
    let sub = dir.path().join("sub");
    fs::create_dir(&sub).unwrap();
    let link = sub.join("link");
    symlink(".././Media", &link).unwrap();
    let parent = dir.path().join("parent");
    symlink("sub", &parent).unwrap();
    let through_parent = parent.join("link");
    let result = evidence(&inventory(&[&through_parent])).unwrap();
    assert_eq!(
        result.snapshot.sources[0].identity.inode,
        fs::metadata(&through_parent).unwrap().ino()
    );
    assert_eq!(
        result.snapshot.sources[0].resolved_path,
        file.to_str().unwrap()
    );
}
#[test]
fn link_count_target_bytes_control_characters_and_non_file_types_are_bounded() {
    let (dir, _file) = fixture();
    for i in 0..40 {
        let target = if i == 39 {
            "Media".into()
        } else {
            format!("link{}", i + 1)
        };
        symlink(target, dir.path().join(format!("link{i}"))).unwrap();
    }
    evidence(&inventory(&[&dir.path().join("link0")])).unwrap();
    let extra = dir.path().join("extra");
    symlink("link0", &extra).unwrap();
    assert!(evidence(&inventory(&[&extra])).is_err());
    for (name, target) in [("long", "a/".repeat(300)), ("control", "Media\n".into())] {
        let link = dir.path().join(name);
        symlink(target, &link).unwrap();
        assert!(evidence(&inventory(&[&link])).is_err());
    }
    let fifo = dir.path().join("fifo");
    rustix::fs::mkfifoat(rustix::fs::CWD, &fifo, Mode::from_bits_truncate(0o600)).unwrap();
    assert!(matches!(
        evidence(&inventory(&[&fifo])),
        Err(Failure::UnsafePath)
    ));
}
#[test]
fn named_volume_facts_bind_the_inventory_and_empty_inventory_is_explicit() {
    let (dir, _file) = fixture();
    let mut value = inventory(&[dir.path()]);
    value.containers[0].mounts[0].source = ContainerStorageSource::Volume {
        path: dir.path().to_str().unwrap().into(),
        name: "fixture-data".into(),
        driver: "local".into(),
    };
    let result = evidence(&value).unwrap();
    assert_eq!(result.snapshot.inventory_digest, digest(&value).unwrap());
    value.containers.clear();
    let empty = evidence(&value).unwrap();
    assert!(empty.snapshot.sources.is_empty());
    assert!(empty.snapshot.mounts.is_empty());
    assert_ne!(
        empty.snapshot.inventory_digest,
        result.snapshot.inventory_digest
    );
}
#[test]
fn literal_case_spaces_and_distinct_files_remain_distinct() {
    let (dir, file) = fixture();
    let other = dir.path().join("media space");
    fs::write(&other, b"other").unwrap();
    let result = evidence(&inventory(&[&file, &other])).unwrap();
    assert_ne!(
        result.snapshot.sources[0].identity.inode,
        result.snapshot.sources[1].identity.inode
    );
    assert_eq!(
        result.snapshot.sources[1].resolved_path,
        other.to_str().unwrap()
    );
}
#[test]
fn missing_cyclic_and_magic_links_never_yield_source_evidence() {
    let (dir, file) = fixture();
    let missing = dir.path().join("missing");
    assert!(evidence(&inventory(&[&file, &missing])).is_err());
    assert!(!missing.exists());
    let cycle = dir.path().join("cycle");
    symlink("cycle", &cycle).unwrap();
    assert!(evidence(&inventory(&[&cycle])).is_err());
    let context = Context::new().unwrap();
    let table = read_table().unwrap();
    let fd = context
        .open(file.to_str().unwrap(), &table, deadline())
        .unwrap();
    let magic = format!("/proc/{}/fd/{}", std::process::id(), fd.as_raw_fd());
    assert!(context.open(&magic, &table, deadline()).is_err());
}
#[test]
fn invalid_names_do_not_reach_kernel_lookup() {
    let context = Context::new().unwrap();
    let table = read_table().unwrap();
    for path in [
        "relative",
        "/tmp/../tmp",
        "/tmp//x",
        "/tmp/./x",
        "/tmp/x/",
        "/tmp/a\n",
        "/tmp/a\0",
    ] {
        assert_eq!(
            context.open(path, &table, deadline()).unwrap_err(),
            Failure::InvalidPlan
        );
    }
    assert_eq!(
        context
            .open(&format!("/{}", "a".repeat(512)), &table, deadline())
            .unwrap_err(),
        Failure::InvalidPlan
    );
}
#[test]
fn replacement_deletion_and_parent_rename_invalidate_retained_evidence() {
    for change in ["replace", "delete", "rename-parent"] {
        let (dir, file) = fixture();
        let result = evidence(&inventory(&[&file])).unwrap();
        match change {
            "replace" => {
                let replacement = dir.path().join("replacement");
                fs::write(&replacement, b"replacement").unwrap();
                fs::rename(&replacement, &file).unwrap();
            }
            "delete" => fs::remove_file(&file).unwrap(),
            _ => fs::rename(dir.path(), dir.path().with_extension("moved")).unwrap(),
        }
        assert!(result.revalidate_inner(deadline()).is_err(), "{change}");
        if change == "rename-parent" {
            fs::remove_dir_all(dir.path().with_extension("moved")).unwrap();
        }
    }
}
#[test]
fn symlink_retargeting_invalidates_but_file_content_changes_preserve_dependencies() {
    let (dir, file) = fixture();
    let link = dir.path().join("link");
    symlink("Media", &link).unwrap();
    let result = evidence(&inventory(&[&link])).unwrap();
    fs::write(&file, b"changed contents").unwrap();
    result.revalidate_inner(deadline()).unwrap();
    fs::write(dir.path().join("other"), b"other").unwrap();
    fs::remove_file(&link).unwrap();
    symlink("other", &link).unwrap();
    assert!(result.revalidate_inner(deadline()).is_err());
}
#[test]
fn wrong_mount_device_missing_mount_and_remote_filesystems_refuse_metadata() {
    let (_dir, file) = fixture();
    let context = Context::new().unwrap();
    let table = read_table().unwrap();
    let fd = context
        .open(file.to_str().unwrap(), &table, deadline())
        .unwrap();
    let mount_id = descriptor_mount_id(&fd).unwrap();
    for kind in ["device", "missing", "nfs", "fuse.mergerfs", "autofs"] {
        let mut altered = table.clone();
        match kind {
            "missing" => altered.retain(|m| m.id != mount_id),
            "device" => {
                altered
                    .iter_mut()
                    .find(|m| m.id == mount_id)
                    .unwrap()
                    .device
                    .minor += 1
            }
            _ => {
                altered
                    .iter_mut()
                    .find(|m| m.id == mount_id)
                    .unwrap()
                    .filesystem = kind.into()
            }
        }
        assert!(context.inspect(&fd, &altered).is_err(), "{kind}");
    }
}

fn row(id: u64, minor: u32, root: &str, path: &str) -> mounts::Mount {
    mounts::Mount {
        id,
        device: DeviceNumber { major: 8, minor },
        root: root.into(),
        path: path.into(),
        filesystem: "ext4".into(),
        writable: true,
    }
}
#[test]
fn alias_roots_and_nested_mounts_preserve_device_identity_and_component_boundaries() {
    let table = vec![
        row(1, 1, "/", "/"),
        row(2, 16, "/", "/mnt/storage"),
        row(3, 16, "/Media", "/alias"),
        row(4, 32, "/", "/alias/TV"),
        row(5, 48, "/", "/alias-old"),
        row(6, 64, "/", "/Alias"),
    ];
    let identity = ContainerSourceIdentity {
        device: table[2].device,
        mount_id: 3,
        inode: 42,
        kind: ContainerSourceKind::Directory,
    };
    assert_eq!(identity.device, table[1].device);
    assert_eq!(table[2].root, "/Media");
    assert_eq!(candidates(&identity, "/alias", &table).unwrap(), [3, 4]);
    let file = ContainerSourceIdentity {
        kind: ContainerSourceKind::RegularFile,
        ..identity.clone()
    };
    assert_eq!(candidates(&file, "/alias", &table).unwrap(), [3]);
    assert!(
        candidates(
            &ContainerSourceIdentity {
                mount_id: 99,
                ..identity
            },
            "/alias",
            &table
        )
        .is_err()
    );
}
#[test]
fn root_ancestor_and_per_source_mount_reference_limits_refuse_clipping() {
    let table: Vec<_> = (1..=512)
        .map(|i| row(i, 1, "/", &format!("/mnt/{i}")))
        .collect();
    let identity = ContainerSourceIdentity {
        device: table[0].device,
        mount_id: 1,
        inode: 1,
        kind: ContainerSourceKind::Directory,
    };
    assert_eq!(candidates(&identity, "/", &table).unwrap().len(), 512);
    let mut extra = table;
    extra.push(row(513, 2, "/", "/last"));
    assert_eq!(
        candidates(&identity, "/", &extra).unwrap_err(),
        Failure::Unavailable
    );
}
#[test]
fn aggregate_mount_reference_limit_covers_multiple_containers_and_duplicate_sources() {
    let (dir, _file) = fixture();
    let mut value = inventory(&vec![dir.path(); 64]);
    let original = value.containers[0].clone();
    value.containers = (0..4)
        .map(|i| ContainerStorageConsumer {
            container: ContainerSnapshot {
                resource: format!("container:{i:064x}"),
                ..original.container.clone()
            },
            mounts: original.mounts.clone(),
        })
        .collect();
    let mut table = read_table().unwrap();
    let ns = host_view(&table, &read_host_table().unwrap()).unwrap();
    let id = table.iter().map(|m| m.id).max().unwrap() + 1;
    table.extend([
        row(id, 16, "/", &format!("{}/one", dir.path().display())),
        row(id + 1, 32, "/", &format!("{}/two", dir.path().display())),
    ]);
    assert!(matches!(
        collect(&value, Context::new().unwrap(), table, ns, deadline()),
        Err(Failure::Unavailable)
    ));
}
#[test]
fn oversized_reports_are_refused_after_bounded_collection() {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("a".repeat(200));
    fs::create_dir(&parent).unwrap();
    let file = parent.join("b".repeat(220));
    fs::write(&file, b"fixture").unwrap();
    let value = inventory(&vec![file.as_path(); 64]);
    assert!(value.validate(now().unwrap()).is_ok());
    assert!(serde_json::to_vec(&value).unwrap().len() < 64 * 1024);
    assert!(matches!(evidence(&value), Err(Failure::Unavailable)));
}
#[test]
fn changed_mount_tables_namespace_and_both_age_clocks_invalidate_evidence() {
    let (_dir, file) = fixture();
    let mut result = evidence(&inventory(&[&file])).unwrap();
    result.table[0].writable = !result.table[0].writable;
    assert_eq!(
        result.revalidate_inner(deadline()).unwrap_err(),
        Failure::Conflict
    );
    result.table = read_table().unwrap();
    result.snapshot.host_root_mount_id += 1;
    assert!(result.revalidate_inner(deadline()).is_err());
    result.snapshot.host_root_mount_id -= 1;
    result.inventory.observed_at = now().unwrap() - 6;
    assert_eq!(
        result.revalidate_inner(deadline()).unwrap_err(),
        Failure::Conflict
    );
    result.inventory.observed_at = now().unwrap() + 1;
    assert_eq!(
        result.revalidate_inner(deadline()).unwrap_err(),
        Failure::Conflict
    );
    result.inventory.observed_at = now().unwrap();
    result.expires = Instant::now();
    assert_eq!(
        result.revalidate_inner(deadline()).unwrap_err(),
        Failure::TimedOut
    );
}
#[test]
fn raw_mount_digest_detects_parent_propagation_and_option_changes() {
    let original = "1 0 8:1 / / rw,nosuid shared:2 - ext4 /dev/sda1 rw\n";
    let table = mounts::parse(original).unwrap();
    let digest = limeos_identity::digest(original);
    table_matches(original, &table, &digest).unwrap();
    for changed in [
        "1 99 8:1 / / rw,nosuid shared:2 - ext4 /dev/sda1 rw\n",
        "1 0 8:1 / / rw,nosuid shared:3 - ext4 /dev/sda1 rw\n",
        "1 0 8:1 / / rw,nodev shared:2 - ext4 /dev/sda1 rw\n",
    ] {
        assert_eq!(mounts::parse(changed).unwrap(), table);
        assert_eq!(
            table_matches(changed, &table, &digest).unwrap_err(),
            Failure::Conflict
        );
    }
}
#[test]
fn malformed_proc_ids_expired_work_and_unprivileged_boundary_fail_closed() {
    assert_eq!(mount_id("pos:\t0\nmnt_id:\t123\n").unwrap(), 123);
    for text in [
        "",
        "mnt_id: 0",
        "mnt_id: -1",
        "mnt_id: 1\nmnt_id: 2",
        "mnt_id: garbage",
    ] {
        assert_eq!(mount_id(text).unwrap_err(), Failure::Unavailable);
    }
    let (_dir, file) = fixture();
    let table = read_table().unwrap();
    let ns = host_view(&table, &read_host_table().unwrap()).unwrap();
    let value = inventory(&[&file]);
    assert!(matches!(
        collect(&value, Context::new().unwrap(), table, ns, Instant::now()),
        Err(Failure::TimedOut)
    ));
    if !rustix::process::geteuid().is_root() {
        assert!(matches!(
            inspect_container_sources(&value),
            Err(Failure::Unavailable)
        ));
        assert!(matches!(
            evidence(&value).unwrap().revalidate(),
            Err(Failure::Unavailable)
        ));
    }
}
