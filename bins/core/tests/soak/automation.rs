//! The known automation limit (`crates/persistence/src/retention.rs`, "Known
//! limit: automation results"), measured rather than forgotten. No producer
//! runs in the shipped scope, so the main soak drives none; this scenario
//! enables one observe-health schedule at the 30 s minimum, observes every
//! occurrence through the store's real reservation, lease and completion
//! paths with retention once a simulated minute, and stops at the first
//! refused write. It asserts that the limit is still there and is the one
//! measured: the reservation refused `StateNotDurable` at the audit cap
//! (not at the occurrence or result caps) between days 22 and 23.5, so
//! hitting a different limit earlier fails; once result and job retention
//! land, it fails too, and the schedule belongs in the main soak instead.
//!
//! Measured (20 November 2026): the first refusal comes after 22.76
//! simulated days, a reservation refused with `StateNotDurable` at the
//! audit cap (262,143 events, 65,535 results, core.sqlite at 202.6 MiB).
//! It takes about 14 minutes in a debug build and up to 210 MiB of disk.
use crate::{
    sim::soak_dir,
    support::{DAY, START},
};
use limeos_domain::{
    Error, ErrorCode,
    automation::{
        AlertKind, AlertSignal, DueDecision, MAX_TICK_SCHEDULES, Reservation, Settlement, Severity,
        SignalStatus,
    },
};
use limeos_persistence::{
    AuditUsage, HealthObservation, ObservationAdmission, ScopedHealthSignal, Store, test_support,
};
use std::time::{Duration, Instant};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(60)
}

#[test]
#[ignore = "measurement of a known limit outside the shipped scope; run explicitly"]
fn automation_limit_at_minimum_interval() {
    let dir = soak_dir();
    let path = dir.path().join("core.sqlite");
    let mut store = Store::open(&path).unwrap();
    store.issue_bootstrap("bootstrap", START).unwrap();
    store
        .enroll("bootstrap", "marc", "$argon2id$soak", START)
        .unwrap();
    let schedule = crate::occurrences::observe_health_schedule(&mut store);
    let batch = HealthObservation {
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
    let started = Instant::now();
    let instant = |at: i64| started + Duration::from_secs((at - START) as u64);
    let mut observed = 0u64;
    let mut refused: Option<(i64, &str, Error)> = None;
    let mut now = START + 30;
    while now < START + 60 * DAY {
        // The writer thread's retention pass, once a simulated minute.
        if now % 60 == 0
            && let Some(pass) = test_support::maintain(&mut store, instant(now))
            && let Some((_, code)) = pass.failed.first()
        {
            refused = Some((now, "retention pass", Error(*code)));
            break;
        }
        let row = store
            .automation_schedule_page(None, MAX_TICK_SCHEDULES)
            .unwrap()
            .into_iter()
            .find(|row| row.id == schedule)
            .unwrap();
        if let DueDecision::Ready(key) = row.due(now).unwrap() {
            let step = (|| {
                let Reservation::Queued(job) = store
                    .automation_reserve_current(
                        &row,
                        &Settlement::Observation(key),
                        || now,
                        deadline(),
                    )
                    .map_err(|e| ("reserve", e))?
                else {
                    return Ok(());
                };
                let ObservationAdmission::Lease(lease) = store
                    .automation_observation_begin(
                        &job,
                        vec!["host:system".into()],
                        || now,
                        deadline(),
                    )
                    .map_err(|e| ("begin", e))?
                else {
                    return Ok(());
                };
                store
                    .automation_observation_complete(&lease, &batch, || now)
                    .map_err(|e| ("complete", e))?;
                observed += 1;
                Ok::<(), (&str, Error)>(())
            })();
            if let Err((op, error)) = step {
                refused = Some((now, op, error));
                break;
            }
        }
        now += 30;
    }
    drop(store);
    let conn = crate::checks::reader(&path).unwrap();
    let count = |table: &str| -> i64 {
        conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    };
    let (events, results, jobs, occurrences) = (
        count("events"),
        count("automation_observation_results"),
        count("jobs"),
        count("automation_occurrences"),
    );
    let file = std::fs::metadata(&path).unwrap().len();
    eprintln!(
        "automation limit: {observed} observations; events {events}, results {results}, jobs {jobs}, occurrences {occurrences}, file {:.1} MiB; {:.1} s",
        crate::support::mib(file),
        started.elapsed().as_secs_f64()
    );
    let Some((at, op, Error(code))) = refused else {
        panic!(
            "one schedule at the 30 s minimum ran 60 simulated days without a refusal: the known limit is gone, so update retention.rs and docs/soak.md and drive the producer in the main soak"
        );
    };
    let day = (at - START) as f64 / DAY as f64;
    eprintln!("automation limit: first refusal after {day:.2} simulated days ({op}: {code:?})");
    // The known limit, and no other: the reservation (two events) refused
    // because the audit table is at its cap, filled by the proofs pinned to
    // observation jobs, with the results one short of their own cap and the
    // occurrences held at their budget by the pruner.
    let per_event = test_support::EVENT_BYTES;
    let (budget, batch) = crate::sim::rule_limits("automation_occurrences");
    let results_cap = AuditUsage::LIMIT / (4 * per_event);
    let expected = [
        (
            op == "reserve" && code == ErrorCode::StateNotDurable,
            format!("the first refusal is {op}: {code:?}, not the reservation's StateNotDurable"),
        ),
        (
            (events + 2) * per_event > AuditUsage::LIMIT,
            format!("{events} events: the audit table is not at its cap"),
        ),
        (
            results < results_cap,
            format!("{results} results: their own cap ({results_cap}) was reached first"),
        ),
        (
            occurrences <= budget + batch,
            format!(
                "{occurrences} occurrences: the occurrence pruner fell behind its budget of {budget}"
            ),
        ),
        (
            (22.0..23.5).contains(&day),
            format!("day {day:.2}, outside the window measured (22.76) of 22 to 23.5"),
        ),
    ];
    let wrong: Vec<&String> = expected
        .iter()
        .filter(|(ok, _)| !ok)
        .map(|(_, why)| why)
        .collect();
    assert!(
        wrong.is_empty(),
        "the automation limit changed:\n{}",
        wrong
            .iter()
            .map(|w| w.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
}
