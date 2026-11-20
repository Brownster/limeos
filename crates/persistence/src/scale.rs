//! Fixtures built through the real write paths, so every startup proof holds:
//! a principal with automation grants, observed occurrences, bulk history, and
//! the PR-13 measurement store (about 250k events and 5k jobs).
use crate::*;
use limeos_domain::automation::*;
use std::time::{Duration, Instant};

pub(crate) fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

/// Enrols `alice` as an administrator with every health read, container
/// management and the observe-health management grant.
pub(crate) fn actor(store: &mut Store) -> Principal {
    store.issue_bootstrap("bootstrap", 100).unwrap();
    store
        .enroll("bootstrap", "alice", "$argon2id$fixture", 101)
        .unwrap();
    let initial = store.login_record("alice").unwrap().unwrap().principal;
    let scope = |operation, resource: &str| Scope {
        operation,
        resource: resource.into(),
    };
    store
        .revise_grants(
            &initial.id,
            Role::Administrator,
            &[
                scope(Operation::HealthRead, "*"),
                scope(Operation::ContainerManage, "*"),
                scope(Operation::AutomationManage, "automation:observe-health"),
            ],
            102,
        )
        .unwrap();
    store.login_record("alice").unwrap().unwrap().principal
}

pub(crate) fn schedule(
    store: &mut Store,
    actor: &Principal,
    operation: ScheduledOperation,
    interval_seconds: u32,
    now: i64,
) -> Schedule {
    store
        .automation_create(
            actor,
            &CreateSchedule {
                definition: ScheduleDefinition {
                    operation,
                    anchor: 0,
                    interval_seconds,
                    grace_seconds: 5,
                    missed: MissedRunPolicy::CoalesceLatest,
                    window: None,
                },
            },
            now,
        )
        .unwrap()
        .schedule
}

pub(crate) fn current(store: &Store, id: &str) -> Schedule {
    store
        .automation_schedule_page(None, MAX_TICK_SCHEDULES)
        .unwrap()
        .into_iter()
        .find(|row| row.id == id)
        .unwrap()
}

/// Reserves the observation due at `now` and leaves its job queued.
pub(crate) fn reserve(store: &mut Store, id: &str, now: i64) -> ObservationJob {
    let row = current(store, id);
    let DueDecision::Ready(key) = row.due(now).unwrap() else {
        panic!("observation due")
    };
    let Reservation::Queued(job) = store
        .automation_reserve_current(&row, &Settlement::Observation(key), || now, deadline())
        .unwrap()
    else {
        panic!("observation queued")
    };
    job
}

/// Reserves, reads and settles the observation due at `now`.
pub(crate) fn observe(store: &mut Store, id: &str, now: i64) -> ObservationJob {
    observe_with(
        store,
        id,
        now,
        &host(&[("host:load", SignalStatus::Healthy)]),
    )
}

/// A complete host observation with one signal per `(key, status)`.
pub(crate) fn host(signals: &[(&str, SignalStatus)]) -> HealthObservation {
    HealthObservation {
        signals: signals
            .iter()
            .map(|(key, status)| ScopedHealthSignal {
                resource: "host:system".into(),
                signal: AlertSignal {
                    key: (*key).into(),
                    kind: AlertKind::Host,
                    status: *status,
                    severity: Severity::Warning,
                    summary: "Host load assessment".into(),
                },
            })
            .collect(),
        complete: true,
    }
}

/// Reserves, reads and settles the observation due at `now` with `batch`.
pub(crate) fn observe_with(
    store: &mut Store,
    id: &str,
    now: i64,
    batch: &HealthObservation,
) -> ObservationJob {
    let job = reserve(store, id, now);
    let ObservationAdmission::Lease(lease) = store
        .automation_observation_begin(&job, vec!["host:system".into()], || now, deadline())
        .unwrap()
    else {
        panic!("observation lease")
    };
    store
        .automation_observation_complete(&lease, batch, || now)
        .unwrap();
    job
}

/// Host signals per notification cycle: each cycle (failing, failing,
/// healthy) opens and recovers one incident per key, 256 notifications.
const NOTIFYING_KEYS: usize = 128;

