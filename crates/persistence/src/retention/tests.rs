use super::*;
use crate::scale::{self, events, fill};
use limeos_domain::automation::*;

/// Crash-test hook: a child process stops here, mid-batch and uncommitted,
/// until the parent kills it.
pub(super) fn pause_mid_batch() {
    if let Some(path) = std::env::var_os("LIMEOS_PRUNE_PAUSE_PATH") {
        std::fs::write(path, b"paused").unwrap();
        loop {
            std::thread::park();
        }
    }
}

const BUDGET_ROWS: i64 = EVENT_BUDGET / EVENT_BYTES;

fn setup() -> (tempfile::TempDir, Store, Principal) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    let actor = scale::actor(&mut store);
    (dir, store, actor)
}
fn probe(store: &mut Store, actor: &Principal, key: &str, resource: &str, now: i64) -> String {
    store
        .queue_job(
            actor,
            key,
            &Intent::HealthProbe {
                resource: resource.into(),
            },
            now + 600,
            now,
        )
        .unwrap()
}
fn job_events(store: &Store, job: &str) -> Vec<(i64, String)> {
    store
        .conn
        .prepare("SELECT cursor,kind FROM events WHERE job=? ORDER BY cursor")
        .unwrap()
        .query_map([job], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap()
}
fn pinned_loose(store: &Store) -> Vec<(i64, String)> {
    store
        .conn
        .prepare("SELECT cursor,kind FROM events WHERE job IS NULL AND json_extract(kind,'$.kind') GLOB 'automation_*' AND json_extract(kind,'$.kind') NOT IN ('automation_schedule_created','automation_occurrence_settled') ORDER BY cursor")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap()
}
fn authority(store: &Store) -> Vec<(i64, String)> {
    store
        .conn
        .prepare(&format!(
            "SELECT cursor,kind FROM events WHERE json_extract(kind,'$.kind') IN ({}) ORDER BY cursor",
            quoted(&AUTHORITY_KINDS)
        ))
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap()
}
fn audit(store: &Store) -> i64 {
    store.audit_usage().unwrap().bytes
}
/// Prunes until a pass removes nothing.
fn drain(store: &mut Store) {
    while !store.prune().unwrap().is_empty() {}
}

#[test]
fn pinned_kind_lists_match_the_sql() {
    let conn = Connection::open_in_memory().unwrap();
    let loose = |kind: &str| -> bool {
        conn.query_row(
            &format!(
                "SELECT {} FROM (SELECT NULL AS job,json_object('kind',?) AS kind) e",
                super::loose()
            ),
            [kind],
            |r| r.get(0),
        )
        .unwrap()
    };
    for kind in PINNED_KINDS {
        assert!(!loose(kind), "{kind} must be pinned");
    }
    for kind in UNREAD_AUTOMATION_KINDS {
        assert!(loose(kind), "{kind} is unread and prunable");
    }
    // A kind no list knows about fails safe.
    assert!(!loose("automation_future_proof"));
    for kind in [
        "session_created",
        "session_revoked",
        "task_issued",
        "task_revoked",
        "plan_created",
    ] {
        assert!(loose(kind));
    }
    // Authority events are their own class, never bulk.
    for kind in AUTHORITY_KINDS {
        assert!(!loose(kind), "{kind} is an authority event");
    }
    for kind in [
        EventKind::BootstrapIssued,
        EventKind::AdministratorEnrolled,
        EventKind::LegacyIdentityImported,
        EventKind::GrantsRevised,
        EventKind::JobResolutionRequested {
            declared: limeos_domain::DeclaredOutcome::NotDone,
        },
        EventKind::JobResolutionWithdrawn,
    ] {
        let value: serde_json::Value = serde_json::from_str(&json(&kind).unwrap()).unwrap();
        assert!(AUTHORITY_KINDS.contains(&value["kind"].as_str().unwrap()));
    }
    // Every job-less automation kind the store writes is in one of the lists.
    for kind in [
        EventKind::AutomationScheduleCreated {
            schedule: "s".into(),
        },
        EventKind::AutomationScheduleChanged {
            schedule: "s".into(),
            revision: 2,
        },
        EventKind::AutomationOccurrenceSettled {
            schedule: "s".into(),
            revision: 1,
            due: 0,
        },
        EventKind::AutomationNotificationPublished {
            notification: "n".into(),
        },
        EventKind::AutomationNotificationPruned {
            notification: "n".into(),
        },
        EventKind::AutomationNotificationAcknowledged {
            notification: "n".into(),
        },
        EventKind::AutomationHealthPolicyChanged {
            revision: 1,
            digest: "d".into(),
        },
        EventKind::AutomationNotificationPolicyChecked {
            notification: "n".into(),
            revision: 1,
        },
    ] {
        let value: serde_json::Value = serde_json::from_str(&json(&kind).unwrap()).unwrap();
        let name = value["kind"].as_str().unwrap();
        assert!(PINNED_KINDS.contains(&name) || UNREAD_AUTOMATION_KINDS.contains(&name));
    }
}

/// Ten times the cap through the ordinary write gate, with the once-a-minute
/// pass between batches. Open work, locks, interventions, an open resolution
/// and automation proofs are the oldest rows throughout and must survive.
#[test]
fn ten_times_the_audit_cap_keeps_the_store_opening_and_writing() {
    let (dir, mut store, actor) = setup();
    let path = dir.path().join("core.sqlite");
    let base = 1_000_000;
    let observed = scale::schedule(
        &mut store,
        &actor,
        ScheduledOperation::ObserveHealth,
        30,
        base,
    );
    let automation = scale::observe(&mut store, &observed.id, base);
    // Pausing records a schedule change, a job-less proof no rule may remove.
    store
        .automation_pause(
            &actor,
            &observed.id,
            &PauseSchedule {
                revision: 1,
                paused: true,
            },
            base,
        )
        .unwrap();
    let held = probe(&mut store, &actor, "held", "held", base);
    store
        .transition(&held, JobState::Queued, JobState::Running, base)
        .unwrap();
    let queued = probe(&mut store, &actor, "queued", "queued", base);
    let stuck = probe(&mut store, &actor, "stuck", "stuck", base);
    store
        .transition(&stuck, JobState::Queued, JobState::Running, base)
        .unwrap();
    store
        .transition(&stuck, JobState::Running, JobState::NeedsIntervention, base)
        .unwrap();
    // A request left open on a job that later failed keeps its history too.
    let requested = probe(&mut store, &actor, "requested", "requested", base);
    store
        .transition(&requested, JobState::Queued, JobState::Running, base)
        .unwrap();
    store.conn.execute("INSERT INTO job_resolutions VALUES(?,?,'not_done','operator checking',?,?,?,NULL,NULL)",params![requested,actor.id,"{}","e".repeat(64),base]).unwrap();
    store
        .transition(&requested, JobState::Running, JobState::Failed, base)
        .unwrap();
    let settled = probe(&mut store, &actor, "settled", "settled", base);
    // A withdrawn resolution request on a job that settles later: its events
    // are the only record of it and outlive the job's history.
    store
        .write(|tx| {
            event(
                tx,
                Some(&actor.id),
                Some(&settled),
                EventKind::JobResolutionRequested {
                    declared: limeos_domain::DeclaredOutcome::NotDone,
                },
                base,
            )?;
            event(
                tx,
                Some(&actor.id),
                Some(&settled),
                EventKind::JobResolutionWithdrawn,
                base,
            )
        })
        .unwrap();
    store
        .transition(&settled, JobState::Queued, JobState::Canceled, base)
        .unwrap();
    store
        .import_legacy_user("bob", "pbkdf2:sha256:1000000$IgjABokZwwl2gggv$686315920b24fb807abeb0a436ce6385be3f3026e35d934ffbaf15be1a0e7ce1", base)
        .unwrap();
    let granted = authority(&store);
    // Enrolment, the grant revision, the request, its withdrawal, the import.
    assert_eq!(granted.len(), 6);
    let kept: Vec<_> = [&automation.id, &held, &queued, &stuck, &requested]
        .into_iter()
        .map(|job| (job.clone(), job_events(&store, job)))
        .collect();
    assert!(kept.iter().all(|(_, rows)| !rows.is_empty()));
    let loose = pinned_loose(&store);
    assert!(!loose.is_empty());
    let locks: i64 = store
        .conn
        .query_row("SELECT count(*) FROM resource_locks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(locks, 2);

    let total = 10 * AUDIT_LIMIT / EVENT_BYTES;
    let mut written = 0;
    let mut minute = 0;
    while written < total {
        fill(&mut store, &actor.id, EVENT_BATCH, base + minute * 60);
        written += EVENT_BATCH;
        minute += 1;
        let before = audit(&store);
        assert!(before < AUDIT_WARNING);
        store.prune().unwrap();
        if minute % 640 == 0 {
            drop(store);
            store = Store::open(&path).unwrap();
        }
    }
    let usage = store.audit_usage().unwrap();
    assert_eq!(usage.bytes, events(&store) * EVENT_BYTES);
    assert!(usage.bytes <= EVENT_BUDGET + EVENT_BATCH * EVENT_BYTES);
    assert_eq!(usage.level(), AuditLevel::Normal);
    // Recovery after a reopen may append to a held job; nothing is removed.
    for (job, rows) in &kept {
        assert!(job_events(&store, job).starts_with(rows));
    }
    assert_eq!(pinned_loose(&store), loose);
    // Bulk pressure evicted no authority event, not even the pruned job's.
    assert_eq!(authority(&store), granted);
    assert_eq!(job_events(&store, &settled).len(), 2);
    assert_eq!(store.job_state(&settled).unwrap(), "canceled");
    assert_eq!(store.job_state(&stuck).unwrap(), "needs_intervention");
    drop(store);
    let mut store = Store::open(&path).unwrap();
    // Reopening recovered the running job: it keeps its lock and history.
    assert_eq!(store.job_state(&held).unwrap(), "needs_intervention");
    let locks: i64 = store
        .conn
        .query_row("SELECT count(*) FROM resource_locks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(locks, 2);
    let next = probe(&mut store, &actor, "after", "after", base + minute * 60);
    assert_eq!(job_events(&store, &next).len(), 1);
}

#[test]
fn an_unchanged_pass_writes_nothing() {
    let (dir, mut store, actor) = setup();
    let path = dir.path().join("core.sqlite");
    let queued = probe(&mut store, &actor, "queued", "queued", 200);
    let files = || {
        ["core.sqlite", "core.sqlite-wal"].map(|name| {
            std::fs::metadata(dir.path().join(name))
                .ok()
                .map(|m| (m.len(), m.modified().unwrap()))
        })
    };
    let changes = |store: &Store| -> i64 {
        store
            .conn
            .query_row("SELECT total_changes()", [], |r| r.get(0))
            .unwrap()
    };
    // Under budget: one read, no transaction.
    let (before, counted) = (files(), changes(&store));
    let now = Instant::now();
    assert!(store.maintain(now, 0).is_some());
    assert!(store.maintain(now + Duration::from_secs(30), 0).is_none());
    assert!(store.prune().unwrap().is_empty());
    assert_eq!((files(), changes(&store)), (before, counted));
    // Over budget with nothing eligible (only open work): the batch rolls
    // back and still writes nothing.
    store
        .conn
        .execute_batch(&format!(
            "DELETE FROM events WHERE job IS NULL; UPDATE meta SET audit_bytes={} WHERE id=1",
            EVENT_BUDGET * 2
        ))
        .unwrap();
    let (before, counted) = (files(), changes(&store));
    assert!(store.maintain(now + INTERVAL, 0).is_some());
    assert!(store.prune().unwrap().is_empty());
    assert_eq!((files(), changes(&store)), (before, counted));
    assert_eq!(job_events(&store, &queued).len(), 1);
    // Startup reconciles the counter with the table instead of refusing.
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(audit(&store), events(&store) * EVENT_BYTES);
}

#[tokio::test]
async fn the_writer_thread_runs_the_pass_and_reports_usage() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    {
        let mut store = Store::open(&path).unwrap();
        let actor = scale::actor(&mut store);
        fill(&mut store, &actor.id, BUDGET_ROWS + 10, 200);
        // As an older build left it: the counter claims the cap.
        store
            .conn
            .execute("UPDATE meta SET audit_bytes=? WHERE id=1", [AUDIT_LIMIT])
            .unwrap();
    }
    let db = Database::open(&path).unwrap();
    // The first pass runs before any request, so the table is already back
    // within budget and the reported usage reflects it.
    let usage = db.call(|s| s.audit_usage()).await.unwrap();
    assert_eq!(usage, db.audit_usage());
    assert!(usage.bytes <= EVENT_BUDGET);
    assert_eq!(usage.level(), AuditLevel::Normal);
    let rows = db.call(|s| Ok(events(s))).await.unwrap();
    assert_eq!(rows * EVENT_BYTES, usage.bytes);
}

#[test]
fn audit_levels_warn_at_three_quarters_and_stop_at_the_cap() {
    let level = |bytes| AuditUsage { bytes }.level();
    assert_eq!(level(0), AuditLevel::Normal);
    assert_eq!(level(EVENT_BUDGET), AuditLevel::Normal);
    assert_eq!(level(AUDIT_WARNING - 1), AuditLevel::Normal);
    assert_eq!(level(AUDIT_WARNING), AuditLevel::Warning);
    assert_eq!(level(AUDIT_LIMIT - 1), AuditLevel::Warning);
    assert_eq!(level(AUDIT_LIMIT), AuditLevel::Full);
    assert_eq!(AUDIT_WARNING, 48 * 1024 * 1024);
}

/// A reader's cursor stays meaningful: a pruned job returns its final state
/// and no events for any cursor; a retained job returns its whole history.
#[test]
fn stale_cursors_get_the_job_snapshot_and_never_a_gap() {
    let (_dir, mut store, actor) = setup();
    let snapshot = |byte: &str| limeos_domain::ContainerSnapshot {
        resource: format!("container:{}", byte.repeat(64)),
        image: format!("sha256:{}", "b".repeat(64)),
        started_at: "2026-10-05T07:00:00Z".into(),
        running: true,
    };
    let queue = |store: &mut Store, key: &str, byte: &str| {
        let before = snapshot(byte);
        let plan = store.plan_restart(&actor, &before, 200).unwrap();
        let approval = store
            .approve_container(&actor, &plan.plan.id, &plan.digest, 201)
            .unwrap();
        store
            .queue_container(&actor, key, &plan, &approval.token, &before, 202)
            .unwrap()
    };
    let old = queue(&mut store, "old", "a");
    let open = queue(&mut store, "open", "c");
    store.cancel_container_job(&actor, &old.id, 203).unwrap();
    let history = store.container_events(&actor, &old.id, 0).unwrap();
    assert_eq!(history.len(), 2);
    let cursor = history[0].cursor;
    let pending = store.container_events(&actor, &open.id, 0).unwrap();
    fill(&mut store, &actor.id, BUDGET_ROWS, 204);
    drain(&mut store);
    for after in [0, cursor, cursor + 1_000_000] {
        assert!(
            store
                .container_events(&actor, &old.id, after)
                .unwrap()
                .is_empty()
        );
    }
    assert_eq!(
        store.container_job(&actor, &old.id).unwrap().state,
        JobState::Canceled
    );
    assert_eq!(
        store.container_events(&actor, &open.id, 0).unwrap(),
        pending
    );
    assert_eq!(
        store
            .container_events(&actor, &open.id, pending[0].cursor)
            .unwrap(),
        pending[1..]
    );
}

/// Events in the long-history fixture: a health-probe-like job cycling
/// through `verifying` and `outcome_unknown` before it settles.
const LONG: i64 = 50_000;

/// A queued container job owned by `actor`.
fn container(store: &mut Store, actor: &Principal, key: &str, byte: &str) -> String {
    let before = limeos_domain::ContainerSnapshot {
        resource: format!("container:{}", byte.repeat(64)),
        image: format!("sha256:{}", "b".repeat(64)),
        started_at: "2026-10-05T07:00:00Z".into(),
        running: true,
    };
    let plan = store.plan_restart(actor, &before, 200).unwrap();
    let approval = store
        .approve_container(actor, &plan.plan.id, &plan.digest, 201)
        .unwrap();
    store
        .queue_container(actor, key, &plan, &approval.token, &before, 202)
        .unwrap()
        .id
}

/// A container job with `LONG` events: queued, running, verifying, then
/// `outcome_unknown`/`verifying` cycles, then succeeded. The state changes
/// run through the jobs triggers; the cycles are bulk-inserted and accounted.
fn long_history(store: &mut Store, actor: &Principal, now: i64) -> String {
    let job = container(store, actor, "long", "d");
    let transition = |state| json(&EventKind::JobTransition { state }).unwrap();
    let (unknown, verifying) = (
        transition(JobState::OutcomeUnknown),
        transition(JobState::Verifying),
    );
    let cycles = LONG - 4;
    assert_eq!(cycles % 2, 0);
    store
        .write(|tx| {
            for state in [JobState::Running, JobState::Verifying] {
                tx.execute(
                    "UPDATE jobs SET state=? WHERE id=?",
                    params![state.as_str(), job],
                )
                .map_err(durable)?;
                event(tx, Some(&actor.id), Some(&job), EventKind::JobTransition { state }, now)?;
            }
            let added = tx.execute("WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x<?) INSERT INTO events(principal,job,kind,created) SELECT ?,?,CASE x%2 WHEN 1 THEN ? ELSE ? END,? FROM c", params![cycles, actor.id, job, unknown, verifying, now]).map_err(durable)?;
            account(tx, added as i64)?;
            tx.execute("UPDATE jobs SET state='succeeded' WHERE id=?", [&job])
                .map_err(durable)?;
            event(
                tx,
                Some(&actor.id),
                Some(&job),
                EventKind::JobTransition {
                    state: JobState::Succeeded,
                },
                now,
            )
        })
        .unwrap();
    assert_eq!(count(store, &job), LONG);
    job
}
fn count(store: &Store, job: &str) -> i64 {
    store
        .conn
        .query_row("SELECT count(*) FROM events WHERE job=?", [job], |r| {
            r.get(0)
        })
        .unwrap()
}

#[test]
fn the_retained_history_mark_is_the_job_queued_event() {
    assert!(RETAINED_HISTORY.contains(&format!(
        "q.kind='{}'",
        json(&EventKind::JobQueued).unwrap()
    )));
}

/// A settled job with 50,000 events is removed in transactions of at most
/// one batch, and after every one of them each reader sees the job either
/// with its whole history or with none, for any cursor.
#[test]
fn a_long_history_is_pruned_in_bounded_batches_and_never_read_partially() {
    let (_dir, mut store, actor) = setup();
    let job = long_history(&mut store, &actor, 300);
    let first = store.container_events(&actor, &job, 0).unwrap();
    assert_eq!(first.len(), 128);
    let middle = job_events(&store, &job)[(LONG / 2) as usize].0;
    let later = store.container_events(&actor, &job, middle).unwrap();
    assert_eq!(later.len(), 128);
    fill(&mut store, &actor.id, BUDGET_ROWS, 301);
    let mut passes = 0;
    loop {
        let changes = |store: &Store| -> i64 {
            store
                .conn
                .query_row("SELECT total_changes()", [], |r| r.get(0))
                .unwrap()
        };
        let before = changes(&store);
        let removed = store.prune().unwrap();
        // Each rule is one transaction: its deletes plus the counter update.
        for (table, rows) in &removed {
            assert!(*rows <= EVENT_BATCH, "{table}: {rows} rows in one batch");
        }
        assert!(changes(&store) - before <= removed.len() as i64 * (EVENT_BATCH + 1));
        let left = count(&store, &job);
        if left == LONG {
            assert_eq!(store.container_events(&actor, &job, 0).unwrap(), first);
            assert_eq!(store.container_events(&actor, &job, middle).unwrap(), later);
        } else {
            for after in [0, first[0].cursor, middle, middle + LONG] {
                assert!(
                    store
                        .container_events(&actor, &job, after)
                        .unwrap()
                        .is_empty(),
                    "partial history visible with {left} events left"
                );
            }
        }
        if removed.is_empty() {
            break;
        }
        passes += 1;
    }
    assert_eq!(count(&store, &job), 0);
    assert!(passes >= LONG / EVENT_BATCH);
    assert_eq!(
        store.container_job(&actor, &job).unwrap().state,
        JobState::Succeeded
    );
    assert_eq!(audit(&store), events(&store) * EVENT_BYTES);
}

#[test]
fn prune_between_passes_helper() {
    let Some(path) = std::env::var_os("LIMEOS_PRUNE_BETWEEN_PATH") else {
        return;
    };
    let ready = std::env::var_os("LIMEOS_PRUNE_READY_PATH").unwrap();
    let mut store = Store::open(Path::new(&path)).unwrap();
    let actor = scale::actor(&mut store);
    let job = long_history(&mut store, &actor, 300);
    fill(&mut store, &actor.id, BUDGET_ROWS, 301);
    while count(&store, &job) == LONG {
        store.prune().unwrap();
    }
    assert!(count(&store, &job) > 0);
    // Committed passes only: the parent kills this process here.
    std::fs::write(&ready, job.as_bytes()).unwrap();
    loop {
        std::thread::park();
    }
}

#[test]
fn a_kill_between_passes_leaves_a_pruned_job_and_resumes() {
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
            .args(["--exact", "retention::tests::prune_between_passes_helper"])
            .env("LIMEOS_PRUNE_BETWEEN_PATH", &path)
            .env("LIMEOS_PRUNE_READY_PATH", &ready)
            .spawn()
            .unwrap(),
    );
    for _ in 0..6000 {
        if ready.exists() {
            break;
        }
        assert!(child.0.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(10));
    }
    // The file is written in one call; wait for its contents.
    let mut job = String::new();
    for _ in 0..100 {
        job = std::fs::read_to_string(&ready).unwrap_or_default();
        if !job.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!job.is_empty());
    child.0.kill().unwrap();
    assert_eq!(child.0.wait().unwrap().signal(), Some(9));
    let mut store = Store::open(&path).unwrap();
    resources::check(&store.conn).unwrap();
    let actor = store.login_record("alice").unwrap().unwrap().principal;
    let left = count(&store, &job);
    assert!(0 < left && left < LONG);
    assert!(store.container_events(&actor, &job, 0).unwrap().is_empty());
    assert_eq!(audit(&store), events(&store) * EVENT_BYTES);
    drain(&mut store);
    assert_eq!(count(&store, &job), 0);
    assert_eq!(audit(&store), events(&store) * EVENT_BYTES);
    drop(store);
    Store::open(&path).unwrap();
}

/// Over its budget, the authority class loses its oldest prunable events and
/// nothing else, and never an event of a job whose history is still retained.
#[test]
fn authority_events_are_pruned_only_against_their_own_budget() {
    let (_dir, mut store, actor) = setup();
    let open = container(&mut store, &actor, "open", "e");
    store
        .write(|tx| {
            event(
                tx,
                Some(&actor.id),
                Some(&open),
                EventKind::JobResolutionWithdrawn,
                300,
            )
        })
        .unwrap();
    let enrolment = authority(&store);
    assert_eq!(enrolment.len(), 4);
    let kind = json(&EventKind::GrantsRevised).unwrap();
    store
        .write(|tx| {
            let added = tx.execute("WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x<?) INSERT INTO events(principal,job,kind,created) SELECT ?,NULL,?,301 FROM c", params![AUTHORITY_BUDGET + 6, actor.id, kind]).map_err(durable)?;
            account(tx, added as i64)
        })
        .unwrap();
    let before = authority(&store);
    let bulk = events(&store) - before.len() as i64;
    assert_eq!(store.prune().unwrap(), vec![("authority events", 9)]);
    let after = authority(&store);
    // The open job's withdrawal is pinned and outside the budget.
    assert_eq!(after.len() as i64, AUTHORITY_BUDGET + 1);
    // Oldest first: enrolment's three job-less events and the six oldest
    // revisions; the open job's withdrawal stays with its history.
    let removed: Vec<_> = before.iter().filter(|row| !after.contains(row)).collect();
    assert_eq!(removed.len(), 9);
    assert!(enrolment[..3].iter().all(|row| removed.contains(&row)));
    assert!(after.contains(&enrolment[3]));
    assert_eq!(events(&store) - after.len() as i64, bulk);
    assert_eq!(audit(&store), events(&store) * EVENT_BYTES);
    assert!(store.prune().unwrap().is_empty());
}

/// A job in `needs_intervention` (retained history) with `pairs` resolution
/// requests and withdrawals: pinned authority events.
fn uncertain_with_requests(store: &mut Store, actor: &Principal, pairs: usize) -> String {
    let job = probe(store, actor, "uncertain", "uncertain", 300);
    store
        .transition(&job, JobState::Queued, JobState::Running, 300)
        .unwrap();
    store
        .transition(&job, JobState::Running, JobState::NeedsIntervention, 300)
        .unwrap();
    store
        .write(|tx| {
            for _ in 0..pairs {
                event(
                    tx,
                    Some(&actor.id),
                    Some(&job),
                    EventKind::JobResolutionRequested {
                        declared: limeos_domain::DeclaredOutcome::NotDone,
                    },
                    301,
                )?;
                event(
                    tx,
                    Some(&actor.id),
                    Some(&job),
                    EventKind::JobResolutionWithdrawn,
                    301,
                )?;
            }
            Ok(())
        })
        .unwrap();
    job
}

/// The reviewer's store: 2,048 request/withdrawal pairs on one retained
/// uncertain job fill the budget on their own. Pinned events do not consume
/// it, so the newest authority event (a legacy import) and the enrolment
/// events stay, and the pinned ones stay too.
#[test]
fn pinned_authority_events_do_not_consume_the_budget() {
    let (_dir, mut store, actor) = setup();
    let job = uncertain_with_requests(&mut store, &actor, 2048);
    store
        .import_legacy_user("bob", "pbkdf2:sha256:1000000$IgjABokZwwl2gggv$686315920b24fb807abeb0a436ce6385be3f3026e35d934ffbaf15be1a0e7ce1", 302)
        .unwrap();
    let before = authority(&store);
    // Enrolment's three, the 4,096 pinned, the import.
    assert_eq!(before.len() as i64, 3 + AUTHORITY_BUDGET + 1);
    assert!(
        before
            .last()
            .unwrap()
            .1
            .contains("legacy_identity_imported")
    );
    drain(&mut store);
    assert_eq!(authority(&store), before);
    assert_eq!(count(&store, &job), 3 + AUTHORITY_BUDGET);
}

/// Over the budget, only prunable (unpinned) authority events count: the
/// newest `AUTHORITY_BUDGET` of them stay, older ones go oldest first, and
/// the newest prunable event is never removed while older ones exist.
#[test]
fn the_budget_keeps_the_newest_prunable_authority_events() {
    let (_dir, mut store, actor) = setup();
    let job = uncertain_with_requests(&mut store, &actor, 300);
    let pinned = count(&store, &job);
    let kind = json(&EventKind::GrantsRevised).unwrap();
    let extra = 2 * AUTHORITY_BATCH + 7;
    store
        .write(|tx| {
            let added = tx.execute("WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x<?) INSERT INTO events(principal,job,kind,created) SELECT ?,NULL,?,303 FROM c", params![AUTHORITY_BUDGET + extra - 3, actor.id, kind]).map_err(durable)?;
            account(tx, added as i64)
        })
        .unwrap();
    let prunable = |store: &Store| -> Vec<(i64, String)> {
        let held = job_events(store, &job);
        authority(store)
            .into_iter()
            .filter(|row| !held.contains(row))
            .collect()
    };
    let before = prunable(&store);
    assert_eq!(before.len() as i64, AUTHORITY_BUDGET + extra);
    let newest = before.last().unwrap().clone();
    loop {
        let removed = store.prune().unwrap();
        let now = prunable(&store);
        assert!(now.contains(&newest));
        assert!(now.len() as i64 >= AUTHORITY_BUDGET);
        // Always a suffix: only the oldest ever go.
        assert_eq!(now, before[before.len() - now.len()..]);
        if removed.is_empty() {
            break;
        }
    }
    assert_eq!(prunable(&store).len() as i64, AUTHORITY_BUDGET);
    assert_eq!(count(&store, &job), pinned);
    assert_eq!(audit(&store), events(&store) * EVENT_BYTES);
}

/// The plans of both candidate queries: job-less rows through `events_job`,
/// settled jobs through `jobs_state` and then their events through
/// `events_job`. No step scans the events table.
#[test]
fn candidate_queries_use_the_indexes() {
    let (_dir, store, _) = setup();
    let plan = |sql: String| -> Vec<String> {
        store
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap()
            .query_map([EVENT_BATCH], |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap()
    };
    let loose = plan(loose_candidates());
    assert!(
        loose
            .iter()
            .any(|step| step.starts_with("SEARCH e USING") && step.contains("INDEX events_job")),
        "{loose:?}"
    );
    let settled = plan(settled_candidates());
    assert!(
        settled
            .iter()
            .any(|step| step.starts_with("SEARCH j USING") && step.contains("INDEX jobs_state")),
        "{settled:?}"
    );
    assert!(
        settled
            .iter()
            .any(|step| step.starts_with("SEARCH e USING") && step.contains("INDEX events_job")),
        "{settled:?}"
    );
    for step in loose.iter().chain(&settled) {
        assert!(!step.starts_with("SCAN e"), "{step}");
    }
}

/// Pinned history at the start of the table (here 50,000 events of a job
/// that is still queued) is not visited again by every pass: the candidate
/// queries cost about what they return.
#[test]
fn a_pass_does_not_rescan_pinned_history() {
    let (_dir, mut store, actor) = setup();
    let open = probe(&mut store, &actor, "open", "open", 300);
    let kind = json(&EventKind::JobTransition {
        state: JobState::Queued,
    })
    .unwrap();
    store
        .write(|tx| {
            let added = tx.execute("WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x<50000) INSERT INTO events(principal,job,kind,created) SELECT ?,?,?,301 FROM c", params![actor.id, open, kind]).map_err(durable)?;
            account(tx, added as i64)
        })
        .unwrap();
    let settled = probe(&mut store, &actor, "settled", "settled", 302);
    store
        .transition(&settled, JobState::Queued, JobState::Canceled, 302)
        .unwrap();
    fill(&mut store, &actor.id, 16, 303);
    for (sql, expected) in [(loose_candidates(), 16), (settled_candidates(), 2)] {
        let mut stmt = store.conn.prepare(&sql).unwrap();
        let rows = stmt
            .query_map([EVENT_BATCH], |r| r.get::<_, i64>(0))
            .unwrap()
            .count();
        // Enrolment's job-less events are authority events, not bulk.
        assert_eq!(rows, expected);
        let steps = stmt.get_status(rusqlite::StatementStatus::VmStep);
        assert!(steps < 2_000, "{steps} steps for {rows} rows: {sql}");
    }
    let removed = store.prune().unwrap();
    assert!(removed.is_empty(), "under budget: {removed:?}");
    assert_eq!(count(&store, &open), 50_001);
}

#[test]
fn prune_crash_helper() {
    let Some(path) = std::env::var_os("LIMEOS_PRUNE_CRASH_PATH") else {
        return;
    };
    let mut store = Store::open(Path::new(&path)).unwrap();
    let actor = scale::actor(&mut store);
    let job = probe(&mut store, &actor, "settled", "settled", 200);
    store
        .transition(&job, JobState::Queued, JobState::Canceled, 201)
        .unwrap();
    fill(&mut store, &actor.id, BUDGET_ROWS + EVENT_BATCH, 202);
    let held = probe(&mut store, &actor, "held", "held", 203);
    store
        .transition(&held, JobState::Queued, JobState::Running, 203)
        .unwrap();
    // The parent set LIMEOS_PRUNE_PAUSE_PATH for this process only: the batch
    // stops after its first delete, before commit, until it is killed.
    store.prune().unwrap();
    unreachable!("the batch pauses until killed");
}

#[test]
fn a_kill_mid_batch_leaves_a_store_that_passes_the_startup_check() {
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
            .args(["--exact", "retention::tests::prune_crash_helper"])
            .env("LIMEOS_PRUNE_CRASH_PATH", &path)
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
    resources::check(&store.conn).unwrap();
    // Nothing of the interrupted batch committed, and the counter agrees.
    // Enrolment (3), the settled job (2), the history, the held job (2) and
    // its recovery event.
    assert_eq!(events(&store), 3 + 2 + BUDGET_ROWS + EVENT_BATCH + 2 + 1);
    assert_eq!(audit(&store), events(&store) * EVENT_BYTES);
    let removed = store.prune().unwrap();
    assert_eq!(removed.len(), 1);
    assert_eq!(audit(&store), events(&store) * EVENT_BYTES);
    drop(store);
    Store::open(&path).unwrap();
}

#[test]
fn occurrence_pruning_keeps_queued_work_the_watermark_and_each_newest() {
    let (dir, mut store, actor) = setup();
    let base = 1_000_020;
    let observed = scale::schedule(
        &mut store,
        &actor,
        ScheduledOperation::ObserveHealth,
        30,
        base,
    );
    let queued = scale::reserve(&mut store, &observed.id, base);
    let backup = scale::schedule(&mut store, &actor, ScheduledOperation::Backup, 30, base);
    let ticks = OCCURRENCE_BUDGET + OCCURRENCE_BATCH;
    let mut row = backup.clone();
    for i in 0..ticks {
        let now = base + 30 * i;
        let DueDecision::Ready(key) = row.due(now).unwrap() else {
            panic!("backup due")
        };
        assert_eq!(
            store
                .automation_reserve(&backup, &Settlement::AdapterUnavailable(key), now)
                .unwrap(),
            Reservation::Blocked
        );
        row.settled_through = Some(now);
    }
    let count = |store: &Store| -> i64 {
        store
            .conn
            .query_row("SELECT count(*) FROM automation_occurrences", [], |r| {
                r.get(0)
            })
            .unwrap()
    };
    assert_eq!(count(&store), ticks + 1);
    drain(&mut store);
    assert_eq!(count(&store), OCCURRENCE_BUDGET);
    let survives = |store: &Store, schedule: &str, due: i64| -> bool {
        store
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM automation_occurrences WHERE schedule=? AND due=?)",
                params![schedule, due],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert!(survives(&store, &observed.id, queued.occurrence.due));
    assert!(survives(&store, &backup.id, base + 30 * (ticks - 1)));
    assert!(!survives(&store, &backup.id, base));
    // A pruned key still reads as already settled.
    let DueDecision::Ready(first) = backup.due(base).unwrap() else {
        panic!("first due")
    };
    assert_eq!(
        store
            .automation_reserve(
                &backup,
                &Settlement::AdapterUnavailable(first),
                base + 30 * ticks
            )
            .unwrap(),
        Reservation::Existing
    );
    drop(store);
    Store::open(&dir.path().join("core.sqlite")).unwrap();
}

/// A rule whose batch fails is reported, and the other rules still run in
/// the same pass; the writer thread keeps the latest failure for the log.
#[test]
fn a_failing_rule_is_reported_and_the_others_still_run() {
    let (_dir, mut store, actor) = setup();
    let base = 1_000_020;
    let backup = scale::schedule(&mut store, &actor, ScheduledOperation::Backup, 30, base);
    let mut row = backup.clone();
    for i in 0..OCCURRENCE_BUDGET + OCCURRENCE_BATCH {
        let now = base + 30 * i;
        let DueDecision::Ready(key) = row.due(now).unwrap() else {
            panic!("backup due")
        };
        store
            .automation_reserve(&backup, &Settlement::AdapterUnavailable(key), now)
            .unwrap();
        row.settled_through = Some(now);
    }
    fill(&mut store, &actor.id, BUDGET_ROWS + EVENT_BATCH, base);
    let occurrences = |store: &Store| -> i64 {
        store
            .conn
            .query_row("SELECT count(*) FROM automation_occurrences", [], |r| {
                r.get(0)
            })
            .unwrap()
    };
    let (rows, before) = (events(&store), occurrences(&store));
    store.failing_rule = Some("events");
    let pass = store.maintain(Instant::now(), 0).unwrap();
    assert_eq!(pass.failed, vec![("events", ErrorCode::StateNotDurable)]);
    assert_eq!(pass.usage, Some(store.audit_usage().unwrap()));
    assert_eq!(events(&store), rows);
    assert_eq!(occurrences(&store), before - OCCURRENCE_BATCH);
    // `prune` runs every rule too, then reports the failure.
    assert!(store.prune().is_err());
    assert_eq!(occurrences(&store), OCCURRENCE_BUDGET);
    store.failing_rule = None;
    assert_eq!(store.prune().unwrap(), vec![("events", EVENT_BATCH)]);
}

#[tokio::test]
async fn the_writer_keeps_the_latest_retention_failure() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("core.sqlite")).unwrap();
    assert_eq!(db.retention_failure(), None);
    for expected in 1..=2 {
        db.call(|s| {
            s.failing_rule = Some("automation_occurrences");
            s.retained_at = None;
            Ok(())
        })
        .await
        .unwrap();
        // The pass runs after that call, before this one.
        db.call(|_| Ok(())).await.unwrap();
        assert_eq!(
            db.retention_failure(),
            Some(RetentionFailure {
                rule: "automation_occurrences",
                code: ErrorCode::StateNotDurable,
                count: expected,
            })
        );
    }
}

