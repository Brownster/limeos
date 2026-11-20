//! Fresh, nonce-bound health collection under the existing independent supervisor.
use crate::{
    read_budget::Sample, read_launch::ProtectedHealthLaunch, read_supervision::Supervisor,
};
use limeos_domain::{
    Error, ErrorCode, Result,
    automation::{ContainerHealthCohort, ContainerHealthMember},
};
use limeos_executor_container::docker::health_evidence::inspect_container_health;
const MIB: usize = 1024 * 1024;
fn unavailable() -> Error {
    Error(ErrorCode::Unavailable)
}
fn sample(rss: usize, fds: usize, soft: u64) -> Result<()> {
    let s = Sample::read().map_err(|_| unavailable())?;
    if s.rss_bytes > rss
        || s.open_fds > fds
        || s.soft_nofile != Some(soft)
        || !(1..=4).contains(&s.threads)
    {
        return Err(unavailable());
    }
    Ok(())
}
pub fn refresh(
    nonce: &str,
    manager: &Supervisor,
    gate: tokio::sync::OwnedMutexGuard<()>,
) -> Result<ContainerHealthCohort> {
    sample(12 * MIB, 24, 256)?;
    let launch = ProtectedHealthLaunch::prepare(nonce).map_err(|_| unavailable())?;
    let report = launch
        .start_with(manager, gate)
        .and_then(|ticket| ticket.wait())
        .map_err(|_| unavailable())?;
    let cohort = ContainerHealthCohort::decode(report.as_bytes())?;
    if cohort.nonce() != nonce {
        return Err(Error(ErrorCode::Conflict));
    }
    Ok(cohort)
}
pub(crate) async fn collect(nonce: &str, cancelled: &dyn Fn() -> bool) -> Result<Vec<u8>> {
    if !limeos_domain::opaque_id(nonce) {
        return Err(Error(ErrorCode::InvalidInput));
    }
    sample(16 * MIB, 16, 226)?;
    if cancelled() {
        return Err(Error(ErrorCode::Conflict));
    }
    let engine = inspect_container_health().await?;
    sample(20 * MIB, 49, 226)?;
    // Alerting reads tolerate one container changing since collection: it is
    // reported with an unavailable signal, never as healthy or gone.
    let changed = engine.revalidate_display().await?;
    let snapshot = engine.snapshot();
    let mut members = Vec::new();
    members
        .try_reserve_exact(snapshot.containers.len())
        .map_err(|_| unavailable())?;
    for member in &snapshot.containers {
        members.push(ContainerHealthMember {
            resource: member.resource.clone(),
            signal: if changed.contains(&member.resource) {
                member.unavailable_signal()?
            } else {
                member.signal.clone()
            },
        });
    }
    members.sort_by(|a, b| a.resource.cmp(&b.resource));
    let cohort = ContainerHealthCohort::new(
        nonce.into(),
        snapshot.engine_id.clone(),
        snapshot.containers_observed_at,
        members,
    )?;
    let bytes = cohort.encode()?;
    sample(20 * MIB, 49, 226)?;
    if cancelled() {
        return Err(Error(ErrorCode::Conflict));
    }
    Ok(bytes)
}
