use super::*;
fn journal(temp: &tempfile::TempDir) -> Result<Journal> {
    let fd = rustix::fs::open(
        temp.path(),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .unwrap();
    Journal::from_directory(fd, rustix::process::geteuid().as_raw())
}
fn receipt() -> TargetPreparationReceipt {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/storage-contracts.json"
    ))
    .unwrap();
    let contract = serde_json::from_value(fixture["cases"][0]["contract"].clone()).unwrap();
    let plan = TargetPreparationPlan {
        version: 1,
        operation: OPERATION.into(),
        action: "a".repeat(64),
        expected: StorageInventory {
            host_root_mount_id: 1,
            topology_digest: "b".repeat(64),
            mounts_digest: "c".repeat(64),
            fstab_digest: "d".repeat(64),
            fstab_entries: vec![],
            devices: vec![],
        },
        contract,
        targets: vec![TargetEvidence {
            mountpoint: "/mnt/data".into(),
            parent: DirectoryIdentity {
                major: 8,
                minor: 1,
                mount_id: 1,
                inode: 12,
                mode: 0o40755,
                uid: 0,
                gid: 0,
            },
            existing: None,
        }],
        created_at: 100,
        expires_at: 400,
    };
    TargetPreparationReceipt {
        digest: limeos_identity::digest(&encode(&plan).unwrap()),
        plan,
        state: PreparationState::Prepared,
        after: vec![],
    }
}
#[test]
fn preparation_is_a_closed_distinct_operation_with_bounded_expiry() {
    let original = receipt().plan;
    original.validate(399).unwrap();
    assert!(original.validate(400).is_err());
    for bad in [
        TargetPreparationPlan {
            operation: "storage.mount".into(),
            ..original.clone()
        },
        TargetPreparationPlan {
            version: 2,
            ..original.clone()
        },
        TargetPreparationPlan {
            expires_at: 401,
            ..original.clone()
        },
        TargetPreparationPlan {
            targets: vec![],
            ..original.clone()
        },
    ] {
        assert!(bad.validate(100).is_err());
    }
    let mut raw = serde_json::to_value(original).unwrap();
    raw["command"] = serde_json::json!("mount --all");
    assert!(serde_json::from_value::<TargetPreparationPlan>(raw).is_err());
}
#[test]
fn durable_prepared_and_unknown_receipts_keep_all_resources_across_restarts() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = journal(&temp).unwrap();
    let mut original = receipt();
    store.begin(&original).unwrap();
    assert!(matches!(journal(&temp), Err(Failure::Conflict)));
    assert_eq!(
        store
            .receipt(&original.plan.action, Some(&original.digest))
            .unwrap(),
        Some(original.clone())
    );
    assert_eq!(
        store
            .receipt(&original.plan.action, Some(&"f".repeat(64)))
            .unwrap_err(),
        Failure::Conflict
    );
    drop(store);
    let mut store = journal(&temp).unwrap();
    let mut conflicting = original.clone();
    conflicting.plan.action = "e".repeat(64);
    conflicting.digest = limeos_identity::digest(&encode(&conflicting.plan).unwrap());
    assert_eq!(store.begin(&conflicting).unwrap_err(), Failure::Conflict);
    assert!(
        store
            .receipt(&conflicting.plan.action, None)
            .unwrap()
            .is_none()
    );
    original.state = PreparationState::OutcomeUnknown;
    store.finish(&original).unwrap();
    drop(store);
    let mut store = journal(&temp).unwrap();
    assert_eq!(
        store.receipt(&original.plan.action, None).unwrap(),
        Some(original)
    );
    assert_eq!(store.begin(&conflicting).unwrap_err(), Failure::Conflict);
}
#[test]
fn only_verified_or_no_effect_outcomes_release_claims_atomically() {
    for state in [
        PreparationState::Verified,
        PreparationState::PreconditionChanged,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let mut store = journal(&temp).unwrap();
        let mut original = receipt();
        store.begin(&original).unwrap();
        original.state = state;
        store.finish(&original).unwrap();
        assert!(store.finish(&original).is_err());
        let mut next = receipt();
        next.plan.action = "e".repeat(64);
        next.digest = limeos_identity::digest(&encode(&next.plan).unwrap());
        store.begin(&next).unwrap();
    }
}
#[test]
fn journal_refuses_symlinks_hardlinks_writable_files_and_future_schemas() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    for name in [
        "targets.lock",
        "targets.sqlite",
        "targets.sqlite-wal",
        "targets.sqlite-shm",
        "targets.sqlite-journal",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside");
        std::fs::write(&outside, b"preserve").unwrap();
        symlink(&outside, temp.path().join(name)).unwrap();
        assert!(journal(&temp).is_err());
        assert_eq!(std::fs::read(outside).unwrap(), b"preserve");
    }
    let temp = tempfile::tempdir().unwrap();
    let store = journal(&temp).unwrap();
    store.conn.pragma_update(None, "user_version", 2).unwrap();
    drop(store);
    assert!(journal(&temp).is_err());
    let temp = tempfile::tempdir().unwrap();
    drop(journal(&temp).unwrap());
    std::fs::set_permissions(
        temp.path().join("targets.sqlite"),
        std::fs::Permissions::from_mode(0o666),
    )
    .unwrap();
    assert!(journal(&temp).is_err());
    let temp = tempfile::tempdir().unwrap();
    drop(journal(&temp).unwrap());
    std::fs::hard_link(
        temp.path().join("targets.sqlite"),
        temp.path().join("alias"),
    )
    .unwrap();
    assert!(journal(&temp).is_err());
}