/// The minimum interval for 30 simulated days/// The minimum interval for 30 simulated days: a job-less schedule settles
/// every 30 s and an observation schedule runs hourly, with the pass every
/// simulated minute. Without pruning the 4097th occurrence (day 1.4) would be
/// refused as overloaded.
#[test]
fn thirty_days_at_the_minimum_interval() {
    let (dir, mut store, actor) = setup();
    let base = 1_000_020;
    let backup = scale::schedule(&mut store, &actor, ScheduledOperation::Backup, 30, base);
    let observed = scale::schedule(
        &mut store,
        &actor,
        ScheduledOperation::ObserveHealth,
        3600,
        base,
    );
    let mut row = backup.clone();
    let days = 30;
    let ticks = days * 86400 / 30;
    let mut peak = 0;
    for i in 0..ticks {
        let now = base + 30 * i;
        let DueDecision::Ready(key) = row.due(now).unwrap() else {
            panic!("backup due")
        };
        assert_eq!(
            store
                .automation_reserve(&backup, &Settlement::AdapterUnavailable(key), now)
                .unwrap(),
            Reservation::Blocked
        );
        row.settled_through = Some(now);
        if (now - base) % 3600 == 0 {
            scale::observe(&mut store, &observed.id, now);
        }
        if i % 2 == 1 {
            store.prune().unwrap();
            let count: i64 = store
                .conn
                .query_row("SELECT count(*) FROM automation_occurrences", [], |r| {
                    r.get(0)
                })
                .unwrap();
            peak = peak.max(count);
        }
    }
    assert!(peak <= OCCURRENCE_BUDGET, "peak {peak}");
    let views = store.automation_schedules(&actor, None).unwrap().items;
    for view in views {
        let last = view.last_occurrence.unwrap();
        let expected = if view.schedule.id == backup.id {
            base + 30 * (ticks - 1)
        } else {
            base + 3600 * (days * 24 - 1)
        };
        assert_eq!(last.key.due, expected);
        assert_eq!(view.schedule.settled_through, Some(expected));
    }
    drop(store);
    let store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    assert_eq!(audit(&store), events(&store) * EVENT_BYTES);
}

