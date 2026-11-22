use super::*;
use crate::scale;
use limeos_domain::{ContainerSnapshot, PlannedRestart};

/// When the fixtures' jobs are queued: their plans expire `TTL` later.
const T0: i64 = 1_000_000;
/// A probe's deadline, past its queueing (`probe`).
const TTL: i64 = 600;
/// The first moment every fixture job queued and settled at `T0` is past
/// the horizon.
const PAST: i64 = T0 + TTL + HORIZON + 1;

fn setup() -> (tempfile::TempDir, Store, Principal) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    let actor = scale::actor(&mut store);
    (dir, store, actor)
}

/// A health probe queued at `at` and canceled then: a settled job with two
/// events.
fn probe(store: &mut Store, actor: &Principal, key: &str, at: i64) -> String {
    let id = store
        .queue_job(
            actor,
            key,
            &Intent::HealthProbe {
                resource: key.into(),
            },
            at + TTL,
            at,
        )
        .unwrap();
    store
        .transition(&id, JobState::Queued, JobState::Canceled, at)
        .unwrap();
    id
}
fn probes(store: &mut Store, actor: &Principal, prefix: &str, n: usize, at: i64) -> Vec<String> {
    (0..n)
        .map(|i| probe(store, actor, &format!("{prefix}{i}"), at))
        .collect()
}

fn snapshot(n: usize) -> ContainerSnapshot {
    ContainerSnapshot {
        resource: format!("container:{n:064x}"),
        image: format!("sha256:{}", "b".repeat(64)),
        started_at: "2026-10-05T07:00:00Z".into(),
        running: true,
    }
}
/// A container job planned, approved and queued at `at`, then canceled: its
/// plan (consumed, naming it) expires `PLAN_TTL_SECONDS` later.
fn container(store: &mut Store, actor: &Principal, n: usize, at: i64) -> (String, PlannedRestart) {
    let before = snapshot(n);
    let plan = store.plan_restart(actor, &before, at).unwrap();
    let approval = store
        .approve_container(actor, &plan.plan.id, &plan.digest, at)
        .unwrap();
    let job = store
        .queue_container(actor, &format!("c{n}"), &plan, &approval.token, &before, at)
        .unwrap();
    store.cancel_container_job(actor, &job.id, at).unwrap();
    (job.id, plan)
}

fn exists(store: &Store, id: &str) -> bool {
    store
        .conn
        .query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE id=?)", [id], |r| {
            r.get(0)
        })
        .unwrap()
}
fn rows(store: &Store, table: &str, id: &str) -> i64 {
    store
        .conn
        .query_row(
            &format!("SELECT count(*) FROM {table} WHERE job=?"),
            [id],
            |r| r.get(0),
        )
        .unwrap()
}
fn audit_agrees(store: &Store) {
    assert_eq!(
        store.audit_usage().unwrap().bytes,
        scale::events(store) * EVENT_BYTES
    );
}
/// The foreign-key and quick checks, and the whole startup check.
fn checks_clean(store: &Store) {
    let violations: i64 = store
        .conn
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(violations, 0);
    let quick: String = store
        .conn
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(quick, "ok");
    crate::resources::check(&store.conn).unwrap();
}
/// Job-row batches at `now` until one removes nothing; the jobs removed.
fn drain_jobs(store: &mut Store, now: i64) -> i64 {
    let mut total = 0;
    loop {
        let tx = store
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let removed = prune(&tx, BATCH, now).unwrap();
        tx.commit().unwrap();
        if !removed.any() {
            return total;
        }
        total += removed.rows;
    }
}

