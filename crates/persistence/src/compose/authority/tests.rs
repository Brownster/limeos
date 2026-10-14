use super::super::tests::{selection, setup};
use super::*;
use serde_json::{Value, json};
use std::cell::Cell;

struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    principal: Principal,
    catalog: ComposeCatalog,
    preview: PlannedCompose,
    scopes: Vec<Scope>,
    session: String,
    task: String,
    uid: u32,
}
impl Fixture {
    fn new(template: &str) -> Self {
        Self::at(template, 200)
    }
    fn at(template: &str, at: i64) -> Self {
        let (dir, mut store, principal, catalog) = setup(template);
        let preview = store
            .plan_compose(&principal, &catalog, &selection(template), at)
            .unwrap();
        let scopes = compose_scopes(&catalog, &selection(template)).unwrap();
        let session = limeos_identity::digest("SYNTHETIC_PRIVATE_SESSION");
        let task = limeos_identity::digest("SYNTHETIC_PRIVATE_TASK");
        let record = store.login_record("alice").unwrap().unwrap();
        store
            .create_session(
                &record,
                None,
                &session,
                &limeos_identity::digest("SYNTHETIC_CSRF"),
                at,
            )
            .unwrap();
        let uid = 1000;
        store
            .issue_task(
                &principal,
                TaskGrant {
                    service_uid: uid,
                    task: "review",
                    scopes: &scopes,
                    expires: at + 100,
                },
                &task,
                at,
            )
            .unwrap();
        Self {
            dir,
            store,
            principal,
            catalog,
            preview,
            scopes,
            session,
            task,
            uid,
        }
    }
    fn human(&self) -> Result<AuthenticatedComposePreview> {
        self.store.resolve_compose_authority(
            Identity::Session(&self.session),
            &self.catalog,
            &self.preview,
            &self.scopes,
            || Ok(202),
        )
    }
    fn model(&self) -> Result<AuthenticatedComposePreview> {
        self.store.resolve_compose_authority(
            Identity::Task {
                digest: &self.task,
                peer_uid: self.uid,
                task: "review",
            },
            &self.catalog,
            &self.preview,
            &self.scopes,
            || Ok(202),
        )
    }
}
fn refusal<T>(result: Result<T>, expected: ErrorCode) {
    assert_eq!(
        result.err().expect("unexpected authority success").0,
        expected
    );
}
fn state(s: &Store) -> Value {
    let mut tables = serde_json::Map::new();
    for name in [
        "events",
        "jobs",
        "job_resources",
        "resource_locks",
        "sessions",
        "task_tokens",
        "compose_plans",
    ] {
        let count: i64 = s
            .conn
            .query_row(&format!("SELECT count(*) FROM {name}"), [], |r| r.get(0))
            .unwrap();
        tables.insert(name.into(), json!(count));
    }
    let meta: (i64, i64) = s
        .conn
        .query_row(
            "SELECT generation,audit_bytes FROM meta WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let rows:Vec<(String,String,i64,i64,Option<String>,i64)>=s.conn.prepare(
        "SELECT body,digest,revision,expires,approval_digest,canceled FROM compose_plans ORDER BY id"
    ).unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap().map(|r|r.unwrap()).collect();
    json!({"tables":tables,"meta":meta,"preview_rows":rows,"schema":s.conn.pragma_query_value::<u32,_>(None,"user_version",|r|r.get(0)).unwrap()})
}

#[test]
fn session_and_task_resolve_identical_actual_stored_preview_and_exact_scopes() {
    for template in ["standard", "elevated"] {
        let f = Fixture::new(template);
        let human = f.human().unwrap();
        let model = f.model().unwrap();
        let h = human.snapshot();
        let m = model.snapshot();
        assert_eq!(h.kind, ComposeAuthorityKind::HumanSession);
        assert_eq!(m.kind, ComposeAuthorityKind::Task);
        assert_eq!(h.principal, &f.principal);
        assert_eq!(m.principal, h.principal);
        assert_eq!(h.preview, &f.preview);
        assert_eq!(m.preview, h.preview);
        assert_eq!(h.required_scopes, f.scopes);
        assert_eq!(m.required_scopes, h.required_scopes);
        assert_eq!(h.checked_at, 202);
        assert_eq!(h.generation, f.store.generation);
        assert_eq!(m.identity_expires_at, 300);
        assert_eq!(
            h.required_scopes.len(),
            if template == "standard" { 1 } else { 2 }
        );
    }
}