/// One transaction on the writer's connection with its cache at `cache_kib`,
/// from an empty WAL with automatic checkpoints off: the frames it commits
/// and the WAL bytes on disk just before its commit. SQLite writes a
/// transaction's pages to the WAL at commit, unless its cache cannot hold
/// them: then it spills dirty pages into the WAL early, and a page changed
/// again after its spill is written again. Any WAL bytes before the commit
/// are such a spill.
fn measured<T>(
    store: &mut Store,
    cache_kib: u32,
    work: impl FnOnce(&Transaction<'_>) -> T,
) -> (u64, u64, T) {
    let wal = store.directory.join("core.sqlite-wal");
    store
        .conn
        .execute_batch(&format!(
            "PRAGMA wal_checkpoint(TRUNCATE); PRAGMA wal_autocheckpoint=0; PRAGMA cache_size=-{cache_kib}; PRAGMA shrink_memory;"
        ))
        .unwrap();
    let page: i64 = store
        .conn
        .pragma_query_value(None, "page_size", |r| r.get(0))
        .unwrap();
    let page = page as u64;
    assert_eq!(page, 4096);
    assert_eq!(std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0), 0);
    let tx = store
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let value = work(&tx);
    let spilled = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
    tx.commit().unwrap();
    let total = std::fs::metadata(&wal).unwrap().len();
    ((total.saturating_sub(32)) / (page + 24), spilled, value)
}

