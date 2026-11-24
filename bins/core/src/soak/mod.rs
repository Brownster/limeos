//! PR-41: the long-uptime soak. One simulated clock drives core's own code
//! (the real `Core`: its dispatcher pass, recovery, resolution retries and
//! request handlers, its store's writer thread and retention schedule)
//! against the real core store (`limeos_persistence`) and, over a real Unix
//! socket, the container executor's own request handling around the real
//! receipt store (`limeos_executor::container_requests`,
//! `limeos_executor_container::ReceiptStore`), through months of a household
//! NAS's jobs, logins, polls, faults and restarts, and checks that no
//! counter, table or cap turns that use into a refusal needing hand repair;
//! separate tests build the states that use never reaches. It is a unit-test
//! module of the core binary so that it can call core's code. See
//! `docs/soak.md` for what it covers and how to read a failure.
//!
//! - `soak_smoke`: three simulated days, in every `cargo test`.
//! - `soak_ninety_days`: the acceptance run, core's store open from day 30.
//! - `soak_ninety_days_restarts`: alongside it, restarts for all 90 days.
//! - `caps::*`: both stores at their caps and with a retention backlog.
//! - `occurrences::*`, `metrics::*`: the occurrence pruner and
//!   `metrics.sqlite`, which the household run does not exercise.
//! - `writer::*`: the writer thread's own retention schedule, at startup and
//!   once a simulated minute, with no help from the harness.
//! - `automation_limit_at_minimum_interval`: the known automation limit,
//!   measured (`--ignored`; not a pass/fail gate of the shipped scope).
//! - `caps::soak_core_startup_at_high_retained_job_count`: core's open at
//!   5,120 retained jobs, built directly, held to the startup bounds.
//! - `startup_at_high_retained_job_count`: core's open as the jobs grow to
//!   about 5,100 through 40 simulated days of a hammering storm; measured
//!   (`--ignored`, not a gate, about 8.5 minutes; `docs/soak.md`).
//!
//! The ignored tests named `soak_*` are CI's soak job.
mod automation;
mod caps;
mod checks;
mod engine;
mod executor;
mod fixtures;
mod metrics;
mod occurrences;
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
    storm_days: 1,
    hammer_days: 1,
    idle_every: 1,
    trends: false,
    core_restarts_until: Some(2),
    replacements: &[(1, false), (2, true)],
    held_open: None,
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
    storm_days: 1,
    hammer_days: 1,
    idle_every: 7,
    trends: true,
    core_restarts_until: Some(30),
    // Empty stores only before the action storm refills them; none in the
    // last third, whose flatness is asserted.
    replacements: &[
        (5, false),
        (12, true),
        (22, false),
        (33, true),
        (47, true),
        (55, true),
    ],
    held_open: Some((19, 21)),
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
#[ignore = "the 90-day acceptance run: CI's soak job (cargo test -p limeos-core --bin limeos-core -- --ignored soak_)"]
async fn soak_ninety_days() {
    soak(NINETY_DAYS).await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "the 90-day recovery run, alongside soak_ninety_days in CI's soak job"]
async fn soak_ninety_days_restarts() {
    soak(NINETY_DAYS_RESTARTS).await;
}

/// Core's open at a high retained-job count (`docs/soak.md`, "Core startup
/// at high job counts"): the restart scenario with a four-day storm that
/// hammers throughout, for 40 days, so several thousand jobs are kept
/// through retention's 30 days (a four-day storm backing off after its first
/// day keeps about 2,700). Not a gate (CI holds the same operating point to
/// the startup bounds in `caps::soak_core_startup_at_high_retained_job_count`,
/// built directly): it prints, per nightly copy, the jobs, pages, cold
/// startup reads per page against the bound's factor and the open's time,
/// and the statements reading most at the worst point, and asserts only
/// that the operating point was reached.
const HIGH_JOBS: Profile = Profile {
    name: "high-jobs",
    days: 40,
    storm_days: 4,
    hammer_days: 4,
    trends: false,
    held_open: None,
    ..NINETY_DAYS_RESTARTS
};

#[tokio::test(flavor = "current_thread")]
#[ignore = "measures a known limit (docs/soak.md); not part of CI's soak job"]
async fn startup_at_high_retained_job_count() {
    let sim = workload::run(HIGH_JOBS).await;
    let jobs = checks::report_startup_scaling(&sim);
    assert!(
        jobs >= 3_500,
        "the operating point was not reached: {jobs} jobs"
    );
}
