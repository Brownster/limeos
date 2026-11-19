use super::*;
#[tokio::test]
async fn observations_and_history_use_current_resource_grants_and_reject_revocation() {
    use limeos_contracts::{
        Availability, Freshness, ObservationBatch, Resource, ResourceKind, Source,
    };
    use limeos_domain::{Operation, Role};
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("core.sqlite")).unwrap();
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/werkzeug-hashes.json")).unwrap();
    let encoded = fixtures[0]["hash"].as_str().unwrap().to_owned();
    let token = "a".repeat(64);
    let digest = limeos_identity::digest(&token);
    let principal = db
        .call(move |s| {
            let p = s.import_legacy_user("reader", &encoded, now())?;
            s.revise_grants(
                &p.id,
                Role::Viewer,
                &[Scope {
                    operation: Operation::HealthRead,
                    resource: "uuid:allowed".into(),
                }],
                now(),
            )?;
            let record = s.login_record("reader")?.unwrap();
            s.create_session(&record, None, &digest, "csrf", now())?;
            Ok(record.principal)
        })
        .await
        .unwrap();
    let cache = limeos_observations::Cache::default();
    cache.update(ObservationBatch {
        host: Some(Default::default()),
        resources: ["allowed", "private"]
            .map(|name| {
                Resource::new(
                    format!("uuid:{name}"),
                    ResourceKind::Partition,
                    name.into(),
                    "mounted".into(),
                    Source::Storage,
                )
            })
            .to_vec(),
        sources: vec![Freshness {
            source: Source::Storage,
            state: Availability::Fresh,
            sampled_at: Some(now()),
            max_age_seconds: 90,
            warnings: Vec::new(),
        }],
    });
    let core = Core {
        automation_status: Default::default(),
        db,
        observations: cache,
        telemetry: None,
        container_socket: Arc::new("/unused".into()),
        storage_socket: Arc::new("/unused".into()),
        storage_target_socket: Arc::new("/unused-targets".into()),
        storage_mount_socket: Arc::new("/unused-mounts".into()),
        storage_readers: Arc::new(Semaphore::new(1)),
        stacks: Default::default(),
        compose_catalog: None,
        password_workers: Arc::new(PasswordWorkers::new(2)),
        dummy_hash: "unused".into(),
    };
    let view = core.observations(token.clone()).await.unwrap();
    assert!(view.host.is_none());
    assert_eq!(view.resources.len(), 1);
    assert_eq!(view.resources[0].id, "uuid:allowed");
    assert_eq!(
        core.history(token.clone(), limeos_contracts::HistoryRange::Day)
            .await
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    core.db
        .call(move |s| s.revise_grants(&principal.id, Role::Viewer, &[], now()))
        .await
        .unwrap();
    assert_eq!(
        core.observations(token).await.unwrap_err().0,
        ErrorCode::Unauthenticated
    );
}
#[tokio::test]
async fn imported_hash_upgrades_only_after_successful_login_and_session_is_durable() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("core.sqlite")).unwrap();
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/werkzeug-hashes.json")).unwrap();
    let core = Core {
        automation_status: Default::default(),
        db,
        password_workers: Arc::new(PasswordWorkers::new(2)),
        dummy_hash: limeos_identity::hash("dummy-long-password").unwrap().into(),
        observations: limeos_observations::Cache::default(),
        telemetry: None,
        container_socket: Arc::new("/unused".into()),
        storage_socket: Arc::new("/unused".into()),
        storage_target_socket: Arc::new("/unused-targets".into()),
        storage_mount_socket: Arc::new("/unused-mounts".into()),
        storage_readers: Arc::new(Semaphore::new(1)),
        stacks: Default::default(),
        compose_catalog: None,
    };
    for (index, fixture) in fixtures.as_array().unwrap().iter().enumerate() {
        let encoded = fixture["hash"].as_str().unwrap().to_owned();
        let username = format!("legacy-{index}");
        let imported_name = username.clone();
        let stored_hash = encoded.clone();
        core.db
            .call(move |s| s.import_legacy_user(&imported_name, &stored_hash, now()))
            .await
            .unwrap();
        let wrong = core
            .login(Login {
                username: username.clone(),
                password: "wrong".into(),
            })
            .await;
        assert_eq!(wrong.check, limeos_api::PasswordCheck::Rejected);
        assert_eq!(wrong.result.err().unwrap().0, ErrorCode::Unauthenticated);
        let user = username.clone();
        assert_eq!(
            core.db
                .call(move |s| Ok(s.login_record(&user)?.unwrap().password_hash))
                .await
                .unwrap(),
            encoded
        );
        let right = core
            .login(Login {
                username: username.clone(),
                password: fixture["password"].as_str().unwrap().into(),
            })
            .await;
        assert_eq!(right.check, limeos_api::PasswordCheck::Accepted);
        let issued = right.result.unwrap();
        assert!(core.session(issued.token.clone(), None).await.is_ok());
        let upgraded = core
            .db
            .call(move |s| Ok(s.login_record(&username)?.unwrap().password_hash))
            .await
            .unwrap();
        assert!(upgraded.starts_with("$argon2id$"));
        core.logout(issued.token.clone(), issued.view.csrf_token)
            .await
            .unwrap();
        assert!(core.session(issued.token, None).await.is_err());
    }
}
#[tokio::test]
async fn batch_imported_legacy_users_log_in_and_rerun_changes_nothing() {
    use limeos_contracts::LegacyUser;
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("core.sqlite")).unwrap();
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/werkzeug-hashes.json")).unwrap();
    let core = Core {
        automation_status: Default::default(),
        db,
        password_workers: Arc::new(PasswordWorkers::new(2)),
        dummy_hash: limeos_identity::hash("dummy-long-password").unwrap().into(),
        observations: limeos_observations::Cache::default(),
        telemetry: None,
        container_socket: Arc::new("/unused".into()),
        storage_socket: Arc::new("/unused".into()),
        storage_target_socket: Arc::new("/unused-targets".into()),
        storage_mount_socket: Arc::new("/unused-mounts".into()),
        storage_readers: Arc::new(Semaphore::new(1)),
        stacks: Default::default(),
        compose_catalog: None,
    };
    let users = || -> Vec<LegacyUser> {
        fixtures
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, f)| LegacyUser {
                username: format!("legacy-{i}"),
                password_hash: f["hash"].as_str().unwrap().into(),
            })
            .collect()
    };
    assert_eq!(
        core.import_legacy_users(users()).await.unwrap(),
        (2, vec![])
    );
    for (i, fixture) in fixtures.as_array().unwrap().iter().enumerate() {
        let username = format!("legacy-{i}");
        let wrong = core
            .login(Login {
                username: username.clone(),
                password: "wrong-password".into(),
            })
            .await;
        assert_eq!(wrong.check, limeos_api::PasswordCheck::Rejected);
        assert_eq!(wrong.result.err().unwrap().0, ErrorCode::Unauthenticated);
        let right = core
            .login(Login {
                username: username.clone(),
                password: fixture["password"].as_str().unwrap().into(),
            })
            .await;
        assert_eq!(right.check, limeos_api::PasswordCheck::Accepted);
        right.result.unwrap();
        let stored = core
            .db
            .call(move |s| Ok(s.login_record(&username)?.unwrap().password_hash))
            .await
            .unwrap();
        assert!(stored.starts_with("$argon2id$"));
    }
    // After login upgraded the hashes, a re-run skips both users and changes nothing.
    let before = core
        .db
        .call(|s| {
            Ok((
                s.login_record("legacy-0")?.unwrap().password_hash,
                s.login_record("legacy-1")?.unwrap().password_hash,
            ))
        })
        .await
        .unwrap();
    assert_eq!(
        core.import_legacy_users(users()).await.unwrap(),
        (0, vec!["legacy-0".to_owned(), "legacy-1".to_owned()])
    );
    let after = core
        .db
        .call(|s| {
            Ok((
                s.login_record("legacy-0")?.unwrap().password_hash,
                s.login_record("legacy-1")?.unwrap().password_hash,
            ))
        })
        .await
        .unwrap();
    assert_eq!(before, after);
    // A mixed batch imports only the new user.
    let mut mixed = users();
    mixed.truncate(1);
    mixed.push(limeos_contracts::LegacyUser {
        username: "legacy-new".into(),
        password_hash: fixtures[1]["hash"].as_str().unwrap().into(),
    });
    assert_eq!(
        core.import_legacy_users(mixed).await.unwrap(),
        (1, vec!["legacy-0".to_owned()])
    );
}
#[tokio::test]
async fn legacy_import_request_is_refused_for_a_non_root_peer_and_writes_nothing() {
    if rustix::process::getuid().is_root() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("core.sqlite")).unwrap();
    let core = Core {
        automation_status: Default::default(),
        db,
        password_workers: Arc::new(PasswordWorkers::new(2)),
        dummy_hash: limeos_identity::hash("dummy-long-password").unwrap().into(),
        observations: limeos_observations::Cache::default(),
        telemetry: None,
        container_socket: Arc::new("/unused".into()),
        storage_socket: Arc::new("/unused".into()),
        storage_target_socket: Arc::new("/unused-targets".into()),
        storage_mount_socket: Arc::new("/unused-mounts".into()),
        storage_readers: Arc::new(Semaphore::new(1)),
        stacks: Default::default(),
        compose_catalog: None,
    };
    let db = core.db.clone();
    let (mut client, server) = UnixStream::pair().unwrap();
    let worker = tokio::spawn(handle_rpc(core, server));
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/werkzeug-hashes.json")).unwrap();
    limeos_contracts::write_frame(
        &mut client,
        &CoreRequest::ImportLegacyUsers {
            version: VERSION,
            users: vec![limeos_contracts::LegacyUser {
                username: "holly".into(),
                password_hash: fixtures[0]["hash"].as_str().unwrap().into(),
            }],
        },
    )
    .await
    .unwrap();
    let response: CoreResponse = limeos_contracts::read_frame(&mut client).await.unwrap();
    let CoreResponse::Error(error) = response else {
        panic!("non-root peer was served");
    };
    assert_eq!(error.code, ErrorCode::Forbidden);
    let _ = worker.await;
    assert!(
        db.call(|s| s.login_record("holly"))
            .await
            .unwrap()
            .is_none()
    );
}
