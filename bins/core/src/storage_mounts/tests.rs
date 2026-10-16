use super::*;
use limeos_api::Backend;
use std::os::unix::fs::MetadataExt;

async fn fixture(
    role: limeos_domain::Role,
) -> (tempfile::TempDir, Core, String, String, Principal) {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("core.sqlite")).unwrap();
    let token = "a".repeat(64);
    let csrf = "b".repeat(64);
    let token_hash = limeos_identity::digest(&token);
    let csrf_hash = limeos_identity::digest(&csrf);
    let principal = db
        .call(move |store| {
            store.issue_bootstrap("bootstrap", now())?;
            store.enroll("bootstrap", "admin", "$argon2id$fixture", now())?;
            let id = store.login_record("admin")?.unwrap().principal.id;
            store.revise_grants(&id, role, &[limeos_persistence::storage_scope()], now())?;
            let record = store.login_record("admin")?.unwrap();
            store.create_session(&record, None, &token_hash, &csrf_hash, now())?;
            Ok(record.principal)
        })
        .await
        .unwrap();
    let core = Core {
        db,
        storage_socket: Arc::new("/unused-reader".into()),
        storage_target_socket: Arc::new("/unused-targets".into()),
        storage_mount_socket: Arc::new("/unused-mounts".into()),
        storage_readers: Arc::new(Semaphore::new(1)),
        container_socket: Arc::new("/unused-container".into()),
        compose_catalog: None,
        password_workers: Arc::new(Semaphore::new(2)),
        dummy_hash: "unused".into(),
        observations: Default::default(),
        telemetry: None,
    };
    (dir, core, token, csrf, principal)
}

fn inspection(at: i64) -> StorageMountInspection {
    let identity = serde_json::json!({"major":8,"minor":1,"mount_id":10,"inode":2,"mode":16877,"uid":0,"gid":0});
    // Past facts for DB/read tests only; no helper or physical admission exists.
    serde_json::from_value(serde_json::json!({
        "target_id":"data", "device":{"id":"data","role":"data","filesystem_uuid":"uuid-data","filesystem":"ext4","mountpoint":"/mnt/data","serial":null},
        "expected":{"host_root_mount_id":10,"topology_digest":"a".repeat(64),"mounts_digest":"b".repeat(64),"fstab_digest":"c".repeat(64),"fstab_entries":[],
            "devices":[{"device":{"major":8,"minor":16},"filesystem_uuid":"uuid-data","filesystem":"ext4","serial":null,"boot_backing":false,"mounts":[]}]},
        "target":{"mountpoint":"/mnt/data","parent":identity,"existing":identity},
        "physical_devices":[{"major":8,"minor":16}],"consumer_digest":"d".repeat(64),"observed_at":at
    })).unwrap()
}

#[tokio::test]
async fn mount_current_human_authority_and_csrf_precede_reader_access() {
    for role in [
        limeos_domain::Role::Viewer,
        limeos_domain::Role::Operator,
        limeos_domain::Role::Administrator,
    ] {
        let (_dir, core, token, csrf, _) = fixture(role).await;
        let expected = if role == limeos_domain::Role::Administrator {
            ErrorCode::Unavailable
        } else {
            ErrorCode::Forbidden
        };
        assert_eq!(
            core.human_plan_mount(
                token.clone(),
                csrf,
                StorageMountSelection {
                    target_id: "data".into()
                }
            )
            .await
            .unwrap_err()
            .0,
            expected
        );
        assert_eq!(
            core.human_plan_mount(
                token,
                "wrong-csrf".into(),
                StorageMountSelection {
                    target_id: "data".into()
                }
            )
            .await
            .unwrap_err()
            .0,
            ErrorCode::Forbidden
        );
    }
    for request in [
        "plan_storage_mount",
        "approve_storage_mount",
        "queue_storage_mount",
        "execute_storage_mount",
    ] {
        let raw =
            format!(r#"{{"request":"{request}","version":1,"token":"task","principal":"admin"}}"#);
        assert!(serde_json::from_str::<CoreRequest>(&raw).is_err());
    }
}

#[tokio::test]
async fn mount_stored_get_list_and_expired_queue_replay_never_contact_executor() {
    let (dir, mut core, token, csrf, principal) = fixture(limeos_domain::Role::Administrator).await;
    let path = dir.path().join("unused-mount.sock");
    let listener = UnixListener::bind(&path).unwrap();
    core.storage_mount_socket = Arc::new(path);
    core.storage_readers = Arc::new(Semaphore::new(0));
    let (input, job) = core
        .db
        .call(move |store| {
            let past = now() - 1000;
            let facts = inspection(past);
            let proposal = store.plan_storage_mount(&principal, &facts, past)?;
            let approval = store.approve_storage_mount(
                &principal,
                &proposal.plan.id,
                &proposal.digest,
                &facts,
                past + 1,
            )?;
            let job = store.queue_storage_mount(
                &principal,
                "same-mount-request",
                &proposal,
                &approval.token,
                &facts,
                past + 2,
            )?;
            Ok((
                limeos_contracts::QueueStorageMountInput {
                    proposal,
                    approval: approval.token,
                    idempotency_key: "same-mount-request".into(),
                },
                job,
            ))
        })
        .await
        .unwrap();
    assert!(job.plan.effect.expires_at < now());
    assert_eq!(
        core.storage_mount_plan(token.clone(), job.plan.id.clone())
            .await
            .unwrap(),
        input.proposal
    );
    assert_eq!(
        core.storage_mount_job(token.clone(), job.id.clone())
            .await
            .unwrap(),
        job
    );
    assert_eq!(
        core.storage_mount_jobs(token.clone()).await.unwrap(),
        vec![job.clone()]
    );
    assert_eq!(
        core.human_queue_mount(token, csrf, input).await.unwrap(),
        job
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn mount_unprivileged_executor_cannot_forge_inspection_and_receives_no_request() {
    let (dir, mut core, _, _, _) = fixture(limeos_domain::Role::Administrator).await;
    if dir.path().metadata().unwrap().uid() == 0 {
        return;
    }
    let path = dir.path().join("unprivileged.sock");
    let listener = UnixListener::bind(&path).unwrap();
    core.storage_mount_socket = Arc::new(path);
    assert_eq!(
        core.inspect_mount(StorageMountSelection {
            target_id: "data".into()
        })
        .await
        .unwrap_err()
        .0,
        ErrorCode::Forbidden
    );
    let (mut stream, _) = listener.accept().await.unwrap();
    use tokio::io::AsyncReadExt;
    assert_eq!(stream.read(&mut [0; 4]).await.unwrap(), 0);
}
