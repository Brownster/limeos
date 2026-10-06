use super::*;
use limeos_domain::{DirectoryIdentity, TargetEvidence};
fn setup() -> (
    tempfile::TempDir,
    Store,
    Principal,
    StorageContract,
    StorageTargetSnapshot,
) {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("core.sqlite")).unwrap();
    s.issue_bootstrap("bootstrap", 100).unwrap();
    s.enroll("bootstrap", "alice", "$argon2id$fixture", 101)
        .unwrap();
    let p = s.login_record("alice").unwrap().unwrap().principal;
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/storage-contracts.json"
    ))
    .unwrap();
    let contract: StorageContract =
        serde_json::from_value(fixtures["cases"][0]["contract"].clone()).unwrap();
    let inventory = limeos_domain::StorageInventory {
        host_root_mount_id: 1,
        topology_digest: "a".repeat(64),
        mounts_digest: "b".repeat(64),
        fstab_digest: "c".repeat(64),
        fstab_entries: vec![],
        devices: vec![limeos_domain::StorageObservedDevice {
            device: limeos_domain::StorageBlockDevice { major: 8, minor: 1 },
            filesystem_uuid: contract.devices[0].filesystem_uuid.clone(),
            filesystem: "ext4".into(),
            serial: None,
            boot_backing: false,
            in_use_as_swap: false,
            mounts: vec![],
        }],
    };
    let snapshot = StorageTargetSnapshot {
        inventory,
        targets: vec![TargetEvidence {
            mountpoint: contract.devices[0].mountpoint.clone(),
            parent: DirectoryIdentity {
                major: 8,
                minor: 0,
                mount_id: 1,
                inode: 5,
                mode: 0o40755,
                uid: 0,
                gid: 0,
            },
            existing: None,
        }],
    };
    (dir, s, p, contract, snapshot)
}
fn approved(
    s: &mut Store,
    p: &Principal,
    c: &StorageContract,
    current: &StorageTargetSnapshot,
) -> (PlannedStorageTargets, String) {
    let proposal = s.plan_storage_targets(p, c, current, 200).unwrap();
    let approval = s
        .approve_storage_targets(p, &proposal.plan.id, &proposal.digest, current, 201)
        .unwrap();
    (proposal, approval.token)
}
#[test]
fn a_full_waiting_queue_cannot_starve_the_receipt_that_holds_its_claims() {
    let (_dir, mut s, p, c, current) = setup();
    let (proposal, token) = approved(&mut s, &p, &c, &current);
    let job = s
        .queue_storage_targets(&p, "running", &proposal, &token, &current, 202)
        .unwrap();
    assert!(s.storage_target_resources_available(&job.id).unwrap());
    s.claim_storage_targets(&p, &job.id, &current, 203).unwrap();
    s.transition(&job.id, JobState::Running, JobState::NeedsIntervention, 204)
        .unwrap();
    for n in 0..40 {
        let (proposal, token) = approved(&mut s, &p, &c, &current);
        let waiting = s
            .queue_storage_targets(
                &p,
                &format!("waiting-{n}"),
                &proposal,
                &token,
                &current,
                202,
            )
            .unwrap();
        assert!(!s.storage_target_resources_available(&waiting.id).unwrap());
    }
    let changes = s.conn.total_changes();
    let candidates = s.storage_target_candidates().unwrap();
    assert_eq!(candidates.len(), 32);
    assert_eq!(candidates[0].1.id, job.id);
    assert_eq!(candidates[0].1.state, JobState::NeedsIntervention);
    assert_eq!(s.conn.total_changes(), changes);
}
#[test]
fn preview_nonce_cannot_queue_targets_and_authority_and_audit_are_required() {
    let (_dir, mut s, p, c, current) = setup();
    let preview = s
        .plan_storage(
            &p,
            &limeos_domain::StorageSetupInput {
                contract: c.clone(),
                inventory_digest: limeos_identity::digest(&json(&current.inventory).unwrap()),
                timeout_seconds: 10,
            },
            &current.inventory,
            200,
        )
        .unwrap();
    let preview_approval = s
        .approve_storage(
            &p,
            &preview.plan.id,
            &preview.digest,
            &current.inventory,
            201,
        )
        .unwrap();
    let proposal = s.plan_storage_targets(&p, &c, &current, 200).unwrap();
    assert_eq!(proposal.plan.version, 2);
    assert_eq!(
        s.queue_storage_targets(
            &p,
            "queue",
            &proposal,
            &preview_approval.token,
            &current,
            202
        )
        .unwrap_err()
        .0,
        ErrorCode::Forbidden
    );
    assert_eq!(
        s.job_state(&proposal.plan.preparation.action)
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
    let mut other = p.clone();
    other.role = Role::Operator;
    assert_eq!(
        s.plan_storage_targets(&other, &c, &current, 200)
            .unwrap_err()
            .0,
        ErrorCode::Expired
    );
    s.conn
        .execute("UPDATE meta SET audit_bytes=?", [AUDIT_LIMIT])
        .unwrap();
    assert_eq!(
        s.approve_storage_targets(&p, &proposal.plan.id, &proposal.digest, &current, 201)
            .err()
            .unwrap()
            .0,
        ErrorCode::StateNotDurable
    );
    assert!(
        s.conn
            .query_row(
                "SELECT approval_digest IS NULL FROM storage_target_plans",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
    );
}
#[test]
fn fresh_targets_uuid_fstab_namespace_and_owner_block_approval_queue_and_claim() {
    let (_dir, mut s, p, c, current) = setup();
    let (proposal, approval) = approved(&mut s, &p, &c, &current);
    for name in ["fstab", "uuid", "parent", "target", "namespace"] {
        let mut changed = current.clone();
        match name {
            "fstab" => changed.inventory.fstab_digest = "d".repeat(64),
            "uuid" => changed.inventory.devices[0].filesystem_uuid = "replacement".into(),
            "parent" => changed.targets[0].parent.inode += 1,
            "namespace" => changed.inventory.host_root_mount_id += 1,
            _ => changed.targets[0].existing = Some(changed.targets[0].parent.clone()),
        }
        assert_eq!(
            s.approve_storage_targets(&p, &proposal.plan.id, &proposal.digest, &changed, 202)
                .err()
                .unwrap()
                .0,
            ErrorCode::Conflict,
            "{name}"
        );
        assert_eq!(
            s.queue_storage_targets(&p, "queue", &proposal, &approval, &changed, 202)
                .unwrap_err()
                .0,
            ErrorCode::Conflict,
            "{name}"
        );
    }
    let job = s
        .queue_storage_targets(&p, "queue", &proposal, &approval, &current, 202)
        .unwrap();
    let mut changed = current.clone();
    changed.targets[0].parent.inode += 1;
    assert_eq!(
        s.claim_storage_targets(&p, &job.id, &changed, 203)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert_eq!(s.job_state(&job.id).unwrap(), "queued");
    s.revise_grants(&p.id, Role::Administrator, &[], 203)
        .unwrap();
    assert_eq!(
        s.claim_storage_targets(&p, &job.id, &current, 204)
            .unwrap_err()
            .0,
        ErrorCode::Expired
    );
}
#[test]
fn replay_is_pure_after_expiry_and_consumed_approval_cannot_authorize_another_key() {
    let (dir, mut s, p, c, current) = setup();
    let (proposal, approval) = approved(&mut s, &p, &c, &current);
    let job = s
        .queue_storage_targets(&p, "queue", &proposal, &approval, &current, 202)
        .unwrap();
    let claims: i64 = s
        .conn
        .query_row(
            "SELECT count(*) FROM job_resources WHERE job=?",
            [&job.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(claims, 3);
    assert_eq!(
        s.conn
            .query_row("SELECT count(*) FROM resource_locks", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        s.queue_storage_targets(&p, "another", &proposal, &approval, &current, 203)
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    drop(s);
    let s = Store::open(&dir.path().join("core.sqlite")).unwrap();
    let before = s.conn.total_changes();
    assert_eq!(
        s.replay_storage_targets(&p, "queue", &proposal, &approval)
            .unwrap()
            .unwrap(),
        job
    );
    assert_eq!(
        s.storage_target_plan(&p, &proposal.plan.id).unwrap(),
        proposal
    );
    assert_eq!(before, s.conn.total_changes());
    assert!(
        s.replay_storage_targets(&p, "queue", &proposal, &"f".repeat(64))
            .is_err()
    );
}
#[test]
fn shared_claims_and_verified_bound_receipts_control_release_and_restart() {
    let (dir, mut s, p, c, current) = setup();
    let (proposal, approval) = approved(&mut s, &p, &c, &current);
    let job = s
        .queue_storage_targets(&p, "first", &proposal, &approval, &current, 202)
        .unwrap();
    let (next, nonce) = approved(&mut s, &p, &c, &current);
    let waiting = s
        .queue_storage_targets(&p, "second", &next, &nonce, &current, 202)
        .unwrap();
    let job = s.claim_storage_targets(&p, &job.id, &current, 203).unwrap();
    assert_eq!(
        s.claim_storage_targets(&p, &waiting.id, &current, 203)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert_eq!(
        s.cancel_storage_target_job(&p, &job.id, 203).unwrap_err().0,
        ErrorCode::Conflict
    );
    s.transition(&job.id, JobState::Running, JobState::Verifying, 203)
        .unwrap();
    assert_eq!(
        s.transition(&job.id, JobState::Verifying, JobState::Succeeded, 203)
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    drop(s);
    let mut s = Store::open(&dir.path().join("core.sqlite")).unwrap();
    assert_eq!(s.job_state(&job.id).unwrap(), "needs_intervention");
    assert_eq!(
        s.conn
            .query_row(
                "SELECT count(*) FROM resource_locks WHERE job=?",
                [&job.id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        3
    );
    let mut after = current.clone();
    let mut created = current.targets[0].parent.clone();
    created.inode = 10;
    after.targets[0].existing = Some(created);
    let mut receipt = TargetPreparationReceipt {
        plan: job.plan.preparation.clone(),
        digest: limeos_identity::digest(&json(&job.plan.preparation).unwrap()),
        state: PreparationState::Verified,
        after: after.targets.clone(),
    };
    receipt.digest = "e".repeat(64);
    assert_eq!(
        s.record_storage_target_result(&job, &receipt, Some(&after), 900)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    receipt.digest = limeos_identity::digest(&json(&job.plan.preparation).unwrap());
    s.conn
        .execute("UPDATE meta SET audit_bytes=?", [AUDIT_LIMIT])
        .unwrap();
    assert_eq!(
        s.record_storage_target_result(&job, &receipt, Some(&after), 900)
            .unwrap_err()
            .0,
        ErrorCode::StateNotDurable
    );
    assert_eq!(
        s.conn
            .query_row(
                "SELECT count(*) FROM resource_locks WHERE job=?",
                [&job.id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        3
    );
    s.conn.execute("UPDATE meta SET audit_bytes=0", []).unwrap();
    assert_eq!(
        s.record_storage_target_result(&job, &receipt, None, 900)
            .unwrap(),
        JobState::NeedsIntervention
    );
    assert_eq!(
        s.record_storage_target_result(&job, &receipt, Some(&after), 900)
            .unwrap(),
        JobState::Succeeded
    );
    let before = s.conn.total_changes();
    assert_eq!(
        s.record_storage_target_result(&job, &receipt, Some(&after), 901)
            .unwrap(),
        JobState::Succeeded
    );
    assert_eq!(before, s.conn.total_changes());
    assert_eq!(
        s.conn
            .query_row(
                "SELECT count(*) FROM resource_locks WHERE job=?",
                [&job.id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}
#[test]
fn lost_queued_dependency_refuses_startup_without_repair_or_recovery_writes() {
    let (dir, mut s, p, c, current) = setup();
    let (proposal, approval) = approved(&mut s, &p, &c, &current);
    let job = s
        .queue_storage_targets(&p, "queue", &proposal, &approval, &current, 202)
        .unwrap();
    s.conn
        .execute(
            "DELETE FROM job_resources WHERE job=? AND resource LIKE 'storage:uuid:%'",
            [&job.id],
        )
        .unwrap();
    let generation = s.generation;
    drop(s);
    assert_eq!(
        Store::open(&dir.path().join("core.sqlite"))
            .err()
            .unwrap()
            .0,
        ErrorCode::StateNotDurable
    );
    let conn = Connection::open(dir.path().join("core.sqlite")).unwrap();
    assert_eq!(
        conn.query_row("SELECT generation FROM meta", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        generation
    );
    assert_eq!(
        conn.query_row("SELECT state FROM jobs WHERE id=?", [&job.id], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "queued"
    );
}
#[test]
fn genuine_v7_schema_upgrade_preserves_preview_bytes_and_dependent_claims_or_rolls_back() {
    for fail in [false, true] {
        let (dir, mut s, p, c, current) = setup();
        let preview = s
            .plan_storage(
                &p,
                &limeos_domain::StorageSetupInput {
                    contract: c,
                    inventory_digest: limeos_identity::digest(&json(&current.inventory).unwrap()),
                    timeout_seconds: 10,
                },
                &current.inventory,
                200,
            )
            .unwrap();
        let approval = s
            .approve_storage(
                &p,
                &preview.plan.id,
                &preview.digest,
                &current.inventory,
                201,
            )
            .unwrap();
        let id = s
            .queue_job(
                &p,
                "legacy",
                &Intent::HealthProbe {
                    resource: "health:storage".into(),
                },
                1000,
                202,
            )
            .unwrap();
        for resource in [
            "storage:configuration",
            "storage:uuid:legacy",
            "storage:mount:/mnt/Legacy",
        ] {
            s.conn
                .execute(
                    "INSERT INTO job_resources VALUES(?,?)",
                    params![id, resource],
                )
                .unwrap();
        }
        s.transition(&id, JobState::Queued, JobState::Running, 203)
            .unwrap();
        let before: (String, String) = s
            .conn
            .query_row("SELECT intent,digest FROM jobs WHERE id=?", [&id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        s.conn.execute_batch("DROP TABLE storage_target_results; DROP TABLE storage_target_plans; PRAGMA user_version=7;").unwrap();
        if fail {
            s.conn
                .execute_batch("CREATE INDEX storage_target_plans_expiry ON users(username);")
                .unwrap();
        }
        let generation = s.generation;
        drop(s);
        if fail {
            assert_eq!(
                Store::open(&dir.path().join("core.sqlite"))
                    .err()
                    .unwrap()
                    .0,
                ErrorCode::StateNotDurable
            );
            let conn = Connection::open(dir.path().join("core.sqlite")).unwrap();
            assert_eq!(
                conn.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                    .unwrap(),
                7
            );
            assert_eq!(
                conn.query_row("SELECT generation FROM meta", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                generation
            );
            assert_eq!(
                conn.query_row(
                    "SELECT count(*) FROM sqlite_schema WHERE name='storage_target_plans'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
            assert_eq!(
                conn.query_row(
                    "SELECT count(*) FROM resource_locks WHERE job=?",
                    [&id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                4
            );
        } else {
            let s = Store::open(&dir.path().join("core.sqlite")).unwrap();
            assert_eq!(s.storage_plan(&p, &preview.plan.id, 204).unwrap(), preview);
            assert_eq!(
                s.conn
                    .query_row("SELECT approval_digest FROM storage_plans", [], |r| r
                        .get::<_, String>(0))
                    .unwrap(),
                limeos_identity::digest(&approval.token)
            );
            assert_eq!(
                s.conn
                    .query_row("SELECT intent,digest FROM jobs WHERE id=?", [&id], |r| Ok(
                        (r.get::<_, String>(0)?, r.get::<_, String>(1)?)
                    ))
                    .unwrap(),
                before
            );
            assert_eq!(s.job_state(&id).unwrap(), "needs_intervention");
            assert_eq!(
                s.conn
                    .query_row(
                        "SELECT count(*) FROM resource_locks WHERE job=?",
                        [&id],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                4
            );
        }
    }
}
