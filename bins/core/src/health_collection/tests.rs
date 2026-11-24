use super::*;
use limeos_domain::{Operation, Role, Scope, automation::*};
use limeos_persistence::{Database, Store};
use std::{
    sync::atomic::{AtomicI64, AtomicUsize, Ordering},
    time::Duration,
};
use tokio::net::UnixListener;

struct Fixture {
    dir: tempfile::TempDir,
    reader: HealthReader,
    requests: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
    release: Arc<tokio::sync::Notify>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn resource() -> String {
    format!("container:{:064x}", 0)
}
fn cohort(nonce: String, at: i64) -> ContainerHealthCohort {
    ContainerHealthCohort::new(
        nonce,
        "fixture-engine".into(),
        at,
        vec![ContainerHealthMember {
            resource: resource(),
            signal: Some(AlertSignal {
                key: resource(),
                kind: AlertKind::Container,
                status: SignalStatus::Failing,
                severity: Severity::Warning,
                summary: "Fixture is not running".into(),
            }),
        }],
    )
    .unwrap()
}
async fn fixture(mode: u8, at: i64) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = super::test_socket(dir.path());
    let listener = UnixListener::bind(&path).unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let clock = Arc::new(AtomicI64::new(at));
    let sampled = clock.clone();
    let endpoint = path.clone();
    let release = Arc::new(tokio::sync::Notify::new());
    let released = release.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let Ok(Request::ContainerHealth { version, nonce }) =
                limeos_contracts::read_frame::<Request>(&mut stream).await
            else {
                continue;
            };
            assert_eq!(version, limeos_contracts::VERSION);
            seen.fetch_add(1, Ordering::SeqCst);
            if mode == 13 {
                released.notified().await;
            }
            if mode == 5 {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            let source = if mode == 2 {
                at - 1
            } else if mode == 9 {
                at + 1
            } else {
                at
            };
            let nonce = if mode == 1 { "f".repeat(64) } else { nonce };
            let mut receipt = Receipt::empty();
            receipt.container_health = Some(if mode == 12 {
                ContainerHealthCohort::new(nonce, "fixture-engine".into(), source, vec![]).unwrap()
            } else {
                cohort(nonce, source)
            });
            match mode {
                3 => receipt.error = Some(ErrorCode::Unavailable),
                4 => {
                    receipt.stack_cohort = Some(
                        limeos_domain::deployment::discovery::DiscoveryCohort::new(None, None)
                            .unwrap(),
                    )
                }
                6 => receipt.container_health = None,
                7 => receipt.ready = false,
                8 => receipt.version = 2,
                10 => {
                    sampled.store(at - 1, Ordering::SeqCst);
                }
                11 => {
                    std::fs::remove_file(&endpoint).unwrap();
                    let _replacement = UnixListener::bind(&endpoint).unwrap();
                }
                _ => {}
            }
            let _ = limeos_contracts::write_frame(&mut stream, &receipt).await;
        }
    });
    let mut reader = HealthReader::new(path, Arc::new(Semaphore::new(1)));
    reader.owner = super::test_owner();
    let wall = clock.clone();
    reader.clock = Arc::new(move || wall.load(Ordering::SeqCst));
    Fixture {
        dir,
        reader,
        requests,
        task,
        release,
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(2)
}