/// `rows` events, one job-less session event (prunable) then `pinned` events
/// of the open job `job` (kept), repeating, through the audit accounting but
/// not the write gate, so it can fill past the cap.
fn interleaved(store: &mut Store, actor: &Principal, job: &str, rows: i64, pinned: i64, now: i64) {
    let loose = json(&EventKind::SessionCreated).unwrap();
    let held = json(&EventKind::JobTransition {
        state: JobState::Running,
    })
    .unwrap();
    let mut written = 0;
    while written < rows {
        let chunk = (rows - written).min(8192);
        let tx = store.conn.transaction().unwrap();
        let added = tx.execute(
            "WITH RECURSIVE c(x) AS (SELECT ? UNION ALL SELECT x+1 FROM c WHERE x<?) \
             INSERT INTO events(principal,job,kind,created) \
             SELECT ?, CASE WHEN x%?=0 THEN NULL ELSE ? END, CASE WHEN x%?=0 THEN ? ELSE ? END, ? FROM c",
            params![written, written + chunk - 1, actor.id, pinned + 1, job, pinned + 1, loose, held, now],
        )
        .unwrap();
        account(&tx, added as i64).unwrap();
        tx.commit().unwrap();
        written += chunk;
    }
}

/// A store at its event budget plus one batch, prunable events one in
/// `pinned + 1` among an open job's history, then reopened by path.
fn sparse_store(pinned: i64) -> (tempfile::TempDir, Store) {
    let (dir, mut store, actor) = setup();
    let base = 1_000_000;
    let held = probe(&mut store, &actor, "held", "held", base);
    store
        .transition(&held, JobState::Queued, JobState::Running, base)
        .unwrap();
    interleaved(
        &mut store,
        &actor,
        &held,
        BUDGET_ROWS + EVENT_BATCH,
        pinned,
        base,
    );
    (dir, store)
}