/// Every table that names a job, or a job's resource set, is either removed
/// with it or keeps it: read from the schema's foreign keys, so a table
/// added later fails here until it is placed.
#[test]
fn every_table_that_names_a_job_is_placed() {
    let (_dir, store, _) = setup();
    let tables: Vec<String> = store
        .conn
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    let mut naming = Vec::new();
    for table in tables {
        let parents: Vec<String> = store
            .conn
            .prepare(&format!(
                "SELECT \"table\" FROM pragma_foreign_key_list('{table}')"
            ))
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        if parents.iter().any(|p| p == "jobs" || p == "job_resources") {
            naming.push(table);
        }
    }
    naming.sort();
    let mut placed: Vec<String> = DEPENDENTS
        .iter()
        .chain(&PROTECTING)
        .map(|t| (*t).to_owned())
        .collect();
    placed.sort();
    // `automation_notifications` names a result, not a job: its job is never
    // removed (rule 2), so neither is the result it names.
    assert_eq!(naming, placed);
}

/// The rule removes a settled probe past the horizon and the floor, and
/// keeps every job any reference protects: unsettled ones, an open request,
/// an authority event, any recent time, another operation, a row in a
/// protecting table.
#[test]
fn every_reference_keeps_its_job() {
    let (_dir, mut store, actor) = setup();
    // The floor of newer probes, so the fixtures below are past it.
    let older = probes(&mut store, &actor, "old", 8, T0);
    // Unsettled: queued, held (needs_intervention, with its locks).
    let queued = store
        .queue_job(
            &actor,
            "queued",
            &Intent::HealthProbe {
                resource: "queued".into(),
            },
            T0 + TTL,
            T0,
        )
        .unwrap();
    let held = store
        .queue_job(
            &actor,
            "held",
            &Intent::HealthProbe {
                resource: "held".into(),
            },
            T0 + TTL,
            T0,
        )
        .unwrap();
    store
        .transition(&held, JobState::Queued, JobState::Running, T0)
        .unwrap();
    store
        .transition(&held, JobState::Running, JobState::NeedsIntervention, T0)
        .unwrap();
    // Settled with its resolution request still open.
    let open = store
        .queue_job(
            &actor,
            "open",
            &Intent::HealthProbe {
                resource: "open".into(),
            },
            T0 + TTL,
            T0,
        )
        .unwrap();
    store
        .transition(&open, JobState::Queued, JobState::Running, T0)
        .unwrap();
    store.conn.execute(
        "INSERT INTO job_resolutions(job,resolved_by,declared,reason,evidence,evidence_digest,requested) VALUES(?,?,'not_done','checked by hand','{}',?,?)",
        params![open, actor.id, "e".repeat(64), T0],
    ).unwrap();
    store
        .conn
        .execute("UPDATE jobs SET state='failed' WHERE id=?", [&open])
        .unwrap();
    // Settled, with an authority event (an operator's declaration).
    let declared = probe(&mut store, &actor, "declared", T0);
    store
        .write(|tx| {
            event(
                tx,
                Some(&actor.id),
                Some(&declared),
                EventKind::JobResolutionWithdrawn,
                T0,
            )
        })
        .unwrap();
    // Settled long ago, but with an event, a result or a deadline inside
    // the horizon.
    let late_event = probe(&mut store, &actor, "late-event", T0);
    store
        .write(|tx| {
            event(
                tx,
                Some(&actor.id),
                Some(&late_event),
                EventKind::JobTransition {
                    state: JobState::Canceled,
                },
                PAST - HORIZON,
            )
        })
        .unwrap();
    let late_result = probe(&mut store, &actor, "late-result", T0);
    store
        .conn
        .execute(
            "INSERT INTO container_results VALUES(?,'{}',NULL,?)",
            params![late_result, PAST - HORIZON],
        )
        .unwrap();
    let late_deadline = store
        .queue_job(
            &actor,
            "late-deadline",
            &Intent::HealthProbe {
                resource: "late-deadline".into(),
            },
            T0 + TTL,
            T0,
        )
        .unwrap();
    store
        .transition(&late_deadline, JobState::Queued, JobState::Canceled, T0)
        .unwrap();
    store
        .conn
        .execute_batch("DROP TRIGGER jobs_identity_immutable")
        .unwrap();
    store
        .conn
        .execute(
            "UPDATE jobs SET deadline=? WHERE id=?",
            params![PAST - HORIZON, late_deadline],
        )
        .unwrap();
    // A row in a table that keeps its job.
    let protected = probe(&mut store, &actor, "protected", T0);
    store
        .conn
        .execute(
            "INSERT INTO storage_target_plans(id,principal,body,digest,revision,expires,job) VALUES('p',?,'{}',?,1,0,?)",
            params![actor.id, "f".repeat(64), protected],
        )
        .unwrap();
    // Another operation, settled and old.
    let other = format!("{:064x}", 7);
    store
        .conn
        .execute(
            "INSERT INTO jobs(id,principal,idempotency,intent,digest,resource,state,generation,revision,deadline) VALUES(?,?,'other',?,?,'storage:configuration','queued',1,1,?)",
            params![
                other,
                actor.id,
                r#"{"operation":"storage_prepare_targets"}"#,
                "d".repeat(64),
                T0 + TTL
            ],
        )
        .unwrap();
    store
        .conn
        .execute("UPDATE jobs SET state='canceled' WHERE id=?", [&other])
        .unwrap();
    // The floor: newer settled probes than all of the above.
    probes(&mut store, &actor, "floor", FLOOR as usize, T0);
    let kept = [
        &queued,
        &held,
        &open,
        &declared,
        &late_event,
        &late_result,
        &late_deadline,
        &protected,
        &other,
    ];
    assert_eq!(drain_jobs(&mut store, PAST), older.len() as i64);
    for id in &older {
        assert!(!exists(&store, id));
    }
    for id in kept {
        assert!(exists(&store, id), "{id} was removed");
    }
    // Each recent time protects only until it too is past the horizon.
    assert_eq!(drain_jobs(&mut store, PAST + 1), 3);
    for id in [&late_event, &late_result, &late_deadline] {
        assert!(!exists(&store, id));
    }
    audit_agrees(&store);
}

