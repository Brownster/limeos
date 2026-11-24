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
    let contract: StorageContract =
        serde_json::from_value(fixture["cases"][0]["contract"].clone()).unwrap();
    let mountpoint = contract.devices[0].mountpoint.clone();
    let uuid = contract.devices[0].filesystem_uuid.clone();
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
            devices: vec![limeos_domain::StorageObservedDevice {
                device: limeos_domain::StorageBlockDevice { major: 8, minor: 1 },
                filesystem_uuid: uuid,
                filesystem: "ext4".into(),
                serial: None,
                boot_backing: false,
                in_use_as_swap: false,
                mounts: vec![],
            }],
        },
        contract,
        targets: vec![TargetEvidence {
            mountpoint,
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
fn missing_and_extra_dependent_claims_refuse_a_healthy_root_journal() {
    for fault in ["missing", "extra", "schema"] {
        let temp = tempfile::tempdir().unwrap();
        let mut store = journal(&temp).unwrap();
        let original = receipt();
        store.begin(&original).unwrap();
        if fault == "missing" {
            store
                .conn
                .execute("DELETE FROM locks WHERE resource LIKE 'storage:uuid:%'", [])
                .unwrap();
        } else if fault == "extra" {
            store
                .conn
                .execute(
                    "INSERT INTO locks VALUES('storage:uuid:unexpected',?)",
                    [&original.plan.action],
                )
                .unwrap();
        } else {
            store.conn.execute_batch("ALTER TABLE locks RENAME TO old_locks; CREATE TABLE locks(resource TEXT, action TEXT NOT NULL REFERENCES receipts(action)) STRICT; INSERT INTO locks SELECT * FROM old_locks; DROP TABLE old_locks;").unwrap();
        }
        assert_eq!(
            store
                .conn
                .query_row("PRAGMA quick_check(1)", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        drop(store);
        assert!(matches!(journal(&temp), Err(Failure::StateNotDurable)));
    }
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

// Crash points the mounts journal (or the container receipt store) covered
// and the target journal did not, before both storage journals shared one
// implementation (`journal`).

#[test]
fn unexpected_trigger_refuses_the_target_journal() {
    let temp = tempfile::tempdir().unwrap();
    let store = journal(&temp).unwrap();
    store
        .conn
        .execute_batch(
            "CREATE TRIGGER unexpected AFTER INSERT ON receipts BEGIN DELETE FROM locks; END;",
        )
        .unwrap();
    drop(store);
    assert!(matches!(journal(&temp), Err(Failure::StateNotDurable)));
}

#[test]
fn a_settling_commit_that_fails_keeps_the_prepared_barrier_and_its_claims() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = journal(&temp).unwrap();
    let mut original = receipt();
    store.begin(&original).unwrap();
    store.conn.execute_batch("CREATE TRIGGER inject_failure BEFORE UPDATE ON receipts BEGIN SELECT RAISE(ABORT,'injected publication failure'); END;").unwrap();
    let prepared = original.clone();
    original.state = PreparationState::Verified;
    assert_eq!(store.finish(&original), Err(Failure::StateNotDurable));
    store
        .conn
        .execute_batch("DROP TRIGGER inject_failure;")
        .unwrap();
    drop(store);
    let mut store = journal(&temp).unwrap();
    assert_eq!(
        store.receipt(&prepared.plan.action, None).unwrap(),
        Some(prepared.clone())
    );
    let mut next = receipt();
    next.plan.action = "e".repeat(64);
    next.digest = limeos_identity::digest(&encode(&next.plan).unwrap());
    assert_eq!(store.begin(&next), Err(Failure::Conflict));
}

#[test]
fn dropped_target_journal_releases_its_lock_while_a_child_shares_the_descriptor() {
    let temp = tempfile::tempdir().unwrap();
    let store = journal(&temp).unwrap();
    let child_copy = store._lock.0.try_clone().unwrap();
    drop(store);
    assert!(journal(&temp).is_ok());
    drop(child_copy);
}

/// Stand-in for a target-preparation process that dies after its prepared
/// receipt is durable (between `mkdir` and the outcome): it exits without
/// running a destructor, so neither the connection nor the lock is closed.
#[test]
fn target_crash_helper() {
    let Some(directory) = std::env::var_os("LIMEOS_TARGET_JOURNAL_CRASH") else {
        return;
    };
    let fd = rustix::fs::open(
        Path::new(&directory),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .unwrap();
    let mut store = Journal::from_directory(fd, rustix::process::geteuid().as_raw()).unwrap();
    store.begin(&receipt()).unwrap();
    std::process::exit(86);
}

#[test]
fn process_death_after_the_prepared_receipt_keeps_every_claim() {
    let temp = tempfile::tempdir().unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "targets::tests::target_crash_helper",
            "--nocapture",
        ])
        .env("LIMEOS_TARGET_JOURNAL_CRASH", temp.path())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(86));
    let mut store = journal(&temp).unwrap();
    let original = receipt();
    assert_eq!(
        store
            .receipt(&original.plan.action, Some(&original.digest))
            .unwrap(),
        Some(original.clone())
    );
    let mut next = receipt();
    next.plan.action = "e".repeat(64);
    next.digest = limeos_identity::digest(&encode(&next.plan).unwrap());
    assert_eq!(store.begin(&next), Err(Failure::Conflict));
}
