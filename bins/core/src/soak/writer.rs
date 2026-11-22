//! The production retention schedule, on its own: a real `Database` (its
//! writer thread, queues and all) on the simulated clock, and nothing but
//! the clock moving. The writer thread decides when to prune: a pass as it
//! starts, before its first call, and one whenever a minute has passed since
//! the last (`Store::maintain`'s gate), which on simulated time comes when
//! the clock advances, as on the system clock it comes from the writer's own
//! idle timeout. The 90-day runs rely on the same schedule; these two tests
//! name each half of it.
use super::{
    fixtures::{SESSION, add_events, core_store, value},
    sim::{rule_limits, soak_dir},
    support::{Clock, MINUTE, START},
};
use limeos_persistence::test_support::{self, EVENT_BYTES};

/// The writer's startup pass: the first call it serves finds one batch
/// already removed. The writer is held before its first pass until that call
/// is queued, so the order is the writer's own and not a race.
#[tokio::test(flavor = "current_thread")]
async fn writer_thread_runs_retention_unprompted() {
    let dir = soak_dir();
    let (path, admin) = core_store(dir.path());
    let (budget, batch) = rule_limits("events");
    add_events(&path, &admin.id, None, SESSION, budget + 3 * batch, START);
    let before = value(&path, "SELECT count(*) FROM events");
    let clock = Clock::new(START);
    clock.sim().hold();
    let db = test_support::open(&path, clock.sim()).expect("the store opens");
    let first = {
        let db = db.clone();
        tokio::spawn(async move {
            db.call(|store| {
                let events: i64 = store.audit_usage().map(|usage| usage.bytes / EVENT_BYTES)?;
                Ok(events)
            })
            .await
        })
    };
    // Queued (the call's first poll sends it), then the writer goes on.
    tokio::task::yield_now().await;
    clock.sim().release();
    let served = first
        .await
        .unwrap()
        .expect("the writer serves its first call");
    assert_eq!(
        served,
        before - batch,
        "the writer thread ran no retention pass on its own before its first call: {before} events before it started, \
         {served} when it served that call; one batch of {batch} was due (budget {budget})"
    );
    assert_eq!(
        db.audit_usage().bytes,
        served * EVENT_BYTES,
        "the writer's pass did not report the audit usage it left"
    );
}

/// The writer's periodic passes: a backlog of twelve batches is worked off
/// by the writer thread itself, one batch each time the clock passes a
/// minute since its last pass, and none sooner, while ordinary writes go on.
#[tokio::test(flavor = "current_thread")]
async fn writer_thread_works_off_a_backlog_on_its_own() {
    let dir = soak_dir();
    let (path, admin) = core_store(dir.path());
    let (budget, batch) = rule_limits("events");
    let backlog = 12 * batch;
    add_events(&path, &admin.id, None, SESSION, budget + backlog, START);
    let events = || value(&path, "SELECT count(*) FROM events");
    let before = events() - budget;
    let clock = Clock::new(START);
    let db = test_support::open(&path, clock.sim()).expect("the store opens");
    clock.sim().settle();
    let mut excess = events() - budget;
    assert_eq!(excess, before - batch, "the startup pass removed one batch");
    let mut passes = 0;
    let mut minute = 0;
    while excess > 0 {
        minute += 1;
        // Half a minute: the gate keeps the writer from another pass.
        clock.set(START + minute * MINUTE - MINUTE / 2);
        assert_eq!(
            events() - budget,
            excess,
            "the writer ran a retention pass less than a minute after the last"
        );
        // A full minute since its last pass: the writer's own next pass.
        clock.set(START + minute * MINUTE);
        let now = events() - budget;
        assert_eq!(
            excess - now,
            excess.min(batch),
            "after {minute} simulated minutes the writer thread removed {} events with {excess} over budget: \
             it runs one pass of at most {batch} each minute on its own",
            excess - now
        );
        passes += 1;
        excess = now;
        if excess > 0 {
            // An ordinary write between passes (its event joins the backlog).
            let token = limeos_identity::digest(&format!("between-{passes}"));
            let at = clock.now();
            db.call(move |s| {
                let record = s.login_record("marc")?.unwrap();
                s.create_session(&record, None, &token, &token, at)
                    .map(drop)
            })
            .await
            .expect("a login while the backlog is worked off");
            excess += 1;
        }
        assert!(
            passes <= backlog / batch + 2,
            "the backlog is not shrinking"
        );
    }
    let passes_seen = clock.sim().take_passes();
    assert!(
        passes_seen
            .iter()
            .flat_map(|pass| &pass.removed)
            .all(|(_, rows)| *rows <= batch),
        "a pass removed more than one batch per rule"
    );
    let rows = events();
    assert_eq!(
        db.audit_usage().bytes,
        rows * EVENT_BYTES,
        "the counter drifted from the table while the writer worked off the backlog"
    );
    drop(db);
    clock.sim().wait_closed();
}