/// Every way a container, stack or probe job settles without a dated row of
/// its own outside its events: the generic transition (a cancellation, a
/// probe's failure, success or changed precondition) and an execute that
/// was never sent. A job queued long ago and settled only now is kept for
/// the whole horizon after it settled, even once the events rule has taken
/// every event it had (it removes a settled job's events whatever their
/// age, oldest first, under pressure), and goes once that horizon is past.
/// A result or a completed resolution is a dated row of its own.
#[test]
fn a_late_settlement_keeps_its_job_for_the_horizon() {
    let (_dir, mut store, actor) = setup();
    // Long after every fixture's plan expired.
    let late = T0 + 10 * HORIZON;
    let queue = |store: &mut Store, key: &str| -> String {
        store
            .queue_job(
                &actor,
                key,
                &Intent::HealthProbe {
                    resource: key.into(),
                },
                T0 + TTL,
                T0,
            )
            .unwrap()
    };
    let canceled = queue(&mut store, "canceled");
    store
        .transition(&canceled, JobState::Queued, JobState::Canceled, late)
        .unwrap();
    let changed = queue(&mut store, "changed");
    store
        .transition(
            &changed,
            JobState::Queued,
            JobState::PreconditionChanged,
            late,
        )
        .unwrap();
    let failed = queue(&mut store, "failed");
    store
        .transition(&failed, JobState::Queued, JobState::Running, T0)
        .unwrap();
    store
        .transition(&failed, JobState::Running, JobState::Failed, late)
        .unwrap();
    let succeeded = queue(&mut store, "succeeded");
    store
        .transition(&succeeded, JobState::Queued, JobState::Running, T0)
        .unwrap();
    store
        .transition(&succeeded, JobState::Running, JobState::Verifying, T0)
        .unwrap();
    store
        .transition(&succeeded, JobState::Verifying, JobState::Succeeded, late)
        .unwrap();
    let queue_container = |store: &mut Store, n: usize| -> String {
        let before = snapshot(n);
        let plan = store.plan_restart(&actor, &before, T0).unwrap();
        let approval = store
            .approve_container(&actor, &plan.plan.id, &plan.digest, T0)
            .unwrap();
        store
            .queue_container(&actor, &format!("c{n}"), &plan, &approval.token, &before, T0)
            .unwrap()
            .id
    };
    let container_canceled = queue_container(&mut store, 0);
    store
        .cancel_container_job(&actor, &container_canceled, late)
        .unwrap();
    let unsent = queue_container(&mut store, 1);
    store
        .claim_container(&actor, &unsent, &snapshot(1), &"1".repeat(64), T0)
        .unwrap();
    store.record_unsent_execute(&unsent, late).unwrap();
    // Newer than all of them in both families, settled just after them.
    for n in 0..FLOOR as usize {
        let id = queue(&mut store, &format!("floor{n}"));
        store
            .transition(&id, JobState::Queued, JobState::Canceled, late)
            .unwrap();
        let id = queue_container(&mut store, 2 + n);
        store.cancel_container_job(&actor, &id, late).unwrap();
    }
    let kept = [
        &canceled,
        &changed,
        &failed,
        &succeeded,
        &container_canceled,
        &unsent,
    ];
    // The events rule under pressure: every event it may remove, gone.
    let tx = store
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    super::super::prune_events(&tx, i64::MAX, late).unwrap();
    tx.commit().unwrap();
    for id in kept {
        assert_eq!(rows(&store, "events", id), 0);
    }
    assert_eq!(drain_jobs(&mut store, late + 1), 0);
    for id in kept {
        assert!(exists(&store, id), "{id} was removed as soon as it settled");
    }
    assert_eq!(drain_jobs(&mut store, late + HORIZON), 0);
    assert_eq!(drain_jobs(&mut store, late + HORIZON + 1), kept.len() as i64);
    for id in kept {
        assert!(!exists(&store, id));
    }
    checks_clean(&store);
    audit_agrees(&store);
}