/// A full event batch: pages committed and bytes spilled at `cache_kib`.
fn event_batch(store: &mut Store, cache_kib: u32) -> (u64, u64, i64) {
    let (pages, spilled, removed) = measured(store, cache_kib, |tx| {
        prune_events(tx, EVENT_BATCH, 0, &mut jobs::Scan::default())
            .unwrap()
            .rows
    });
    (pages, spilled, removed)
}

/// `jobs::FLOOR + jobs_past_floor` settled health-probe jobs (four events
/// each), each with `extra_events` more interleaved with the others' and
/// with prunable session events, so a job-row batch deletes from scattered
/// pages. Returns the time past their horizon.
fn job_store(jobs_past_floor: i64, extra_events: usize) -> (tempfile::TempDir, Store, i64) {
    let (dir, mut store, actor) = setup();
    let base = 1_000_000;
    let mut ids = Vec::new();
    for i in 0..(jobs::FLOOR + jobs_past_floor) {
        let job = probe(
            &mut store,
            &actor,
            &format!("job{i}"),
            &format!("r{i}"),
            base,
        );
        for (from, to) in [
            (JobState::Queued, JobState::Running),
            (JobState::Running, JobState::Verifying),
            (JobState::Verifying, JobState::Succeeded),
        ] {
            store.transition(&job, from, to, base).unwrap();
        }
        ids.push(job);
    }
    let transition = json(&EventKind::JobTransition {
        state: JobState::Verifying,
    })
    .unwrap();
    let loose = json(&EventKind::SessionCreated).unwrap();
    let tx = store.conn.transaction().unwrap();
    let mut added = 0;
    for _ in 0..extra_events {
        for job in &ids {
            tx.execute(
                "INSERT INTO events(principal,job,kind,created) VALUES(?,?,?,?)",
                params![actor.id, job, transition, base],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO events(principal,job,kind,created) VALUES(?,NULL,?,?)",
                params![actor.id, loose, base],
            )
            .unwrap();
            added += 2;
        }
    }
    account(&tx, added).unwrap();
    tx.commit().unwrap();
    (dir, store, base + jobs::HORIZON + 86_400)
}

