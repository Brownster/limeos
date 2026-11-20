use super::*;
use limeos_domain::{Operation, Principal, Role, Scope};
use limeos_executor_protocol::{Receipt, Request};
use limeos_persistence::{Database, Store};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicI64, AtomicUsize, Ordering},
};
use tokio::net::UnixListener;

struct Fixture {
    dir: tempfile::TempDir,
    db: Database,
    bridge: ObservationStoreBridge,
    reader: HealthReader,
    actors: Vec<Principal>,
    schedules: BTreeMap<String, Vec<Schedule>>,
    wall: Arc<AtomicI64>,
    requests: Arc<AtomicUsize>,
    mode: Arc<AtomicUsize>,
    release: Arc<tokio::sync::Notify>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn resource() -> String {
    format!("container:{:064x}", 0)
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(8)
}
fn token(index: usize) -> String {
    format!("{:064x}", index + 1)
}
fn csrf(index: usize) -> String {
    format!("{:064x}", index + 1000)
}
async fn fixture(owners: usize, per_owner: usize) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reader.sock");
    let at = crate::now();
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    let mut actors = Vec::new();
    let mut schedules = BTreeMap::new();
    for index in 0..owners {
        let name = format!("fixture_{index}");
        let actor = store
            .import_legacy_user(
                &name,
                &format!("pbkdf2:sha256:1$salt${}", "00".repeat(32)),
                at - 100,
            )
            .unwrap();
        store
            .revise_grants(
                &actor.id,
                Role::Operator,
                &[
                    Scope {
                        operation: Operation::HealthRead,
                        resource: "host:system".into(),
                    },
                    Scope {
                        operation: Operation::HealthRead,
                        resource: resource(),
                    },
                    Scope {
                        operation: Operation::AutomationManage,
                        resource: "automation:observe-health".into(),
                    },
                ],
                at - 99,
            )
            .unwrap();
        let record = store.login_record(&name).unwrap().unwrap();
        store
            .create_session(
                &record,
                None,
                &limeos_identity::digest(&token(index)),
                &limeos_identity::digest(&csrf(index)),
                at - 90,
            )
            .unwrap();
        store
            .observe_health_policy_replace(
                &limeos_identity::digest(&token(index)),
                &limeos_identity::digest(&csrf(index)),
                &ReplaceHealthPolicy {
                    revision: 0,
                    definition: HealthPolicyDefinition {
                        fail_threshold: 1,
                        ..Default::default()
                    },
                },
                || at,
                deadline(),
            )
            .unwrap();
        let rows: Vec<_> = (0..per_owner)
            .map(|_| {
                store
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
            })
            .collect();
        schedules.insert(actor.id.clone(), rows);
        actors.push(record.principal);
    }
    drop(store);
    let db = Database::open(&dir.path().join("core.sqlite")).unwrap();
    let wall = Arc::new(AtomicI64::new(at));
    let clock = wall.clone();
    let bridge = ObservationStoreBridge::new(db.clone())
        .with_test_clock(Arc::new(move || clock.load(Ordering::SeqCst)));
    let listener = UnixListener::bind(&path).unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let mode = Arc::new(AtomicUsize::new(0));
    let selected = mode.clone();
    let sampled = wall.clone();
    let release = Arc::new(tokio::sync::Notify::new());
    let resumed = release.clone();
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
            let mode = selected.load(Ordering::SeqCst);
            if mode == 3 {
                resumed.notified().await;
            }
            let cohort = ContainerHealthCohort::new(
                if mode == 1 { "f".repeat(64) } else { nonce },
                "fixture-engine".into(),
                sampled.load(Ordering::SeqCst),
                vec![ContainerHealthMember {
                    resource: resource(),
                    signal: Some(AlertSignal {
                        key: resource(),
                        kind: AlertKind::Container,
                        status: if mode == 2 {
                            SignalStatus::Failing
                        } else {
                            SignalStatus::Healthy
                        },
                        severity: Severity::Warning,
                        summary: "Fixture container state".into(),
                    }),
                }],
            )
            .unwrap();
            let mut receipt = Receipt::empty();
            receipt.container_health = Some(cohort);
            let _ = limeos_contracts::write_frame(&mut stream, &receipt).await;
        }
    });
    let clock = wall.clone();
    let reader = HealthReader::for_test(
        path,
        Arc::new(tokio::sync::Semaphore::new(1)),
        Arc::new(move || clock.load(Ordering::SeqCst)),
    );
    Fixture {
        dir,
        db,
        bridge,
        reader,
        actors,
        schedules,
        wall,
        requests,
        mode,
        release,
        task,
    }
}
impl Fixture {
    fn runtime(&self, status: Status) -> ObservationRuntime {
        ObservationRuntime::new(self.bridge.clone(), self.reader.clone(), status)
    }
    async fn counts(&self) -> (i64, i64, i64, i64) {
        // The same sole worker is drained before observing owned SQLite state.
        self.db.call(|_| Ok(())).await.unwrap();
        rusqlite::Connection::open_with_flags(self.dir.path().join("core.sqlite"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap().query_row("SELECT (SELECT count(*) FROM jobs),(SELECT count(*) FROM automation_observation_results),(SELECT count(*) FROM automation_notifications),audit_bytes FROM meta WHERE id=1", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap()
    }
    async fn view(&self, status: &Status, index: usize) -> AutomationPage<ScheduleView> {
        let session = limeos_identity::digest(&token(index));
        let at = self.wall.load(Ordering::SeqCst);
        let page = self
            .db
            .call(move |store| {
                store.observe_health_schedule_page(&session, None, || at, deadline())
            })
            .await
            .unwrap();
        status.project_page(page).unwrap()
    }
}

#[tokio::test]
async fn more_than_one_owner_page_uses_one_fresh_read_per_page_without_starvation() {
    let f = fixture(33, 1).await;
    let status = Status::default();
    let _running = status.enter().unwrap();
    let mut runtime = f.runtime(status.clone());
    let first = runtime.step(deadline()).await.unwrap();
    assert_eq!(
        (first.owners, first.admitted, first.reads, first.completed),
        (32, 32, 1, 32)
    );
    let last = runtime.step(deadline()).await.unwrap();
    assert_eq!(
        (last.owners, last.admitted, last.reads, last.completed),
        (1, 1, 1, 1)
    );
    assert_eq!(f.requests.load(Ordering::SeqCst), 2);
    assert_eq!(f.counts().await.0, 33);
    for index in 0..33 {
        let page = f.view(&status, index).await;
        assert!(page.partial);
        assert_eq!(page.items.len(), 1);
        assert_eq!(
            page.items[0].runtime.as_ref().unwrap().phase,
            ObservationPhase::Partial
        );
    }
}

#[tokio::test]
async fn same_owner_schedule_cursor_rotates_instead_of_selecting_the_first_row_forever() {
    let f = fixture(1, 3).await;
    let status = Status::default();
    let _running = status.enter().unwrap();
    let mut runtime = f.runtime(status.clone());
    for _ in 0..3 {
        let pass = runtime.step(deadline()).await.unwrap();
        assert_eq!((pass.admitted, pass.completed), (1, 1));
    }
    assert_eq!(f.requests.load(Ordering::SeqCst), 3);
    assert_eq!(f.counts().await.0, 1); // Other identical samples use volatile watermarks.
    let page = f.view(&status, 0).await;
    assert!(page.items.iter().all(|v| {
        v.runtime
            .as_ref()
            .is_some_and(|v| v.phase == ObservationPhase::Partial)
    }));
    let stable = f.counts().await;
    assert_eq!(runtime.step(deadline()).await.unwrap().reads, 0);
    assert_eq!(f.counts().await, stable);
    assert_eq!(f.requests.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn one_owners_missing_concrete_grant_does_not_block_other_owners() {
    let f = fixture(2, 1).await;
    let actor = f.actors[0].clone();
    let at = f.wall.load(Ordering::SeqCst);
    // Keep current host/manage admission but deliberately omit the discovered member.
    f.db.call(move |store| {
        store.revise_grants(
            &actor.id,
            Role::Operator,
            &[
                Scope {
                    operation: Operation::HealthRead,
                    resource: "host:system".into(),
                },
                Scope {
                    operation: Operation::AutomationManage,
                    resource: "automation:observe-health".into(),
                },
            ],
            at,
        )?;
        let record = store.login_record("fixture_0")?.unwrap();
        let row = store
            .automation_schedules(&record.principal, None)?
            .items
            .remove(0);
        store.automation_pause(
            &record.principal,
            &row.schedule.id,
            &PauseSchedule {
                revision: row.schedule.revision,
                paused: false,
            },
            at,
        )?;
        store.revoke_session(&limeos_identity::digest(&token(0)), at)?;
        store.create_session(
            &record,
            None,
            &limeos_identity::digest(&token(0)),
            &limeos_identity::digest(&csrf(0)),
            at,
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let status = Status::default();
    let _running = status.enter().unwrap();
    let pass = f.runtime(status.clone()).step(deadline()).await.unwrap();
    assert_eq!((pass.reads, pass.completed, pass.unavailable), (1, 1, 1));
    assert_eq!(f.requests.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.view(&status, 0).await.items[0]
            .runtime
            .as_ref()
            .unwrap()
            .phase,
        ObservationPhase::Unavailable
    );
    assert_eq!(
        f.view(&status, 1).await.items[0]
            .runtime
            .as_ref()
            .unwrap()
            .phase,
        ObservationPhase::Partial
    );
}

#[tokio::test]
async fn failed_source_is_visible_and_next_pass_retries_without_replacing_original_jobs() {
    let f = fixture(1, 1).await;
    let row = f.schedules[&f.actors[0].id][0].clone();
    let at = f.wall.load(Ordering::SeqCst);
    let job =
        f.db.call(move |store| {
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
                panic!("job")
            };
            Ok(job)
        })
        .await
        .unwrap();
    let status = Status::default();
    let _running = status.enter().unwrap();
    let mut runtime = f.runtime(status.clone());
    f.mode.store(1, Ordering::SeqCst);
    let before = f.counts().await;
    assert_eq!(runtime.step(deadline()).await.unwrap().unavailable, 1);
    assert_eq!(f.counts().await, before);
    assert_eq!(
        f.view(&status, 0).await.items[0]
            .runtime
            .as_ref()
            .unwrap()
            .phase,
        ObservationPhase::Unavailable
    );
    f.mode.store(2, Ordering::SeqCst);
    assert_eq!(runtime.step(deadline()).await.unwrap().completed, 1);
    let count = f.counts().await;
    assert_eq!((count.0, count.1, count.2), (1, 1, 1));
    assert!(
        f.bridge
            .queued_page(None, deadline())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        f.bridge
            .recover_container_job(&f.reader, job.clone(), deadline())
            .await
            .unwrap()
            .job,
        job.id
    );
    runtime.step(deadline()).await.unwrap(); // Publishes the original event before any read.
    assert_eq!(f.requests.load(Ordering::SeqCst), 2);
    let session = limeos_identity::digest(&token(0));
    let page =
        f.db.call(move |store| {
            store.observe_health_notification_page(&session, None, || at, deadline())
        })
        .await
        .unwrap();
    assert!(matches!(
        page.items[0].delivery,
        DeliveryState::Delivered { .. }
    ));
}

#[tokio::test]
async fn no_work_and_paused_candidates_contact_no_reader() {
    let empty = fixture(0, 0).await;
    assert_eq!(
        empty
            .runtime(Status::default())
            .step(deadline())
            .await
            .unwrap()
            .reads,
        0
    );
    assert_eq!(empty.requests.load(Ordering::SeqCst), 0);
    let f = fixture(1, 1).await;
    let actor = f.actors[0].clone();
    let row = f.schedules[&actor.id][0].clone();
    let at = f.wall.load(Ordering::SeqCst);
    f.db.call(move |store| {
        store.automation_pause(
            &actor,
            &row.id,
            &PauseSchedule {
                revision: row.revision,
                paused: true,
            },
            at,
        )
    })
    .await
    .unwrap();
    let before = f.counts().await;
    assert_eq!(
        f.runtime(Status::default())
            .step(deadline())
            .await
            .unwrap()
            .reads,
        0
    );
    assert_eq!(f.requests.load(Ordering::SeqCst), 0);
    assert_eq!(f.counts().await, before);
}

#[tokio::test]
async fn owned_run_has_single_flight_and_cancellation_removes_registration() {
    let f = fixture(1, 1).await;
    f.mode.store(3, Ordering::SeqCst);
    let status = Status::default();
    let task = tokio::spawn(f.runtime(status.clone()).run());
    tokio::time::timeout(Duration::from_secs(2), async {
        while f.requests.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        f.view(&status, 0).await.items[0]
            .runtime
            .as_ref()
            .unwrap()
            .phase,
        ObservationPhase::Checking
    );
    assert_eq!(
        f.runtime(status.clone()).run().await,
        Err(Error(ErrorCode::Conflict))
    );
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let page = f.view(&status, 0).await;
    assert_eq!(page.items[0].status, ScheduleStatus::AwaitingProducer);
    assert!(page.items[0].runtime.is_none());
    assert_eq!(f.requests.load(Ordering::SeqCst), 1);
    f.release.notify_one();
}

#[test]
fn runtime_diagnostics_are_bounded_and_revision_checked() {
    let status = Status::default();
    let running = status.enter().unwrap();
    for index in 0..=MAX_SCHEDULES {
        status
            .mark(&format!("{index:064x}"), 1, ObservationPhase::Partial, 100)
            .unwrap();
    }
    assert_eq!(status.0.lock().unwrap().rows.len(), MAX_SCHEDULES);
    drop(running);
    assert!(status.0.lock().unwrap().rows.is_empty());
}

#[tokio::test]
async fn critical_durable_failure_stops_batch_before_completing_another_owner() {
    let f = fixture(2, 1).await;
    f.mode.store(3, Ordering::SeqCst);
    let status = Status::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let clock = f.wall.clone();
    let bridge = f.bridge.clone().with_test_clock(Arc::new(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        clock.load(Ordering::SeqCst)
    }));
    let runtime = ObservationRuntime::new(bridge, f.reader.clone(), status.clone());
    let task = tokio::spawn(runtime.run());
    tokio::time::timeout(Duration::from_secs(2), async {
        while f.requests.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // Corrupt only this owned fixture's mutable audit counter, without dropping
    // immutable-job guards or changing production limits.
    let connection = rusqlite::Connection::open(f.dir.path().join("core.sqlite")).unwrap();
    connection
        .execute("UPDATE meta SET audit_bytes=? WHERE id=1", [i64::MAX / 2])
        .unwrap();
    calls.store(0, Ordering::SeqCst);
    f.release.notify_one();
    assert!(matches!(
        task.await.unwrap(),
        Err(Error(ErrorCode::StateNotDurable))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!status.0.lock().unwrap().running);
    assert!(status.0.lock().unwrap().rows.is_empty());
    let jobs: i64 = connection
        .query_row("SELECT count(*) FROM jobs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(jobs, 0);
}

fn file_bytes(path: PathBuf) -> Vec<u8> {
    std::fs::read(path).unwrap()
}
fn write_bytes() -> u64 {
    std::fs::read_to_string("/proc/self/io")
        .unwrap()
        .lines()
        .find_map(|line| {
            line.strip_prefix("write_bytes: ")
                .map(|v| v.parse().unwrap())
        })
        .unwrap()
}
#[tokio::test]
async fn stable_real_runtime_passes_leave_database_wal_and_process_write_bytes_unchanged() {
    let f = fixture(1, 1).await;
    let status = Status::default();
    let _running = status.enter().unwrap();
    let mut runtime = f.runtime(status);
    runtime.step(deadline()).await.unwrap();
    let before = f.counts().await;
    let database = file_bytes(f.dir.path().join("core.sqlite"));
    let wal = file_bytes(f.dir.path().join("core.sqlite-wal"));
    let io = write_bytes();
    for _ in 0..32 {
        f.wall.fetch_add(30, Ordering::SeqCst);
        assert_eq!(runtime.step(deadline()).await.unwrap().completed, 1);
    }
    let after_io = write_bytes();
    assert_eq!(f.counts().await, before);
    assert!(
        file_bytes(f.dir.path().join("core.sqlite")) == database,
        "database changed"
    );
    assert!(
        file_bytes(f.dir.path().join("core.sqlite-wal")) == wal,
        "WAL changed"
    );
    assert_eq!(after_io, io);
    println!(
        "LIMEOS_OBSERVATION_RUNTIME_IDLE passes=32 unchanged_database=true unchanged_wal=true write_bytes_delta={} simulated_seconds=960 installed_soak=false",
        after_io - io
    );
}

#[tokio::test]
async fn authenticated_core_views_project_only_current_owned_authority_and_registration() {
    let f = fixture(2, 1).await;
    let status = Status::default();
    let running = status.enter().unwrap();
    let mut core = crate::automation_schedules::tests::core(f.db.clone());
    core.automation_status = status.clone();
    f.runtime(status.clone()).step(deadline()).await.unwrap();
    let page = core
        .human_schedule_page(token(0), None, deadline())
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert!(page.partial);
    assert_eq!(
        page.items[0].schedule.id,
        f.schedules[&f.actors[0].id][0].id
    );
    assert_eq!(
        page.items[0].runtime.as_ref().unwrap().phase,
        ObservationPhase::Partial
    );
    assert_eq!(
        core.human_schedule_page("f".repeat(64), None, deadline())
            .await,
        Err(Error(ErrorCode::Unauthenticated))
    );
    let row = page.items[0].clone();
    let paused = core
        .human_schedule_pause(
            token(0),
            csrf(0),
            row.schedule.id.clone(),
            PauseSchedule {
                revision: row.schedule.revision,
                paused: true,
            },
            deadline(),
        )
        .await
        .unwrap();
    assert_eq!(paused.status, ScheduleStatus::Paused);
    assert!(paused.runtime.is_none());
    let resumed = core
        .human_schedule_pause(
            token(0),
            csrf(0),
            paused.schedule.id,
            PauseSchedule {
                revision: paused.schedule.revision,
                paused: false,
            },
            deadline(),
        )
        .await
        .unwrap();
    assert_eq!(resumed.status, ScheduleStatus::Scheduled);
    assert_eq!(
        resumed.runtime.as_ref().unwrap().phase,
        ObservationPhase::Waiting
    );
    drop(running);
    assert_eq!(
        core.human_schedule_page(token(0), None, deadline())
            .await
            .unwrap()
            .items[0]
            .status,
        ScheduleStatus::AwaitingProducer
    );
}

async fn http_health(address: std::net::SocketAddr) -> limeos_contracts::Health {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    stream
        .write_all(b"GET /api/v1/health HTTP/1.1\r\nhost: localhost\r\nconnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.starts_with("HTTP/1.1 200"), "{text}");
    serde_json::from_str(text.split("\r\n\r\n").nth(1).unwrap()).unwrap()
}

#[tokio::test]
async fn a_store_failure_during_a_pass_leaves_core_serving_and_is_reported_until_a_pass_succeeds() {
    let f = fixture(1, 1).await;
    let status = Status::default();
    let mut core = crate::automation_schedules::tests::core(f.db.clone());
    core.automation_status = status.clone();
    // The audit cap refuses every write with `StateNotDurable`, the same
    // refusal a full disk or a SQLite error gives.
    let connection = rusqlite::Connection::open(f.dir.path().join("core.sqlite")).unwrap();
    let audit: i64 = connection
        .query_row("SELECT audit_bytes FROM meta WHERE id=1", [], |row| {
            row.get(0)
        })
        .unwrap();
    connection
        .execute("UPDATE meta SET audit_bytes=? WHERE id=1", [i64::MAX / 2])
        .unwrap();
    let http = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = http.local_addr().unwrap();
    let rpc = UnixListener::bind(f.dir.path().join("core.sock")).unwrap();
    let services = tokio::spawn(crate::services(
        core,
        http,
        rpc,
        "https://localhost".into(),
        f.bridge.clone(),
        f.reader.clone(),
        crate::supervision::Backoff {
            base: Duration::from_millis(10),
            cap: Duration::from_millis(40),
        },
    ));
    let degraded = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let health = http_health(address).await;
            if let Some(degraded) = health.degraded.into_iter().find(|d| d.failures >= 2) {
                break degraded;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the failed runtime is reported while HTTP keeps answering");
    assert_eq!(
        degraded.subsystem,
        limeos_contracts::Subsystem::ObservationRuntime
    );
    assert_eq!(degraded.code, ErrorCode::StateNotDurable);
    // At least two failures: the runtime was restarted, not left stopped.
    assert!(!services.is_finished(), "a failed pass ended core");

    connection
        .execute("UPDATE meta SET audit_bytes=? WHERE id=1", [audit])
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !http_health(address).await.degraded.is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("a successful pass clears the degraded state");
    assert!(status.0.lock().unwrap().running);
    assert!(!services.is_finished());
    services.abort();
}
