use super::*;
use limeos_domain::*;

fn setup() -> (tempfile::TempDir, Store, Principal, StorageMountInspection) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    store.issue_bootstrap("bootstrap", 100).unwrap();
    store
        .enroll("bootstrap", "alice", "$argon2id$fixture", 101)
        .unwrap();
    let principal = store.login_record("alice").unwrap().unwrap().principal;
    let block = StorageBlockDevice {
        major: 8,
        minor: 16,
    };
    let identity = DirectoryIdentity {
        major: 8,
        minor: 1,
        mount_id: 1,
        inode: 2,
        mode: 0o40755,
        uid: 0,
        gid: 0,
    };
    let inspection = StorageMountInspection {
        target_id: "data".into(),
        device: StorageDevice {
            id: "data".into(),
            role: StorageRole::Data,
            filesystem_uuid: "uuid-data".into(),
            filesystem: StorageFilesystem::Ext4,
            mountpoint: "/mnt/data".into(),
            serial: None,
        },
        expected: StorageInventory {
            host_root_mount_id: 1,
            topology_digest: "a".repeat(64),
            mounts_digest: "b".repeat(64),
            fstab_digest: "c".repeat(64),
            fstab_entries: vec![],
            devices: vec![StorageObservedDevice {
                device: block,
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
            parent: identity.clone(),
            existing: Some(DirectoryIdentity {
                inode: 3,
                ..identity
            }),
        },
        physical_devices: vec![block],
        consumer_digest: "d".repeat(64),
        observed_at: 200,
    };
    (dir, store, principal, inspection)
}
fn queued(
    store: &mut Store,
    principal: &Principal,
    current: &StorageMountInspection,
) -> (PlannedStorageMount, String, StorageMountJob) {
    let proposal = store.plan_storage_mount(principal, current, 200).unwrap();
    let approval = store
        .approve_storage_mount(principal, &proposal.plan.id, &proposal.digest, current, 201)
        .unwrap();
    let job = store
        .queue_storage_mount(
            principal,
            "mount-request",
            &proposal,
            &approval.token,
            current,
            202,
        )
        .unwrap();
    (proposal, approval.token, job)
}
// These are explicitly synthetic authenticated-receipt facts for Store tests.
// Genuine helper/process/physical authority is tested at the executor boundary.
fn verified(job: &StorageMountJob) -> (StorageMountReceipt, StorageMountedSnapshot) {
    let after = StorageMountedSnapshot {
        filesystem_uuid: job.plan.effect.device.filesystem_uuid.clone(),
        device: job.plan.effect.physical_devices[0],
        mountpoint: job.plan.effect.device.mountpoint.clone(),
        filesystem: "ext4".into(),
        filesystem_root: "/".into(),
        writable: true,
        nosuid: true,
        nodev: true,
        mount_id: 20,
        host_root_mount_id: 1,
        target: DirectoryIdentity {
            major: 8,
            minor: 16,
            mount_id: 20,
            inode: 2,
            mode: 0o40755,
            uid: 1000,
            gid: 1000,
        },
    };
    let receipt = StorageMountReceipt {
        plan: job.plan.effect.clone(),
        digest: limeos_identity::digest(&json(&job.plan.effect).unwrap()),
        state: MountExecutionState::Verified,
        after: Some(after.clone()),
        quiescence: MountEffectQuiescence::Reaped {
            helper: MountHelperIdentity {
                pid: 42,
                start_time_ticks: 123,
                mount_namespace_inode: 5,
                cgroup: "/system.slice/limeos-storage-mounts.service".into(),
                image_digest: "e".repeat(64),
            },
            outcome: MountHelperOutcome::Exited { code: 0 },
        },
    };
    (receipt, after)
}
fn locks(store: &Store, id: &str) -> i64 {
    store
        .conn
        .query_row(
            "SELECT count(*) FROM resource_locks WHERE job=?",
            [id],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn approval_replay_and_reads_do_not_touch_hardware_or_write_authority() {
    let (_dir, mut store, principal, current) = setup();
    let (proposal, approval, job) = queued(&mut store, &principal, &current);
    let changes = store.conn.total_changes();
    assert_eq!(
        store
            .replay_storage_mount(&principal, "mount-request", &proposal, &approval)
            .unwrap(),
        Some(job.clone())
    );
    assert_eq!(
        store.storage_mount_jobs(&principal).unwrap(),
        vec![job.clone()]
    );
    assert_eq!(store.storage_mount_job(&principal, &job.id).unwrap(), job);
    assert_eq!(store.conn.total_changes(), changes);
    assert!(
        store
            .replay_storage_mount(&principal, "mount-request", &proposal, &"f".repeat(64))
            .is_err()
    );
    let mut stale = current.clone();
    stale.consumer_digest = "f".repeat(64);
    assert!(
        store
            .claim_storage_mount(&principal, &job.id, &stale, 203)
            .is_err()
    );
    assert_eq!(locks(&store, &job.id), 0);
}

#[test]
fn dispatch_claims_are_complete_and_cancel_is_only_before_dispatch() {
    let (_dir, mut store, principal, current) = setup();
    let (_, _, job) = queued(&mut store, &principal, &current);
    let running = store
        .claim_storage_mount(&principal, &job.id, &current, 203)
        .unwrap();
    assert_eq!(
        locks(&store, &job.id) as usize,
        job.plan.effect.resources().len()
    );
    assert_eq!(
        store.cancel_storage_mount_job(&principal, &job.id, 204),
        Err(Error(ErrorCode::Conflict))
    );
    assert!(
        store
            .transition(&job.id, JobState::Running, JobState::Succeeded, 204)
            .is_err()
    );
    let (receipt, after) = verified(&running);
    let mut accepted = receipt.clone();
    accepted.state = MountExecutionState::EffectAccepted;
    assert_eq!(
        store
            .record_storage_mount_result(&running, &accepted, Some(&after), None, 204)
            .unwrap(),
        JobState::NeedsIntervention
    );
    assert!(locks(&store, &job.id) > 0);
    assert_eq!(
        store
            .record_storage_mount_result(&running, &receipt, Some(&after), None, 205)
            .unwrap(),
        JobState::Succeeded
    );
    assert_eq!(locks(&store, &job.id), 0);
}

#[test]
fn wrong_digest_missing_verification_and_pending_helper_never_release_claims() {
    let (_dir, mut store, principal, current) = setup();
    let (_, _, job) = queued(&mut store, &principal, &current);
    let running = store
        .claim_storage_mount(&principal, &job.id, &current, 203)
        .unwrap();
    let (mut receipt, after) = verified(&running);
    let digest = receipt.digest.clone();
    receipt.digest = "f".repeat(64);
    assert!(
        store
            .record_storage_mount_result(&running, &receipt, Some(&after), None, 204)
            .is_err()
    );
    receipt.digest = digest;
    assert_eq!(
        store
            .record_storage_mount_result(&running, &receipt, None, None, 204)
            .unwrap(),
        JobState::NeedsIntervention
    );
    receipt.state = MountExecutionState::OutcomeUnknown;
    receipt.after = None;
    receipt.quiescence = MountEffectQuiescence::Pending { helper: None };
    assert_eq!(
        store
            .record_storage_mount_result(&running, &receipt, None, Some(&current), 205)
            .unwrap(),
        JobState::NeedsIntervention
    );
    assert!(locks(&store, &job.id) > 0);
}

#[test]
fn independent_mount_flags_and_identity_are_required_before_success() {
    let (_dir, mut store, principal, current) = setup();
    let (_, _, job) = queued(&mut store, &principal, &current);
    let running = store
        .claim_storage_mount(&principal, &job.id, &current, 203)
        .unwrap();
    let (receipt, mut after) = verified(&running);
    after.nodev = false;
    assert!(
        store
            .record_storage_mount_result(&running, &receipt, Some(&after), None, 204)
            .is_err()
    );
    after.nodev = true;
    after.filesystem_uuid = "replacement".into();
    assert!(
        store
            .record_storage_mount_result(&running, &receipt, Some(&after), None, 204)
            .is_err()
    );
    assert!(locks(&store, &job.id) > 0);
}

#[test]
fn restart_keeps_uncertain_barriers_and_post_expiry_no_effect_requires_quiescence() {
    let (dir, mut store, principal, current) = setup();
    let (_, _, job) = queued(&mut store, &principal, &current);
    store
        .claim_storage_mount(&principal, &job.id, &current, 203)
        .unwrap();
    drop(store);
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    assert_eq!(
        store.storage_mount_job(&principal, &job.id).unwrap().state,
        JobState::NeedsIntervention
    );
    assert!(locks(&store, &job.id) > 0);
    let receipt = StorageMountReceipt {
        plan: job.plan.effect.clone(),
        digest: limeos_identity::digest(&json(&job.plan.effect).unwrap()),
        state: MountExecutionState::PreconditionChanged,
        quiescence: MountEffectQuiescence::NotLaunched {},
        after: None,
    };
    let mut fresh = current;
    fresh.observed_at = 1000;
    assert_eq!(
        store
            .record_storage_mount_result(&job, &receipt, None, Some(&fresh), 1000)
            .unwrap(),
        JobState::PreconditionChanged
    );
    assert_eq!(locks(&store, &job.id), 0);
}

#[test]
fn queued_cancel_and_revoked_principal_cannot_dispatch_or_list_private_paths() {
    let (_dir, mut store, principal, current) = setup();
    let (_, _, job) = queued(&mut store, &principal, &current);
    store
        .cancel_storage_mount_job(&principal, &job.id, 203)
        .unwrap();
    assert_eq!(
        store.storage_mount_job(&principal, &job.id).unwrap().state,
        JobState::Canceled
    );
    assert!(
        store
            .claim_storage_mount(&principal, &job.id, &current, 204)
            .is_err()
    );
    store
        .conn
        .execute(
            "UPDATE users SET grant_revision=grant_revision+1 WHERE id=?",
            [&principal.id],
        )
        .unwrap();
    assert!(store.storage_mount_jobs(&principal).is_err());
}

#[test]
fn schema9_upgrade_is_atomic_with_startup_audit_and_preserves_users() {
    let (dir, store, principal, _) = setup();
    super::super::remove_v10_schema(&store.conn);
    store.conn.pragma_update(None, "user_version", 9).unwrap();
    store
        .conn
        .execute("UPDATE meta SET audit_bytes=?", [AUDIT_LIMIT])
        .unwrap();
    drop(store);
    assert!(matches!(
        Store::open(&dir.path().join("core.sqlite")),
        Err(Error(ErrorCode::StateNotDurable))
    ));
    let conn = Connection::open(dir.path().join("core.sqlite")).unwrap();
    assert_eq!(
        conn.pragma_query_value::<u32, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        9
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name='storage_mount_plans'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    conn.execute("UPDATE meta SET audit_bytes=0", []).unwrap();
    drop(conn);
    let store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    assert_eq!(
        store.login_record("alice").unwrap().unwrap().principal,
        principal
    );
    assert_eq!(
        store
            .conn
            .pragma_query_value::<u32, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        SCHEMA_VERSION
    );
}

#[test]
fn schema10_upgrade_preserves_mount_plans_results_and_uncertain_resource_claims() {
    let (dir, mut store, principal, current) = setup();
    let (proposal, _, job) = queued(&mut store, &principal, &current);
    let running = store
        .claim_storage_mount(&principal, &job.id, &current, 203)
        .unwrap();
    let (receipt, after) = verified(&running);
    store
        .record_storage_mount_result(&running, &receipt, Some(&after), None, 204)
        .unwrap();
    let receipt_before: String = store
        .conn
        .query_row(
            "SELECT receipt FROM storage_mount_results WHERE job=?",
            [&job.id],
            |r| r.get(0),
        )
        .unwrap();
    let unknown = store
        .queue_job(
            &principal,
            "uncertain-health",
            &Intent::HealthProbe {
                resource: "system".into(),
            },
            300,
            205,
        )
        .unwrap();
    store
        .transition(&unknown, JobState::Queued, JobState::Running, 206)
        .unwrap();
    store
        .transition(&unknown, JobState::Running, JobState::OutcomeUnknown, 207)
        .unwrap();
    super::super::remove_v11_schema(&store.conn);
    store.conn.pragma_update(None, "user_version", 10).unwrap();
    drop(store);
    let reopened = Store::open(&dir.path().join("core.sqlite")).unwrap();
    assert_eq!(
        reopened
            .storage_mount_plan(&principal, &proposal.plan.id)
            .unwrap(),
        proposal
    );
    assert_eq!(
        reopened
            .storage_mount_job(&principal, &job.id)
            .unwrap()
            .state,
        JobState::Succeeded
    );
    assert_eq!(
        reopened
            .conn
            .query_row(
                "SELECT receipt FROM storage_mount_results WHERE job=?",
                [&job.id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        receipt_before
    );
    assert_eq!(reopened.job_state(&unknown).unwrap(), "outcome_unknown");
    assert_eq!(locks(&reopened, &unknown), 1);
    assert_eq!(
        reopened
            .conn
            .query_row(
                "SELECT resource FROM job_resources WHERE job=?",
                [&unknown],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "system"
    );
    assert_eq!(
        reopened
            .conn
            .query_row("SELECT count(*) FROM automation_schedules", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn failed_terminal_audit_rolls_back_result_state_and_resource_release() {
    let (_dir, mut store, principal, current) = setup();
    let (_, _, job) = queued(&mut store, &principal, &current);
    let running = store
        .claim_storage_mount(&principal, &job.id, &current, 203)
        .unwrap();
    let (receipt, after) = verified(&running);
    store.conn.execute_batch("CREATE TRIGGER reject_mount_audit BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'injected audit refusal'); END;").unwrap();
    assert!(
        store
            .record_storage_mount_result(&running, &receipt, Some(&after), None, 204)
            .is_err()
    );
    assert_eq!(store.job_state(&job.id).unwrap(), "running");
    assert_eq!(
        locks(&store, &job.id) as usize,
        job.plan.effect.resources().len()
    );
    assert_eq!(
        store
            .conn
            .query_row("SELECT count(*) FROM storage_mount_results", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
}

#[test]
fn startup_rejects_an_extra_unapproved_physical_claim() {
    let (dir, mut store, principal, current) = setup();
    let (_, _, job) = queued(&mut store, &principal, &current);
    store
        .conn
        .execute(
            "INSERT INTO job_resources VALUES(?,?)",
            params![job.id, "storage:device:8:99"],
        )
        .unwrap();
    drop(store);
    assert!(matches!(
        Store::open(&dir.path().join("core.sqlite")),
        Err(Error(ErrorCode::StateNotDurable))
    ));
}

#[test]
fn startup_requires_retained_proof_for_success_and_dispatched_precondition_change() {
    for success in [true, false] {
        let (dir, mut store, principal, mut current) = setup();
        let (_, _, job) = queued(&mut store, &principal, &current);
        let running = store
            .claim_storage_mount(&principal, &job.id, &current, 203)
            .unwrap();
        let (mut receipt, after) = verified(&running);
        let next = if success {
            store
                .record_storage_mount_result(&running, &receipt, Some(&after), None, 204)
                .unwrap()
        } else {
            current.observed_at = 204;
            receipt.state = MountExecutionState::PreconditionChanged;
            receipt.quiescence = MountEffectQuiescence::NotLaunched {};
            receipt.after = None;
            store
                .record_storage_mount_result(&running, &receipt, None, Some(&current), 204)
                .unwrap()
        };
        assert_eq!(
            next,
            if success {
                JobState::Succeeded
            } else {
                JobState::PreconditionChanged
            }
        );
        assert_eq!(locks(&store, &job.id), 0);
        assert_eq!(
            store
                .conn
                .query_row(
                    "SELECT count(*) FROM storage_mount_results WHERE job=?",
                    [&job.id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        let terminal: EventKind = parse(
            &store
                .conn
                .query_row(
                    "SELECT kind FROM events WHERE job=? ORDER BY cursor DESC LIMIT 1",
                    [&job.id],
                    |r| r.get::<_, String>(0),
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(terminal, EventKind::JobTransition { state: next });
        drop(store);
        // A legitimate terminal result reopens before the corruption is applied.
        let store = Store::open(&dir.path().join("core.sqlite")).unwrap();
        store
            .conn
            .execute("DELETE FROM storage_mount_results WHERE job=?", [&job.id])
            .unwrap();
        drop(store);
        assert!(matches!(
            Store::open(&dir.path().join("core.sqlite")),
            Err(Error(ErrorCode::StateNotDurable))
        ));
    }
}

#[test]
fn never_dispatched_precondition_change_needs_no_effect_receipt() {
    let (dir, mut store, principal, current) = setup();
    let (_, _, job) = queued(&mut store, &principal, &current);
    store
        .transition(
            &job.id,
            JobState::Queued,
            JobState::PreconditionChanged,
            203,
        )
        .unwrap();
    assert_eq!(locks(&store, &job.id), 0);
    drop(store);
    let store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    assert_eq!(
        store.storage_mount_job(&principal, &job.id).unwrap().state,
        JobState::PreconditionChanged
    );
}
