//! One shared display cache. Reads do no file, Engine, RPC or SQL work.
use limeos_domain::{
    Result,
    deployment::discovery::{
        CatalogFacts, DiscoveryCohort, EngineFacts, MAX_AGE_SECONDS, StackInventory,
    },
};
use std::{
    path::PathBuf,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

trait Facts: Clone + PartialEq {
    fn at(&self) -> i64;
    fn engine(&self) -> &str;
}
impl Facts for CatalogFacts {
    fn at(&self) -> i64 {
        self.sampled_at
    }
    fn engine(&self) -> &str {
        &self.engine_id
    }
}
impl Facts for EngineFacts {
    fn at(&self) -> i64 {
        self.sampled_at
    }
    fn engine(&self) -> &str {
        &self.engine_id
    }
}
#[derive(Clone)]
struct Source<T> {
    facts: Option<T>,
    current: bool,
    expires: Instant,
}
impl<T: Facts> Source<T> {
    fn new(at: Instant) -> Self {
        Self {
            facts: None,
            current: false,
            expires: at,
        }
    }
    fn update(
        &mut self,
        facts: Option<&T>,
        current: bool,
        remaining_millis: u32,
        wall: i64,
        monotonic: Instant,
    ) {
        let previous_current = self.current;
        self.current = false;
        let Some(facts) = facts else {
            return;
        };
        if let Some(old) = &self.facts {
            if facts.at() < old.at() {
                return;
            }
            if facts.at() == old.at() && facts.engine() == old.engine() {
                // A replay, failed retry or reconnect cannot renew an old sample.
                self.current = previous_current
                    && current
                    && old == facts
                    && wall >= facts.at()
                    && monotonic <= self.expires;
                self.expires = self
                    .expires
                    .min(monotonic + Duration::from_millis(u64::from(remaining_millis)));
                return;
            }
        }
        self.facts = Some(facts.clone());
        let age = wall.saturating_sub(facts.at());
        self.current = current && (0..=MAX_AGE_SECONDS).contains(&age);
        self.expires = monotonic
            + Duration::from_secs(
                MAX_AGE_SECONDS
                    .saturating_sub(age)
                    .clamp(0, MAX_AGE_SECONDS) as u64,
            )
            .min(Duration::from_millis(u64::from(remaining_millis)));
    }
    fn current(&self, at: Instant) -> bool {
        self.current && at <= self.expires
    }
    fn remaining(&self, at: Instant) -> u32 {
        if self.current(at) {
            self.expires
                .saturating_duration_since(at)
                .as_millis()
                .min(90_000) as u32
        } else {
            0
        }
    }
}
struct State {
    catalog: Source<CatalogFacts>,
    engine: Source<EngineFacts>,
    wall_high_water: i64,
}
impl State {
    fn clock(&mut self, wall: i64) {
        if wall < self.wall_high_water {
            self.catalog.current = false;
            self.engine.current = false;
        }
        self.wall_high_water = self.wall_high_water.max(wall);
    }
    fn cohort(&self, at: Instant) -> Result<DiscoveryCohort> {
        DiscoveryCohort::with_current(
            self.catalog.facts.clone(),
            self.engine.facts.clone(),
            self.catalog.current(at),
            self.engine.current(at),
        )?
        .with_remaining(self.catalog.remaining(at), self.engine.remaining(at))
    }
}
/// The clock a cache judges freshness by: the wall time (seconds since the
/// Unix epoch) and the monotonic instant, read together.
pub type Clock = fn() -> (i64, Instant);
/// The system clocks, which every cache but a test's reads.
fn system_clock() -> (i64, Instant) {
    (super::now(), Instant::now())
}
#[derive(Clone)]
pub struct Cache(Arc<RwLock<State>>, Clock);
impl Default for Cache {
    fn default() -> Self {
        Self::new()
    }
}
impl Cache {
    pub fn new() -> Self {
        Self::with_clock(system_clock)
    }
    /// A cache that reads `clock` wherever [`Cache::new`]'s reads the system
    /// clocks. Core's unit tests (the soak) pass core's own clock, so its
    /// stack proposals judge the cohort's freshness on simulated time.
    pub fn with_clock(clock: Clock) -> Self {
        let (wall, at) = clock();
        Self(
            Arc::new(RwLock::new(State {
                catalog: Source::new(at),
                engine: Source::new(at),
                wall_high_water: wall,
            })),
            clock,
        )
    }
    pub fn update(&self, cohort: &DiscoveryCohort) -> Result<()> {
        let (wall, at) = (self.1)();
        self.update_at(cohort, wall, at)
    }
    fn update_at(&self, cohort: &DiscoveryCohort, wall: i64, at: Instant) -> Result<()> {
        let mut state = self.0.write().unwrap_or_else(|e| e.into_inner());
        state.clock(wall);
        let mut catalog = state.catalog.clone();
        let mut engine = state.engine.clone();
        let (c, e) = cohort.current_sources();
        let (cr, er) = cohort.remaining_millis();
        catalog.update(
            cohort.catalog(),
            c && wall >= state.wall_high_water,
            cr,
            wall,
            at,
        );
        engine.update(
            cohort.engine(),
            e && wall >= state.wall_high_water,
            er,
            wall,
            at,
        );
        // A mix of retained and new facts must satisfy the same complete bounds.
        if let Err(error) = DiscoveryCohort::with_current(
            catalog.facts.clone(),
            engine.facts.clone(),
            catalog.current(at),
            engine.current(at),
        ) {
            state.catalog.current = false;
            state.engine.current = false;
            return Err(error);
        }
        state.catalog = catalog;
        state.engine = engine;
        Ok(())
    }
    pub fn failed(&self) {
        let mut state = self.0.write().unwrap_or_else(|e| e.into_inner());
        state.catalog.current = false;
        state.engine.current = false;
    }
    pub fn cohort(&self) -> Result<DiscoveryCohort> {
        let (wall, at) = (self.1)();
        self.cohort_at(wall, at)
    }
    fn cohort_at(&self, wall: i64, at: Instant) -> Result<DiscoveryCohort> {
        let mut state = self.0.write().unwrap_or_else(|e| e.into_inner());
        state.clock(wall);
        state.cohort(at)
    }
    pub fn snapshot(&self) -> Result<StackInventory> {
        self.cohort()?.project((self.1)().0)
    }
}

/// Background-only RPC. Every browser uses the same cache; GET has no wake-up.
/// The reader at `socket` is trusted by `owner`'s rule: a socket owned by
/// neither root nor core, answered by its owner.
pub fn start_proxy(socket: PathBuf, owner: limeos_contracts::ExecutorOwner) -> Cache {
    let cache = Cache::new();
    let writer = cache.clone();
    tokio::spawn(async move {
        loop {
            let result = tokio::time::timeout(Duration::from_secs(5), async {
                let started = Instant::now();
                let mut stream = owner.connect(&socket).await.map_err(|_| ())?;
                limeos_contracts::write_frame(
                    &mut stream,
                    &limeos_executor_protocol::Request::StackInventory {
                        version: limeos_contracts::VERSION,
                    },
                )
                .await
                .map_err(|_| ())?;
                let receipt: limeos_executor_protocol::Receipt =
                    limeos_contracts::read_frame(&mut stream)
                        .await
                        .map_err(|_| ())?;
                if receipt.version != limeos_contracts::VERSION
                    || !receipt.ready
                    || receipt.error.is_some()
                {
                    return Err(());
                }
                let cohort = receipt.stack_cohort.ok_or(())?;
                let (c, e) = cohort.remaining_millis();
                let elapsed = started.elapsed().as_millis().min(90_000) as u32;
                cohort
                    .with_remaining(c.saturating_sub(elapsed), e.saturating_sub(elapsed))
                    .map_err(|_| ())
            })
            .await;
            match result {
                Ok(Ok(cohort)) => {
                    if writer.update(&cohort).is_err() {
                        writer.failed();
                    }
                }
                _ => writer.failed(),
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });
    cache
}

#[cfg(test)]
mod tests;
