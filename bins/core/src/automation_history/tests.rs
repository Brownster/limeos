//! Actual Store worker and HTTP routes; health inputs are explicit fixtures.
use super::*;
use limeos_domain::automation::{
    AlertKind, AlertSignal, CreateSchedule, DueDecision, HealthPolicyDefinition,
    HealthPolicySilence, MissedRunPolicy, ReplaceHealthPolicy, Reservation, ScheduleDefinition,
    ScheduledOperation, Settlement, Severity, SignalStatus,
};
use limeos_domain::{Operation, Principal, Role};
use limeos_persistence::{HealthObservation, ObservationAdmission, ScopedHealthSignal};
use rusqlite::{Connection, OpenFlags};
use std::time::Duration;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

#[tokio::test]
async fn owned_policy_uses_real_http_cas_csrf_current_grants_and_restart() {
    let (dir, initial, actor, _) = setup(true);
    let (address, server) = http(initial.clone()).await;
    let path = "/api/v1/automation/health-policy";
    let token = "a".repeat(64);
    let csrf = "c".repeat(64);
    let before = counts(&dir);
    let (status, value) = request(address, "GET", path, &token, None, "").await;
    assert_eq!(status, 200);
    assert_eq!(value["revision"], 0);
    assert_eq!(counts(&dir), before);
    assert_eq!(
        request(address, "GET", path, &"b".repeat(64), None, "")
            .await
            .0,
        403
    );
    let definition = HealthPolicyDefinition {
        fail_threshold: 3,
        required_mounts: vec!["/mnt/Media".into(), "/mnt/media".into()],
        silences: vec![HealthPolicySilence {
            kind: AlertKind::Container,
            key: "container:fixture".into(),
            created_at: "2026-10-20T12:00:00.123456Z".into(),
            expires_at: None,
            reason: "fixture maintenance".into(),
        }],
        ..Default::default()
    };
    let input = serde_json::to_string(&ReplaceHealthPolicy {
        revision: 0,
        definition: definition.clone(),
    })
    .unwrap();
    assert_eq!(
        request(address, "POST", path, &token, None, &input).await.0,
        403
    );
    assert_eq!(counts(&dir), before);
    let (status, saved) = request(address, "POST", path, &token, Some(&csrf), &input).await;
    assert_eq!(status, 200);
    assert_eq!(saved["revision"], 1);
    assert_eq!(
        saved["definition"],
        serde_json::to_value(&definition).unwrap()
    );
    let after = counts(&dir);
    assert_eq!(after.3, before.3 + 1);
    assert_eq!(
        (after.0, after.1, after.2),
        (before.0, before.1, before.2),
        "configuration creates no observation, job or delivery"
    );
    assert_eq!(
        request(address, "POST", path, &token, Some(&csrf), &input)
            .await
            .1,
        saved
    );
    assert_eq!(counts(&dir), after);
    let changed = input.replace("\"fail_threshold\":3", "\"fail_threshold\":4");
    assert_eq!(
        request(address, "POST", path, &token, Some(&csrf), &changed)
            .await
            .0,
        409
    );
    assert_eq!(
        request(
            address,
            "GET",
            &format!("{path}?owner=other"),
            &token,
            None,
            ""
        )
        .await
        .0,
        400
    );
    initial
        .db
        .call(|store| {
            let bob = store.login_record("bob")?.unwrap().principal;
            store.revise_grants(&bob.id, Role::Operator, &scopes(), now())?;
            session(store, "bob", &"e".repeat(64), &"f".repeat(64));
            Ok(())
        })
        .await
        .unwrap();
    let bob = request(address, "GET", path, &"e".repeat(64), None, "").await;
    assert_eq!(bob.0, 200);
    assert_eq!(
        bob.1["revision"], 0,
        "another manager cannot read the owner's policy"
    );
    initial
        .db
        .call(move |store| {
            store.revise_grants(
                &actor.id,
                Role::Viewer,
                &[Scope {
                    operation: Operation::HealthRead,
                    resource: "host:system".into(),
                }],
                now(),
            )?;
            session(store, "alice", &"1".repeat(64), &"2".repeat(64));
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(request(address, "GET", path, &token, None, "").await.0, 401);
    assert_eq!(
        request(address, "GET", path, &"1".repeat(64), None, "")
            .await
            .0,
        403
    );
    server.abort();
    let _ = server.await;
    drop(initial);
    let mut database = None;
    for _ in 0..20 {
        match Database::open(&dir.path().join("core.sqlite")) {
            Ok(value) => {
                database = Some(value);
                break;
            }
            Err(Error(ErrorCode::Conflict)) => tokio::time::sleep(Duration::from_millis(10)).await,
            Err(error) => panic!("reopen {error:?}"),
        }
    }
    let reopened = core(database.expect("prior worker released"));
    let value = reopened
        .db
        .call(|store| {
            let actor = store.login_record("alice")?.unwrap().principal;
            store.revise_grants(&actor.id, Role::Operator, &scopes(), now())?;
            session(store, "alice", &"3".repeat(64), &"4".repeat(64));
            Ok(())
        })
        .await;
    value.unwrap();
    assert_eq!(
        serde_json::to_value(
            reopened
                .human_health_policy("3".repeat(64), deadline())
                .await
                .unwrap()
        )
        .unwrap(),
        saved
    );
}

#[tokio::test]
async fn policy_deadlines_refuse_expired_queued_work_and_reconcile_late_commits() {
    let (dir, initial, _, _) = setup(true);
    let before = counts(&dir);
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, hold) = std::sync::mpsc::channel();
    let db = initial.db.clone();
    let blocked = tokio::spawn(async move {
        db.call(move |_| {
            entered.send(()).unwrap();
            hold.recv_timeout(Duration::from_secs(2)).unwrap();
            Ok(())
        })
        .await
    });
    ready.await.unwrap();
    let input = ReplaceHealthPolicy {
        revision: 0,
        definition: HealthPolicyDefinition {
            fail_threshold: 3,
            ..Default::default()
        },
    };
    assert_eq!(
        initial
            .human_health_policy_replace(
                "a".repeat(64),
                "c".repeat(64),
                input.clone(),
                Instant::now() + Duration::from_millis(80)
            )
            .await,
        Err(Error(ErrorCode::Expired))
    );
    release.send(()).unwrap();
    blocked.await.unwrap().unwrap();
    initial.db.call(|_| Ok(())).await.unwrap();
    assert_eq!(counts(&dir), before);
    let (committed, ready) = tokio::sync::oneshot::channel();
    let (release, hold) = std::sync::mpsc::channel();
    let worker = initial.clone();
    let original = Instant::now() + Duration::from_millis(250);
    let pending = tokio::spawn(async move {
        worker
            .history_store(original, move |store| {
                let value = store.observe_health_policy_replace(
                    &limeos_identity::digest(&"a".repeat(64)),
                    &limeos_identity::digest(&"c".repeat(64)),
                    &input,
                    now,
                    original,
                )?;
                committed.send(()).unwrap();
                hold.recv_timeout(Duration::from_secs(2)).unwrap();
                Ok(value)
            })
            .await
    });
    ready.await.unwrap();
    assert_eq!(pending.await.unwrap(), Err(Error(ErrorCode::Expired)));
    release.send(()).unwrap();
    initial.db.call(|_| Ok(())).await.unwrap();
    let value = initial
        .human_health_policy("a".repeat(64), deadline())
        .await
        .unwrap();
    assert_eq!(value.revision, 1);
    assert_eq!(value.definition.fail_threshold, 3);
    assert_eq!(
        counts(&dir).3,
        before.3 + 1,
        "one committed replacement, never retried"
    );
}

#[tokio::test]
async fn concurrent_policy_replacements_commit_one_owned_revision() {
    let (dir, initial, _, _) = setup(true);
    let before = counts(&dir);
    let left = ReplaceHealthPolicy {
        revision: 0,
        definition: HealthPolicyDefinition {
            fail_threshold: 3,
            ..Default::default()
        },
    };
    let right = ReplaceHealthPolicy {
        revision: 0,
        definition: HealthPolicyDefinition {
            fail_threshold: 4,
            ..Default::default()
        },
    };
    let (left, right) = tokio::join!(
        initial.human_health_policy_replace("a".repeat(64), "c".repeat(64), left, deadline()),
        initial.human_health_policy_replace("a".repeat(64), "c".repeat(64), right, deadline())
    );
    let winner = match (left, right) {
        (Ok(value), Err(Error(ErrorCode::Conflict)))
        | (Err(Error(ErrorCode::Conflict)), Ok(value)) => value,
        values => panic!("one CAS winner: {values:?}"),
    };
    assert_eq!(winner.revision, 1);
    assert_eq!(
        initial
            .human_health_policy("a".repeat(64), deadline())
            .await
            .unwrap(),
        winner
    );
    assert_eq!(counts(&dir).3, before.3 + 1);
}
fn scopes() -> Vec<Scope> {
    vec![
        Scope {
            operation: Operation::HealthRead,
            resource: "host:system".into(),
        },
        Scope {
            operation: Operation::HealthRead,
            resource: format!("container:{}", "a".repeat(64)),
        },
        Scope {
            operation: Operation::AutomationManage,
            resource: "automation:observe-health".into(),
        },
    ]
}
fn core(db: Database) -> Core {
    Core {
        automation_status: Default::default(),
        db,
        password_workers: Arc::new(Semaphore::new(2)),
        dummy_hash: "unused".into(),
        observations: Default::default(),
        telemetry: None,
        container_socket: Arc::new("/unused".into()),
        compose_catalog: None,
        storage_socket: Arc::new("/unused".into()),
        storage_target_socket: Arc::new("/unused".into()),
        storage_mount_socket: Arc::new("/unused".into()),
        storage_readers: Arc::new(Semaphore::new(1)),
        stacks: Default::default(),
    }
}
fn fixture_signal(store: &mut Store, actor: &Principal, at: i64) {
    let row = store
        .automation_create(
            actor,
            &CreateSchedule {
                definition: ScheduleDefinition {
                    operation: ScheduledOperation::ObserveHealth,
                    anchor: 0,
                    interval_seconds: 30,
                    grace_seconds: 5,
                    missed: MissedRunPolicy::CoalesceLatest,
                    window: None,
                },
            },
            at,
        )
        .unwrap()
        .schedule;
    let DueDecision::Ready(key) = row.due(at).unwrap() else {
        panic!("fixture due")
    };
    let Reservation::Queued(job) = store
        .automation_reserve_current(&row, &Settlement::Observation(key), || at, deadline())
        .unwrap()
    else {
        panic!("fixture queued")
    };
    let resource = format!("container:{}", "a".repeat(64));
    let ObservationAdmission::Lease(lease) = store
        .automation_observation_begin(
            &job,
            vec!["host:system".into(), resource.clone()],
            || at,
            deadline(),
        )
        .unwrap()
    else {
        panic!("fixture lease")
    };
    store
        .automation_observation_complete(
            &lease,
            &HealthObservation {
                signals: vec![ScopedHealthSignal {
                    resource,
                    signal: AlertSignal {
                        key: "container:fixture".into(),
                        kind: AlertKind::Container,
                        status: SignalStatus::Failing,
                        severity: Severity::Warning,
                        summary: "Fixture health fact; no protected collector invoked".into(),
                    },
                }],
                complete: true,
            },
            || at,
        )
        .unwrap();
}
fn session(store: &mut Store, username: &str, token: &str, csrf: &str) {
    let record = store.login_record(username).unwrap().unwrap();
    store
        .create_session(
            &record,
            None,
            &limeos_identity::digest(token),
            &limeos_identity::digest(csrf),
            now(),
        )
        .unwrap();
}
fn setup(publish: bool) -> (tempfile::TempDir, Core, Principal, String) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let mut store = Store::open(&path).unwrap();
    let at = now() - 3;
    store.issue_bootstrap("bootstrap", at - 30).unwrap();
    store
        .enroll("bootstrap", "alice", "$argon2id$fixture", at - 29)
        .unwrap();
    let first = store.login_record("alice").unwrap().unwrap().principal;
    store
        .revise_grants(&first.id, Role::Operator, &scopes(), at - 28)
        .unwrap();
    let actor = store.login_record("alice").unwrap().unwrap().principal;
    let bob = store.import_legacy_user("bob", "pbkdf2:sha256:1000000$IgjABokZwwl2gggv$686315920b24fb807abeb0a436ce6385be3f3026e35d934ffbaf15be1a0e7ce1", at - 27).unwrap();
    store
        .revise_grants(&bob.id, Role::Viewer, &scopes()[..2], at - 26)
        .unwrap();
    fixture_signal(&mut store, &actor, at);
    fixture_signal(&mut store, &actor, at + 1);
    if publish {
        store
            .automation_publish_in_app(None, 32, || at + 1, deadline())
            .unwrap();
    }
    session(&mut store, "alice", &"a".repeat(64), &"c".repeat(64));
    session(&mut store, "bob", &"b".repeat(64), &"d".repeat(64));
    let notes = store
        .observe_health_notification_page(
            &limeos_identity::digest(&"a".repeat(64)),
            None,
            now,
            deadline(),
        )
        .unwrap();
    let id = notes.items[0].id.clone();
    drop(store);
    (dir, core(Database::open(&path).unwrap()), actor, id)
}
fn counts(dir: &tempfile::TempDir) -> (i64, i64, i64, i64, i64) {
    let conn = Connection::open_with_flags(
        dir.path().join("core.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    conn.query_row("SELECT (SELECT count(*) FROM jobs),(SELECT count(*) FROM automation_observation_results),(SELECT count(*) FROM automation_notifications),(SELECT count(*) FROM events),audit_bytes FROM meta WHERE id=1", [], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap()
}
async fn http(
    core: Core,
) -> (
    std::net::SocketAddr,
    tokio::task::JoinHandle<std::io::Result<()>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = limeos_api::router(core, "https://localhost".into());
    (
        address,
        tokio::spawn(async move { limeos_api::serve(listener, app).await }),
    )
}
async fn request(
    address: std::net::SocketAddr,
    method: &str,
    path: &str,
    token: &str,
    csrf: Option<&str>,
    body: &str,
) -> (u16, serde_json::Value) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let csrf = csrf
        .map(|value| format!("X-CSRF-Token: {value}\r\n"))
        .unwrap_or_default();
    stream.write_all(format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nOrigin: https://localhost\r\nCookie: __Host-limeos={token}\r\n{csrf}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(5),
        stream.take(70_000).read_to_end(&mut bytes),
    )
    .await
    .unwrap()
    .unwrap();
    let response = String::from_utf8(bytes).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    assert!(
        head.to_ascii_lowercase()
            .contains("cache-control: no-store")
    );
    (
        status,
        serde_json::from_str(body).unwrap_or(serde_json::Value::Null),
    )
}

#[tokio::test]
async fn real_http_history_acknowledgment_and_restart_use_existing_store_proofs() {
    let (dir, initial, _, id) = setup(false);
    let before = counts(&dir);
    let (address, server) = http(initial.clone()).await;
    let path = format!("/api/v1/automation/notifications/{id}/acknowledgment");
    let token = "a".repeat(64);
    let csrf = "c".repeat(64);
    for route in [
        "/api/v1/automation/alerts",
        "/api/v1/automation/notifications",
    ] {
        let (status, page) = request(address, "GET", route, &token, None, "").await;
        assert_eq!(status, 200);
        assert_eq!(page["items"].as_array().unwrap().len(), 1);
    }
    assert_eq!(counts(&dir), before);
    assert_eq!(
        request(
            address,
            "POST",
            &path,
            &token,
            Some(&csrf),
            r#"{"revision":1}"#
        )
        .await
        .0,
        409
    );
    assert_eq!(
        request(address, "POST", &path, &token, None, r#"{"revision":1}"#)
            .await
            .0,
        403
    );
    assert_eq!(
        request(address, "GET", &path, &token, None, "").await.0,
        405
    );
    assert_eq!(counts(&dir), before);
    initial
        .db
        .call(|store| store.automation_publish_in_app(None, 32, now, deadline()))
        .await
        .unwrap();
    let visible = initial
        .human_notification_page(token.clone(), None, deadline())
        .await
        .unwrap()
        .items
        .remove(0);
    let revision = visible.revision;
    let body = serde_json::json!({"revision": revision}).to_string();
    let (status, acknowledged) = request(address, "POST", &path, &token, Some(&csrf), &body).await;
    assert_eq!(status, 200);
    assert_eq!(acknowledged["revision"], revision + 1);
    assert!(acknowledged["acknowledged_at"].is_number());
    let after = counts(&dir);
    assert_eq!(
        request(address, "POST", &path, &token, Some(&csrf), &body).await,
        (200, acknowledged.clone())
    );
    assert_eq!(
        request(
            address,
            "POST",
            &path,
            &token,
            Some(&csrf),
            &serde_json::json!({"revision": revision + 3}).to_string()
        )
        .await
        .0,
        409
    );
    assert_eq!(counts(&dir), after);
    server.abort();
    let _ = server.await;
    drop(initial);
    let mut database = None;
    for _ in 0..20 {
        match Database::open(&dir.path().join("core.sqlite")) {
            Ok(value) => {
                database = Some(value);
                break;
            }
            Err(Error(ErrorCode::Conflict)) => tokio::time::sleep(Duration::from_millis(10)).await,
            Err(error) => panic!("unexpected reopen failure: {error:?}"),
        }
    }
    let reopened = core(database.expect("prior database worker exited"));
    let page = reopened
        .human_notification_page(token, None, deadline())
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(&page.items[0]).unwrap(), acknowledged);
    assert_eq!(counts(&dir), after);
}

#[tokio::test]
async fn current_owner_and_resource_grants_control_real_http_history() {
    let (_dir, initial, actor, id) = setup(true);
    let (address, server) = http(initial.clone()).await;
    let bob = "b".repeat(64);
    let token = "a".repeat(64);
    let path = format!("/api/v1/automation/notifications/{id}/acknowledgment");
    assert_eq!(
        request(address, "GET", "/api/v1/automation/alerts", &bob, None, "")
            .await
            .1["items"],
        serde_json::json!([])
    );
    assert_eq!(
        request(
            address,
            "POST",
            &path,
            &bob,
            Some(&"d".repeat(64)),
            r#"{"revision":1}"#
        )
        .await
        .0,
        404
    );
    let owner = actor.id;
    initial
        .db
        .call(move |store| {
            store.revise_grants(
                &owner,
                Role::Viewer,
                &[Scope {
                    operation: Operation::HealthRead,
                    resource: "host:system".into(),
                }],
                now(),
            )?;
            session(store, "alice", &"e".repeat(64), &"f".repeat(64));
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        request(
            address,
            "GET",
            "/api/v1/automation/alerts",
            &token,
            None,
            ""
        )
        .await
        .0,
        401
    );
    let current = "e".repeat(64);
    for route in [
        "/api/v1/automation/alerts",
        "/api/v1/automation/notifications",
    ] {
        let (status, page) = request(address, "GET", route, &current, None, "").await;
        assert_eq!(status, 200);
        assert_eq!(page["items"], serde_json::json!([]));
        assert_eq!(page["partial"], true);
    }
    assert_eq!(
        request(
            address,
            "POST",
            &path,
            &current,
            Some(&"f".repeat(64)),
            r#"{"revision":1}"#
        )
        .await
        .0,
        403
    );
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn original_deadline_prevents_a_queued_acknowledgment_after_timeout() {
    let (dir, initial, _, id) = setup(true);
    let revision = initial
        .human_notification_page("a".repeat(64), None, deadline())
        .await
        .unwrap()
        .items[0]
        .revision;
    let before = counts(&dir);
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, hold) = std::sync::mpsc::channel();
    let db = initial.db.clone();
    let blocked = tokio::spawn(async move {
        db.call(move |_| {
            entered.send(()).unwrap();
            hold.recv_timeout(Duration::from_secs(2)).unwrap();
            Ok(())
        })
        .await
    });
    ready.await.unwrap();
    assert_eq!(
        initial
            .human_notification_acknowledge(
                "a".repeat(64),
                "c".repeat(64),
                id.clone(),
                AcknowledgeNotification { revision },
                Instant::now() + Duration::from_millis(80)
            )
            .await,
        Err(Error(ErrorCode::Expired))
    );
    release.send(()).unwrap();
    blocked.await.unwrap().unwrap();
    initial.db.call(|_| Ok(())).await.unwrap();
    assert_eq!(counts(&dir), before);
    assert!(
        initial
            .human_notification_page("a".repeat(64), None, deadline())
            .await
            .unwrap()
            .items[0]
            .acknowledged_at
            .is_none()
    );
}

#[tokio::test]
async fn a_late_committed_acknowledgment_is_discovered_by_read_without_repeating_it() {
    let (dir, initial, _, id) = setup(true);
    let revision = initial
        .human_notification_page("a".repeat(64), None, deadline())
        .await
        .unwrap()
        .items[0]
        .revision;
    let (committed, ready) = tokio::sync::oneshot::channel();
    let (release, hold) = std::sync::mpsc::channel();
    let worker = initial.clone();
    let original_deadline = Instant::now() + Duration::from_millis(250);
    let request = tokio::spawn(async move {
        worker
            .history_store(original_deadline, move |store| {
                let row = store.observe_health_notification_acknowledge(
                    &limeos_identity::digest(&"a".repeat(64)),
                    &limeos_identity::digest(&"c".repeat(64)),
                    &id,
                    &AcknowledgeNotification { revision },
                    now,
                    original_deadline,
                )?;
                committed.send(()).unwrap();
                hold.recv_timeout(Duration::from_secs(2)).unwrap();
                Ok(row)
            })
            .await
    });
    ready.await.unwrap();
    assert_eq!(request.await.unwrap(), Err(Error(ErrorCode::Expired)));
    release.send(()).unwrap();
    initial.db.call(|_| Ok(())).await.unwrap();
    let after = counts(&dir);
    assert!(
        initial
            .human_notification_page("a".repeat(64), None, deadline())
            .await
            .unwrap()
            .items[0]
            .acknowledged_at
            .is_some()
    );
    assert_eq!(counts(&dir), after);
}