/// Notifications through the real paths: two rounds of `MAX_NOTIFICATIONS`.
/// Every notification is published and every other one acknowledged; the
/// first round is pruned a week later, leaving its publication,
/// acknowledgement and prune proofs, and the second stays live at the cap.
/// Returns the time after the last step, a multiple of 30 s after `start`.
pub(crate) fn notify(store: &mut Store, schedule: &str, start: i64) -> i64 {
    let keys: Vec<String> = (0..NOTIFYING_KEYS)
        .map(|k| format!("host:disk-{k}"))
        .collect();
    let batch = |status| {
        let signals: Vec<(&str, SignalStatus)> =
            keys.iter().map(|key| (key.as_str(), status)).collect();
        host(&signals)
    };
    let mut now = start;
    let round = |store: &mut Store, now: &mut i64, session: &str| {
        for _ in 0..MAX_NOTIFICATIONS / (2 * NOTIFYING_KEYS) {
            for status in [
                SignalStatus::Failing,
                SignalStatus::Failing,
                SignalStatus::Healthy,
            ] {
                observe_with(store, schedule, *now, &batch(status));
                *now += 30;
            }
        }
        // A recovery waits for its opening, so sweep until nothing is left.
        loop {
            let (mut after, mut published) = (None, 0);
            loop {
                let page = store
                    .automation_publish_in_app(
                        after.as_deref(),
                        MAX_TICK_SCHEDULES,
                        || *now,
                        deadline(),
                    )
                    .unwrap();
                published += page.published;
                after = page.next;
                if after.is_none() {
                    break;
                }
            }
            if published == 0 {
                break;
            }
        }
        let undelivered: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM automation_notifications WHERE json_extract(body,'$.delivery.state')<>'delivered'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(undelivered, 0);
        let record = store.login_record("alice").unwrap().unwrap();
        store
            .create_session(&record, None, session, "csrf", *now)
            .unwrap();
        let pending: Vec<(String, i64)> = store
            .conn
            .prepare("SELECT id,json_extract(body,'$.revision') FROM automation_notifications WHERE json_extract(body,'$.acknowledged_at') IS NULL ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        for (id, revision) in pending.iter().step_by(2) {
            store
                .observe_health_notification_acknowledge(
                    session,
                    "csrf",
                    id,
                    &AcknowledgeNotification {
                        revision: *revision,
                    },
                    || *now,
                    deadline(),
                )
                .unwrap();
        }
    };
    round(store, &mut now, "notify-first");
    let live = |store: &Store| -> i64 {
        store
            .conn
            .query_row("SELECT count(*) FROM automation_notifications", [], |r| {
                r.get(0)
            })
            .unwrap()
    };
    assert_eq!(live(store), MAX_NOTIFICATIONS as i64);
    now += DELIVERED_RETENTION_SECONDS + 30;
    while live(store) > 0 {
        let mut after = None;
        loop {
            let (_, next) = store
                .automation_prune_notifications(
                    after.as_deref(),
                    MAX_TICK_SCHEDULES,
                    || now,
                    deadline(),
                )
                .unwrap();
            after = next;
            if after.is_none() {
                break;
            }
        }
    }
    round(store, &mut now, "notify-second");
    assert_eq!(live(store), MAX_NOTIFICATIONS as i64);
    now
}

/// `rows` job-less session events through the ordinary write gate and the
/// audit accounting, in one transaction.
pub(crate) fn fill(store: &mut Store, principal: &str, rows: i64, now: i64) {
    let kind = json(&EventKind::SessionCreated).unwrap();
    store
        .write(|tx| {
            let added = tx.execute("WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x<?) INSERT INTO events(principal,job,kind,created) SELECT ?,NULL,?,? FROM c", params![rows, principal, kind, now]).map_err(durable)?;
            retention::account(tx, added as i64)
        })
        .unwrap();
}

pub(crate) fn events(store: &Store) -> i64 {
    store
        .conn
        .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
        .unwrap()
}

/// `automation` settled observation jobs, `probes` terminal probe jobs and
/// job-less history until the table holds at least `rows` events. History is
/// interleaved with the jobs, as it is in service. With `notifications`, the
/// same schedule then produces two rounds of notifications ([`notify`]).
pub(crate) fn build(
    path: &Path,
    automation: usize,
    probes: usize,
    rows: i64,
    notifications: bool,
) -> Principal {
    let mut store = Store::open(path).unwrap();
    let actor = actor(&mut store);
    let base = 1_000_000;
    let observed = schedule(
        &mut store,
        &actor,
        ScheduledOperation::ObserveHealth,
        30,
        base,
    );
    let jobs = automation + probes;
    for i in 0..jobs {
        let now = base + 30 * i as i64;
        if i < automation {
            observe(&mut store, &observed.id, now);
        } else {
            let id = store
                .queue_job(
                    &actor,
                    &format!("probe-{i}"),
                    &Intent::HealthProbe {
                        resource: "system".into(),
                    },
                    now + 600,
                    now,
                )
                .unwrap();
            if i % 4 == 0 {
                store
                    .transition(&id, JobState::Queued, JobState::Canceled, now)
                    .unwrap();
            } else {
                for (from, to) in [
                    (JobState::Queued, JobState::Running),
                    (JobState::Running, JobState::Verifying),
                    (JobState::Verifying, JobState::Succeeded),
                ] {
                    store.transition(&id, from, to, now).unwrap();
                }
            }
        }
        let left = (jobs - i) as i64;
        let want = (rows - events(&store)).max(0) / left;
        if want > 0 {
            fill(&mut store, &actor.id, want, now);
        }
    }
    if notifications {
        notify(&mut store, &observed.id, base + 30 * jobs as i64);
    }
    actor
}

