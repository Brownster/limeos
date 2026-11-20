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
    for kind in ["session_created", "task_revoked", "plan_created"] {
        assert!(loose(kind));
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
    store
        .transition(&settled, JobState::Queued, JobState::Canceled, base)
        .unwrap();
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
    assert!(job_events(&store, &settled).is_empty());
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
    assert!(store.maintain(now).is_some());
    assert!(store.maintain(now + Duration::from_secs(30)).is_none());
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
    assert!(store.maintain(now + INTERVAL).is_some());
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

/// The minimum interval for 30 simulated days: a job-less schedule settles
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
