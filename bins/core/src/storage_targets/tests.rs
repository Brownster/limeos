use super::*;
use std::os::unix::fs::MetadataExt;
async fn core(
    dir: &tempfile::TempDir,
    role: limeos_domain::Role,
) -> (Core, String, String, Principal) {
    let db = Database::open(&dir.path().join("core.sqlite")).unwrap();
    let token = "a".repeat(64);
    let csrf = "b".repeat(64);
    let digest = limeos_identity::digest(&token);
    let csrf_hash = limeos_identity::digest(&csrf);
    let p = db
        .call(move |s| {
            s.issue_bootstrap("bootstrap", now())?;
            s.enroll("bootstrap", "admin", "$argon2id$fixture", now())?;
            let id = s.login_record("admin")?.unwrap().principal.id;
            s.revise_grants(&id, role, &[limeos_persistence::storage_scope()], now())?;
            let record = s.login_record("admin")?.unwrap();
            s.create_session(&record, None, &digest, &csrf_hash, now())?;
            Ok(record.principal)
        })
        .await
        .unwrap();
    (
        Core {
            db,
            storage_socket: Arc::new("/unused-reader".into()),
            storage_target_socket: Arc::new("/unused-targets".into()),
            storage_readers: Arc::new(Semaphore::new(1)),
            container_socket: Arc::new("/unused-container".into()),
            compose_catalog: None,
            password_workers: Arc::new(Semaphore::new(2)),
            dummy_hash: "unused".into(),
            observations: Default::default(),
            telemetry: None,
        },
        token,
        csrf,
        p,
    )
}
fn fixture() -> StorageContract {
    let value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/storage-contracts.json"
    ))
    .unwrap();
    serde_json::from_value(value["cases"][0]["contract"].clone()).unwrap()
}
#[tokio::test]
async fn target_authority_and_csrf_are_checked_before_reader_access_and_tasks_cannot_approve() {
    for role in [
        limeos_domain::Role::Viewer,
        limeos_domain::Role::Operator,
        limeos_domain::Role::Administrator,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (c, token, csrf, _) = core(&dir, role).await;
        assert_eq!(
            c.human_plan_targets(token, csrf, fixture())
                .await
                .unwrap_err()
                .0,
            if role == limeos_domain::Role::Administrator {
                ErrorCode::Unavailable
            } else {
                ErrorCode::Forbidden
            }
        );
        assert_eq!(
            c.human_plan_targets("a".repeat(64), "bad-csrf".into(), fixture())
                .await
                .unwrap_err()
                .0,
            ErrorCode::Forbidden
        );
    }
    for request in [
        "approve_storage_targets",
        "queue_storage_targets",
        "prepare_storage_targets",
    ] {
        let raw =
            format!(r#"{{"request":"{request}","version":1,"token":"task","principal":"admin"}}"#);
        assert!(serde_json::from_str::<CoreRequest>(&raw).is_err());
    }
}
#[tokio::test]
async fn an_unprivileged_target_socket_cannot_forge_evidence_and_receives_no_frame() {
    let dir = tempfile::tempdir().unwrap();
    if dir.path().metadata().unwrap().uid() == 0 {
        return;
    }
    let path = dir.path().join("target.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let (mut c, _, _, _) = core(&dir, limeos_domain::Role::Administrator).await;
    c.storage_target_socket = Arc::new(path);
    assert_eq!(
        c.target_snapshot(&fixture()).await.unwrap_err().0,
        ErrorCode::Forbidden
    );
    let (mut stream, _) = listener.accept().await.unwrap();
    use tokio::io::AsyncReadExt;
    assert_eq!(stream.read(&mut [0u8; 4]).await.unwrap(), 0);
}