/// The notification fixture passes the startup check: live notifications at
/// the cap and a round of pruned ones, each with its proofs.
#[test]
fn notification_fixture_passes_the_startup_check() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let mut store = Store::open(&path).unwrap();
    let actor = actor(&mut store);
    let observed = schedule(
        &mut store,
        &actor,
        ScheduledOperation::ObserveHealth,
        30,
        1_000_000,
    );
    notify(&mut store, &observed.id, 1_000_000);
    let pruned: i64 = store
        .conn
        .query_row(
            "SELECT count(*) FROM events WHERE json_extract(kind,'$.kind')='automation_notification_pruned'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pruned, MAX_NOTIFICATIONS as i64);
    drop(store);
    Store::open(&path).unwrap();
}

/// Every startup query that runs once per job, notification or policy
/// revision searches an index: none scans `events`, which grows to 262k rows.
#[test]
fn per_row_startup_queries_use_an_index() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    let actor = actor(&mut store);
    let observed = schedule(
        &mut store,
        &actor,
        ScheduledOperation::ObserveHealth,
        30,
        1_000_000,
    );
    observe(&mut store, &observed.id, 1_000_000);
    for sql in automation::startup_event_queries() {
        let mut stmt = store
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap_or_else(|e| panic!("{e}: {sql}"));
        let count = stmt.parameter_count();
        let steps: Vec<String> = stmt
            .query_map(
                rusqlite::params_from_iter(std::iter::repeat_n(rusqlite::types::Null, count)),
                |r| r.get(3),
            )
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert!(
            steps.iter().any(|step| step.contains("INDEX events_")),
            "{steps:?}: {sql}"
        );
        for step in &steps {
            assert!(
                !step.starts_with("SCAN") || step == "SCAN CONSTANT ROW",
                "{step}: {sql}"
            );
        }
    }
}

/// Startup cost at the PR-13 size. Run explicitly, in release:
/// `cargo test --release -p limeos-persistence --lib -- --ignored --nocapture startup_at_scale`
#[test]
#[ignore = "measurement; run explicitly in release"]
fn startup_at_scale() {
    // LIMEOS_SCALE_PATH keeps the fixture (built once if missing) so two
    // builds can be measured on the same file; LIMEOS_SCALE_OPENS sets the
    // number of timed opens (default 5).
    let dir = tempfile::tempdir().unwrap();
    let path = std::env::var_os("LIMEOS_SCALE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.path().join("core.sqlite"));
    let opens: usize = std::env::var("LIMEOS_SCALE_OPENS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    if !path.exists() {
        let started = Instant::now();
        build(&path, 1000, 4000, 250_000, true);
        eprintln!("fixture built in {:?}", started.elapsed());
    }
    {
        let conn = Connection::open(&path).unwrap();
        let (events, jobs, notifications, proofs): (i64, i64, i64, i64) = conn
            .query_row(
                "SELECT (SELECT count(*) FROM events),(SELECT count(*) FROM jobs),(SELECT count(*) FROM automation_notifications),(SELECT count(*) FROM events WHERE json_extract(kind,'$.kind') GLOB 'automation_notification_*')",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        let bytes = std::fs::metadata(&path).unwrap().len();
        eprintln!(
            "events={events} jobs={jobs} notifications={notifications} notification proofs={proofs} file={bytes} bytes"
        );
        let quick = Instant::now();
        let _: String = conn
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .unwrap();
        let quick = quick.elapsed();
        let check = Instant::now();
        resources::check(&conn).unwrap();
        eprintln!(
            "quick_check={quick:?} invariant check={:?}",
            check.elapsed()
        );
    }
    let mut times = Vec::new();
    for _ in 0..opens {
        let at = Instant::now();
        let store = Store::open(&path).unwrap();
        times.push(at.elapsed());
        drop(store);
    }
    times.sort();
    eprintln!(
        "Store::open n={opens} min={:?} median={:?} max={:?}",
        times[0],
        times[opens / 2],
        times[opens - 1]
    );
}
