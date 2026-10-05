use super::*;
fn catalog() -> ComposeCatalog {
    let mut c: ComposeCatalog = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/compose-catalog.json"
    ))
    .unwrap();
    c.normalize().unwrap();
    c
}
fn selection(template: &str) -> ComposeSelection {
    ComposeSelection {
        stack: "media".into(),
        template: template.into(),
    }
}
fn setup(template: &str) -> (tempfile::TempDir, Store, Principal, ComposeCatalog) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    store.issue_bootstrap("bootstrap", 100).unwrap();
    store
        .enroll("bootstrap", "alice", "$argon2id$fixture", 101)
        .unwrap();
    let catalog = catalog();
    let id = store.login_record("alice").unwrap().unwrap().principal.id;
    store
        .revise_grants(
            &id,
            Role::Administrator,
            &compose_scopes(&catalog, &selection(template)).unwrap(),
            102,
        )
        .unwrap();
    let p = store.login_record("alice").unwrap().unwrap().principal;
    (dir, store, p, catalog)
}
#[test]
fn exact_elevated_grant_is_required_for_template_stack_and_content_even_for_admin() {
    let (_dir, mut store, p, catalog) = setup("standard");
    assert_eq!(
        store
            .plan_compose(&p, &catalog, &selection("elevated"), 200)
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    let mut grants = compose_scopes(&catalog, &selection("standard")).unwrap();
    grants.push(Scope {
        operation: Operation::DeploymentElevated,
        resource: "*".into(),
    });
    store
        .revise_grants(&p.id, Role::Administrator, &grants, 201)
        .unwrap();
    let p = store.login_record("alice").unwrap().unwrap().principal;
    assert_eq!(
        store
            .plan_compose(&p, &catalog, &selection("elevated"), 202)
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    let (_dir, mut store, p, catalog) = setup("elevated");
    assert!(
        store
            .plan_compose(&p, &catalog, &selection("elevated"), 200)
            .is_ok()
    );
    let mut changed = catalog.clone();
    changed.stacks[0].templates[0].project.services[0].user = None;
    assert_eq!(
        store
            .plan_compose(&p, &changed, &selection("elevated"), 201)
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    let mut changed = catalog.clone();
    changed.stacks[0].id = "other".into();
    assert_eq!(
        store
            .plan_compose(
                &p,
                &changed,
                &ComposeSelection {
                    stack: "other".into(),
                    template: "elevated".into()
                },
                201
            )
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
}
#[test]
fn preview_approval_survives_restart_and_read_does_not_write_authority() {
    let (dir, mut store, p, catalog) = setup("standard");
    let proposal = store
        .plan_compose(&p, &catalog, &selection("standard"), 200)
        .unwrap();
    let approval = store
        .approve_compose(&p, &catalog, &proposal.plan.id, &proposal.digest, 201)
        .unwrap();
    let hashed: String = store
        .conn
        .query_row(
            "SELECT approval_digest FROM compose_plans WHERE id=?",
            [&proposal.plan.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hashed, limeos_identity::digest(&approval.token));
    assert_ne!(hashed, approval.token);
    drop(store);
    let store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    let before = store.conn.total_changes();
    assert_eq!(
        store
            .compose_plan(&p, &catalog, &proposal.plan.id, 202)
            .unwrap(),
        proposal
    );
    assert_eq!(store.conn.total_changes(), before);
    assert_eq!(
        store
            .conn
            .query_row("SELECT count(*) FROM jobs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .conn
            .query_row(
                "SELECT approval_digest FROM compose_plans WHERE id=?",
                [&proposal.plan.id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        hashed
    );
}
#[test]
fn approval_checks_digest_expiry_current_grants_catalog_source_and_cancellation() {
    let (_dir, mut store, p, catalog) = setup("standard");
    let proposal = store
        .plan_compose(&p, &catalog, &selection("standard"), 200)
        .unwrap();
    assert_eq!(
        store
            .approve_compose(&p, &catalog, &proposal.plan.id, &"a".repeat(64), 201)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    let mut changed = catalog.clone();
    changed.stacks[0].operator_files[0].sha256 = "2".repeat(64);
    assert_eq!(
        store
            .approve_compose(&p, &changed, &proposal.plan.id, &proposal.digest, 201)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert_eq!(
        store
            .approve_compose(&p, &catalog, &proposal.plan.id, &proposal.digest, 500)
            .unwrap_err()
            .0,
        ErrorCode::Expired
    );
    store
        .approve_compose(&p, &catalog, &proposal.plan.id, &proposal.digest, 201)
        .unwrap();
    store
        .cancel_compose(&p, &catalog, &proposal.plan.id, 202)
        .unwrap();
    assert_eq!(
        store
            .compose_plan(&p, &catalog, &proposal.plan.id, 203)
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
    let proposal = store
        .plan_compose(&p, &catalog, &selection("standard"), 204)
        .unwrap();
    store
        .revise_grants(&p.id, Role::Administrator, &[], 205)
        .unwrap();
    assert_eq!(
        store
            .approve_compose(&p, &catalog, &proposal.plan.id, &proposal.digest, 206)
            .unwrap_err()
            .0,
        ErrorCode::Expired
    );
}
#[test]
fn preview_is_principal_scoped_and_refuses_writes_without_durable_space() {
    let (_dir, mut store, p, catalog) = setup("standard");
    let proposal = store
        .plan_compose(&p, &catalog, &selection("standard"), 200)
        .unwrap();
    let other = Principal {
        id: "other".into(),
        ..p.clone()
    };
    assert!(
        store
            .compose_plan(&other, &catalog, &proposal.plan.id, 201)
            .is_err()
    );
    store
        .conn
        .execute("UPDATE meta SET audit_bytes=?", [AUDIT_LIMIT])
        .unwrap();
    assert_eq!(
        store
            .plan_compose(&p, &catalog, &selection("standard"), 202)
            .unwrap_err()
            .0,
        ErrorCode::StateNotDurable
    );
    assert_eq!(
        store
            .approve_compose(&p, &catalog, &proposal.plan.id, &proposal.digest, 202)
            .unwrap_err()
            .0,
        ErrorCode::StateNotDurable
    );
}
#[test]
fn v4_migration_preserves_container_approvals_and_failure_leaves_v4_untouched() {
    use limeos_domain::ContainerSnapshot;
    for fail in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.sqlite");
        let mut store = Store::open(&path).unwrap();
        store.issue_bootstrap("bootstrap", 100).unwrap();
        store
            .enroll("bootstrap", "alice", "$argon2id$fixture", 101)
            .unwrap();
        let p = store.login_record("alice").unwrap().unwrap().principal;
        let snapshot = ContainerSnapshot {
            resource: format!("container:{}", "a".repeat(64)),
            image: format!("sha256:{}", "b".repeat(64)),
            started_at: "2026-10-05T07:00:00Z".into(),
            running: true,
        };
        let proposal = store.plan_restart(&p, &snapshot, 200).unwrap();
        let approval = store
            .approve_container(&p, &proposal.plan.id, &proposal.digest, 201)
            .unwrap();
        remove_v7_schema(&store.conn);
        store
            .conn
            .execute_batch(
                "DROP TABLE storage_plans; DROP TABLE compose_plans; PRAGMA user_version=4;",
            )
            .unwrap();
        if fail {
            store
                .conn
                .execute_batch("CREATE INDEX compose_plans_expiry ON users(username);")
                .unwrap();
        }
        drop(store);
        let reopened = Store::open(&path);
        if fail {
            assert_eq!(reopened.err().unwrap().0, ErrorCode::StateNotDurable);
            let conn = Connection::open(&path).unwrap();
            assert_eq!(
                conn.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                    .unwrap(),
                4
            );
            assert_eq!(
                conn.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name='compose_plans'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
            assert_eq!(
                conn.query_row("SELECT digest FROM container_plans", [], |r| r
                    .get::<_, String>(0))
                    .unwrap(),
                proposal.digest
            );
        } else {
            let mut store = reopened.unwrap();
            assert_eq!(
                store.container_plan(&p, &proposal.plan.id).unwrap(),
                proposal
            );
            assert!(
                store
                    .queue_container(&p, "preserved", &proposal, &approval.token, &snapshot, 202)
                    .is_ok()
            );
            assert_eq!(
                store
                    .conn
                    .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                    .unwrap(),
                SCHEMA_VERSION
            );
        }
    }
}
