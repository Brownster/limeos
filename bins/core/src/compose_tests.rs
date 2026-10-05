use super::*;
use limeos_domain::{ComposeCatalog, ComposeSelection, ErrorCode, Role};

async fn fixture(
    role: Role,
    template: &str,
    narrow_task: bool,
) -> (tempfile::TempDir, Core, String, String, String, u32) {
    use std::os::unix::fs::MetadataExt;
    let dir = tempfile::tempdir().unwrap();
    let uid = dir.path().metadata().unwrap().uid();
    let db = Database::open(&dir.path().join("core.sqlite")).unwrap();
    let mut catalog: ComposeCatalog =
        serde_json::from_str(include_str!("../../../tests/fixtures/compose-catalog.json")).unwrap();
    catalog.normalize().unwrap();
    let mut scopes = limeos_persistence::compose_scopes(&catalog, &selection(template)).unwrap();
    let task_scopes = if role == Role::MediaRequester {
        let health = Scope {
            operation: limeos_domain::Operation::HealthRead,
            resource: "host:system".into(),
        };
        scopes.push(health.clone());
        vec![health]
    } else if narrow_task {
        scopes[..1].to_vec()
    } else {
        scopes.clone()
    };
    let token = "a".repeat(64);
    let csrf = limeos_identity::digest(&format!("csrf:{token}"));
    let digest = limeos_identity::digest(&token);
    let csrf_digest = limeos_identity::digest(&csrf);
    let task_token = "b".repeat(64);
    let task_digest = limeos_identity::digest(&task_token);
    db.call(move |s| {
        s.issue_bootstrap("bootstrap", now())?;
        s.enroll("bootstrap", "admin", "$argon2id$fixture", now())?;
        let id = s.login_record("admin")?.unwrap().principal.id;
        s.revise_grants(&id, role, &scopes, now())?;
        let record = s.login_record("admin")?.unwrap();
        s.create_session(&record, None, &digest, &csrf_digest, now())?;
        s.issue_task(
            &record.principal,
            limeos_persistence::TaskGrant {
                service_uid: uid,
                task: "compose-preview",
                scopes: &task_scopes,
                expires: now() + 60,
            },
            &task_digest,
            now(),
        )
    })
    .await
    .unwrap();
    let core = Core {
        db,
        compose_catalog: Some(Arc::new(catalog)),
        container_socket: Arc::new("/unreachable".into()),
        storage_socket: Arc::new("/unreachable".into()),
        storage_readers: Arc::new(Semaphore::new(1)),
        password_workers: Arc::new(Semaphore::new(2)),
        dummy_hash: "unused".into(),
        observations: Default::default(),
        telemetry: None,
    };
    (dir, core, token, csrf, task_token, uid)
}
fn selection(template: &str) -> ComposeSelection {
    ComposeSelection {
        stack: "media".into(),
        template: template.into(),
    }
}

#[tokio::test]
async fn browser_and_task_have_identical_compose_policy_and_task_cannot_approve() {
    for (role, template, allowed) in [
        (Role::Operator, "standard", true),
        (Role::MediaRequester, "standard", false),
        (Role::Administrator, "elevated", true),
    ] {
        let (_dir, core, token, csrf, task, uid) = fixture(role, template, false).await;
        let human = core
            .plan_compose(token.clone(), csrf.clone(), selection(template))
            .await;
        let model = core
            .propose_compose_task(
                task.clone(),
                uid,
                "compose-preview".into(),
                selection(template),
            )
            .await;
        assert_eq!(human.is_ok(), allowed);
        assert_eq!(model.is_ok(), allowed);
        if allowed {
            let human = human.unwrap();
            let model = model.unwrap();
            assert_eq!(human.plan.desired, model.plan.desired);
            assert_eq!(human.plan.impact, model.plan.impact);
            assert_eq!(
                core.approve_compose(task, csrf, model.plan.id, model.digest)
                    .await
                    .err()
                    .unwrap()
                    .0,
                ErrorCode::Unauthenticated
            );
        } else {
            assert_eq!(human.unwrap_err().0, ErrorCode::Forbidden);
            assert_eq!(model.unwrap_err().0, ErrorCode::Forbidden);
        }
    }
    assert!(serde_json::from_str::<CoreRequest>(r#"{"request":"approve_compose","version":1,"token":"task","id":"plan","digest":"digest"}"#).is_err());
}
#[tokio::test]
async fn task_needs_both_scopes_and_kernel_identity_and_current_authority() {
    let (_dir, core, token, csrf, task, uid) = fixture(Role::Administrator, "elevated", true).await;
    assert!(
        core.plan_compose(token, csrf, selection("elevated"))
            .await
            .is_ok()
    );
    assert_eq!(
        core.propose_compose_task(
            task.clone(),
            uid,
            "compose-preview".into(),
            selection("elevated")
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::Forbidden
    );
    assert_eq!(
        core.propose_compose_task(
            task,
            uid + 1,
            "compose-preview".into(),
            selection("standard")
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::Expired
    );
    let (_dir, core, token, csrf, task, uid) =
        fixture(Role::Administrator, "standard", false).await;
    let proposal = core
        .plan_compose(token.clone(), csrf.clone(), selection("standard"))
        .await
        .unwrap();
    let digest = limeos_identity::digest(&task);
    core.db
        .call(move |s| s.revoke_task(&digest, now()))
        .await
        .unwrap();
    assert_eq!(
        core.propose_compose_task(task, uid, "compose-preview".into(), selection("standard"))
            .await
            .unwrap_err()
            .0,
        ErrorCode::Expired
    );
    let principal = core.session(token.clone(), None).await.unwrap().principal;
    core.db
        .call(move |s| s.revise_grants(&principal.id, Role::Administrator, &[], now()))
        .await
        .unwrap();
    assert_eq!(
        core.approve_compose(token, csrf, proposal.plan.id, proposal.digest)
            .await
            .err()
            .unwrap()
            .0,
        ErrorCode::Unauthenticated
    );
}
#[tokio::test]
async fn framed_task_preview_uses_peer_credentials_and_never_contacts_engine() {
    let (_dir, core, _token, _csrf, task, _uid) =
        fixture(Role::Administrator, "standard", false).await;
    let (mut client, server) = UnixStream::pair().unwrap();
    let worker = tokio::spawn(handle_rpc(core, server));
    limeos_contracts::write_frame(
        &mut client,
        &CoreRequest::ProposeCompose {
            version: VERSION,
            token: task,
            task: "compose-preview".into(),
            selection: selection("standard"),
        },
    )
    .await
    .unwrap();
    let response: CoreResponse = limeos_contracts::read_frame(&mut client).await.unwrap();
    assert!(matches!(response, CoreResponse::ComposePlan(_)));
    worker.await.unwrap().unwrap();
}
