use super::*;

fn binding() -> BackupPreparationBinding {
    BackupPreparationBinding {
        custody: limeos_domain::CustodyBinding {
            format_version: 1,
            record_id: "a".repeat(32),
            archive_sha256: "b".repeat(64),
            archive_bytes: 100,
            policy_identity: "c".repeat(64),
            manifest_identity: "d".repeat(64),
        },
        preparation_identity: "e".repeat(64),
        selected_resources: vec!["limeos-config".into(), "app-config".into()],
        workspace_policy_identity: "f".repeat(64),
    }
}
fn setup() -> (tempfile::TempDir, Store, Principal) {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("core.sqlite")).unwrap();
    s.issue_bootstrap("bootstrap", 100).unwrap();
    s.enroll("bootstrap", "alice", "$argon2id$fixture", 101)
        .unwrap();
    let p = s.login_record("alice").unwrap().unwrap().principal;
    (dir, s, p)
}
fn approved(s: &mut Store, p: &Principal) -> (PlannedBackupPreparation, String) {
    let plan = s.plan_backup_preparation(p, &binding(), 200).unwrap();
    let approval = s
        .approve_backup_preparation(p, &plan.plan.id, &plan.digest, &binding(), 201)
        .unwrap();
    (plan, approval.token)
}
fn count(s: &Store, table: &str) -> i64 {
    s.conn
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn exact_lost_response_replay_survives_expiry_restart_without_new_writes() {
    let (dir, mut s, p) = setup();
    let (plan, token) = approved(&mut s, &p);
    let job = s
        .queue_backup_preparation(&p, "request", &plan, &token, &binding(), 202)
        .unwrap();
    let changes = s.conn.total_changes();
    assert_eq!(
        s.queue_backup_preparation(&p, "request", &plan, &token, &binding(), 900)
            .unwrap(),
        job
    );
    assert_eq!(s.conn.total_changes(), changes);
    assert_eq!(count(&s, "resource_locks"), 0);
    drop(s);
    let mut s = Store::open(&dir.path().join("core.sqlite")).unwrap();
    let before = s.conn.total_changes();
    assert_eq!(
        s.replay_backup_preparation(&p, "request", &plan, &token)
            .unwrap(),
        Some(job.clone())
    );
    let mut unavailable = binding();
    unavailable.preparation_identity = "a".repeat(64);
    assert_eq!(
        s.queue_backup_preparation(&p, "request", &plan, &token, &unavailable, 900)
            .unwrap(),
        job
    );
    assert_eq!(s.conn.total_changes(), before);
    assert_eq!(s.backup_preparation_plan(&p, &plan.plan.id).unwrap(), plan);
    let resources = s
        .conn
        .prepare("SELECT resource FROM job_resources WHERE job=? ORDER BY resource")
        .unwrap()
        .query_map([&job.id], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(resources, binding().resources());
}

#[test]
fn changed_full_facts_key_and_approval_never_replay_or_consume() {
    let (_dir, mut s, p) = setup();
    let (plan, token) = approved(&mut s, &p);
    for n in 0..6 {
        let mut current = binding();
        match n {
            0 => current.custody.archive_sha256 = "a".repeat(64),
            1 => current.custody.policy_identity = "a".repeat(64),
            2 => current.custody.manifest_identity = "a".repeat(64),
            3 => current.preparation_identity = "a".repeat(64),
            4 => current.workspace_policy_identity = "a".repeat(64),
            _ => current.selected_resources.reverse(),
        }
        assert_eq!(
            s.queue_backup_preparation(&p, "request", &plan, &token, &current, 202)
                .unwrap_err()
                .0,
            ErrorCode::Conflict
        );
    }
    assert_eq!(count(&s, "jobs"), 0);
    let rotated = s
        .approve_backup_preparation(&p, &plan.plan.id, &plan.digest, &binding(), 202)
        .unwrap();
    assert_eq!(
        s.queue_backup_preparation(&p, "request", &plan, &token, &binding(), 203)
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    s.queue_backup_preparation(&p, "request", &plan, &rotated.token, &binding(), 203)
        .unwrap();
    assert_eq!(
        s.replay_backup_preparation(&p, "request", &plan, &token)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert!(
        s.queue_backup_preparation(&p, "other-key", &plan, &rotated.token, &binding(), 204)
            .is_err()
    );
    let mut changed = plan.clone();
    changed.plan.binding.custody.record_id = "b".repeat(32);
    assert_eq!(
        s.replay_backup_preparation(&p, "request", &changed, &rotated.token)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert!(
        s.approve_backup_preparation(&p, &plan.plan.id, &plan.digest, &binding(), 204)
            .is_err()
    );
    assert_eq!(count(&s, "jobs"), 1);
}

#[test]
fn current_authorization_and_historical_revision_are_distinct() {
    let (dir, mut s, p) = setup();
    let (plan, token) = approved(&mut s, &p);
    let job = s
        .queue_backup_preparation(&p, "request", &plan, &token, &binding(), 202)
        .unwrap();
    for role in [Role::Operator, Role::Viewer, Role::MediaRequester] {
        s.revise_grants(&p.id, role, &[storage_scope()], 203)
            .unwrap();
        let current = s.login_record("alice").unwrap().unwrap().principal;
        assert_eq!(
            s.plan_backup_preparation(&current, &binding(), 204)
                .unwrap_err()
                .0,
            ErrorCode::Forbidden
        );
        assert_eq!(
            s.replay_backup_preparation(&current, "request", &plan, &token)
                .unwrap_err()
                .0,
            ErrorCode::Forbidden
        );
    }
    s.revise_grants(&p.id, Role::Administrator, &[], 204)
        .unwrap();
    let current = s.login_record("alice").unwrap().unwrap().principal;
    assert_eq!(
        s.backup_preparation_job(&current, &job.id).unwrap_err().0,
        ErrorCode::Forbidden
    );
    s.revise_grants(&p.id, Role::Administrator, &[storage_scope()], 205)
        .unwrap();
    let current = s.login_record("alice").unwrap().unwrap().principal;
    assert_eq!(
        s.replay_backup_preparation(&p, "request", &plan, &token)
            .unwrap_err()
            .0,
        ErrorCode::Expired
    );
    assert_eq!(
        s.replay_backup_preparation(&current, "request", &plan, &token)
            .unwrap(),
        Some(job.clone())
    );
    drop(s);
    let s = Store::open(&dir.path().join("core.sqlite")).unwrap();
    assert_eq!(
        s.replay_backup_preparation(&current, "request", &plan, &token)
            .unwrap(),
        Some(job)
    );
}

#[test]
fn expired_and_revoked_unconsumed_plans_cannot_gain_new_authority() {
    let (_dir, mut s, p) = setup();
    let (plan, token) = approved(&mut s, &p);
    assert_eq!(
        s.queue_backup_preparation(&p, "request", &plan, &token, &binding(), 500)
            .unwrap_err()
            .0,
        ErrorCode::Expired
    );
    s.revise_grants(&p.id, Role::Administrator, &[storage_scope()], 203)
        .unwrap();
    let current = s.login_record("alice").unwrap().unwrap().principal;
    assert_eq!(
        s.approve_backup_preparation(&current, &plan.plan.id, &plan.digest, &binding(), 204)
            .unwrap_err()
            .0,
        ErrorCode::Expired
    );
    assert_eq!(
        s.queue_backup_preparation(&current, "request", &plan, &token, &binding(), 204)
            .unwrap_err()
            .0,
        ErrorCode::Expired
    );
    assert_eq!(count(&s, "jobs"), 0);
}

#[test]
fn queue_and_approval_audit_failure_roll_back_every_join() {
    let (_dir, mut s, p) = setup();
    let (plan, token) = approved(&mut s, &p);
    let events = count(&s, "events");
    s.conn.execute_batch("CREATE TRIGGER refuse_audit BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(
        s.queue_backup_preparation(&p, "request", &plan, &token, &binding(), 202)
            .is_err()
    );
    assert!(
        s.approve_backup_preparation(&p, &plan.plan.id, &plan.digest, &binding(), 202)
            .is_err()
    );
    assert_eq!(count(&s, "jobs"), 0);
    assert_eq!(count(&s, "job_resources"), 0);
    assert_eq!(count(&s, "events"), events);
    let saved: (Option<String>, Option<String>) = s
        .conn
        .query_row(
            "SELECT job,consumed_approval_digest FROM backup_preparation_plans",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(saved, (None, None));
    s.conn.execute_batch("DROP TRIGGER refuse_audit").unwrap();
    s.queue_backup_preparation(&p, "request", &plan, &token, &binding(), 203)
        .unwrap();
}

#[test]
fn only_cancellation_is_possible_and_dispatchers_exclude_the_intent() {
    let (_dir, mut s, p) = setup();
    let (plan, token) = approved(&mut s, &p);
    let intent = Intent::BackupPreparePrivate {
        plan: Box::new(plan.plan.clone()),
    };
    assert_eq!(
        s.queue_job(&p, "generic", &intent, 400, 202).unwrap_err().0,
        ErrorCode::Forbidden
    );
    let job = s
        .queue_backup_preparation(&p, "request", &plan, &token, &binding(), 202)
        .unwrap();
    assert!(s.container_candidates().unwrap().is_empty());
    assert!(s.storage_target_candidates().unwrap().is_empty());
    for state in [
        JobState::Running,
        JobState::Failed,
        JobState::PreconditionChanged,
        JobState::Succeeded,
        JobState::OutcomeUnknown,
        JobState::NeedsIntervention,
    ] {
        assert!(s.transition(&job.id, JobState::Queued, state, 203).is_err());
        assert!(
            s.conn
                .execute(
                    "UPDATE jobs SET state=? WHERE id=?",
                    params![state.as_str(), job.id]
                )
                .is_err()
        );
    }
    assert_eq!(s.job_state(&job.id).unwrap(), "queued");
    assert!(s.cancel_backup_preparation(&p, &plan.plan.id, 203).is_err());
    s.cancel_backup_preparation_job(&p, &job.id, 204).unwrap();
    assert_eq!(
        s.replay_backup_preparation(&p, "request", &plan, &token)
            .unwrap()
            .unwrap()
            .state,
        JobState::Canceled
    );
    let plan = s.plan_backup_preparation(&p, &binding(), 205).unwrap();
    s.cancel_backup_preparation(&p, &plan.plan.id, 206).unwrap();
    assert_eq!(
        s.backup_preparation_plan(&p, &plan.plan.id).unwrap_err().0,
        ErrorCode::NotFound
    );
}

#[test]
fn bounds_and_cancel_audit_failures_make_no_partial_changes() {
    let (_dir, mut s, p) = setup();
    let mut oversized = binding();
    oversized.selected_resources = (0..65).map(|n| n.to_string()).collect();
    assert!(s.plan_backup_preparation(&p, &oversized, 200).is_err());
    assert!(s.plan_backup_preparation(&p, &binding(), i64::MAX).is_err());
    assert_eq!(count(&s, "backup_preparation_plans"), 0);
    let (plan, token) = approved(&mut s, &p);
    s.conn.execute_batch("CREATE TRIGGER refuse_audit BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(s.cancel_backup_preparation(&p, &plan.plan.id, 202).is_err());
    assert_eq!(s.backup_preparation_plan(&p, &plan.plan.id).unwrap(), plan);
    s.conn.execute_batch("DROP TRIGGER refuse_audit").unwrap();
    let job = s
        .queue_backup_preparation(&p, "request", &plan, &token, &binding(), 203)
        .unwrap();
    s.conn.execute_batch("CREATE TRIGGER refuse_audit BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(s.cancel_backup_preparation_job(&p, &job.id, 204).is_err());
    assert_eq!(s.job_state(&job.id).unwrap(), "queued");
}

#[test]
fn startup_refuses_missing_resources_guards_and_consumed_approval_before_generation() {
    for mutation in [
        "DELETE FROM job_resources WHERE resource='storage:configuration'",
        "DROP TRIGGER backup_preparation_update_state",
        "DROP TRIGGER backup_preparation_plan_immutable; UPDATE backup_preparation_plans SET approval_digest=upper(approval_digest),consumed_approval_digest=upper(consumed_approval_digest)",
    ] {
        let (dir, mut s, p) = setup();
        let (plan, token) = approved(&mut s, &p);
        s.queue_backup_preparation(&p, "request", &plan, &token, &binding(), 202)
            .unwrap();
        let generation = s.generation;
        let events = count(&s, "events");
        let guard: String = s
            .conn
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name='backup_preparation_plan_immutable'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        s.conn.execute_batch(mutation).unwrap();
        if mutation.contains("DROP TRIGGER backup_preparation_plan_immutable") {
            s.conn.execute_batch(&guard).unwrap();
            assert_eq!(check(&s.conn).unwrap_err().0, ErrorCode::StateNotDurable);
        }
        drop(s);
        assert_eq!(
            Store::open(&dir.path().join("core.sqlite"))
                .err()
                .unwrap()
                .0,
            ErrorCode::StateNotDurable
        );
        let c = Connection::open(dir.path().join("core.sqlite")).unwrap();
        assert_eq!(
            c.query_row("SELECT generation FROM meta", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            generation
        );
        assert_eq!(
            c.query_row("SELECT count(*) FROM events", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            events
        );
    }
}

#[test]
fn genuine_compiled_schema8_upgrade_is_additive_and_failures_roll_back() {
    for failure in ["none", "migration", "audit"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.sqlite");
        let c = Connection::open(&path).unwrap();
        c.execute_batch(concat!(
            include_str!("../schema.sql"),
            include_str!("../migration-v2.sql"),
            include_str!("../migration-v3.sql"),
            include_str!("../migration-v4.sql"),
            include_str!("../migration-v5.sql"),
            include_str!("../migration-v6.sql"),
            include_str!("../migration-v7.sql"),
            include_str!("../migration-v8.sql")
        ))
        .unwrap();
        c.execute_batch("INSERT INTO users VALUES('alice','alice','hash','\"administrator\"',1); INSERT INTO sessions VALUES('session','alice','csrf',1,999,100); INSERT INTO jobs VALUES('legacy','alice','legacy','{\"operation\":\"health_probe\",\"resource\":\"fixture:legacy\"}','digest','fixture:legacy','queued',0,1,999); UPDATE jobs SET state='running' WHERE id='legacy'; INSERT INTO container_plans VALUES('plan','alice','plan-bytes','plan-digest',1,999,'approval','legacy',0);").unwrap();
        let fixtures: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/storage-contracts.json"
        ))
        .unwrap();
        let contract: limeos_domain::StorageContract =
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
        let target = limeos_domain::StorageTargetPlan {
            id: "b".repeat(64),
            version: limeos_domain::STORAGE_TARGET_VERSION,
            principal: "alice".into(),
            grant_revision: 1,
            preparation: limeos_domain::TargetPreparationPlan {
                version: 1,
                operation: limeos_domain::STORAGE_TARGET_OPERATION.into(),
                action: "c".repeat(64),
                expected: inventory,
                contract: contract.clone(),
                created_at: 200,
                expires_at: 500,
                targets: vec![limeos_domain::TargetEvidence {
                    mountpoint: contract.devices[0].mountpoint.clone(),
                    parent: limeos_domain::DirectoryIdentity {
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
            },
        };
        target.validate(200).unwrap();
        let intent = json(&Intent::StoragePrepareTargets {
            plan: Box::new(target.clone()),
        })
        .unwrap();
        c.execute(
            "INSERT INTO jobs VALUES(?,'alice','target',?,?,?,'queued',0,1,500)",
            params![
                target.preparation.action,
                intent,
                limeos_identity::digest(&intent),
                "storage:configuration"
            ],
        )
        .unwrap();
        for resource in target.preparation.resources() {
            if resource != "storage:configuration" {
                c.execute(
                    "INSERT INTO job_resources VALUES(?,?)",
                    params![target.preparation.action, resource],
                )
                .unwrap();
            }
        }
        c.execute("INSERT INTO storage_target_plans(id,principal,body,digest,revision,expires,approval_digest,job) VALUES(?,'alice',?,?,1,500,?,?)",params![target.id,json(&target).unwrap(),limeos_identity::digest(&json(&target).unwrap()),"d".repeat(64),target.preparation.action]).unwrap();
        c.execute(
            "UPDATE jobs SET state='running' WHERE id=?",
            [&target.preparation.action],
        )
        .unwrap();
        c.execute(
            "INSERT INTO storage_target_results VALUES(?,?,?,203)",
            params![
                target.preparation.action,
                "saved-protected-receipt-bytes",
                "saved-verification-bytes"
            ],
        )
        .unwrap();
        c.execute_batch("INSERT INTO grants VALUES('alice','\"storage_manage\"','storage:configuration'); INSERT INTO container_results VALUES('legacy','container-receipt-bytes','container-verification-bytes',203);").unwrap();
        let authority_tables = [
            "sessions",
            "grants",
            "container_plans",
            "container_results",
            "storage_target_plans",
            "storage_target_results",
            "job_resources",
            "resource_locks",
        ];
        let retained: Vec<_> = authority_tables
            .iter()
            .map(|name| {
                let mut stmt = c
                    .prepare(&format!("SELECT * FROM {name} ORDER BY 1,2"))
                    .unwrap();
                let width = stmt.column_count();
                let rows = stmt
                    .query_map([], |r| {
                        (0..width)
                            .map(|i| r.get::<_, rusqlite::types::Value>(i))
                            .collect::<std::result::Result<Vec<_>, _>>()
                    })
                    .unwrap()
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .unwrap();
                (name, rows)
            })
            .collect();
        let old_job:String=c.query_row("SELECT json_array(id,principal,idempotency,intent,digest,resource,revision,deadline) FROM jobs WHERE id='legacy'",[],|r|r.get(0)).unwrap();
        let old_plan:String=c.query_row("SELECT json_array(id,principal,body,digest,revision,expires,approval_digest,job,canceled) FROM container_plans",[],|r|r.get(0)).unwrap();
        if failure == "migration" {
            c.execute_batch("CREATE INDEX backup_preparation_plans_expiry ON users(username)")
                .unwrap();
        }
        if failure == "audit" {
            c.execute_batch("CREATE TRIGGER refuse_audit BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        }
        drop(c);
        let reopened = Store::open(&path);
        if failure == "none" {
            let s = reopened.unwrap();
            assert_eq!(s.job_state("legacy").unwrap(), "needs_intervention");
            drop(s);
        } else {
            assert_eq!(reopened.err().unwrap().0, ErrorCode::StateNotDurable);
        }
        let c = Connection::open(&path).unwrap();
        assert_eq!(
            c.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                .unwrap(),
            if failure == "none" {
                crate::SCHEMA_VERSION
            } else {
                8
            }
        );
        assert_eq!(c.query_row("SELECT json_array(id,principal,idempotency,intent,digest,resource,revision,deadline) FROM jobs WHERE id='legacy'",[],|r|r.get::<_,String>(0)).unwrap(),old_job);
        for (name, before) in retained {
            let mut stmt = c
                .prepare(&format!("SELECT * FROM {name} ORDER BY 1,2"))
                .unwrap();
            let width = stmt.column_count();
            let after = stmt
                .query_map([], |r| {
                    (0..width)
                        .map(|i| r.get::<_, rusqlite::types::Value>(i))
                        .collect::<std::result::Result<Vec<_>, _>>()
                })
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(after, before, "{name}: {failure}");
        }
        assert_eq!(c.query_row("SELECT json_array(id,principal,body,digest,revision,expires,approval_digest,job,canceled) FROM container_plans",[],|r|r.get::<_,String>(0)).unwrap(),old_plan);
        assert_eq!(
            c.query_row("SELECT csrf_digest FROM sessions", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "csrf"
        );
        assert_eq!(
            c.query_row(
                "SELECT resource FROM resource_locks WHERE job='legacy'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "fixture:legacy"
        );
        if failure != "none" {
            assert_eq!(
                c.query_row("SELECT generation FROM meta", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                0
            );
            assert_eq!(
                c.query_row(
                    "SELECT count(*) FROM sqlite_schema WHERE name='backup_preparation_plans'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
        }
    }
}

#[test]
fn oversized_metadata_refuses_before_allocation_with_compiled_guards_present() {
    let (dir, mut s, p) = setup();
    let plan = s.plan_backup_preparation(&p, &binding(), 200).unwrap();
    let guard: String = s
        .conn
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name='backup_preparation_plan_immutable'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    s.conn
        .execute_batch(
            "PRAGMA ignore_check_constraints=ON; DROP TRIGGER backup_preparation_plan_immutable;",
        )
        .unwrap();
    s.conn
        .execute(
            "UPDATE backup_preparation_plans SET id=? WHERE id=?",
            params!["a".repeat(1024 * 1024), plan.plan.id],
        )
        .unwrap();
    s.conn.execute_batch(&guard).unwrap();
    assert_eq!(check(&s.conn).unwrap_err().0, ErrorCode::StateNotDurable);
    let generation = s.generation;
    drop(s);
    assert_eq!(
        Store::open(&dir.path().join("core.sqlite"))
            .err()
            .unwrap()
            .0,
        ErrorCode::StateNotDurable
    );
    let c = Connection::open(dir.path().join("core.sqlite")).unwrap();
    assert_eq!(
        c.query_row("SELECT generation FROM meta", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        generation
    );
}
