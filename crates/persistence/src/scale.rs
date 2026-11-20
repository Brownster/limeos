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
    let job = reserve(store, id, now);
    let ObservationAdmission::Lease(lease) = store
        .automation_observation_begin(&job, vec!["host:system".into()], || now, deadline())
        .unwrap()
    else {
        panic!("observation lease")
    };
    let healthy = HealthObservation {
        signals: vec![ScopedHealthSignal {
            resource: "host:system".into(),
            signal: AlertSignal {
                key: "host:load".into(),
                kind: AlertKind::Host,
                status: SignalStatus::Healthy,
                severity: Severity::Warning,
                summary: "Host load assessment".into(),
            },
        }],
        complete: true,
    };
    store
        .automation_observation_complete(&lease, &healthy, || now)
        .unwrap();
    job
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
/// interleaved with the jobs, as it is in service.
pub(crate) fn build(path: &Path, automation: usize, probes: usize, rows: i64) -> Principal {
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
    actor
}

/// Startup cost at the PR-13 size. Run explicitly, in release:
/// `cargo test --release -p limeos-persistence --lib -- --ignored --nocapture startup_at_scale`
#[test]
#[ignore = "measurement; run explicitly in release"]
fn startup_at_scale() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let started = Instant::now();
    build(&path, 1000, 4000, 250_000);
    eprintln!("fixture built in {:?}", started.elapsed());
    {
        let conn = Connection::open(&path).unwrap();
        let (events, jobs): (i64, i64) = conn
            .query_row(
                "SELECT (SELECT count(*) FROM events),(SELECT count(*) FROM jobs)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        let bytes = std::fs::metadata(&path).unwrap().len();
        eprintln!("events={events} jobs={jobs} file={bytes} bytes");
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
    for _ in 0..5 {
        let at = Instant::now();
        let store = Store::open(&path).unwrap();
        times.push(at.elapsed());
        drop(store);
    }
    times.sort();
    eprintln!(
        "Store::open min={:?} median={:?} max={:?}",
        times[0], times[2], times[4]
    );
}
