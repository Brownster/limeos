use super::*;
use limeos_domain::deployment::discovery::{Declaration, FileStatus, SourceState};

fn sample(at: i64) -> DiscoveryCohort {
    DiscoveryCohort::new(
        Some(CatalogFacts {
            engine_id: "engine".into(),
            sampled_at: at,
            stacks: vec![Declaration {
                stack: "media".into(),
                project: "media".into(),
                working_directory: "/opt/Media".into(),
                config_files: vec!["/opt/Media/compose.yaml".into()],
                services: vec!["web".into()],
                files: FileStatus::NotInspected,
            }],
        }),
        Some(EngineFacts {
            engine_id: "engine".into(),
            sampled_at: at,
            members: vec![],
            truncated: 0,
            omitted: Vec::new(),
            hidden_projects: Vec::new(),
            unattributed: 0,
        }),
    )
    .unwrap()
}
fn cache(at: Instant) -> Cache {
    Cache(
        Arc::new(RwLock::new(State {
            catalog: Source::new(at),
            engine: Source::new(at),
            wall_high_water: 100,
        })),
        system_clock,
    )
}
thread_local! {
    /// A test clock: (seconds since the epoch, seconds past `ORIGIN`).
    static TEST_CLOCK: std::cell::Cell<(i64, u64)> = const { std::cell::Cell::new((0, 0)) };
}
static ORIGIN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
fn test_clock() -> (i64, Instant) {
    let (wall, elapsed) = TEST_CLOCK.with(std::cell::Cell::get);
    let origin = *ORIGIN.get_or_init(Instant::now);
    (wall, origin + Duration::from_secs(elapsed))
}
/// A cache on an injected clock judges freshness, the sample's age and the
/// monotonic deadline alike, by that clock and never by the system's: a
/// cohort sampled years ago by the system clock is fresh while the injected
/// clock says it is, and stale once it has moved past `MAX_AGE_SECONDS`.
#[test]
fn a_cache_on_an_injected_clock_judges_freshness_by_that_clock_alone() {
    let start = 1_000_000_000;
    TEST_CLOCK.with(|c| c.set((start, 0)));
    let c = Cache::with_clock(test_clock);
    c.update(&sample(start)).unwrap();
    for elapsed in [0, 45, 90, 91] {
        TEST_CLOCK.with(|c| c.set((start + elapsed, elapsed as u64)));
        let view = c.snapshot().unwrap();
        assert_eq!(view.catalog.sampled_at, Some(start));
        assert_eq!(
            view.catalog.state,
            if elapsed <= MAX_AGE_SECONDS {
                SourceState::Fresh
            } else {
                SourceState::Stale
            },
            "{elapsed} s after the sample"
        );
    }
    // A sample from the injected clock's past is refused as a step back.
    TEST_CLOCK.with(|c| c.set((start + 100, 100)));
    c.update(&sample(start + 100)).unwrap();
    assert_eq!(c.snapshot().unwrap().catalog.state, SourceState::Fresh);
    TEST_CLOCK.with(|c| c.set((start + 50, 101)));
    assert_eq!(c.snapshot().unwrap().catalog.state, SourceState::Stale);
}
#[test]
fn repeated_reads_and_same_cohort_cannot_refresh_the_monotonic_deadline() {
    let at = Instant::now();
    let c = cache(at);
    c.update_at(&sample(100), 100, at).unwrap();
    for elapsed in [1, 45, 90, 91, 180] {
        let now = at + Duration::from_secs(elapsed);
        c.update_at(&sample(100), 100, now).unwrap();
        let view = c.cohort_at(100, now).unwrap().project(100).unwrap();
        assert_eq!(view.catalog.sampled_at, Some(100));
        assert_eq!(
            view.catalog.state,
            if elapsed <= 90 {
                SourceState::Fresh
            } else {
                SourceState::Stale
            }
        );
    }
}
#[test]
fn failure_keeps_facts_but_only_a_new_sample_can_restore_freshness() {
    let at = Instant::now();
    let c = cache(at);
    c.update_at(&sample(100), 100, at).unwrap();
    c.failed();
    c.update_at(&sample(100), 101, at + Duration::from_secs(1))
        .unwrap();
    let view = c
        .cohort_at(101, at + Duration::from_secs(1))
        .unwrap()
        .project(101)
        .unwrap();
    assert_eq!(view.stacks.len(), 1);
    assert_eq!(view.stacks[0].counts, None);
    assert_eq!(view.catalog.state, SourceState::Stale);
    c.update_at(&sample(102), 102, at + Duration::from_secs(2))
        .unwrap();
    assert_eq!(
        c.cohort_at(102, at + Duration::from_secs(2))
            .unwrap()
            .project(102)
            .unwrap()
            .catalog
            .state,
        SourceState::Fresh
    );
}
#[test]
fn rollback_and_catchup_cannot_revive_an_observed_cohort() {
    let at = Instant::now();
    let c = cache(at);
    c.update_at(&sample(100), 100, at).unwrap();
    let rollback = c
        .cohort_at(99, at + Duration::from_secs(1))
        .unwrap()
        .project(99)
        .unwrap();
    assert_eq!(rollback.catalog.state, SourceState::Stale);
    assert_eq!(
        c.cohort_at(101, at + Duration::from_secs(2))
            .unwrap()
            .project(101)
            .unwrap()
            .catalog
            .state,
        SourceState::Stale
    );
    c.update_at(&sample(98), 102, at + Duration::from_secs(3))
        .unwrap();
    assert_eq!(
        c.cohort_at(102, at + Duration::from_secs(3))
            .unwrap()
            .catalog()
            .unwrap()
            .sampled_at,
        100
    );
}
#[test]
fn source_failure_is_independent_and_cross_cache_transport_preserves_it() {
    let at = Instant::now();
    let c = cache(at);
    c.update_at(&sample(100), 100, at).unwrap();
    let partial = DiscoveryCohort::new(None, sample(101).engine().cloned()).unwrap();
    c.update_at(&partial, 101, at + Duration::from_secs(1))
        .unwrap();
    let wire = c.cohort_at(101, at + Duration::from_secs(1)).unwrap();
    assert_eq!(wire.current_sources(), (false, true));
    let downstream = cache(at);
    downstream
        .update_at(&wire, 101, at + Duration::from_secs(1))
        .unwrap();
    let view = downstream
        .cohort_at(101, at + Duration::from_secs(1))
        .unwrap()
        .project(101)
        .unwrap();
    assert_eq!(view.catalog.sampled_at, Some(100));
    assert_eq!(view.catalog.state, SourceState::Stale);
    assert_eq!(view.engine.sampled_at, Some(101));
    assert_eq!(view.engine.state, SourceState::Fresh);
    assert_eq!(view.stacks[0].counts, None);
}

#[test]
fn a_restarted_proxy_inherits_remaining_monotonic_age_even_if_wall_clock_stops() {
    let at = Instant::now();
    let upstream = cache(at);
    upstream.update_at(&sample(100), 100, at).unwrap();
    let wire = upstream
        .cohort_at(100, at + Duration::from_secs(60))
        .unwrap();
    assert_eq!(wire.remaining_millis(), (30_000, 30_000));
    let downstream = cache(at);
    downstream
        .update_at(&wire, 100, at + Duration::from_secs(60))
        .unwrap();
    assert_eq!(
        downstream
            .cohort_at(100, at + Duration::from_secs(91))
            .unwrap()
            .project(100)
            .unwrap()
            .catalog
            .state,
        SourceState::Stale
    );
}