/// A store just under the audit cap plus the revocation reserve, prunable
/// events one in two among the history of a job held for an operator: an
/// open that finds three interrupted jobs must make room for their recovery
/// events inside its own transaction (`make_room`, given the count).
fn capped_store() -> (tempfile::TempDir, Store) {
    let (dir, mut store, actor) = setup();
    let base = 1_000_000;
    let held = probe(&mut store, &actor, "held", "held", base);
    store
        .transition(&held, JobState::Queued, JobState::Running, base)
        .unwrap();
    store
        .transition(&held, JobState::Running, JobState::NeedsIntervention, base)
        .unwrap();
    let rows = (AUDIT_LIMIT + AUDIT_RESERVE) / EVENT_BYTES - events(&store) - 2;
    interleaved(&mut store, &actor, &held, rows, 1, base);
    (dir, store)
}

/// `store`'s file, checkpointed, copied to a new directory and opened there,
/// so each cache size starts from the same bytes.
fn copied(store: Store, dir: &tempfile::TempDir) -> impl Fn() -> (tempfile::TempDir, Store) {
    store
        .conn
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    drop(store);
    let path = dir.path().join("core.sqlite");
    move || {
        let copy = tempfile::tempdir().unwrap();
        std::fs::copy(&path, copy.path().join("core.sqlite")).unwrap();
        let store = Store::open(&copy.path().join("core.sqlite")).unwrap();
        (copy, store)
    }
}

