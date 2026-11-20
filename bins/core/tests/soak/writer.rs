//! The production retention schedule. Everywhere else the harness calls the
//! writer's `maintain` itself, on the simulated clock, so a writer thread that
//! never ran it would go unnoticed. Here a real `Database` (its writer thread,
//! queues and all) opens a store with events over budget, and the first call
//! it serves must find one batch already removed: the writer runs a pass as it
//! starts, before any work and without being asked.
//!
//! What this cannot show: the once-a-minute pass of an idle writer
//! (`Bell::wait(retention::INTERVAL)`) waits a real minute, and the interval
//! is a constant. Proving it without a sleep needs an injectable retention
//! interval (or clock) for the writer thread, a seam for a later round.
use crate::{
    fixtures::{SESSION, add_events, core_store, value},
    sim::{rule_limits, soak_dir},
    support::START,
};
use limeos_persistence::{Database, test_support::EVENT_BYTES};

#[tokio::test(flavor = "current_thread")]
async fn writer_thread_runs_retention_unprompted() {
    let dir = soak_dir();
    let (path, admin) = core_store(dir.path());
    let (budget, batch) = rule_limits("events");
    add_events(&path, &admin.id, None, SESSION, budget + 3 * batch, START);
    let before = value(&path, "SELECT count(*) FROM events");
    let db = Database::open(&path).expect("the store opens");
    // Served after the writer's first pass, which runs before its loop.
    let served = db
        .call(|store| store.audit_usage())
        .await
        .expect("the writer serves a read");
    let after = value(&path, "SELECT count(*) FROM events");
    assert_eq!(
        after,
        before - batch,
        "the writer thread ran no retention pass on its own: {before} events before it started, \
         {after} when it served its first call; one batch of {batch} was due (budget {budget})"
    );
    assert_eq!(
        (served.bytes, db.audit_usage().bytes),
        (after * EVENT_BYTES, after * EVENT_BYTES),
        "the writer's pass did not report the audit usage it left"
    );
}
