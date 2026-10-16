use super::*;
use crate::StorageObservedDevice;

fn identity(inode: u64) -> DirectoryIdentity {
    DirectoryIdentity {
        major: 8,
        minor: 1,
        mount_id: 10,
        inode,
        mode: 0o040755,
        uid: 0,
        gid: 0,
    }
}
fn inspection() -> StorageMountInspection {
    let block = StorageBlockDevice {
        major: 8,
        minor: 16,
    };
    StorageMountInspection {
        target_id: "data".into(),
        device: StorageDevice {
            id: "data".into(),
            role: StorageRole::Data,
            filesystem_uuid: "uuid-data".into(),
            filesystem: StorageFilesystem::Ext4,
            mountpoint: "/mnt/data".into(),
            serial: Some("disk-data".into()),
        },
        expected: StorageInventory {
            host_root_mount_id: 10,
            topology_digest: "a".repeat(64),
            mounts_digest: "b".repeat(64),
            fstab_digest: "c".repeat(64),
            fstab_entries: vec![],
            devices: vec![StorageObservedDevice {
                device: block,
                filesystem_uuid: "uuid-data".into(),
                filesystem: "ext4".into(),
                serial: Some("disk-data".into()),
                boot_backing: false,
                in_use_as_swap: false,
                mounts: vec![],
            }],
        },
        target: TargetEvidence {
            mountpoint: "/mnt/data".into(),
            parent: identity(2),
            existing: Some(identity(9)),
        },
        physical_devices: vec![block],
        consumer_digest: "d".repeat(64),
        observed_at: 100,
    }
}
fn effect() -> StorageMountEffectPlan {
    let facts = inspection();
    StorageMountEffectPlan {
        version: 1,
        operation: STORAGE_MOUNT_OPERATION.into(),
        action: "e".repeat(64),
        device: facts.device,
        expected: facts.expected,
        target: facts.target,
        physical_devices: facts.physical_devices,
        consumer_digest: facts.consumer_digest,
        created_at: 100,
        expires_at: 400,
    }
}
fn receipt() -> StorageMountReceipt {
    let plan = effect();
    StorageMountReceipt {
        after: Some(StorageMountedSnapshot {
            filesystem_uuid: plan.device.filesystem_uuid.clone(),
            device: plan.physical_devices[0],
            mountpoint: plan.device.mountpoint.clone(),
            filesystem: "ext4".into(),
            filesystem_root: "/".into(),
            writable: true,
            nosuid: true,
            nodev: true,
            mount_id: 20,
            host_root_mount_id: 10,
            target: DirectoryIdentity {
                major: 8,
                minor: 16,
                mount_id: 20,
                inode: 2,
                mode: 0o040755,
                uid: 1000,
                gid: 1000,
            },
        }),
        plan,
        digest: "f".repeat(64),
        state: MountExecutionState::EffectAccepted,
        quiescence: MountEffectQuiescence::Reaped {
            helper: MountHelperIdentity {
                pid: 23,
                start_time_ticks: 1000,
                mount_namespace_inode: 50,
                cgroup: "/system.slice/limeos-storage-mount.service".into(),
                image_digest: "a".repeat(64),
            },
            outcome: MountHelperOutcome::Exited { code: 0 },
        },
    }
}

#[test]
fn only_managed_target_id_can_be_selected() {
    StorageMountSelection {
        target_id: "data".into(),
    }
    .validate()
    .unwrap();
    for name in ["", "/mnt/data", "../data", "DATA", "data\n"] {
        assert!(
            StorageMountSelection {
                target_id: name.into()
            }
            .validate()
            .is_err()
        );
    }
    assert!(
        serde_json::from_str::<StorageMountSelection>(
            r#"{"target_id":"data","mountpoint":"/mnt/evil"}"#
        )
        .is_err()
    );
}

#[test]
fn rejects_boot_swap_duplicates_unsupported_type_and_unprepared_target() {
    let valid = effect();
    valid.validate(100).unwrap();
    let mut changed = valid.clone();
    changed.expected.devices[0].boot_backing = true;
    assert!(changed.validate(100).is_err());
    let mut changed = valid.clone();
    changed.expected.devices[0].in_use_as_swap = true;
    assert!(changed.validate(100).is_err());
    let mut changed = valid.clone();
    changed.device.filesystem = StorageFilesystem::Btrfs;
    assert!(changed.validate(100).is_err());
    let mut changed = valid.clone();
    changed.target.existing = None;
    assert!(changed.validate(100).is_err());
    let mut changed = valid.clone();
    changed.expected.devices[0].serial = Some("replacement".into());
    assert!(changed.validate(100).is_err());
    let mut changed = valid.clone();
    let mut duplicate = changed.expected.devices[0].clone();
    duplicate.device.minor += 1;
    changed.expected.devices.push(duplicate);
    assert!(changed.validate(100).is_err());
}

