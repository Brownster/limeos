use super::*;
use limeos_domain::{
    DirectoryIdentity, StorageBlockDevice, StorageDevice, StorageFilesystem, StorageInventory,
    StorageObservedDevice, StorageRole, TargetEvidence,
};
use std::os::{fd::AsFd, unix::fs::MetadataExt};

fn fixture() -> (tempfile::TempDir, MountJournal) {
    // Repository disk only; these source scenarios never create loop devices or
    // mount anything. Qualification must use a separate disposable VM data disk.
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let scratch = repository.join(".cache/p04-a1-journal-fixtures");
    std::fs::create_dir_all(&scratch).unwrap();
    let directory = tempfile::Builder::new()
        .prefix("journal-")
        .tempdir_in(scratch)
        .unwrap();
    let fd = rustix::fs::open(
        directory.path(),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .unwrap();
    let owner = std::fs::metadata(directory.path()).unwrap().uid();
    let journal = MountJournal::from_directory(fd, owner).unwrap();
    (directory, journal)
}
fn plan() -> StorageMountEffectPlan {
    let device = StorageBlockDevice {
        major: 8,
        minor: 16,
    };
    let identity = |inode| DirectoryIdentity {
        major: 8,
        minor: 1,
        mount_id: 10,
        inode,
        mode: 0o040755,
        uid: 0,
        gid: 0,
    };
    StorageMountEffectPlan {
        version: 1,
        operation: "storage.mount".into(),
        action: "a".repeat(64),
        device: StorageDevice {
            id: "data".into(),
            role: StorageRole::Data,
            filesystem_uuid: "uuid-data".into(),
            filesystem: StorageFilesystem::Ext4,
            mountpoint: "/mnt/data".into(),
            serial: None,
        },
        expected: StorageInventory {
            host_root_mount_id: 10,
            topology_digest: "b".repeat(64),
            mounts_digest: "c".repeat(64),
            fstab_digest: "d".repeat(64),
            fstab_entries: vec![],
            devices: vec![StorageObservedDevice {
                device,
                filesystem_uuid: "uuid-data".into(),
                filesystem: "ext4".into(),
                serial: None,
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
        physical_devices: vec![device],
        consumer_digest: "e".repeat(64),
        created_at: 100,
        expires_at: 400,
    }
}
fn reopen(directory: &Path) -> Result<MountJournal> {
    let fd = rustix::fs::open(
        directory,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .unwrap();
    let owner = std::fs::metadata(directory).unwrap().uid();
    MountJournal::from_directory(fd, owner)
}
fn helper() -> MountHelperIdentity {
    MountHelperIdentity {
        pid: 23,
        start_time_ticks: 1000,
        mount_namespace_inode: 50,
        cgroup: "/system.slice/limeos-storage-mounts.service".into(),
        image_digest: "f".repeat(64),
    }
}
fn accepted_fixture(journal: &mut MountJournal) -> StorageMountReceipt {
    // Receipt-journal facts only; no production admission/termination owner is
    // manufactured and no executable/host effect is reached by this fixture.
    let prepared = journal.begin(&plan(), 100).unwrap();
    let pending = journal.arm(&prepared).unwrap();
    let (bound, permit) = journal.bind_helper(&pending, helper()).unwrap();
    drop(permit);
    let mut stopped = bound.clone();
    stopped.quiescence = MountEffectQuiescence::Reaped {
        helper: helper(),
        outcome: limeos_domain::MountHelperOutcome::Exited { code: 0 },
    };
    journal.record(&bound, &stopped).unwrap();
    let mut accepted = stopped.clone();
    accepted.state = MountExecutionState::EffectAccepted;
    accepted.after = Some(limeos_domain::StorageMountedSnapshot {
        filesystem_uuid: "uuid-data".into(),
        device: StorageBlockDevice {
            major: 8,
            minor: 16,
        },
        mountpoint: "/mnt/data".into(),
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
    });
    journal.record(&stopped, &accepted).unwrap();
    accepted
}

#[test]
fn durable_prepared_and_pending_receipts_survive_restart_without_reexecution() {
    let (directory, mut journal) = fixture();
    let effect = plan();
    let prepared = journal.begin(&effect, 100).unwrap();
    assert_eq!(prepared.quiescence, MountEffectQuiescence::NotLaunched {});
    let pending = journal.arm(&prepared).unwrap();
    let digest = pending.digest.clone();
    drop(journal);
    let mut journal = reopen(directory.path()).unwrap();
    assert_eq!(
        journal.lookup(&effect.action, Some(&digest)).unwrap(),
        Some(pending.clone())
    );
    // Replay works after expiry and cannot reset Pending to NotLaunched.
    assert_eq!(journal.begin(&effect, 500).unwrap(), pending);
    let locks: u32 = journal
        .conn
        .query_row("SELECT count(*) FROM locks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(locks as usize, effect.resources().len());
}

#[test]
fn unknown_with_pending_helper_cannot_become_no_effect() {
    let (_directory, mut journal) = fixture();
    let prepared = journal.begin(&plan(), 100).unwrap();
    let pending = journal.arm(&prepared).unwrap();
    let mut unknown = pending.clone();
    unknown.state = MountExecutionState::OutcomeUnknown;
    journal.record(&pending, &unknown).unwrap();
    let mut falsely_absent = unknown.clone();
    falsely_absent.state = MountExecutionState::PreconditionChanged;
    assert_eq!(
        journal.record(&unknown, &falsely_absent),
        Err(Failure::InvalidPlan)
    );
    // Caller-forged removal of Pending cannot create a no-launch fact either.
    falsely_absent.quiescence = MountEffectQuiescence::NotLaunched {};
    assert_eq!(
        journal.record(&unknown, &falsely_absent),
        Err(Failure::Conflict)
    );
}

#[test]
fn claimed_device_target_and_configuration_cannot_dispatch_twice() {
    let (_directory, mut journal) = fixture();
    let effect = plan();
    journal.begin(&effect, 100).unwrap();
    let mut next = effect.clone();
    next.action = "f".repeat(64);
    assert_eq!(journal.begin(&next, 100), Err(Failure::Conflict));
    let mut replaced = effect.clone();
    replaced.consumer_digest = "f".repeat(64);
    assert_eq!(journal.begin(&replaced, 100), Err(Failure::Conflict));
}

#[test]
fn missing_retained_claim_is_corruption_after_restart() {
    let (directory, mut journal) = fixture();
    journal.begin(&plan(), 100).unwrap();
    journal
        .conn
        .execute("DELETE FROM locks WHERE resource='storage:device:8:16'", [])
        .unwrap();
    drop(journal);
    assert!(matches!(
        reopen(directory.path()),
        Err(Failure::StateNotDurable)
    ));
}

#[test]
fn unexpected_trigger_cannot_modify_receipt_semantics() {
    let (directory, journal) = fixture();
    journal
        .conn
        .execute_batch(
            "CREATE TRIGGER unexpected AFTER INSERT ON receipts BEGIN DELETE FROM locks; END;",
        )
        .unwrap();
    drop(journal);
    assert!(matches!(
        reopen(directory.path()),
        Err(Failure::StateNotDurable)
    ));
}

#[test]
fn symlink_and_hardlink_journal_auxiliaries_refuse() {
    let (directory, journal) = fixture();
    drop(journal);
    let outside = directory.path().join("sentinel");
    std::fs::write(&outside, b"preserve").unwrap();
    let name = directory.path().join("mounts.sqlite-journal");
    std::os::unix::fs::symlink(&outside, &name).unwrap();
    assert!(matches!(
        reopen(directory.path()),
        Err(Failure::StateNotDurable)
    ));
    assert_eq!(std::fs::read(&outside).unwrap(), b"preserve");
    std::fs::remove_file(&name).unwrap();
    std::fs::hard_link(&outside, &name).unwrap();
    assert!(matches!(
        reopen(directory.path()),
        Err(Failure::StateNotDurable)
    ));
}

#[test]
fn process_lock_is_shared_with_directory_preparation() {
    let (directory, journal) = fixture();
    let lock = rustix::fs::open(
        directory.path().join(LOCK),
        OFlags::RDWR | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .unwrap();
    assert_eq!(
        rustix::fs::flock(lock.as_fd(), FlockOperation::NonBlockingLockExclusive),
        Err(rustix::io::Errno::WOULDBLOCK)
    );
    drop(journal);
    rustix::fs::flock(lock.as_fd(), FlockOperation::NonBlockingLockExclusive).unwrap();
}

#[test]
fn replay_cannot_mint_second_helper_release_permit() {
    let (_directory, mut journal) = fixture();
    let prepared = journal.begin(&plan(), 100).unwrap();
    let pending = journal.arm(&prepared).unwrap();
    // Journal identity fixture only. It creates no live supervisor/termination
    // owner; a real gate would also require that exact genuine owned identity.
    let helper = MountHelperIdentity {
        pid: 23,
        start_time_ticks: 1000,
        mount_namespace_inode: 50,
        cgroup: "/system.slice/limeos-storage-mounts.service".into(),
        image_digest: "f".repeat(64),
    };
    let (bound, permit) = journal.bind_helper(&pending, helper.clone()).unwrap();
    assert_eq!(permit.action(), prepared.plan.action);
    assert_eq!(permit.digest(), prepared.digest);
    assert_eq!(permit.helper(), &helper);
    assert_eq!(permit.expires_at(), prepared.plan.expires_at);
    assert!(matches!(
        journal.bind_helper(&pending, helper),
        Err(Failure::Conflict)
    ));
    drop(permit);
    assert_eq!(
        journal
            .lookup(&bound.plan.action, Some(&bound.digest))
            .unwrap(),
        Some(bound)
    );
}

#[test]
fn imported_prepared_or_pending_receipt_cannot_mint_release_cursor() {
    for armed in [false, true] {
        let (directory, mut journal) = fixture();
        let prepared = journal.begin(&plan(), 100).unwrap();
        let stored = if armed {
            journal.arm(&prepared).unwrap()
        } else {
            prepared
        };
        let body = encoded(&stored).unwrap();
        drop(journal);
        let mut journal = reopen(directory.path()).unwrap();
        let imported: StorageMountReceipt = serde_json::from_str(&body).unwrap();
        assert!(matches!(journal.arm(&imported), Err(Failure::Conflict)));
        assert!(matches!(
            journal.bind_helper(&imported, helper()),
            Err(Failure::Conflict)
        ));
        assert_eq!(journal.begin(&plan(), 101).unwrap(), imported);
        assert!(matches!(
            journal.bind_helper(&imported, helper()),
            Err(Failure::Conflict)
        ));
    }
}

#[test]
fn permit_requires_exact_action_digest_helper_and_original_expiry() {
    let (_directory, mut journal) = fixture();
    let prepared = journal.begin(&plan(), 100).unwrap();
    let pending = journal.arm(&prepared).unwrap();
    let (_bound, permit) = journal.bind_helper(&pending, helper()).unwrap();
    permit
        .check_binding(&plan().action, &prepared.digest, &helper(), 399)
        .unwrap();
    assert_eq!(
        permit.check_binding(&"f".repeat(64), &prepared.digest, &helper(), 100),
        Err(Failure::Conflict)
    );
    assert_eq!(
        permit.check_binding(&plan().action, &"e".repeat(64), &helper(), 100),
        Err(Failure::Conflict)
    );
    let mut changed = helper();
    changed.start_time_ticks += 1;
    assert_eq!(
        permit.check_binding(&plan().action, &prepared.digest, &changed, 100),
        Err(Failure::Conflict)
    );
    assert_eq!(
        permit.check_binding(&plan().action, &prepared.digest, &helper(), 400),
        Err(Failure::Conflict)
    );
}

#[test]
fn failed_durable_helper_binding_clears_cursor_without_creating_permit() {
    let (_directory, mut journal) = fixture();
    let prepared = journal.begin(&plan(), 100).unwrap();
    let pending = journal.arm(&prepared).unwrap();
    journal.conn.execute_batch("CREATE TRIGGER inject_failure BEFORE UPDATE ON receipts BEGIN SELECT RAISE(ABORT,'injected publication failure'); END;").unwrap();
    assert!(matches!(
        journal.bind_helper(&pending, helper()),
        Err(Failure::StateNotDurable)
    ));
    journal
        .conn
        .execute_batch("DROP TRIGGER inject_failure;")
        .unwrap();
    assert_eq!(
        journal
            .lookup(&pending.plan.action, Some(&pending.digest))
            .unwrap(),
        Some(pending.clone())
    );
    assert!(matches!(
        journal.bind_helper(&pending, helper()),
        Err(Failure::Conflict)
    ));
}

#[test]
fn accepted_reconciliation_changes_only_verified_state_without_expiry_extension() {
    let (directory, mut journal) = fixture();
    let accepted = accepted_fixture(&mut journal);
    let original_plan = accepted.plan.clone();
    drop(journal);
    let mut journal = reopen(directory.path()).unwrap();
    let mut wrong = accepted.after.clone().unwrap();
    wrong.mount_id += 1;
    wrong.target.mount_id += 1;
    assert!(matches!(
        journal.verify_accepted(&accepted, &wrong),
        Err(Failure::Conflict)
    ));
    assert_eq!(
        journal
            .lookup(&accepted.plan.action, Some(&accepted.digest))
            .unwrap(),
        Some(accepted.clone())
    );
    let verified = journal
        .verify_accepted(&accepted, accepted.after.as_ref().unwrap())
        .unwrap();
    assert_eq!(verified.state, MountExecutionState::Verified);
    assert_eq!(verified.plan, original_plan);
    assert_eq!(verified.quiescence, accepted.quiescence);
    assert_eq!(
        journal
            .conn
            .query_row("SELECT count(*) FROM locks", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        0
    );
    assert!(matches!(
        journal.verify_accepted(&accepted, accepted.after.as_ref().unwrap()),
        Err(Failure::Conflict)
    ));
}

#[test]
fn unknown_or_pending_never_refines_through_accepted_only_path() {
    let (_directory, mut journal) = fixture();
    let accepted = accepted_fixture(&mut journal);
    let mut unknown = accepted.clone();
    unknown.state = MountExecutionState::OutcomeUnknown;
    journal.record(&accepted, &unknown).unwrap();
    assert!(matches!(
        journal.verify_accepted(&unknown, unknown.after.as_ref().unwrap()),
        Err(Failure::Conflict)
    ));
    assert_eq!(journal.begin(&unknown.plan, 500).unwrap(), unknown);
}
