//! Genuine retained collection, unregistered until independent supervision exists.
//! Cooperative checks cannot terminate synchronous kernel work or reap children.
use crate::read_budget::{self, BudgetPlan, BudgetSession, Phase, ReadReportBytes};
use limeos_domain::{ErrorCode, storage_dependency_graph::ConfiguredStorageGraphPreview};
use limeos_executor_container::docker::{
    Docker,
    process_evidence::{EnginePeerPolicy, EngineStorageProcessSnapshot},
};
use limeos_executor_storage::{
    ContainerDependencySnapshot,
    configuration::{
        ConfigurationPaths, StorageConfigurationSnapshot,
        inspect_storage_configuration_with_profile,
    },
    destinations::{ContainerDestinationSnapshot, inspect_container_destinations},
    inspect_container_dependencies_with_profile,
    processes::{
        RunningContainerProcessBinding, RunningContainerProcessSnapshot,
        inspect_running_container_processes_with_profile,
    },
};
use serde::Serialize;
use std::{
    cell::Cell,
    future::Future,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(test)]
mod tests;
const WINDOW: Duration = Duration::from_secs(4);
const MAX_RUNNING: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Admission,
    Collect(Phase),
    ConfigurationBeforeValidation,
    EngineBeforeValidation,
    DestinationValidation,
    EngineAfterValidation,
    ConfigurationAfterValidation,
    Report,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Budget(read_budget::Refusal),
    Storage(limeos_executor_storage::Failure),
    Engine(ErrorCode),
    InvalidPlan,
    Cancelled,
    Expired,
    Clock,
    Unavailable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub stage: Stage,
    pub failure: Failure,
}
pub type Result<T> = std::result::Result<T, Refusal>;

/// One complete, bounded past observation. It carries no retained authority.
/// ```compile_fail
/// use limeos_executor::read_collection::CollectedReadReport;
/// let forged: CollectedReadReport = serde_json::from_str("{}").unwrap();
/// ```
pub struct CollectedReadReport(ReadReportBytes);
impl CollectedReadReport {
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

#[derive(Serialize)]
struct Report<'a> {
    version: u16,
    collection_started_at: i64,
    budget: BudgetPlan,
    configuration: &'a StorageConfigurationSnapshot,
    engine: &'a EngineStorageProcessSnapshot,
    processes: &'a RunningContainerProcessSnapshot,
    physical: &'a ContainerDependencySnapshot,
    destinations: &'a ContainerDestinationSnapshot,
    configured_graph: &'a ConfiguredStorageGraphPreview,
}

/// Requires a fresh, independently supervised worker with soft NOFILE=226.
/// The caller selects only trusted protected configuration paths. Engine endpoint
/// and root authentication are fixed. No launch or installed service uses this yet.
/// Cancellation and the four-second window are cooperative, not kill guarantees.
pub async fn collect(
    paths: ConfigurationPaths<'_>,
    cancelled: &dyn Fn() -> bool,
) -> Result<CollectedReadReport> {
    let window = CollectionWindow::new().map_err(|failure| Refusal {
        stage: Stage::Admission,
        failure,
    })?;
    let mut budget = window
        .checked(Stage::Admission, cancelled, async {
            BudgetSession::admit_worker().map_err(Failure::Budget)
        })
        .await?;
    let poll = || window.check(cancelled).is_err();

    let profile = budget.configuration_profile();
    let configuration = window
        .phase(&mut budget, Phase::Configuration, cancelled, async {
            inspect_storage_configuration_with_profile(paths, profile, &poll)
                .map_err(Failure::Storage)
        })
        .await?;
    let profile = budget.engine_profile();
    let engine = window
        .phase(&mut budget, Phase::Engine, cancelled, async {
            Docker::new("/run/docker.sock".into())
                .inspect_storage_processes_with_profile(EnginePeerPolicy::root(), profile)
                .await
                .map_err(|error| Failure::Engine(error.0))
        })
        .await?;
    let profile = budget.process_profile();
    let processes = window
        .phase(&mut budget, Phase::Processes, cancelled, async {
            // The genuine closed Engine owner bounds these records before copies.
            if engine.running().len() > MAX_RUNNING {
                return Err(Failure::Unavailable);
            }
            let mut bindings = Vec::new();
            bindings
                .try_reserve_exact(engine.running().len())
                .map_err(|_| Failure::Unavailable)?;
            for process in engine.running() {
                bindings.push(RunningContainerProcessBinding {
                    container: process.container().clone(),
                    pid: process.pid(),
                });
            }
            inspect_running_container_processes_with_profile(
                engine.declarations(),
                &bindings,
                profile,
            )
            .map_err(Failure::Storage)
        })
        .await?;
    let profile = budget.storage_profile();
    let physical = window
        .phase(&mut budget, Phase::Physical, cancelled, async {
            inspect_container_dependencies_with_profile(
                &configuration.configuration().contract,
                engine.declarations(),
                profile,
            )
            .await
            .map_err(Failure::Storage)
        })
        .await?;
    let limits = budget.destination_limits();
    let destinations = window
        .phase(&mut budget, Phase::Destinations, cancelled, async {
            inspect_container_destinations(&physical, &processes, limits, &poll)
                .await
                .map_err(Failure::Storage)
        })
        .await?;
    let configured_graph = window
        .phase(&mut budget, Phase::Graph, cancelled, async {
            let config = configuration.configuration();
            limeos_domain::storage_dependency_graph::configured_storage_dependency_graph(
                &config.contract,
                &config.pools,
                &config.protection,
                engine.declarations(),
                &config.shares,
                wall_now()?,
            )
            .map_err(|_| Failure::InvalidPlan)
        })
        .await?;

    window
        .checked(Stage::ConfigurationBeforeValidation, cancelled, async {
            configuration.revalidate(&poll).map_err(Failure::Storage)
        })
        .await?;
    window
        .checked(Stage::EngineBeforeValidation, cancelled, async {
            engine
                .revalidate()
                .await
                .map_err(|error| Failure::Engine(error.0))
        })
        .await?;
    window
        .checked(Stage::DestinationValidation, cancelled, async {
            // This genuine owner brackets its paths with both original parents.
            destinations
                .revalidate(&poll)
                .await
                .map_err(Failure::Storage)
        })
        .await?;
    window
        .checked(Stage::EngineAfterValidation, cancelled, async {
            engine
                .revalidate()
                .await
                .map_err(|error| Failure::Engine(error.0))
        })
        .await?;
    window
        .checked(Stage::ConfigurationAfterValidation, cancelled, async {
            configuration.revalidate(&poll).map_err(Failure::Storage)
        })
        .await?;

    window
        .checked(Stage::Report, cancelled, async {
            // All genuine owners remain live through borrowed serialization.
            budget
                .report(&Report {
                    version: 1,
                    collection_started_at: window.started_wall,
                    budget: read_budget::plan(),
                    configuration: configuration.snapshot(),
                    engine: engine.snapshot(),
                    processes: processes.snapshot(),
                    physical: physical.snapshot(),
                    destinations: destinations.snapshot(),
                    configured_graph: &configured_graph,
                })
                .map(CollectedReadReport)
                .map_err(Failure::Budget)
        })
        .await
}