#[test]
#[ignore = "measurement: run with --ignored --nocapture"]
fn measure_writer_transactions_at_each_cache_size() {
    let sizes = [512u32, 1024, 1536, 2000];
    {
        let (dir, store, now) = job_store(jobs::BATCH, 30);
        let open = copied(store, &dir);
        for kib in sizes {
            let (_copy, mut store) = open();
            let (pages, spilled, removed) = measured(&mut store, kib, |tx| {
                jobs::prune(tx, jobs::BATCH, now, &mut jobs::Scan::default()).unwrap()
            });
            eprintln!(
                "job batch: {kib} KiB cache: {} jobs, {} rows removed, {pages} pages, {spilled} bytes spilled",
                removed.jobs.len(),
                removed.rows
            );
        }
    }
    {
        let (dir, store) = capped_store();
        let open = copied(store, &dir);
        for kib in sizes {
            let (_copy, mut store) = open();
            let (pages, spilled, ()) = measured(&mut store, kib, |tx| {
                make_room(tx, 3, 2_000_000).unwrap();
            });
            eprintln!(
                "make_room at the cap, 1 prunable in 2: {kib} KiB cache: {pages} pages, {spilled} bytes spilled"
            );
        }
    }
    let sizes = [256u32, 512, 768, 1024, 1536, 2000];
    for pinned in [0i64, 1, 3, 7] {
        let (dir, store) = sparse_store(pinned);
        let path = dir.path().join("core.sqlite");
        store
            .conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        drop(store);
        for kib in sizes {
            let copy = tempfile::tempdir().unwrap();
            std::fs::copy(&path, copy.path().join("core.sqlite")).unwrap();
            let mut store = Store::open(&copy.path().join("core.sqlite")).unwrap();
            let (pages, spilled, removed) = event_batch(&mut store, kib);
            eprintln!(
                "event batch, 1 prunable in {}: {kib} KiB cache: {removed} removed, {pages} pages, {spilled} bytes spilled",
                pinned + 1
            );
        }
    }
}