#[test]
fn authority_reads_work_under_query_only_and_leave_approval_audit_generation_and_locks_untouched() {
    let mut f = Fixture::new("elevated");
    let approval = f
        .store
        .approve_compose(
            &f.principal,
            &f.catalog,
            &f.preview.plan.id,
            &f.preview.digest,
            201,
        )
        .unwrap();
    let before = state(&f.store);
    let changes = f.store.conn.total_changes();
    f.store.conn.execute_batch("PRAGMA query_only=ON").unwrap();
    f.human().unwrap();
    f.model().unwrap();
    assert_eq!(before, state(&f.store));
    assert_eq!(changes, f.store.conn.total_changes());
    assert!(f.store.conn.is_autocommit());
    assert_eq!(
        f.store
            .conn
            .pragma_query_value::<u32, _>(None, "query_only", |r| r.get(0))
            .unwrap(),
        1
    );
    let saved: Option<String> = f
        .store
        .conn
        .query_row("SELECT approval_digest FROM compose_plans", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(saved, Some(limeos_identity::digest(&approval.token)));
}

#[test]
fn unknown_revoked_expired_or_preview_approval_session_credentials_refuse() {
    for problem in 0..4 {
        let mut f = Fixture::new("standard");
        match problem {
            0 => f.session = "a".repeat(64),
            1 => f.store.revoke_session(&f.session, 201).unwrap(),
            2 => {
                f.store
                    .conn
                    .execute("UPDATE sessions SET expires=202", [])
                    .unwrap();
            }
            _ => {
                let approval = f
                    .store
                    .approve_compose(
                        &f.principal,
                        &f.catalog,
                        &f.preview.plan.id,
                        &f.preview.digest,
                        201,
                    )
                    .unwrap();
                f.session = limeos_identity::digest(&approval.token);
            }
        }
        let before = state(&f.store);
        refusal(f.human(), ErrorCode::Unauthenticated);
        assert_eq!(before, state(&f.store));
    }
}

#[test]
fn tasks_require_exact_uid_id_generation_revision_expiry_revocation_and_every_elevated_scope() {
    for problem in 0..7 {
        let mut f = Fixture::new("elevated");
        let mut task_id = "review";
        let expected = match problem {
            0 => {
                f.uid += 1;
                ErrorCode::Expired
            }
            1 => {
                task_id = "different";
                ErrorCode::Expired
            }
            2 => {
                f.store
                    .conn
                    .execute("UPDATE task_tokens SET generation=generation+1", [])
                    .unwrap();
                ErrorCode::Expired
            }
            3 => {
                f.store
                    .conn
                    .execute("UPDATE task_tokens SET revision=revision+1", [])
                    .unwrap();
                ErrorCode::Expired
            }
            4 => {
                f.store
                    .conn
                    .execute("UPDATE task_tokens SET expires=202", [])
                    .unwrap();
                ErrorCode::Expired
            }
            5 => {
                f.store.revoke_task(&f.task, 201).unwrap();
                ErrorCode::Expired
            }
            _ => {
                f.store
                    .conn
                    .execute(
                        "UPDATE task_tokens SET scopes=?",
                        [json(&&f.scopes[..1]).unwrap()],
                    )
                    .unwrap();
                ErrorCode::Forbidden
            }
        };
        refusal(
            f.store.resolve_compose_authority(
                Identity::Task {
                    digest: &f.task,
                    peer_uid: f.uid,
                    task: task_id,
                },
                &f.catalog,
                &f.preview,
                &f.scopes,
                || Ok(202),
            ),
            expected,
        );
    }
}

#[test]
fn forged_changed_or_oversized_caller_preview_never_replaces_the_bounded_durable_row() {
    let f = Fixture::new("standard");
    for problem in 0..10 {
        let mut expected = f.preview.clone();
        match problem {
            0 => expected.plan.principal = "forged-owner".into(),
            1 => expected.plan.grant_revision += 1,
            2 => expected.plan.catalog_digest = "e".repeat(64),
            3 => expected.plan.selection.template = "different".into(),
            4 => expected.plan.expires_at += 1,
            5 => expected.plan.before.services.clear(),
            6 => expected.plan.desired.services[0].image = "x".repeat(100_000),
            7 => expected.plan.impact.services.clear(),
            8 => expected.plan.operator_files.clear(),
            _ => expected.digest = "e".repeat(64),
        }
        let changes = f.store.conn.total_changes();
        refusal(
            f.store.resolve_compose_authority(
                Identity::Session(&f.session),
                &f.catalog,
                &expected,
                &f.scopes,
                || Ok(202),
            ),
            ErrorCode::Conflict,
        );
        assert_eq!(changes, f.store.conn.total_changes());
        assert!(f.store.conn.is_autocommit());
    }
}

#[test]
fn requirements_must_equal_trusted_catalog_scopes_without_omissions_extras_reordering_or_wildcards()
{
    let f = Fixture::new("elevated");
    for problem in 0..6 {
        let mut scopes = f.scopes.clone();
        match problem {
            0 => scopes.clear(),
            1 => {
                scopes.pop();
            }
            2 => scopes.push(scopes[0].clone()),
            3 => scopes.reverse(),
            4 => scopes[1].resource = "*".into(),
            _ => scopes[1].resource.push('x'),
        }
        refusal(
            f.store.resolve_compose_authority(
                Identity::Session(&f.session),
                &f.catalog,
                &f.preview,
                &scopes,
                || Ok(202),
            ),
            ErrorCode::Forbidden,
        );
    }
}

#[test]
fn current_catalog_cancellation_body_digest_and_row_metadata_changes_refuse() {
    for problem in 0..6 {
        let mut f = Fixture::new("standard");
        let expected = match problem {
            0 => {
                f.catalog.stacks[0]
                    .templates
                    .iter_mut()
                    .find(|t| t.id == "standard")
                    .unwrap()
                    .project
                    .services[0]
                    .privileged = true;
                ErrorCode::Conflict
            }
            1 => {
                f.store
                    .cancel_compose(&f.principal, &f.catalog, &f.preview.plan.id, 201)
                    .unwrap();
                ErrorCode::NotFound
            }
            2 => {
                f.store
                    .conn
                    .execute("UPDATE compose_plans SET body='{}'", [])
                    .unwrap();
                ErrorCode::StateNotDurable
            }
            3 => {
                f.store
                    .conn
                    .execute("UPDATE compose_plans SET digest=?", ["a".repeat(64)])
                    .unwrap();
                ErrorCode::StateNotDurable
            }
            4 => {
                f.store
                    .conn
                    .execute("UPDATE compose_plans SET revision=revision+1", [])
                    .unwrap();
                ErrorCode::StateNotDurable
            }
            _ => {
                f.store
                    .conn
                    .execute("UPDATE compose_plans SET expires=expires+1", [])
                    .unwrap();
                ErrorCode::StateNotDurable
            }
        };
        refusal(f.human(), expected);
    }
}

#[test]
fn grant_and_role_changes_use_actual_identity_and_no_media_container_or_wildcard_elevated_authority()
 {
    for problem in 0..4 {
        let f = Fixture::new("elevated");
        match problem {
            0 => {
                f.store
                    .conn
                    .execute("UPDATE users SET role=?", [json(&Role::Operator).unwrap()])
                    .unwrap();
            }
            1 => {
                f.store
                    .conn
                    .execute(
                        "UPDATE grants SET operation=? WHERE operation=?",
                        params![
                            json(&Operation::ContainerManage).unwrap(),
                            json(&Operation::DeploymentManage).unwrap()
                        ],
                    )
                    .unwrap();
            }
            2 => {
                f.store
                    .conn
                    .execute(
                        "UPDATE grants SET operation=? WHERE operation=?",
                        params![
                            json(&Operation::MediaRequest).unwrap(),
                            json(&Operation::DeploymentManage).unwrap()
                        ],
                    )
                    .unwrap();
            }
            _ => {
                f.store
                    .conn
                    .execute(
                        "UPDATE grants SET resource='*' WHERE operation=?",
                        [json(&Operation::DeploymentElevated).unwrap()],
                    )
                    .unwrap();
            }
        }
        refusal(f.human(), ErrorCode::Forbidden);
        refusal(f.model(), ErrorCode::Forbidden);
    }
}

#[test]
fn revised_grants_revoke_old_identity_and_a_new_session_still_cannot_use_old_preview_revision() {
    let mut f = Fixture::new("standard");
    f.store
        .revise_grants(&f.principal.id, Role::Administrator, &f.scopes, 201)
        .unwrap();
    refusal(f.human(), ErrorCode::Unauthenticated);
    refusal(f.model(), ErrorCode::Expired);
    f.store.revoke_session(&f.session, 201).unwrap();
    let record = f.store.login_record("alice").unwrap().unwrap();
    f.store
        .create_session(&record, None, &f.session, "csrf", 201)
        .unwrap();
    refusal(f.human(), ErrorCode::Expired);
}

#[test]
fn wrong_owner_and_missing_plan_are_hidden_even_with_equivalent_current_grants() {
    let mut f = Fixture::new("standard");
    // Actual second identity, without fabricating a caller Principal.
    f.store.conn.execute("INSERT INTO users(id,username,password_hash,role,grant_revision) VALUES('other','other','fixture',?,1)",[json(&Role::Administrator).unwrap()]).unwrap();
    for s in &f.scopes {
        f.store
            .conn
            .execute(
                "INSERT INTO grants VALUES('other',?,?)",
                params![json(&s.operation).unwrap(), s.resource],
            )
            .unwrap();
    }
    let record = f.store.login_record("other").unwrap().unwrap();
    let other = limeos_identity::digest("SYNTHETIC_OTHER_SESSION");
    f.store
        .create_session(&record, None, &other, "csrf", 201)
        .unwrap();
    refusal(
        f.store.resolve_compose_authority(
            Identity::Session(&other),
            &f.catalog,
            &f.preview,
            &f.scopes,
            || Ok(202),
        ),
        ErrorCode::NotFound,
    );
    let mut missing = f.preview.clone();
    missing.plan.id = "a".repeat(64);
    refusal(
        f.store.resolve_compose_authority(
            Identity::Session(&f.session),
            &f.catalog,
            &missing,
            &f.scopes,
            || Ok(202),
        ),
        ErrorCode::NotFound,
    );
}

#[test]
fn old_success_is_past_data_and_a_new_read_rejects_revocation() {
    let mut f = Fixture::new("standard");
    let old = f.human().unwrap();
    let before = serde_json::to_vec(&old.snapshot()).unwrap();
    f.store.revoke_session(&f.session, 203).unwrap();
    refusal(f.human(), ErrorCode::Unauthenticated);
    assert_eq!(before, serde_json::to_vec(&old.snapshot()).unwrap());
}

#[test]
fn late_identity_or_plan_expiry_clock_failure_and_rollback_leave_no_partial_transaction() {
    for problem in 0..5 {
        let f = Fixture::new("standard");
        let calls = Cell::new(0);
        let session_expiry = f.store.session_expiry(&f.session).unwrap();
        let clock = || {
            let n = calls.get();
            calls.set(n + 1);
            if n == 0 {
                return Ok(202);
            }
            match problem {
                0 => Ok(201),
                1 => Ok(session_expiry),
                2 => Ok(500),
                3 => Err(Error(ErrorCode::Unavailable)),
                _ => Ok(300),
            }
        };
        let (identity, expected) = match problem {
            1 => (Identity::Session(&f.session), ErrorCode::Unauthenticated),
            3 => (Identity::Session(&f.session), ErrorCode::Unavailable),
            4 => (
                Identity::Task {
                    digest: &f.task,
                    peer_uid: f.uid,
                    task: "review",
                },
                ErrorCode::Expired,
            ),
            _ => (Identity::Session(&f.session), ErrorCode::Expired),
        };
        let before = state(&f.store);
        let changes = f.store.conn.total_changes();
        refusal(
            f.store
                .resolve_compose_authority(identity, &f.catalog, &f.preview, &f.scopes, clock),
            expected,
        );
        assert_eq!(before, state(&f.store));
        assert_eq!(changes, f.store.conn.total_changes());
        assert!(f.store.conn.is_autocommit());
    }
}

#[test]
fn concurrent_revocation_keeps_one_read_snapshot_and_is_seen_on_the_next_resolution() {
    let f = Fixture::new("standard");
    let calls = Cell::new(0);
    let clock = || {
        if calls.replace(calls.get() + 1) == 1 {
            let other = Connection::open(f.dir.path().join("core.sqlite")).unwrap();
            other
                .execute("DELETE FROM sessions WHERE digest=?", [&f.session])
                .unwrap();
        }
        Ok(202)
    };
    let before = f.store.conn.total_changes();
    f.store
        .resolve_compose_authority(
            Identity::Session(&f.session),
            &f.catalog,
            &f.preview,
            &f.scopes,
            clock,
        )
        .unwrap();
    assert_eq!(before, f.store.conn.total_changes());
    refusal(f.human(), ErrorCode::Unauthenticated);
}

#[test]
fn fixed_public_clock_uses_current_session_and_task_and_restart_invalidates_task_generation() {
    let mut f = Fixture::at("standard", now().unwrap());
    f.store
        .resolve_compose_session(&f.session, &f.catalog, &f.preview, &f.scopes)
        .unwrap();
    f.store
        .resolve_compose_task(&f.task, f.uid, "review", &f.catalog, &f.preview, &f.scopes)
        .unwrap();
    let old = f.store.generation;
    let path = f.dir.path().join("core.sqlite");
    drop(f.store);
    f.store = Store::open(&path).unwrap();
    assert!(f.store.generation > old);
    let current = f
        .store
        .resolve_compose_session(&f.session, &f.catalog, &f.preview, &f.scopes)
        .unwrap();
    assert_eq!(current.snapshot().generation, f.store.generation);
    refusal(
        f.store
            .resolve_compose_task(&f.task, f.uid, "review", &f.catalog, &f.preview, &f.scopes),
        ErrorCode::Expired,
    );
}

#[test]
fn borrowed_authority_report_counts_exact_bytes_and_retains_no_authentication_secrets() {
    let f = Fixture::new("elevated");
    let checked = f.human().unwrap();
    let s = checked.snapshot();
    let bytes = serde_json::to_vec(&s).unwrap();
    observation_limit(&s, bytes.len()).unwrap();
    refusal(
        observation_limit(&s, bytes.len() - 1),
        ErrorCode::Unavailable,
    );
    let text = String::from_utf8(bytes).unwrap();
    for secret in [
        &f.session,
        &f.task,
        "SYNTHETIC_CSRF",
        "$argon2id$fixture",
        "approval_token",
    ] {
        assert!(!text.contains(secret));
    }
    for key in [
        "token",
        "session_digest",
        "password_hash",
        "csrf_digest",
        "approval",
        "receipt",
        "authorized_until",
    ] {
        assert!(serde_json::to_value(&s).unwrap().get(key).is_none());
    }
}

#[test]
fn nested_transaction_and_database_generation_mismatch_refuse_without_changing_outer_state() {
    let f = Fixture::new("standard");
    f.store.conn.execute_batch("BEGIN DEFERRED").unwrap();
    refusal(f.human(), ErrorCode::StateNotDurable);
    assert!(!f.store.conn.is_autocommit());
    f.store.conn.execute_batch("ROLLBACK").unwrap();
    f.store
        .conn
        .execute("UPDATE meta SET generation=generation+1 WHERE id=1", [])
        .unwrap();
    let changes = f.store.conn.total_changes();
    refusal(f.human(), ErrorCode::StateNotDurable);
    assert_eq!(changes, f.store.conn.total_changes());
    assert!(f.store.conn.is_autocommit());
}
