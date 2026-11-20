//! The storage reader's one read gate. Storage inventory, container health
//! and the periodic stack refresh share it because together they would
//! exceed the reader's thread and descriptor budgets. Callers queue for it
//! with a deadline instead of failing at once when another holds it.
use limeos_domain::{Result, deployment::discovery::DiscoveryCohort};
use std::{sync::Arc, time::Duration};
use tokio::sync::{Mutex, OwnedMutexGuard};

/// A request someone is waiting on: long enough to outlast a typical
/// background read, short enough to fit core's 4 to 5 s deadlines with the
/// read itself.
pub const INTERACTIVE_WAIT: Duration = Duration::from_secs(1);
/// The 30 s stack refresh can wait much longer than a request.
pub const PERIODIC_WAIT: Duration = Duration::from_secs(10);

/// The gate, or `None` if it stayed held for `wait`.
pub async fn acquire(gate: &Arc<Mutex<()>>, wait: Duration) -> Option<OwnedMutexGuard<()>> {
    tokio::time::timeout(wait, gate.clone().lock_owned())
        .await
        .ok()
}

/// One periodic stack refresh. A failed refresh marks the cache stale; a
/// gate held by other reads for all of `wait` does not, since nothing was
/// learned: the cached facts stay current until they expire.
pub async fn refresh_stacks<F>(
    gate: &Arc<Mutex<()>>,
    wait: Duration,
    writer: &limeos_observations::stacks::Cache,
    refresh: F,
) where
    F: FnOnce(OwnedMutexGuard<()>) -> Result<DiscoveryCohort> + Send + 'static,
{
    let Some(guard) = acquire(gate, wait).await else {
        return;
    };
    match tokio::task::spawn_blocking(move || refresh(guard))
        .await
        .ok()
        .and_then(Result::ok)
    {
        Some(cohort) => {
            if writer.update(&cohort).is_err() {
                writer.failed();
            }
        }
        None => writer.failed(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use limeos_domain::deployment::discovery::{
        CatalogFacts, Declaration, EngineFacts, FileStatus,
    };

    fn cohort() -> DiscoveryCohort {
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
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
    fn current(cache: &limeos_observations::stacks::Cache) -> bool {
        cache.cohort().unwrap().current_sources() == (true, true)
    }
    /// Holds the gate for `held`, as an inventory or health read would.
    fn interactive_read(gate: &Arc<Mutex<()>>, held: Duration) {
        let guard = gate.clone().try_lock_owned().unwrap();
        tokio::spawn(async move {
            tokio::time::sleep(held).await;
            drop(guard);
        });
    }

    #[tokio::test]
    async fn an_overlapping_read_delays_the_stack_refresh_instead_of_marking_the_cache_stale() {
        let gate = Arc::new(Mutex::new(()));
        let cache = limeos_observations::stacks::Cache::new();
        cache.update(&cohort()).unwrap();
        assert!(current(&cache));
        interactive_read(&gate, Duration::from_millis(100));
        let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let marked = ran.clone();
        refresh_stacks(&gate, Duration::from_secs(5), &cache, move |_guard| {
            marked.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(cohort())
        })
        .await;
        assert!(ran.load(std::sync::atomic::Ordering::SeqCst));
        assert!(current(&cache));

        // Held past the wait: the refresh is skipped, and the cache stays current.
        interactive_read(&gate, Duration::from_secs(1));
        refresh_stacks(&gate, Duration::from_millis(50), &cache, |_guard| {
            panic!("refresh ran without the gate")
        })
        .await;
        assert!(current(&cache));

        // A refresh that runs and fails still marks the cache stale.
        refresh_stacks(&gate, Duration::from_secs(5), &cache, |_guard| {
            Err(limeos_domain::Error(limeos_domain::ErrorCode::Unavailable))
        })
        .await;
        assert!(!current(&cache));
    }

    #[tokio::test]
    async fn a_request_queues_briefly_for_the_gate_instead_of_being_refused() {
        let gate = Arc::new(Mutex::new(()));
        interactive_read(&gate, Duration::from_millis(50));
        assert!(acquire(&gate, INTERACTIVE_WAIT).await.is_some());
        let _held = gate.clone().try_lock_owned().unwrap();
        let started = std::time::Instant::now();
        assert!(acquire(&gate, Duration::from_millis(50)).await.is_none());
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
