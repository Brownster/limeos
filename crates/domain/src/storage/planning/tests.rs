use super::*;
fn fixture() -> (StorageContract, StorageInventory) {
    let value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/storage-contracts.json"
    ))
    .unwrap();
    let contract: StorageContract =
        serde_json::from_value(value["cases"][0]["contract"].clone()).unwrap();
    let inventory = StorageInventory {
        host_root_mount_id: 1,
        topology_digest: "a".repeat(64),
        mounts_digest: "b".repeat(64),
        fstab_digest: "c".repeat(64),
        fstab_entries: vec![],
        devices: vec![StorageObservedDevice {
            device: StorageBlockDevice { major: 8, minor: 1 },
            filesystem_uuid: contract.devices[0].filesystem_uuid.clone(),
            filesystem: "ext4".into(),
            serial: None,
            boot_backing: false,
            mounts: vec![],
        }],
    };
    (contract, inventory)
}
#[test]
fn existing_filesystems_only_unique_uuid_boot_and_serial_are_required() {
    let (mut contract, inventory) = fixture();
    storage_managed_fstab(&contract, 10, &inventory).unwrap();
    let mut changed = inventory.clone();
    changed.devices[0].boot_backing = true;
    assert!(storage_managed_fstab(&contract, 10, &changed).is_err());
    changed = inventory.clone();
    changed.devices.push(StorageObservedDevice {
        device: StorageBlockDevice { major: 8, minor: 2 },
        ..changed.devices[0].clone()
    });
    assert!(storage_managed_fstab(&contract, 10, &changed).is_err());
    changed = inventory.clone();
    changed.devices.clear();
    assert!(storage_managed_fstab(&contract, 10, &changed).is_err());
    changed = inventory.clone();
    changed.devices[0].filesystem = "xfs".into();
    assert!(storage_managed_fstab(&contract, 10, &changed).is_err());
    contract.devices[0].serial = Some("expected".into());
    assert!(storage_managed_fstab(&contract, 10, &inventory).is_err());
    changed = inventory;
    changed.devices[0].serial = Some("expected".into());
    storage_managed_fstab(&contract, 10, &changed).unwrap();
}
#[test]
fn foreign_fstab_and_mount_dependencies_prevent_reassignment() {
    let (contract, mut inventory) = fixture();
    let device = &contract.devices[0];
    for entry in [
        StorageFstabEntry {
            filesystem_uuid: Some(device.filesystem_uuid.clone()),
            device: None,
            mountpoint: "/mnt/other".into(),
            managed: false,
        },
        StorageFstabEntry {
            filesystem_uuid: None,
            device: Some(inventory.devices[0].device),
            mountpoint: "/mnt/other".into(),
            managed: false,
        },
        StorageFstabEntry {
            filesystem_uuid: None,
            device: None,
            mountpoint: "/mnt/storage/nested".into(),
            managed: false,
        },
    ] {
        inventory.fstab_entries = vec![entry];
        assert!(storage_managed_fstab(&contract, 10, &inventory).is_err());
    }
    inventory.fstab_entries.clear();
    let mount = StorageObservedMount {
        mountpoint: device.mountpoint.clone(),
        mount_id: 2,
        filesystem_root: "/".into(),
        filesystem: "ext4".into(),
        writable: true,
    };
    inventory.devices[0].mounts = vec![mount.clone()];
    storage_managed_fstab(&contract, 10, &inventory).unwrap();
    for changed in [
        StorageObservedMount {
            mountpoint: "/mnt/other".into(),
            ..mount.clone()
        },
        StorageObservedMount {
            filesystem_root: "/subtree".into(),
            ..mount.clone()
        },
        StorageObservedMount {
            writable: false,
            ..mount.clone()
        },
        StorageObservedMount {
            filesystem: "ext3".into(),
            ..mount.clone()
        },
    ] {
        inventory.devices[0].mounts = vec![changed];
        assert!(storage_managed_fstab(&contract, 10, &inventory).is_err());
    }
    inventory.devices[0].mounts = vec![
        mount.clone(),
        StorageObservedMount {
            mount_id: 3,
            ..mount
        },
    ];
    assert!(storage_managed_fstab(&contract, 10, &inventory).is_err());
}
#[test]
fn fstab_escapes_literal_case_sensitive_paths_and_limits_device_waits() {
    let (mut contract, inventory) = fixture();
    contract.devices[0].mountpoint = "/mnt/Media Disk\\One".into();
    contract.locations.media_host = "/mnt/Media Disk\\One/TV".into();
    contract.locations.downloads_host = "/mnt/Media Disk\\One/downloads".into();
    let rendered = storage_managed_fstab(&contract, 120, &inventory).unwrap();
    assert!(rendered.contains("/mnt/Media\\040Disk\\134One ext4"));
    assert!(rendered.contains("device-timeout=120s,x-systemd.mount-timeout=120s"));
    assert!(!rendered.contains("/TV"));
    for timeout in [0, 121] {
        assert!(storage_managed_fstab(&contract, timeout, &inventory).is_err());
    }
    for filesystem in [
        StorageFilesystem::Btrfs,
        StorageFilesystem::Ntfs,
        StorageFilesystem::Exfat,
        StorageFilesystem::Vfat,
    ] {
        contract.devices[0].filesystem = filesystem;
        assert_eq!(
            storage_managed_fstab(&contract, 10, &inventory)
                .unwrap_err()
                .0,
            ErrorCode::Unavailable
        );
    }
    let mut raw = serde_json::to_value(&contract).unwrap();
    raw["devices"][0]["options"] = serde_json::json!("exec,foo;evil");
    assert!(serde_json::from_value::<StorageContract>(raw).is_err());
}
#[test]
fn profile_roles_and_nested_physical_mounts_cannot_bypass_setup_validation() {
    let (contract, _) = fixture();
    let input = StorageSetupInput {
        contract,
        inventory_digest: "a".repeat(64),
        timeout_seconds: 10,
    };
    input.validate().unwrap();
    let mut changed = input.clone();
    changed.contract.profile = StorageProfile::SeparateDownloads;
    assert!(changed.validate().is_err());
    changed = input.clone();
    changed.contract.devices.push(StorageDevice {
        id: "backup".into(),
        role: StorageRole::ConfigBackup,
        filesystem_uuid: "backup".into(),
        mountpoint: "/mnt/storage/backup".into(),
        filesystem: StorageFilesystem::Ext4,
        serial: None,
    });
    changed.contract.locations.backup_host = "/mnt/storage/backup/limeos".into();
    changed.contract.validate().unwrap();
    assert!(changed.validate().is_err());
}
