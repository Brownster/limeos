//! Narrow root-reader collection. The process supervisor owns the hard deadline.
use crate::{
    read_budget::Sample,
    read_launch::{ProtectedStackLaunch, StackSelection},
    read_supervision::Supervisor,
};
use limeos_domain::{
    Error, ErrorCode, Result,
    deployment::discovery::{DiscoveryCohort, MAX_REPORT_BYTES},
};
use limeos_executor_container::{
    compose_management::catalog::inspect_management_catalog, deployment::discovery,
    docker::compose_evidence::inspect_compose_membership,
};
use serde::Serialize;
use std::{io::Write, path::Path};

const MIB: usize = 1024 * 1024;
const MANAGER_RSS: usize = 12 * MIB;
const GUARDIAN_RSS: usize = 8 * MIB;
const WORKER_BASELINE_RSS: usize = 16 * MIB;
const WORKER_RSS: usize = 20 * MIB;

/// Conservative component allowances; not measured whole-service acceptance.
#[derive(Debug, Serialize)]
pub struct BudgetPlan {
    manager_rss_bytes: usize,
    guardian_rss_bytes: usize,
    worker_rss_bytes: usize,
    sealed_image_bytes: usize,
    source_image_bytes: usize,
    unmodeled_margin_bytes: usize,
    catalog_fds: usize,
    engine_fds: usize,
    worker_baseline_fds: usize,
    transient_fds: usize,
    service_memory_bytes: usize,
    service_nofile: usize,
    service_tasks: usize,
}
pub fn plan() -> BudgetPlan {
    BudgetPlan {
        manager_rss_bytes: MANAGER_RSS,
        guardian_rss_bytes: GUARDIAN_RSS,
        worker_rss_bytes: WORKER_RSS,
        sealed_image_bytes: 8 * MIB,
        source_image_bytes: 8 * MIB,
        unmodeled_margin_bytes: 8 * MIB,
        catalog_fds: 18,
        engine_fds: 1,
        worker_baseline_fds: 16,
        transient_fds: 32,
        service_memory_bytes: 64 * MIB,
        service_nofile: 256,
        service_tasks: 16,
    }
}
fn unavailable() -> Error {
    Error(ErrorCode::Unavailable)
}
fn sample(max_rss: usize, max_fds: usize, soft: u64) -> Result<()> {
    admit(
        Sample::read().map_err(|_| unavailable())?,
        max_rss,
        max_fds,
        soft,
    )
}
fn admit(s: Sample, max_rss: usize, max_fds: usize, soft: u64) -> Result<()> {
    if s.rss_bytes > max_rss
        || s.open_fds > max_fds
        || s.soft_nofile != Some(soft)
        || !(1..=4).contains(&s.threads)
    {
        return Err(unavailable());
    }
    Ok(())
}
pub fn admit_guardian() -> Result<()> {
    sample(GUARDIAN_RSS, 16, 256)
}

/// Called by one background collector holding the root reader's shared gate.
/// Supervision retains its own flight/image until verified cleanup, even when
/// this waiter times out. No RPC or HTTP handler invokes this collection.
pub fn refresh(
    selection: StackSelection,
    manager: &Supervisor,
    shared_gate: tokio::sync::OwnedMutexGuard<()>,
) -> Result<DiscoveryCohort> {
    let started = std::time::Instant::now();
    sample(MANAGER_RSS, 24, 256)?;
    let launch = ProtectedStackLaunch::prepare(selection).map_err(|_| unavailable())?;
    let report = launch
        .start_with(manager, shared_gate)
        .and_then(|ticket| ticket.wait())
        .map_err(|_| unavailable())?;
    let cohort = DiscoveryCohort::decode(report.as_bytes())?;
    let elapsed = started.elapsed().as_millis().min(90_000) as u32;
    let (c, e) = cohort.remaining_millis();
    cohort.with_remaining(c.saturating_sub(elapsed), e.saturating_sub(elapsed))
}

pub(crate) async fn collect(catalog_path: &Path, cancelled: &dyn Fn() -> bool) -> Result<Vec<u8>> {
    // Reserve the entire bounded cohort before calling either genuine adapter:
    // 2 MiB Engine live ledger + catalog/report/cloning/transport under 4 MiB.
    // Original adapter byte ledgers, leases and final revalidation remain intact.
    sample(WORKER_BASELINE_RSS, 16, 226)?;
    if cancelled() {
        return Err(Error(ErrorCode::Conflict));
    }
    let catalog = inspect_management_catalog(catalog_path, cancelled).ok();
    sample(WORKER_RSS, 67, 226)?;
    let engine = inspect_compose_membership().await.ok();
    sample(WORKER_RSS, 67, 226)?;
    let cohort = discovery::cohort(catalog.as_ref(), engine.as_ref(), cancelled).await?;
    sample(WORKER_RSS, 67, 226)?;
    cohort.check_size()?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(MAX_REPORT_BYTES)
        .map_err(|_| unavailable())?;
    struct Output<'a>(&'a mut Vec<u8>);
    impl Write for Output<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_REPORT_BYTES - self.0.len() {
                return Err(std::io::Error::other("stack report bound"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Output(&mut bytes), &cohort).map_err(|_| unavailable())?;
    sample(WORKER_RSS, 67, 226)?;
    if cancelled() {
        return Err(Error(ErrorCode::Conflict));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn baseline() -> Sample {
        Sample {
            open_fds: 16,
            rss_bytes: 16 * MIB,
            threads: 1,
            soft_nofile: Some(226),
        }
    }
    #[test]
    fn stack_allowances_fit_unchanged_service_and_keep_explicit_margin() {
        let p = plan();
        assert_eq!(
            p.manager_rss_bytes
                + p.guardian_rss_bytes
                + p.worker_rss_bytes
                + p.sealed_image_bytes
                + p.source_image_bytes
                + p.unmodeled_margin_bytes,
            p.service_memory_bytes
        );
        assert_eq!(
            p.worker_baseline_fds + p.catalog_fds + p.engine_fds + p.transient_fds,
            67
        );
        assert!(67 < 226 && 226 < p.service_nofile);
    }
    #[test]
    fn worker_refuses_over_budget_or_wrong_bootstrap_before_adapter_work() {
        assert!(admit(baseline(), WORKER_BASELINE_RSS, 16, 226).is_ok());
        for s in [
            Sample {
                rss_bytes: WORKER_BASELINE_RSS + 1,
                ..baseline()
            },
            Sample {
                open_fds: 17,
                ..baseline()
            },
            Sample {
                threads: 5,
                ..baseline()
            },
            Sample {
                soft_nofile: Some(256),
                ..baseline()
            },
        ] {
            assert!(admit(s, WORKER_BASELINE_RSS, 16, 226).is_err());
        }
    }
}
