use super::*;
use limeos_contracts::{Enrollment, Login, ObserveHealthEnable};
use rusqlite::{Connection, OpenFlags};

fn core(db: Database) -> Core {
    Core {
        db,
        password_workers: Arc::new(Semaphore::new(2)),
        dummy_hash: "unused".into(),
        observations: limeos_observations::Cache::default(),
        telemetry: None,
        container_socket: Arc::new("/unused".into()),
        compose_catalog: None,
        storage_socket: Arc::new("/unused".into()),
        storage_target_socket: Arc::new("/unused".into()),
        storage_mount_socket: Arc::new("/unused".into()),
        storage_readers: Arc::new(Semaphore::new(1)),
    }
}
fn input(password: &str) -> ObserveHealthEnable {
    ObserveHealthEnable {
        password: password.into(),
    }
}
fn deadline() -> Instant {
    Instant::now() + std::time::Duration::from_secs(10)
}

#[derive(Debug, PartialEq, Eq)]
struct State {
    hash: String,
    revision: i64,
    grants: i64,
    events: i64,
    audit: i64,
}
fn state(dir: &tempfile::TempDir) -> State {
    let conn = Connection::open_with_flags(
        dir.path().join("core.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    conn.query_row("SELECT password_hash,grant_revision,(SELECT count(*) FROM grants),(SELECT count(*) FROM events),(SELECT audit_bytes FROM meta) FROM users",[],|r|Ok(State {hash:r.get(0)?,revision:r.get(1)?,grants:r.get(2)?,events:r.get(3)?,audit:r.get(4)?})).unwrap()
}
async fn setup() -> (tempfile::TempDir, Core, limeos_api::IssuedSession) {
    let dir = tempfile::tempdir().unwrap();
    let core = core(Database::open(&dir.path().join("core.sqlite")).unwrap());
    let token = "b".repeat(64);
    let digest = limeos_identity::digest(&token);
    core.db
        .call(move |store| store.issue_bootstrap(&digest, now()))
        .await
        .unwrap();
    core.enroll(Enrollment {
        token,
        username: "alice".into(),
        password: "correct-long-password".into(),
    })
    .await
    .unwrap();
    let session = core
        .login(Login {
            username: "alice".into(),
            password: "correct-long-password".into(),
        })
        .await
        .unwrap();
    (dir, core, session)
}

#[tokio::test]
async fn real_verifier_wrong_password_then_enable_relogin_and_noop() {
    let (dir, core, session) = setup().await;
    let before = state(&dir);
    assert_eq!(
        core.human_enable_observe_health(
            session.token.clone(),
            session.view.csrf_token.clone(),
            input("wrong"),
            deadline()
        )
        .await,
        Err(Error(ErrorCode::Unauthenticated))
    );
    assert_eq!(state(&dir), before);
    let result = core
        .human_enable_observe_health(
            session.token.clone(),
            session.view.csrf_token.clone(),
            input("correct-long-password"),
            deadline(),
        )
        .await
        .unwrap();
    assert!(result.enabled && result.reauthenticate);
    let after = state(&dir);
    assert_eq!(after.revision, before.revision + 1);
    assert_eq!(after.grants, before.grants + 1);
    assert_eq!(after.events, before.events + 1);
    assert_eq!(after.audit, before.audit + 256);
    assert!(core.session(session.token, None).await.is_err());
    let fresh = core
        .login(Login {
            username: "alice".into(),
            password: "correct-long-password".into(),
        })
        .await
        .unwrap();
    let before_noop = state(&dir);
    let result = core
        .human_enable_observe_health(
            fresh.token.clone(),
            fresh.view.csrf_token,
            input("correct-long-password"),
            deadline(),
        )
        .await
        .unwrap();
    assert!(result.enabled && !result.reauthenticate);
    assert_eq!(state(&dir), before_noop);
    assert!(core.session(fresh.token, None).await.is_ok());
}

#[tokio::test]
async fn real_legacy_verification_upgrades_only_a_true_grant_change() {
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/werkzeug-hashes.json"
    ))
    .unwrap();
    for fixture in fixtures.as_array().unwrap() {
        for already_enabled in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let core = core(Database::open(&dir.path().join("core.sqlite")).unwrap());
            let encoded = fixture["hash"].as_str().unwrap().to_owned();
            let token = "a".repeat(64);
            let csrf = "c".repeat(64);
            let digest = limeos_identity::digest(&token);
            let csrf_digest = limeos_identity::digest(&csrf);
            core.db
                .call(move |store| {
                    store.import_legacy_user("alice", &encoded, now())?;
                    let record = store.login_record("alice")?.unwrap();
                    store.create_session(&record, None, &digest, &csrf_digest, now())?;
                    if already_enabled {
                        store.enable_observe_health(
                            &digest,
                            &csrf_digest,
                            &record,
                            None,
                            now,
                            deadline(),
                        )?;
                        let record = store.login_record("alice")?.unwrap();
                        store.revoke_session(&digest, now())?;
                        store.create_session(&record, None, &digest, &csrf_digest, now())?;
                    }
                    Ok(())
                })
                .await
                .unwrap();
            let before = state(&dir);
            let result = core
                .human_enable_observe_health(
                    token,
                    csrf,
                    input(fixture["password"].as_str().unwrap()),
                    deadline(),
                )
                .await
                .unwrap();
            assert_eq!(result.reauthenticate, !already_enabled);
            let after = state(&dir);
            if already_enabled {
                assert_eq!(after, before);
            } else {
                assert!(after.hash.starts_with("$argon2id$"));
                assert_ne!(after.hash, before.hash);
                assert_eq!(after.revision, before.revision + 1);
            }
        }
    }
}