/// The settlement time is set with the state that settles a job and never
/// changes; an unsettled job has none; a state change that does not set it
/// takes the system clock, so no settled job is ever without one.
#[test]
fn the_settlement_time_is_set_once_and_kept() {
    let (_dir, mut store, actor) = setup();
    let settled_of = |store: &Store, id: &str| -> Option<i64> {
        store
            .conn
            .query_row("SELECT settled FROM jobs WHERE id=?", [id], |r| r.get(0))
            .unwrap()
    };
    let id = probe(&mut store, &actor, "p", T0);
    assert_eq!(settled_of(&store, &id), Some(T0));
    assert!(
        store
            .conn
            .execute("UPDATE jobs SET settled=? WHERE id=?", params![PAST, id])
            .is_err()
    );
    let queued = store
        .queue_job(
            &actor,
            "q",
            &Intent::HealthProbe {
                resource: "q".into(),
            },
            T0 + TTL,
            T0,
        )
        .unwrap();
    assert_eq!(settled_of(&store, &queued), None);
    assert!(
        store
            .conn
            .execute("UPDATE jobs SET settled=? WHERE id=?", params![T0, queued])
            .is_err()
    );
    let before = crate::writer_clock::unix_now();
    store
        .conn
        .execute("UPDATE jobs SET state='canceled' WHERE id=?", [&queued])
        .unwrap();
    let at = settled_of(&store, &queued).unwrap();
    assert!(at >= before && at <= crate::writer_clock::unix_now(), "{at}");
}

