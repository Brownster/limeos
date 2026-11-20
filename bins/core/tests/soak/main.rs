//! PR-41: the long-uptime soak. One simulated clock drives the real core
//! store (`limeos_persistence::Store`) and the real container receipt store
//! (`limeos_executor_container::ReceiptStore`) through months of a household
//! NAS's jobs, logins, polls, faults and restarts, and checks that no counter,
//! table or cap turns that use into a refusal needing hand repair. See
//! `docs/soak.md` for what it covers and how to read a failure.
//!
//! - `soak_smoke`: three simulated days, in every `cargo test`.
//! - `soak_ninety_days`: the acceptance run, `--ignored`, its own CI step.
//! - `automation_limit_at_minimum_interval`: the known automation limit,
//!   measured (`--ignored`; not a pass/fail gate of the shipped scope).
//! - `writer_thread_runs_retention_unprompted`: the production schedule the
//!   harness otherwise replaces with its own calls.
mod automation;
mod checks;
mod dispatch;
mod engine;
mod executor;
mod fixtures;
mod sim;
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
};

const NINETY_DAYS: Profile = Profile {
    name: "ninety-days",
    seed: 0x5eed_0090,
    days: 90,
    session_burst: (20, 6, 600),
    action_burst: (25, 8, 720),
    idle_every: 7,
    trends: true,
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
#[ignore = "the 90-day acceptance run: its own CI step (cargo test -p limeos-core --test soak -- --ignored soak_ninety_days)"]
async fn soak_ninety_days() {
    soak(NINETY_DAYS).await;
}