/// The writer's cache holds the largest transactions retention runs in normal
/// operation without spilling: a full event batch, dense and with one
/// prunable event in four among pinned history; a day's job-row batch (ten
/// jobs past the floor, six events each, interleaved); and startup's
/// `make_room` at the cap. A backlog job-row batch (64 jobs, up to 2,048 rows
/// scattered) does spill, at this size and at SQLite's default alike; see
/// `cache` and `measure_writer_transactions_at_each_cache_size`.
#[test]
fn the_writers_normal_transactions_do_not_spill() {
    let kib = crate::cache::WRITER_KIB;
    for pinned in [0, 3] {
        let (_dir, mut store) = sparse_store(pinned);
        let (pages, spilled, removed) = event_batch(&mut store, kib);
        assert_eq!(removed, EVENT_BATCH);
        assert_eq!(
            spilled,
            0,
            "event batch, 1 prunable in {}: {pages} pages spilled {spilled} bytes",
            pinned + 1
        );
        eprintln!("event batch, 1 prunable in {}: {pages} pages", pinned + 1);
    }
    let (_dir, mut store, now) = job_store(10, 2);
    let (pages, spilled, removed) = measured(&mut store, kib, |tx| {
        jobs::prune(tx, jobs::BATCH, now, &mut jobs::Scan::default()).unwrap()
    });
    assert_eq!(removed.jobs.len(), 10);
    assert_eq!(
        spilled, 0,
        "a day's job batch: {pages} pages spilled {spilled} bytes"
    );
    eprintln!("a day's job batch: {pages} pages");
    let (_dir, mut store) = capped_store();
    let (pages, spilled, ()) = measured(&mut store, kib, |tx| {
        make_room(tx, 3, 2_000_000).unwrap();
    });
    assert!(pages > 0);
    assert_eq!(
        spilled, 0,
        "make_room at the cap: {pages} pages spilled {spilled} bytes"
    );
    eprintln!("make_room at the cap: {pages} pages");
}

#[test]
fn every_store_connection_has_its_cache_size() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let cache = |conn: &Connection| -> i64 {
        conn.pragma_query_value(None, "cache_size", |r| r.get(0))
            .unwrap()
    };
    // Created (a migration from nothing), then reopened (no migration).
    let store = Store::open(&path).unwrap();
    assert_eq!(cache(&store.conn), -(crate::cache::WRITER_KIB as i64));
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(cache(&store.conn), -(crate::cache::WRITER_KIB as i64));
    let reader = crate::open_reader(&path).unwrap();
    assert_eq!(cache(&reader), -(crate::cache::READER_KIB as i64));
    drop((store, reader));
    // Every SQLite connection this crate opens outside its tests is one of
    // these, each named with its cache; a new one fails here until it has one.
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sites = Vec::new();
    let mut stack = vec![src.clone()];
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path
                .strip_prefix(&src)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if name.ends_with("tests.rs") || name == "scale.rs" || name.contains("tests/") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            // Production code ends where the file's test module starts.
            let text = text.split("#[cfg(test)]\nmod tests").next().unwrap();
            for _ in text.matches("Connection::open") {
                sites.push(name.clone());
            }
        }
    }
    sites.sort();
    assert_eq!(
        sites,
        [
            // `connect`, three spellings of one open (a test VFS or not), used
            // for the writer, which `open_migrating_at` sizes
            // (`cache::WRITER_KIB`) once any upgrade is over, and for the
            // upgrade's own exclusive connection, which keeps SQLite's default
            // for the upgrade (a migration rewrites whole tables) and is
            // closed when it ends; the reader (`cache::READER_KIB`); and
            // `upgrade_space`'s read-only look at a store before it is opened,
            // closed before it returns.
            "lib.rs", "lib.rs", "lib.rs", "lib.rs", "lib.rs",
            // `migrations`' in-memory reference schema.
            "migrations.rs",
            // The snapshot's verification of its copy, closed straight away.
            "migrations/snapshot.rs",
        ],
        "a new SQLite connection: give it a cache size (see `cache`) or name it here"
    );
}
