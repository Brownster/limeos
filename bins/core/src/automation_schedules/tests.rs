use super::*;
use limeos_contracts::{Enrollment, Login, ObserveHealthEnable, ObserveHealthScheduleInput};
use limeos_domain::automation::{MissedRunPolicy, ScheduleStatus};
use rusqlite::{Connection, OpenFlags};
use std::time::Duration;

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
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn input() -> ObserveHealthScheduleInput {
    ObserveHealthScheduleInput {
        interval_seconds: 60,
        grace_seconds: 10,
        missed: MissedRunPolicy::CoalesceLatest,
        window: None,
    }
}
async fn setup() -> (tempfile::TempDir, Core, IssuedSession) {
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
fn counts(dir: &tempfile::TempDir) -> (i64, i64, i64, i64) {
    let conn = Connection::open_with_flags(
        dir.path().join("core.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    conn.query_row("SELECT (SELECT count(*) FROM automation_schedules),(SELECT count(*) FROM jobs),(SELECT count(*) FROM resource_locks),(SELECT count(*) FROM automation_occurrences)", [], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap()
}

#[tokio::test]
async fn actual_enable_relogin_create_read_restart_and_pause_are_metadata_only() {
    let (dir, initial, session) = setup().await;
    assert_eq!(
        initial
            .human_schedule_create(
                session.token.clone(),
                session.view.csrf_token.clone(),
                input(),
                deadline()
            )
            .await,
        Err(Error(ErrorCode::Forbidden))
    );
    assert_eq!(counts(&dir), (0, 0, 0, 0));
    initial
        .human_enable_observe_health(
            session.token,
            session.view.csrf_token,
            ObserveHealthEnable {
                password: "correct-long-password".into(),
            },
            deadline(),
        )
        .await
        .unwrap();
    let session = initial
        .login(Login {
            username: "alice".into(),
            password: "correct-long-password".into(),
        })
        .await
        .unwrap();
    let before = now();
    let row = initial
        .human_schedule_create(
            session.token.clone(),
            session.view.csrf_token.clone(),
            input(),
            deadline(),
        )
        .await
        .unwrap();
    assert!(row.schedule.definition.anchor >= before && row.schedule.definition.anchor <= now());
    assert_eq!(row.status, ScheduleStatus::AwaitingProducer);
    assert_eq!(counts(&dir), (1, 0, 0, 0));
    assert_eq!(
        initial
            .human_schedule_page(session.token.clone(), None, deadline())
            .await
            .unwrap()
            .items,
        vec![row.clone()]
    );
    assert_eq!(initial.observations.snapshot().revision, 0);
    drop(initial);
    let mut opened = None;
    for _ in 0..20 {
        match Database::open(&dir.path().join("core.sqlite")) {
            Ok(db) => {
                opened = Some(db);
                break;
            }
            Err(Error(ErrorCode::Conflict)) => tokio::time::sleep(Duration::from_millis(10)).await,
            Err(error) => panic!("unexpected reopen error: {error:?}"),
        }
    }
    let current = core(opened.expect("owned prior DB worker exited"));
    assert_eq!(
        current
            .human_schedule_page(session.token.clone(), None, deadline())
            .await
            .unwrap()
            .items,
        vec![row.clone()]
    );
    let paused = current
        .human_schedule_pause(
            session.token.clone(),
            session.view.csrf_token.clone(),
            row.schedule.id.clone(),
            PauseSchedule {
                revision: 1,
                paused: true,
            },
            deadline(),
        )
        .await
        .unwrap();
    assert_eq!(paused.status, ScheduleStatus::Paused);
    assert_eq!(
        paused.schedule.definition.anchor,
        row.schedule.definition.anchor
    );
    assert_eq!(
        current
            .human_schedule_pause(
                session.token.clone(),
                session.view.csrf_token.clone(),
                row.schedule.id.clone(),
                PauseSchedule {
                    revision: 1,
                    paused: false
                },
                deadline()
            )
            .await,
        Err(Error(ErrorCode::Conflict))
    );
    let resumed = current
        .human_schedule_pause(
            session.token,
            session.view.csrf_token,
            row.schedule.id,
            PauseSchedule {
                revision: 2,
                paused: false,
            },
            deadline(),
        )
        .await
        .unwrap();
    assert_eq!(resumed.status, ScheduleStatus::AwaitingProducer);
    assert_eq!(counts(&dir), (1, 0, 0, 0));
    assert_eq!(current.observations.snapshot().revision, 0);
}

#[tokio::test]
async fn csrf_revoked_session_and_original_expired_budget_make_no_rows() {
    let (dir, core, session) = setup().await;
    assert_eq!(
        core.human_schedule_create(session.token.clone(), "wrong".into(), input(), deadline())
            .await,
        Err(Error(ErrorCode::Forbidden))
    );
    assert_eq!(
        core.human_schedule_create(
            session.token.clone(),
            session.view.csrf_token.clone(),
            input(),
            Instant::now() - Duration::from_secs(1)
        )
        .await,
        Err(Error(ErrorCode::Expired))
    );
    core.logout(session.token.clone(), session.view.csrf_token)
        .await
        .unwrap();
    assert_eq!(
        core.human_schedule_page(session.token.clone(), None, deadline())
            .await,
        Err(Error(ErrorCode::Unauthenticated))
    );
    assert_eq!(
        core.human_schedule_create(session.token, "c".repeat(64), input(), deadline())
            .await,
        Err(Error(ErrorCode::Unauthenticated))
    );
    assert_eq!(counts(&dir), (0, 0, 0, 0));
}