#[tokio::test]
async fn successful_real_verification_cannot_survive_session_revocation() {
    let (dir, core, session) = setup().await;
    let digest = limeos_identity::digest(&session.token);
    let csrf = limeos_identity::digest(&session.view.csrf_token);
    let first_digest = digest.clone();
    let first_csrf = csrf.clone();
    let record = core
        .db
        .call(move |store| store.observe_health_reauthentication(&first_digest, &first_csrf, now()))
        .await
        .unwrap();
    let verified = core
        .password(limeos_contracts::PasswordRequest::Verify {
            password: "correct-long-password".into(),
            encoded: record.password_hash.clone(),
        })
        .await
        .unwrap();
    assert!(verified.valid);
    core.logout(session.token, session.view.csrf_token)
        .await
        .unwrap();
    let before = state(&dir);
    assert_eq!(
        core.db
            .call(move |store| store.enable_observe_health(
                &digest,
                &csrf,
                &record,
                verified.upgraded.as_deref(),
                now,
                deadline()
            ))
            .await,
        Err(Error(ErrorCode::Unauthenticated))
    );
    assert_eq!(state(&dir), before);
}

#[tokio::test]
async fn expired_original_budget_and_invalid_password_shape_do_not_change_authority() {
    let (dir, core, session) = setup().await;
    let before = state(&dir);
    assert_eq!(
        core.human_enable_observe_health(
            session.token.clone(),
            session.view.csrf_token.clone(),
            input("correct-long-password"),
            Instant::now()
        )
        .await,
        Err(Error(ErrorCode::Expired))
    );
    for password in [String::new(), "x".repeat(1025)] {
        assert_eq!(
            core.human_enable_observe_health(
                session.token.clone(),
                session.view.csrf_token.clone(),
                input(&password),
                deadline()
            )
            .await,
            Err(Error(ErrorCode::InvalidInput))
        );
    }
    assert_eq!(state(&dir), before);
}

#[tokio::test]
async fn concurrent_real_verifiers_commit_one_grant_and_one_revision_event() {
    let (dir, core, session) = setup().await;
    let before = state(&dir);
    let (first, second) = tokio::join!(
        core.human_enable_observe_health(
            session.token.clone(),
            session.view.csrf_token.clone(),
            input("correct-long-password"),
            deadline()
        ),
        core.human_enable_observe_health(
            session.token.clone(),
            session.view.csrf_token.clone(),
            input("correct-long-password"),
            deadline()
        ),
    );
    let outcomes = [first, second];
    assert_eq!(
        outcomes
            .iter()
            .filter(|result| matches!(result,Ok(value) if value.enabled && value.reauthenticate))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|result| matches!(result, Err(Error(ErrorCode::Unauthenticated))))
            .count(),
        1
    );
    let after = state(&dir);
    assert_eq!(after.revision, before.revision + 1);
    assert_eq!(after.grants, before.grants + 1);
    assert_eq!(after.events, before.events + 1);
    assert_eq!(after.audit, before.audit + 256);
}
