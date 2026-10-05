use super::*;
#[tokio::test]
async fn storage_authority_is_checked_before_contacting_a_reader_and_tasks_cannot_approve() {
    for role in [
        limeos_domain::Role::Viewer,
        limeos_domain::Role::Operator,
        limeos_domain::Role::Administrator,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("core.sqlite")).unwrap();
        let token = "a".repeat(64);
        let csrf = limeos_identity::digest(&format!("csrf:{token}"));
        let digest = limeos_identity::digest(&token);
        let csrf_digest = limeos_identity::digest(&csrf);
        db.call(move |s| {
            s.issue_bootstrap("bootstrap", now())?;
            s.enroll("bootstrap", "admin", "$argon2id$fixture", now())?;
            let id = s.login_record("admin")?.unwrap().principal.id;
            s.revise_grants(&id, role, &[limeos_persistence::storage_scope()], now())?;
            let record = s.login_record("admin")?.unwrap();
            s.create_session(&record, None, &digest, &csrf_digest, now())?;
            Ok(())
        })
        .await
        .unwrap();
        let core = Core {
            db,
            storage_socket: Arc::new("/unreachable".into()),
            storage_readers: Arc::new(Semaphore::new(1)),
            container_socket: Arc::new("/unused".into()),
            compose_catalog: None,
            password_workers: Arc::new(Semaphore::new(2)),
            dummy_hash: "unused".into(),
            observations: Default::default(),
            telemetry: None,
        };
        assert_eq!(
            core.storage_inventory(token.clone()).await.unwrap_err().0,
            if role == limeos_domain::Role::Administrator {
                ErrorCode::Unavailable
            } else {
                ErrorCode::Forbidden
            }
        );
        assert_eq!(
            core.approve_storage("b".repeat(64), csrf, "c".repeat(64), "d".repeat(64))
                .await
                .err()
                .unwrap()
                .0,
            ErrorCode::Unauthenticated
        );
    }
    assert!(
        serde_json::from_str::<CoreRequest>(
            r#"{"request":"approve_storage","version":1,"token":"task"}"#
        )
        .is_err()
    );
}
#[tokio::test]
async fn an_unprivileged_reader_socket_cannot_forge_storage_evidence() {
    use std::os::unix::fs::MetadataExt;
    let dir = tempfile::tempdir().unwrap();
    if dir.path().metadata().unwrap().uid() == 0 {
        return;
    }
    let path = dir.path().join("reader.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let core = Core {
        db: Database::open(&dir.path().join("core.sqlite")).unwrap(),
        storage_socket: Arc::new(path),
        storage_readers: Arc::new(Semaphore::new(1)),
        container_socket: Arc::new("/unused".into()),
        compose_catalog: None,
        password_workers: Arc::new(Semaphore::new(2)),
        dummy_hash: "unused".into(),
        observations: Default::default(),
        telemetry: None,
    };
    assert_eq!(
        core.fresh_storage().await.unwrap_err().0,
        ErrorCode::Forbidden
    );
    let (mut stream, _) = listener.accept().await.unwrap();
    use tokio::io::AsyncReadExt;
    assert_eq!(stream.read(&mut [0u8; 4]).await.unwrap(), 0);
}