/// The upgrade to schema 19 takes every settled job already stored as
/// settled at the upgrade, whatever its history says: none goes for the
/// whole horizon after it.
#[test]
fn an_upgraded_store_keeps_its_settled_jobs_for_the_horizon() {
    let (dir, mut store, actor) = setup();
    let path = dir.path().join("core.sqlite");
    let old = probes(&mut store, &actor, "p", FLOOR as usize + 2, T0);
    crate::remove_v19_schema(&store.conn);
    store
        .conn
        .execute_batch("PRAGMA user_version=18")
        .unwrap();
    drop(store);
    let before = crate::writer_clock::unix_now();
    let mut store = Store::open(&path).unwrap();
    let after = crate::writer_clock::unix_now();
    let (low, high): (i64, i64) = store
        .conn
        .query_row("SELECT min(settled),max(settled) FROM jobs", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert!(low >= before && high <= after, "{low}..{high}");
    assert_eq!(drain_jobs(&mut store, before + HORIZON), 0);
    assert_eq!(drain_jobs(&mut store, after + HORIZON + 1), 2);
    assert!(!exists(&store, &old[0]) && !exists(&store, &old[1]));
    checks_clean(&store);
}

/// Only settled jobs past the newest `FLOOR` of their family go, and the
/// floor is per family: a family under it keeps every job however old.
#[test]
fn the_floor_keeps_the_newest_of_each_family() {
    let (_dir, mut store, actor) = setup();
    let first = probes(&mut store, &actor, "p", FLOOR as usize + 10, T0);
    let (container_job, _) = container(&mut store, &actor, 1, T0);
    assert_eq!(drain_jobs(&mut store, PAST), 10);
    for (n, id) in first.iter().enumerate() {
        assert_eq!(exists(&store, id), n >= 10, "probe {n}");
    }
    // One container job, far under its family's floor.
    assert!(exists(&store, &container_job));
    // At the floor exactly, nothing more goes, at any time.
    assert_eq!(drain_jobs(&mut store, PAST + 10 * HORIZON), 0);
    assert_eq!(settled(&store.conn).unwrap(), FLOOR + 1);
}

/// A batch removes at most `BATCH` jobs and `ROWS` rows; a job whose history
/// alone exceeds a batch loses its oldest events first (its `job_queued`
/// event in the first) over several batches, then goes; every batch keeps
/// the audit counter and the checks clean.
#[test]
fn batches_are_bounded_and_a_long_history_goes_over_several() {
    let (_dir, mut store, actor) = setup();
    let long = probe(&mut store, &actor, "long", T0);
    let kind = json(&EventKind::JobTransition {
        state: JobState::Canceled,
    })
    .unwrap();
    store
        .write(|tx| {
            let added = tx.execute("WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x<5000) INSERT INTO events(principal,job,kind,created) SELECT ?,?,?,? FROM c", params![actor.id, long, kind, T0]).map_err(durable)?;
            account(tx, added as i64)
        })
        .unwrap();
    let many = probes(&mut store, &actor, "m", 2 * BATCH as usize, T0);
    probes(&mut store, &actor, "floor", FLOOR as usize, T0);
    let queued_mark = json(&EventKind::JobQueued).unwrap();
    let (mut batches, mut partial) = (0, 0);
    loop {
        let changes = |store: &Store| -> i64 {
            store
                .conn
                .query_row("SELECT total_changes()", [], |r| r.get(0))
                .unwrap()
        };
        let before = changes(&store);
        let tx = store
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let removed = prune(&tx, BATCH, PAST).unwrap();
        tx.commit().unwrap();
        let changed = changes(&store) - before;
        if !removed.any() {
            break;
        }
        batches += 1;
        assert!(removed.rows <= BATCH, "{} jobs in one batch", removed.rows);
        // Deletes, plus the counter update and the resource sets the
        // triggers never touch: every change is a removed row or the counter.
        assert!(changed <= ROWS + 1, "{changed} changes in one batch");
        if removed.rows == 0 {
            partial += 1;
            assert!(removed.events <= EVENT_BATCH);
            // Partly pruned: its history reads as gone.
            assert_eq!(
                store
                    .conn
                    .query_row(
                        "SELECT count(*) FROM events WHERE job=? AND kind=?",
                        params![long, queued_mark],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                0
            );
        }
        audit_agrees(&store);
        checks_clean(&store);
    }
    assert!(!exists(&store, &long));
    assert_eq!(rows(&store, "events", &long), 0);
    for id in &many {
        assert!(!exists(&store, id));
    }
    // 5,002 events: three batches of 1,024, then the rest with the job and
    // as many probes as fit, then the probes 64 a batch.
    assert_eq!(partial, 3);
    assert!(batches >= partial + 3, "{batches} batches");
}

/// A pruned container job takes its plan, events, resources and result with
/// it, and then reads as never issued: not found by ID or events, gone from
/// the listing, and a replayed approval of its plan cannot queue it again.
/// A resolved job takes its completed resolution, which its job's presence
/// alone kept immutable.
#[test]
fn a_pruned_job_is_unknown_and_nothing_names_it() {
    let (dir, mut store, actor) = setup();
    let (old, plan) = container(&mut store, &actor, 0, T0);
    store
        .conn
        .execute(
            "INSERT INTO container_results VALUES(?,'{}',NULL,?)",
            params![old, T0],
        )
        .unwrap();
    // A resolved probe with its completed resolution, and no authority
    // event left.
    let resolved = store
        .queue_job(
            &actor,
            "resolved",
            &Intent::HealthProbe {
                resource: "resolved".into(),
            },
            T0 + TTL,
            T0,
        )
        .unwrap();
    store
        .transition(&resolved, JobState::Queued, JobState::Running, T0)
        .unwrap();
    store
        .transition(
            &resolved,
            JobState::Running,
            JobState::NeedsIntervention,
            T0,
        )
        .unwrap();
    store.conn.execute(
        "INSERT INTO job_resolutions(job,resolved_by,declared,reason,evidence,evidence_digest,requested) VALUES(?,?,'not_done','checked by hand','{}',?,?)",
        params![resolved, actor.id, "e".repeat(64), T0],
    ).unwrap();
    store
        .conn
        .execute(
            "UPDATE job_resolutions SET confirmation='{}',completed=? WHERE job=?",
            params![T0, resolved],
        )
        .unwrap();
    store
        .conn
        .execute(
            "UPDATE jobs SET state='resolved',settled=? WHERE id=?",
            params![T0, resolved],
        )
        .unwrap();
    // While the job exists its resolution and primary resource are immutable.
    assert!(
        store
            .conn
            .execute("DELETE FROM job_resolutions WHERE job=?", [&resolved])
            .is_err()
    );
    assert!(
        store
            .conn
            .execute("DELETE FROM job_resources WHERE job=?", [&old])
            .is_err()
    );
    for n in 1..=FLOOR as usize {
        container(&mut store, &actor, n, T0);
    }
    probes(&mut store, &actor, "floor", FLOOR as usize, T0);
    assert_eq!(rows(&store, "container_plans", &old), 1);
    assert_eq!(drain_jobs(&mut store, PAST), 2);
    for table in DEPENDENTS {
        assert_eq!(rows(&store, table, &old), 0, "{table}");
        assert_eq!(rows(&store, table, &resolved), 0, "{table}");
    }
    assert_eq!(
        store.container_job(&actor, &old).unwrap_err().0,
        ErrorCode::NotFound
    );
    assert_eq!(
        store.container_events(&actor, &old, 0).unwrap_err().0,
        ErrorCode::NotFound
    );
    assert!(
        store
            .container_jobs(&actor)
            .unwrap()
            .iter()
            .all(|job| job.id != old)
    );
    assert_eq!(
        store.container_plan(&actor, &plan.plan.id).unwrap_err().0,
        ErrorCode::NotFound
    );
    assert_eq!(
        store
            .queue_container(&actor, "c0", &plan, &"0".repeat(64), &snapshot(0), PAST)
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
    checks_clean(&store);
    audit_agrees(&store);
    drop(store);
    Store::open(&dir.path().join("core.sqlite")).unwrap();
}

/// The schema refuses what the rule never does: deleting an unsettled job,
/// a job of another operation or one with an open request, and leaving a
/// dependent behind (the commit fails, nothing changes).
#[test]
fn the_schema_holds_the_rule() {
    let (_dir, mut store, actor) = setup();
    let queued = store
        .queue_job(
            &actor,
            "queued",
            &Intent::HealthProbe {
                resource: "queued".into(),
            },
            T0 + TTL,
            T0,
        )
        .unwrap();
    let settled_probe = probe(&mut store, &actor, "settled", T0);
    let delete = |store: &mut Store, id: &str, dependents: &[&str]| -> bool {
        let tx = store.conn.transaction().unwrap();
        tx.pragma_update(None, "defer_foreign_keys", true).unwrap();
        let mut ok = tx.execute("DELETE FROM jobs WHERE id=?", [id]).is_ok();
        for table in dependents {
            ok &= tx
                .execute(&format!("DELETE FROM {table} WHERE job=?"), [id])
                .is_ok();
        }
        ok && tx.commit().is_ok()
    };
    assert!(!delete(&mut store, &queued, &DEPENDENTS));
    assert!(exists(&store, &queued));
    // A settled job whose intent names no operation (or is no JSON object
    // at all): `NOT IN` alone is NULL there, which would let it go.
    for (n, intent) in [r#"{"resource":"x"}"#, "[]", "1"].into_iter().enumerate() {
        let id = format!("{n:064x}");
        store
            .conn
            .execute(
                "INSERT INTO jobs(id,principal,idempotency,intent,digest,resource,state,generation,revision,deadline) VALUES(?,?,?,?,?,'fixture:x','queued',1,1,?)",
                params![id, actor.id, id, intent, "d".repeat(64), T0 + TTL],
            )
            .unwrap();
        store
            .conn
            .execute("UPDATE jobs SET state='canceled' WHERE id=?", [&id])
            .unwrap();
        assert!(!delete(&mut store, &id, &DEPENDENTS), "{intent}");
        assert!(exists(&store, &id));
    }
    // Its events left behind: the commit refuses.
    assert!(!delete(&mut store, &settled_probe, &["job_resources"]));
    assert!(exists(&store, &settled_probe));
    assert!(delete(&mut store, &settled_probe, &DEPENDENTS));
    assert!(!exists(&store, &settled_probe));
    checks_clean(&store);
}

/// The candidate query reads settled jobs through `jobs_state` and each
/// one's events through `events_job`, and scans neither table.
#[test]
fn the_candidate_query_uses_the_indexes() {
    let (_dir, store, _) = setup();
    let plan: Vec<String> = store
        .conn
        .prepare(&format!("EXPLAIN QUERY PLAN {}", candidates()))
        .unwrap()
        .query_map(params![PAST, 1, 1, 1, BATCH], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert!(
        plan.iter()
            .any(|s| s.starts_with("SEARCH j USING INDEX jobs_state")),
        "{plan:?}"
    );
    assert!(
        plan.iter()
            .any(|s| s.starts_with("SEARCH e USING") && s.contains("events_job")),
        "{plan:?}"
    );
    for step in &plan {
        assert!(
            !step.starts_with("SCAN j") && !step.starts_with("SCAN e"),
            "{plan:?}"
        );
    }
}

/// Through the writer's own entry point: a pass with settled jobs at the
/// floor writes nothing, and one past it removes one bounded batch.
#[test]
fn the_writer_pass_runs_the_rule_and_an_idle_one_writes_nothing() {
    let (_dir, mut store, actor) = setup();
    probes(&mut store, &actor, "p", FLOOR as usize, T0);
    let changes = |store: &Store| -> i64 {
        store
            .conn
            .query_row("SELECT total_changes()", [], |r| r.get(0))
            .unwrap()
    };
    let before = changes(&store);
    assert!(store.prune_at(PAST).unwrap().is_empty());
    assert_eq!(changes(&store), before);
    probes(&mut store, &actor, "q", BATCH as usize + 3, T0);
    assert_eq!(store.prune_at(PAST).unwrap(), vec![("jobs", BATCH)]);
    assert_eq!(store.prune_at(PAST).unwrap(), vec![("jobs", 3)]);
    assert!(store.prune_at(PAST).unwrap().is_empty());
}

#[test]
fn job_prune_crash_helper() {
    let Some(path) = std::env::var_os("LIMEOS_JOB_PRUNE_CRASH_PATH") else {
        return;
    };
    let mut store = Store::open(Path::new(&path)).unwrap();
    let actor = scale::actor(&mut store);
    probes(&mut store, &actor, "p", FLOOR as usize + 5, T0);
    // The parent set LIMEOS_PRUNE_PAUSE_PATH for this process only: the batch
    // stops after deleting the job rows, before their dependents and before
    // commit, until it is killed.
    store.prune_at(PAST).unwrap();
    unreachable!("the batch pauses until killed");
}

/// Killed mid-batch (job rows deleted, their dependents not yet), the store
/// reopens as before the batch: every job and event there, the foreign-key
/// and startup checks clean, and the next batch removes the same jobs.
#[test]
fn a_kill_mid_batch_leaves_every_job_and_the_checks_clean() {
    use std::os::unix::process::ExitStatusExt;
    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let ready = path.with_extension("ready");
    let mut child = ChildGuard(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "retention::jobs::tests::job_prune_crash_helper"])
            .env("LIMEOS_JOB_PRUNE_CRASH_PATH", &path)
            .env("LIMEOS_PRUNE_PAUSE_PATH", &ready)
            .spawn()
            .unwrap(),
    );
    for _ in 0..3000 {
        if ready.exists() {
            break;
        }
        assert!(child.0.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.exists());
    child.0.kill().unwrap();
    assert_eq!(child.0.wait().unwrap().signal(), Some(9));
    let mut store = Store::open(&path).unwrap();
    checks_clean(&store);
    let total = |store: &Store, table: &str| -> i64 {
        store
            .conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    };
    let jobs = FLOOR + 5;
    assert_eq!(total(&store, "jobs"), jobs);
    assert_eq!(total(&store, "job_resources"), jobs);
    // Enrolment's three, two a probe.
    assert_eq!(scale::events(&store), 3 + 2 * jobs);
    audit_agrees(&store);
    assert_eq!(store.prune_at(PAST).unwrap(), vec![("jobs", 5)]);
    assert_eq!(total(&store, "jobs"), FLOOR);
    checks_clean(&store);
    drop(store);
    Store::open(&path).unwrap();
}

/// Restarted between batches, with one job's history partly removed, the
/// store opens and the rule finishes it.
#[test]
fn a_restart_between_batches_resumes() {
    let (dir, mut store, actor) = setup();
    let long = probe(&mut store, &actor, "long", T0);
    let kind = json(&EventKind::JobTransition {
        state: JobState::Canceled,
    })
    .unwrap();
    store
        .write(|tx| {
            let added = tx.execute("WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x<3000) INSERT INTO events(principal,job,kind,created) SELECT ?,?,?,? FROM c", params![actor.id, long, kind, T0]).map_err(durable)?;
            account(tx, added as i64)
        })
        .unwrap();
    probes(&mut store, &actor, "floor", FLOOR as usize, T0);
    assert_eq!(store.prune_at(PAST).unwrap(), vec![("jobs", EVENT_BATCH)]);
    drop(store);
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    checks_clean(&store);
    assert!(exists(&store, &long));
    assert_eq!(drain_jobs(&mut store, PAST), 1);
    assert!(!exists(&store, &long));
    audit_agrees(&store);
}