#[test]
fn physical_claims_share_target_keys_and_are_bounded_exact_sorted_members() {
    let valid = effect();
    assert_eq!(
        valid.resources(),
        vec![
            "storage:configuration",
            "storage:device:8:16",
            "storage:mount:/mnt/data",
            "storage:uuid:uuid-data"
        ]
    );
    let mut changed = valid.clone();
    changed.physical_devices.clear();
    assert!(changed.validate(100).is_err());
    let mut changed = valid.clone();
    changed.physical_devices.push(changed.physical_devices[0]);
    assert!(changed.validate(100).is_err());
    let mut changed = valid.clone();
    changed.physical_devices = (1..=9)
        .map(|minor| StorageBlockDevice { major: 8, minor })
        .collect();
    assert!(changed.validate(100).is_err());
}

#[test]
fn expiry_and_new_consumer_facts_require_new_review() {
    let plan = effect();
    plan.check_current(&inspection(), 100).unwrap();
    assert_eq!(plan.validate(400), Err(Error(ErrorCode::Expired)));
    let mut changed = inspection();
    changed.consumer_digest = "f".repeat(64);
    assert_eq!(
        plan.check_current(&changed, 100),
        Err(Error(ErrorCode::Conflict))
    );
    let mut changed = inspection();
    changed.observed_at = 94;
    assert_eq!(
        plan.check_current(&changed, 100),
        Err(Error(ErrorCode::Expired))
    );
}

#[test]
fn pending_helper_never_proves_success_or_no_effect() {
    let mut value = receipt();
    value.validate().unwrap();
    value.quiescence = MountEffectQuiescence::Pending { helper: None };
    assert!(value.validate().is_err());
    value.after = None;
    value.state = MountExecutionState::PreconditionChanged;
    assert!(value.validate().is_err());
    value.state = MountExecutionState::OutcomeUnknown;
    value.validate().unwrap();
    value.quiescence = MountEffectQuiescence::NotLaunched {};
    value.state = MountExecutionState::PreconditionChanged;
    value.validate().unwrap();
}

#[test]
fn independent_identity_and_helper_result_are_required_even_after_plan_expiry() {
    let accepted = receipt();
    let after = accepted.after.as_ref().unwrap();
    verify_storage_mount(&accepted.plan, &accepted, after).unwrap();
    let mut replaced = after.clone();
    replaced.mount_id += 1;
    replaced.target.mount_id += 1;
    assert!(verify_storage_mount(&accepted.plan, &accepted, &replaced).is_err());
    let mut bad = accepted.clone();
    if let MountEffectQuiescence::Reaped { outcome, .. } = &mut bad.quiescence {
        *outcome = MountHelperOutcome::Signaled { signal: 9 };
    }
    assert!(bad.validate().is_err());
    // Verification uses the historical exact plan, not a refreshed execution TTL.
    assert_eq!(accepted.plan.validate(500), Err(Error(ErrorCode::Expired)));
    verify_storage_mount(&accepted.plan, &accepted, after).unwrap();
}

#[test]
fn oversized_borrowed_facts_refuse_before_device_clone() {
    let mut value = effect();
    value.device.mountpoint = "x".repeat(STORAGE_MOUNT_PLAN_BYTES + 1);
    assert_eq!(value.validate(100), Err(Error(ErrorCode::InvalidInput)));
}

#[test]
fn root_fstab_is_allowed_but_selected_or_overlapping_rows_refuse() {
    let mut value = effect();
    value.expected.fstab_entries.push(crate::StorageFstabEntry {
        filesystem_uuid: Some("boot-root".into()),
        device: Some(StorageBlockDevice { major: 8, minor: 1 }),
        mountpoint: "/".into(),
        managed: false,
    });
    value.validate(100).unwrap();
    for mountpoint in ["/mnt", "/mnt/data", "/mnt/data/nested"] {
        let mut changed = value.clone();
        changed.expected.fstab_entries[0].mountpoint = mountpoint.into();
        assert!(changed.validate(100).is_err());
    }
    value.expected.fstab_entries[0].filesystem_uuid = Some("uuid-data".into());
    assert!(value.validate(100).is_err());
}

#[test]
fn ext4_without_nosuid_or_nodev_cannot_be_verified() {
    let value = receipt();
    for missing_nosuid in [true, false] {
        let mut current = value.after.clone().unwrap();
        if missing_nosuid {
            current.nosuid = false;
        } else {
            current.nodev = false;
        }
        let mut changed = value.clone();
        changed.after = Some(current.clone());
        assert!(changed.validate().is_err());
        assert!(verify_storage_mount(&value.plan, &changed, &current).is_err());
    }
}
