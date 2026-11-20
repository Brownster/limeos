//! PR-41: the long-uptime soak. One simulated clock drives the real core
//! store (`limeos_persistence::Store`) and the real container receipt store
//! (`limeos_executor_container::ReceiptStore`) through months of a household
//! NAS's jobs, logins, polls, faults and restarts, and checks that no counter,
//! table or cap turns that use into a refusal needing hand repair; separate
//! tests check what it cannot. See `docs/soak.md` for what
//! it covers and how to read a failure.
//!
//! - `soak_smoke`: three simulated days, in every `cargo test`.
//! - `soak_ninety_days`: the acceptance run, core's store open from day 30.
//! - `soak_ninety_days_restarts`: alongside it, restarts for all 90 days.
//! - `writer_thread_runs_retention_unprompted`: the production schedule the
//!   harness otherwise replaces with its own calls.
//! - `automation_limit_at_minimum_interval`: the known automation limit,
//!   measured (`--ignored`; not a pass/fail gate of the shipped scope).
//!
//! The ignored tests named `soak_*` are the CI soak step.
mod automation;
mod checks;
mod dispatch;
mod engine;
mod executor;
mod fixtures;
mod sim;
mod startup;
mod support;
mod workload;
mod writer;

use workload::Profile;

const SMOKE: Profile = Profile {
    name: "smoke",
    seed: 0x5eed_0001,
    days: 3,
    session_burst: (1, 6, 10),
    action_burst: (1, 9, 30),
    idle_every: 1,
    trends: false,
    core_restarts_until: Some(2),
};

/// The acceptance run: core restarts (nightly and at awkward moments) in
/// the first month only, then 60 days on one open store, so a counter that
/// drifts is not recomputed away by `Store::open` every night.
const NINETY_DAYS: Profile = Profile {
    name: "ninety-days",
    seed: 0x5eed_0090,
    days: 90,
    session_burst: (20, 6, 600),
    action_burst: (25, 8, 720),
    idle_every: 7,
    trends: true,
    core_restarts_until: Some(30),
};

/// Alongside it, the recovery coverage: the same household with another
/// seed, both daemons restarting every night and at every awkward moment
/// for all 90 days, against full tables from day 25.
const NINETY_DAYS_RESTARTS: Profile = Profile {
    name: "ninety-days-restarts",
    seed: 0x5eed_0091,
    core_restarts_until: None,
    ..NINETY_DAYS
};

async fn soak(profile: Profile) {
    let mut sim = workload::run(profile).await;
    checks::finish(&mut sim, &profile);
}

#[tokio::test(flavor = "current_thread")]
async fn soak_smoke() {
    soak(SMOKE).await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "the 90-day acceptance run: the soak CI step (cargo test -p limeos-core --test soak -- --ignored soak_)"]
async fn soak_ninety_days() {
    soak(NINETY_DAYS).await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "the 90-day recovery run, alongside soak_ninety_days in its CI step"]
async fn soak_ninety_days_restarts() {
    soak(NINETY_DAYS_RESTARTS).await;
}