#[tokio::test]
async fn fresh_owned_response_retains_original_deadline_and_new_reads_use_new_nonces() {
    let f = fixture(0, 120).await;
    let original = Instant::now() + Duration::from_millis(150);
    let fresh = f.reader.read(original).await.unwrap();
    assert_eq!(fresh.deadline(), original);
    assert_eq!(fresh.cohort().members().len(), 1);
    let next = f.reader.read(deadline()).await.unwrap();
    assert_ne!(fresh.cohort().nonce(), next.cohort().nonce());
    assert_eq!(fresh.deadline(), original);
    tokio::time::sleep_until(tokio::time::Instant::from_std(
        original + Duration::from_millis(1),
    ))
    .await;
    assert_eq!(fresh.ensure_live(), Err(Error(ErrorCode::Expired)));
    assert_eq!(f.requests.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn replay_wrong_payload_clock_version_and_socket_replacement_refuse() {
    for mode in [1, 2, 3, 4, 6, 7, 8, 9, 10, 11] {
        let f = fixture(mode, 120).await;
        assert!(f.reader.read(deadline()).await.is_err(), "mode {mode}");
        assert_eq!(f.requests.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn a_socket_core_owns_and_a_symlink_refuse_before_request_bytes() {
    let mut f = fixture(0, 120).await;
    // The socket this test process serves is core's own when core runs as
    // this account, and outside root's /run under the production rule: both
    // refused before connecting. (A peer other than the owner, a root-owned
    // socket and unsafe directories are refused by the same rule,
    // `limeos_contracts::ExecutorOwner`, whose tests cover them.)
    let uid = rustix::process::geteuid().as_raw();
    for owner in [
        limeos_contracts::ExecutorOwner::with_anchor_owner(uid, uid),
        crate::storage::executor_owner(),
    ] {
        f.reader.owner = owner;
        assert_eq!(
            f.reader.read(deadline()).await.err(),
            Some(Error(ErrorCode::Forbidden))
        );
    }
    assert_eq!(f.requests.load(Ordering::SeqCst), 0);
    f.reader.owner = super::test_owner();
    let linked = f.dir.path().join("linked.sock");
    std::os::unix::fs::symlink(&f.reader.socket, &linked).unwrap();
    f.reader.socket = linked;
    assert!(f.reader.read(deadline()).await.is_err());
    assert_eq!(f.requests.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn expired_and_concurrent_readers_cannot_renew_or_queue_a_fresh_request() {
    let f = fixture(5, 120).await;
    let until = Instant::now() + Duration::from_millis(150);
    let first = f.reader.read(until);
    tokio::pin!(first);
    tokio::select! {result=&mut first=>panic!("unexpected {result:?}"),_=tokio::time::sleep(Duration::from_millis(20))=>{}}
    assert_eq!(
        f.reader.read(deadline()).await.err(),
        Some(Error(ErrorCode::Overloaded))
    );
    assert_eq!(first.await.err(), Some(Error(ErrorCode::Expired)));
    assert_eq!(f.requests.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.reader.read(Instant::now()).await.err(),
        Some(Error(ErrorCode::Expired))
    );
}

fn database(granted: bool, legacy_silence: bool) -> (tempfile::TempDir, Database, String) {
    database_at(granted, legacy_silence, super::super::now())
}
fn database_at(
    granted: bool,
    legacy_silence: bool,
    at: i64,
) -> (tempfile::TempDir, Database, String) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let mut store = Store::open(&path).unwrap();
    store.issue_bootstrap("boot", at - 100).unwrap();
    store
        .enroll("boot", "alice", "$argon2id$fixture", at - 99)
        .unwrap();
    let initial = store.login_record("alice").unwrap().unwrap().principal;
    let mut scopes = vec![
        Scope {
            operation: Operation::HealthRead,
            resource: "host:system".into(),
        },
        Scope {
            operation: Operation::AutomationManage,
            resource: "automation:observe-health".into(),
        },
    ];
    if granted {
        scopes.push(Scope {
            operation: Operation::HealthRead,
            resource: resource(),
        });
    }
    store
        .revise_grants(&initial.id, Role::Operator, &scopes, at - 98)
        .unwrap();
    let record = store.login_record("alice").unwrap().unwrap();
    store
        .create_session(&record, None, "session", "csrf", at - 90)
        .unwrap();
    let mut definition = HealthPolicyDefinition {
        fail_threshold: 1,
        ..Default::default()
    };
    if legacy_silence {
        definition.silences.push(HealthPolicySilence {
            kind: AlertKind::Container,
            key: "container:plex".into(),
            created_at: "2026-10-31T00:00:00Z".into(),
            expires_at: None,
            reason: "Needs concrete binding".into(),
        });
    }
    store
        .observe_health_policy_replace(
            "session",
            "csrf",
            &ReplaceHealthPolicy {
                revision: 0,
                definition,
            },
            || at,
            Instant::now() + Duration::from_secs(10),
        )
        .unwrap();
    let id = store
        .automation_create(
            &record.principal,
            &CreateSchedule {
                definition: ScheduleDefinition {
                    operation: ScheduledOperation::ObserveHealth,
                    anchor: at,
                    interval_seconds: 30,
                    grace_seconds: 5,
                    missed: MissedRunPolicy::CoalesceLatest,
                    window: None,
                },
            },
            at,
        )
        .unwrap()
        .schedule
        .id;
    drop(store);
    let db = Database::open(&path).unwrap();
    (dir, db, id)
}
fn counts(dir: &tempfile::TempDir) -> (i64, i64, i64, i64, i64, i64) {
    rusqlite::Connection::open_with_flags(dir.path().join("core.sqlite"),rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap().query_row("SELECT (SELECT count(*) FROM jobs),(SELECT count(*) FROM automation_occurrences),(SELECT count(*) FROM automation_observation_results),(SELECT count(*) FROM automation_notifications),(SELECT count(*) FROM events),audit_bytes FROM meta WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap()
}
#[tokio::test]
async fn actual_rpc_to_store_pass_is_partial_and_creates_one_incident_without_duplicate_runs() {
    let (dir, db, id) = database(true, false);
    let f = fixture(0, super::super::now()).await;
    let bridge = crate::automation_store::ObservationStoreBridge::new(db);
    let result = bridge
        .poll_containers(&f.reader, id.clone(), deadline())
        .await
        .unwrap();
    assert!(matches!(
        result,
        Some(limeos_persistence::ObservationPollOutcome::Committed(
            limeos_persistence::ObservationResult {
                complete: false,
                notifications: 1,
                ..
            }
        ))
    ));
    let before = counts(&dir);
    assert_eq!((before.0, before.1, before.2, before.3), (1, 1, 1, 1));
    assert_eq!(
        bridge
            .poll_containers(&f.reader, id, deadline())
            .await
            .unwrap(),
        None
    );
    assert_eq!(counts(&dir), before);
    assert_eq!(f.requests.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn missing_concrete_grants_and_legacy_silence_bindings_create_no_sample_or_job() {
    for (granted, legacy) in [(false, false), (true, true)] {
        let (dir, db, id) = database(granted, legacy);
        let f = fixture(0, super::super::now()).await;
        let bridge = crate::automation_store::ObservationStoreBridge::new(db);
        let before = counts(&dir);
        assert!(
            bridge
                .poll_containers(&f.reader, id, deadline())
                .await
                .is_err()
        );
        assert_eq!(counts(&dir), before);
        assert!(
            bridge
                .queued_page(None, deadline())
                .await
                .unwrap()
                .items
                .is_empty()
        );
    }
}

#[tokio::test]
async fn admission_expires_in_the_actual_database_queue_without_contacting_the_reader() {
    let (dir, db, id) = database(true, false);
    let f = fixture(0, super::super::now()).await;
    let bridge = crate::automation_store::ObservationStoreBridge::new(db.clone());
    let (started, ready) = std::sync::mpsc::channel();
    let (release, held) = std::sync::mpsc::channel();
    let blocked = tokio::spawn({
        let db = db.clone();
        async move {
            db.call(move |_| {
                started.send(()).unwrap();
                held.recv().unwrap();
                Ok(())
            })
            .await
        }
    });
    // Admission itself is independently blocked before any fresh RPC.
    tokio::task::spawn_blocking(move || ready.recv().unwrap())
        .await
        .unwrap();
    let before = counts(&dir);
    let answer = bridge
        .poll_containers(
            &f.reader,
            id.clone(),
            Instant::now() + Duration::from_millis(120),
        )
        .await;
    assert_eq!(answer, Err(Error(ErrorCode::Expired)));
    assert_eq!(f.requests.load(Ordering::SeqCst), 0);
    release.send(()).unwrap();
    blocked.await.unwrap().unwrap();
    db.call(|_| Ok(())).await.unwrap();
    assert_eq!(counts(&dir), before);
    let admission = bridge
        .poll_begin(id, vec!["host:system".into(), resource()], deadline())
        .await
        .unwrap();
    let limeos_persistence::ObservationPollAdmission::Lease(lease) = admission else {
        panic!("fresh admission")
    };
    bridge.poll_abandon(*lease, deadline()).await.unwrap();
    assert_eq!(counts(&dir), before);
}

#[tokio::test]
async fn a_missing_prior_container_requires_retirement_review_without_clearing_its_incident() {
    let (dir, db, id) = database(true, false);
    let f = fixture(0, super::super::now()).await;
    let bridge = crate::automation_store::ObservationStoreBridge::new(db.clone());
    bridge
        .poll_containers(&f.reader, id, deadline())
        .await
        .unwrap();
    let next = db
        .call(|store| {
            let actor = store.login_record("alice")?.unwrap().principal;
            let now = super::super::now();
            store
                .automation_create(
                    &actor,
                    &CreateSchedule {
                        definition: ScheduleDefinition {
                            operation: ScheduledOperation::ObserveHealth,
                            anchor: now,
                            interval_seconds: 30,
                            grace_seconds: 5,
                            missed: MissedRunPolicy::CoalesceLatest,
                            window: None,
                        },
                    },
                    now,
                )
                .map(|v| v.schedule.id)
        })
        .await
        .unwrap();
    let empty = fixture(12, super::super::now()).await;
    let before = counts(&dir);
    assert_eq!(
        bridge
            .poll_containers(&empty.reader, next, deadline())
            .await,
        Err(Error(ErrorCode::Conflict))
    );
    assert_eq!(counts(&dir), before);
    assert_eq!(before.3, 1);
}

async fn queued(db: &Database, id: String, at: i64) -> ObservationJob {
    db.call(move |store| {
        let row = store
            .automation_schedule_page(None, 32)?
            .into_iter()
            .find(|row| row.id == id)
            .unwrap();
        let DueDecision::Ready(key) = row.due(at)? else {
            panic!("due")
        };
        let Reservation::Queued(job) = store.automation_reserve_current(
            &row,
            &Settlement::Observation(key),
            || at,
            deadline(),
        )?
        else {
            panic!("queued")
        };
        Ok(job)
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn paused_and_revoked_schedules_never_contact_the_protected_reader() {
    for paused in [true, false] {
        let (dir, db, id) = database(true, false);
        let selected = id.clone();
        db.call(move |store| {
            let actor = store.login_record("alice")?.unwrap().principal;
            if paused {
                store.automation_pause(
                    &actor,
                    &selected,
                    &PauseSchedule {
                        revision: 1,
                        paused: true,
                    },
                    super::super::now(),
                )?;
            } else {
                store.revise_grants(&actor.id, Role::Viewer, &[], super::super::now())?;
            }
            Ok(())
        })
        .await
        .unwrap();
        let f = fixture(0, super::super::now()).await;
        let bridge = crate::automation_store::ObservationStoreBridge::new(db);
        let before = counts(&dir);
        let answer = bridge.poll_containers(&f.reader, id, deadline()).await;
        if paused {
            assert_eq!(answer.unwrap(), None);
        } else {
            assert_eq!(answer, Err(Error(ErrorCode::Forbidden)));
        }
        assert_eq!(f.requests.load(Ordering::SeqCst), 0);
        assert_eq!(counts(&dir), before);
    }
}

#[tokio::test]
async fn failed_source_releases_admission_for_an_immediate_fresh_retry() {
    for recovery in [false, true] {
        let at = super::super::now();
        let (dir, db, id) = database_at(true, false, at);
        let job = if recovery {
            Some(queued(&db, id.clone(), at).await)
        } else {
            None
        };
        let bridge = crate::automation_store::ObservationStoreBridge::new(db);
        let bad = fixture(1, super::super::now()).await;
        let before = counts(&dir);
        let failure = if let Some(job) = &job {
            bridge
                .recover_container_job(&bad.reader, job.clone(), deadline())
                .await
                .map(|_| ())
        } else {
            bridge
                .poll_containers(&bad.reader, id.clone(), deadline())
                .await
                .map(|_| ())
        };
        assert_eq!(failure, Err(Error(ErrorCode::Conflict)));
        assert_eq!(counts(&dir), before);
        let good = fixture(0, super::super::now()).await;
        if let Some(job) = job {
            let receipt = bridge
                .recover_container_job(&good.reader, job.clone(), deadline())
                .await
                .unwrap();
            assert_eq!(receipt.job, job.id);
            assert_eq!(counts(&dir).0, before.0);
        } else {
            assert!(
                bridge
                    .poll_containers(&good.reader, id, deadline())
                    .await
                    .unwrap()
                    .is_some()
            );
        }
        assert_eq!(bad.requests.load(Ordering::SeqCst), 1);
        assert_eq!(good.requests.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn cancelled_collection_keeps_only_its_original_bounded_admission() {
    let (dir, db, id) = database(true, false);
    let bridge = crate::automation_store::ObservationStoreBridge::new(db);
    let held = fixture(13, super::super::now()).await;
    let original = Instant::now() + Duration::from_millis(250);
    let before = counts(&dir);
    let task = tokio::spawn({
        let bridge = bridge.clone();
        let reader = held.reader.clone();
        let id = id.clone();
        async move { bridge.poll_containers(&reader, id, original).await }
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while held.requests.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let fresh = fixture(0, super::super::now()).await;
    assert_eq!(
        bridge
            .poll_containers(&fresh.reader, id.clone(), deadline())
            .await,
        Err(Error(ErrorCode::Conflict))
    );
    assert_eq!(fresh.requests.load(Ordering::SeqCst), 0);
    assert_eq!(counts(&dir), before);
    tokio::time::sleep_until(tokio::time::Instant::from_std(
        original + Duration::from_millis(1),
    ))
    .await;
    assert!(
        bridge
            .poll_containers(&fresh.reader, id, deadline())
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(fresh.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn original_read_window_still_expires_in_the_completion_queue_after_admission() {
    let (dir, db, id) = database(true, false);
    let f = fixture(13, super::super::now()).await;
    let bridge = crate::automation_store::ObservationStoreBridge::new(db.clone());
    let before = counts(&dir);
    let read = tokio::spawn({
        let bridge = bridge.clone();
        let reader = f.reader.clone();
        let id = id.clone();
        async move {
            bridge
                .poll_containers(&reader, id, Instant::now() + Duration::from_millis(400))
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while f.requests.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let (started, ready) = std::sync::mpsc::channel();
    let (release, held) = std::sync::mpsc::channel();
    let blocker = tokio::spawn({
        let db = db.clone();
        async move {
            db.call(move |_| {
                started.send(()).unwrap();
                held.recv().unwrap();
                Ok(())
            })
            .await
        }
    });
    tokio::task::spawn_blocking(move || ready.recv().unwrap())
        .await
        .unwrap();
    f.release.notify_one();
    assert_eq!(read.await.unwrap(), Err(Error(ErrorCode::Expired)));
    release.send(()).unwrap();
    blocker.await.unwrap().unwrap();
    db.call(|_| Ok(())).await.unwrap();
    assert_eq!(counts(&dir), before);
    // Late queued completion/cleanup cannot settle work or release later admission.
    let lease = bridge
        .poll_begin(id, vec!["host:system".into(), resource()], deadline())
        .await
        .unwrap();
    let limeos_persistence::ObservationPollAdmission::Lease(lease) = lease else {
        panic!("fresh")
    };
    bridge.poll_abandon(*lease, deadline()).await.unwrap();
}

#[tokio::test]
async fn original_queued_job_recovers_after_reopen_once_and_publishes_in_app() {
    let at = super::super::now();
    let (dir, db, id) = database_at(true, false, at);
    let job = queued(&db, id, at).await;
    drop(db);
    let mut reopened = None;
    for _ in 0..50 {
        match Database::open(&dir.path().join("core.sqlite")) {
            Ok(db) => {
                reopened = Some(db);
                break;
            }
            Err(Error(ErrorCode::Conflict)) => tokio::time::sleep(Duration::from_millis(10)).await,
            Err(error) => panic!("reopen: {error:?}"),
        }
    }
    let bridge = crate::automation_store::ObservationStoreBridge::new(reopened.unwrap());
    let f = fixture(0, super::super::now()).await;
    let result = bridge
        .recover_container_job(&f.reader, job.clone(), deadline())
        .await
        .unwrap();
    assert_eq!(result.job, job.id);
    assert!(!result.complete);
    assert_eq!(result.notifications, 1);
    assert_eq!(
        bridge
            .publish_page(None, deadline())
            .await
            .unwrap()
            .published,
        1
    );
    let before = counts(&dir);
    assert_eq!(
        bridge
            .recover_container_job(&f.reader, job, deadline())
            .await
            .unwrap(),
        result
    );
    assert_eq!(counts(&dir), before);
    assert_eq!((before.0, before.1, before.2, before.3), (1, 1, 1, 1));
    assert_eq!(f.requests.load(Ordering::SeqCst), 1);
    assert!(
        bridge
            .queued_page(None, deadline())
            .await
            .unwrap()
            .items
            .is_empty()
    );
}

#[tokio::test]
async fn expired_or_revoked_queued_jobs_are_retired_without_contacting_the_reader() {
    for revoked in [false, true] {
        let at = super::super::now() - if revoked { 0 } else { 120 };
        let (dir, db, id) = database_at(true, false, at);
        let job = queued(&db, id, at).await;
        if revoked {
            db.call(|store| {
                let actor = store.login_record("alice")?.unwrap().principal;
                store.revise_grants(&actor.id, Role::Viewer, &[], super::super::now())
            })
            .await
            .unwrap();
        }
        let bridge = crate::automation_store::ObservationStoreBridge::new(db);
        let f = fixture(0, super::super::now()).await;
        let result = bridge
            .recover_container_job(&f.reader, job.clone(), deadline())
            .await
            .unwrap();
        assert_eq!(result.job, job.id);
        assert_eq!(
            result.state,
            if revoked {
                limeos_domain::JobState::PreconditionChanged
            } else {
                limeos_domain::JobState::Failed
            }
        );
        assert!(!result.complete);
        let before = counts(&dir);
        assert_eq!(
            bridge
                .recover_container_job(&f.reader, job, deadline())
                .await
                .unwrap(),
            result
        );
        assert_eq!(counts(&dir), before);
        assert_eq!(f.requests.load(Ordering::SeqCst), 0);
    }
}
