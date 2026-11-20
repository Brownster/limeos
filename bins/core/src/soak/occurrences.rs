//! The occurrence pruner (`retention.rs`, rule `automation_occurrences`),
//! which the main run never feeds: no producer runs in the shipped scope, so
//! its budget there is met by an empty table. Here one observe-health
//! schedule at the 30 s minimum settles every occurrence through the store's
//! real reservation API on the simulated clock, for 30 days, with the
//! writer's retention pass once a simulated minute. Each tick comes ten
//! seconds after its due time, past the schedule's 5 s grace, so every
//! occurrence is settled as skipped: a row in `automation_occurrences` and a
//! prunable event, and no observation job (whose results and proofs are the
//! separate, known automation limit in `automation.rs`).
//!
//! The table must sit at its budget after every pass once it got there,
//! never above it and never below; then retention is suspended for a day
//! while reservations go on, and resumed: the backlog of thousands of rows
//! must be worked off in passes of at most one batch each, while the
//! reservations keep succeeding. Without the pruner, the store refuses every
//! reservation once the table reaches its cap (`MAX_OCCURRENCES`, 4096),
//! after 34 hours.
use super::{
    fixtures::core_store,
    sim::{rule_limits, soak_dir},
    support::{DAY, HOUR, START},
};
use limeos_domain::{
    Operation, Role, Scope,
    automation::{
        CreateSchedule, DueDecision, MAX_TICK_SCHEDULES, MissedRunPolicy, Reservation,
        ScheduleDefinition, ScheduledOperation, Settlement,
    },
};
use limeos_persistence::{Store, test_support};
use std::time::{Duration, Instant};

/// Grants the administrator automation management and creates one
/// observe-health schedule at the 30 s minimum. Returns its id.
pub fn observe_health_schedule(store: &mut Store) -> String {
    let initial = store.login_record("marc").unwrap().unwrap().principal;
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
                scope(Operation::AutomationManage, "automation:observe-health"),
            ],
            START,
        )
        .unwrap();
    let actor = store.login_record("marc").unwrap().unwrap().principal;
    store
        .automation_create(
            &actor,
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
            START,
        )
        .unwrap()
        .schedule
        .id
}

#[test]
#[ignore = "30 simulated days of occurrences at the minimum interval: in the soak CI step"]
fn soak_occurrences_stay_at_their_budget() {
    let dir = soak_dir();
    let (path, _) = core_store(dir.path());
    let mut store = Store::open(&path).unwrap();
    let schedule = observe_health_schedule(&mut store);
    let (budget, batch) = rule_limits("automation_occurrences");
    let side = super::checks::reader(&path).unwrap();
    let occurrences = || -> i64 {
        side.query_row("SELECT count(*) FROM automation_occurrences", [], |r| {
            r.get(0)
        })
        .unwrap()
    };
    let epoch = Instant::now();
    let normal_end = START + 30 * DAY;
    let (suspended, resumed) = (normal_end, normal_end + 25 * HOUR);
    let end = resumed + 6 * HOUR;
    let mut problems = Vec::new();
    let (mut reached, mut peak, mut largest, mut passes_to_recover) = (false, 0, 0, None);
    let mut now = START + 40;
    while now < end && problems.len() < 20 {
        let row = store
            .automation_schedule_page(None, MAX_TICK_SCHEDULES)
            .unwrap()
            .into_iter()
            .find(|row| row.id == schedule)
            .unwrap();
        let DueDecision::Skipped(key) = row.due(now).unwrap() else {
            panic!("the schedule did not skip an occurrence ten seconds late");
        };
        let deadline = Instant::now() + Duration::from_secs(60);
        match store.automation_reserve_current(&row, &Settlement::Skipped(key), || now, deadline) {
            Ok(Reservation::Skipped) => {}
            other => problems.push(format!(
                "day {:.2}: the reservation answered {other:?} with {} occurrences kept (budget {budget})",
                (now - START) as f64 / DAY as f64,
                occurrences()
            )),
        }
        let running = now < suspended || now >= resumed;
        let at = epoch + Duration::from_secs((now - START) as u64);
        if running && let Some(pass) = test_support::maintain(&mut store, at) {
            let before = occurrences()
                + pass
                    .removed
                    .iter()
                    .find(|(rule, _)| *rule == "automation_occurrences")
                    .map_or(0, |(_, rows)| *rows);
            let after = occurrences();
            let removed = before - after;
            largest = largest.max(removed);
            peak = peak.max(before);
            if !pass.failed.is_empty() {
                problems.push(format!("a retention rule failed: {:?}", pass.failed));
            }
            if removed != (before - budget).clamp(0, batch) {
                problems.push(format!(
                    "day {:.2}: a pass removed {removed} of {before} occurrences (budget {budget}, batch {batch})",
                    (now - START) as f64 / DAY as f64
                ));
            }
            if now >= resumed && passes_to_recover.is_none() {
                passes_to_recover = Some(0);
            }
            if let Some(passes) = passes_to_recover.as_mut()
                && after > budget
            {
                *passes += 1;
            }
            reached |= after >= budget;
            if reached && after != budget && now < suspended {
                problems.push(format!(
                    "day {:.2}: {after} occurrences after a pass, not the budget of {budget}",
                    (now - START) as f64 / DAY as f64
                ));
            }
        }
        now += 30;
    }
    // Two reservations a minute: at most two since the last pass.
    let after = occurrences();
    let recovered = passes_to_recover.unwrap_or(0);
    let backlog = peak - budget;
    if after > budget + 2 || backlog < 4 * batch || recovered > backlog / (batch - 2) + 1 {
        problems.push(format!(
            "after the suspended day: a backlog of {backlog} took {recovered} passes, leaving {after} occurrences (budget {budget}, batch {batch})"
        ));
    }
    eprintln!(
        "occurrences: peak {peak}, largest batch {largest}, a backlog of {backlog} worked off in {} passes",
        recovered + 1
    );
    assert!(
        problems.is_empty(),
        "occurrence retention:\n{}",
        problems.join("\n")
    );
}