struct CollectionWindow {
    started: Instant,
    started_wall: i64,
    wall_high_water: Cell<i64>,
    failed: Cell<Option<Failure>>,
}
impl CollectionWindow {
    fn new() -> std::result::Result<Self, Failure> {
        let started = Instant::now();
        let wall = wall_now()?;
        Ok(Self {
            started,
            started_wall: wall,
            wall_high_water: Cell::new(wall),
            failed: Cell::new(None),
        })
    }
    fn check(&self, cancelled: &dyn Fn() -> bool) -> std::result::Result<(), Failure> {
        if let Some(failure) = self.failed.get() {
            return Err(failure);
        }
        let result = wall_now().and_then(|wall| self.check_at(Instant::now(), wall, cancelled()));
        if let Err(failure) = result {
            self.failed.set(Some(failure));
        }
        result
    }
    fn check_at(
        &self,
        now: Instant,
        wall: i64,
        cancelled: bool,
    ) -> std::result::Result<(), Failure> {
        if let Some(failure) = self.failed.get() {
            return Err(failure);
        }
        let result = if cancelled {
            Err(Failure::Cancelled)
        } else if wall < self.wall_high_water.get() || wall < 0 || now < self.started {
            Err(Failure::Clock)
        } else if now.duration_since(self.started) >= WINDOW
            || wall - self.started_wall > WINDOW.as_secs() as i64
        {
            Err(Failure::Expired)
        } else {
            self.wall_high_water.set(wall);
            Ok(())
        };
        if let Err(failure) = result {
            self.failed.set(Some(failure));
        }
        result
    }
    async fn checked<T>(
        &self,
        stage: Stage,
        cancelled: &dyn Fn() -> bool,
        operation: impl Future<Output = std::result::Result<T, Failure>>,
    ) -> Result<T> {
        let refused = |failure| Refusal { stage, failure };
        self.check(cancelled).map_err(refused)?;
        let result = operation.await;
        // Discard completed values, including report bytes, on any late refusal.
        self.check(cancelled).map_err(refused)?;
        result.map_err(|failure| {
            self.failed.set(Some(failure));
            refused(failure)
        })
    }
    async fn phase<T>(
        &self,
        budget: &mut BudgetSession,
        phase: Phase,
        cancelled: &dyn Fn() -> bool,
        operation: impl Future<Output = std::result::Result<T, Failure>>,
    ) -> Result<T> {
        self.checked(Stage::Collect(phase), cancelled, async {
            let guard = budget.enter(phase).map_err(Failure::Budget)?;
            let owner = operation.await?;
            self.check(cancelled)?;
            guard.commit().map_err(Failure::Budget)?;
            Ok(owner)
        })
        .await
    }
}
fn wall_now() -> std::result::Result<i64, Failure> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .ok_or(Failure::Clock)
}
