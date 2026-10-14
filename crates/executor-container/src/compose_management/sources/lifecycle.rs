//! Read-only lifecycle proposals bound to original protected source and Engine facts.
use super::*;
use limeos_domain::{
    ComposeProject,
    compose_override::{ManagedOverridePlan, ManagedOverrideSummary},
};
use sha2::{Digest, Sha256};
use std::future::Future;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComposeSourceChange {
    Add,
    Change,
    Remove,
}
/// Member indices refer to full original Engine rows, including stopped and one-off rows.
#[derive(Serialize)]
pub struct ComposeLifecycleChange {
    pub service: String,
    pub source_change: ComposeSourceChange,
    pub member_indices: Vec<usize>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnresolvedLifecycleGate {
    DeployedRuntimeConfiguration,
    IndependentDaemonAndHostIdentity,
    PhysicalMountsAndConsumers,
    PrincipalAndFreshApproval,
    StackAndStorageLocks,
    AtomicWriteAndRecovery,
    ShutdownAndEffectTimeVerification,
    SupervisedAndInstalledQualification,
}
const GATES: [UnresolvedLifecycleGate; 8] = [
    UnresolvedLifecycleGate::DeployedRuntimeConfiguration,
    UnresolvedLifecycleGate::IndependentDaemonAndHostIdentity,
    UnresolvedLifecycleGate::PhysicalMountsAndConsumers,
    UnresolvedLifecycleGate::PrincipalAndFreshApproval,
    UnresolvedLifecycleGate::StackAndStorageLocks,
    UnresolvedLifecycleGate::AtomicWriteAndRecovery,
    UnresolvedLifecycleGate::ShutdownAndEffectTimeVerification,
    UnresolvedLifecycleGate::SupervisedAndInstalledQualification,
];

/// A borrowed proposal is past evidence and carries no permission or shutdown receipt.
/// ```compile_fail
/// use limeos_executor_container::compose_management::sources::lifecycle::ComposeLifecycleSummary;
/// let imported: ComposeLifecycleSummary<'_> = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Serialize)]
pub struct ComposeLifecycleSummary<'a> {
    pub version: u16,
    pub evidence: BoundComposeJsonSnapshot<'a>,
    pub proposal: ManagedOverrideSummary<'a>,
    pub artifact_sha256: &'a str,
    pub changes: &'a [ComposeLifecycleChange],
    pub unresolved: &'a [UnresolvedLifecycleGate],
}
/// Private original owners and exact compiled bytes cannot be reconstructed from JSON.
/// ```compile_fail
/// use limeos_executor_container::compose_management::sources::lifecycle::BoundComposeLifecycleProposal;
/// let imported: BoundComposeLifecycleProposal = serde_json::from_str("{}").unwrap();
/// ```
pub struct BoundComposeLifecycleProposal {
    evidence: BoundComposeJsonEvidence,
    plan: ManagedOverridePlan,
    artifact_sha256: String,
    changes: Vec<ComposeLifecycleChange>,
    state: AtomicU8,
}
impl BoundComposeLifecycleProposal {
    pub fn summary(&self) -> ComposeLifecycleSummary<'_> {
        ComposeLifecycleSummary {
            version: 1,
            evidence: self.evidence.snapshot(),
            proposal: self.plan.summary(),
            artifact_sha256: &self.artifact_sha256,
            changes: &self.changes,
            unresolved: &GATES,
        }
    }
    /// Candidate file bytes, never an executable approval or write capability.
    pub fn artifact(&self) -> &[u8] {
        self.plan.artifact()
    }
    pub fn retained_descriptor_count(&self) -> usize {
        self.evidence.retained_descriptor_count()
    }
    pub async fn revalidate(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        self.validate_with(cancelled, self.evidence.revalidate(cancelled))
            .await
    }
    async fn validate_with(
        &self,
        cancelled: &dyn Fn() -> bool,
        source_check: impl Future<Output = Result<()>>,
    ) -> Result<()> {
        if self
            .state
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            self.state.store(2, Ordering::SeqCst);
            return Err(conflict());
        }
        let guard = Validation {
            state: &self.state,
            completed: false,
        };
        let result = tokio::time::timeout_at(
            tokio::time::Instant::from_std(self.evidence.binding.window.expires),
            async {
                self.evidence.binding.window.poll(cancelled)?;
                source_check.await?;
                self.evidence.binding.window.poll(cancelled)?;
                check_lifecycle_report(&self.summary(), REPORT_BYTES)?;
                self.evidence.binding.window.poll(cancelled)
            },
        )
        .await
        .map_err(|_| Error(ErrorCode::Expired))
        .and_then(|result| result);
        guard.finish(result)
    }
}
impl BoundComposeJsonEvidence {
    /// Consume the actual source owner; callers supply only a desired declaration.
    pub async fn propose_lifecycle(
        self,
        desired: &ComposeProject,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<BoundComposeLifecycleProposal> {
        let proposal = assemble_proposal(self, desired, cancelled)?;
        proposal.revalidate(cancelled).await?;
        Ok(proposal)
    }
}
fn assemble_proposal(
    evidence: BoundComposeJsonEvidence,
    desired: &ComposeProject,
    cancelled: &dyn Fn() -> bool,
) -> Result<BoundComposeLifecycleProposal> {
    if evidence.state.load(Ordering::SeqCst) != 0 {
        return Err(conflict());
    }
    evidence.binding.window.poll(cancelled)?;
    let plan = evidence.sources.plan(desired)?;
    evidence.binding.window.poll(cancelled)?;
    let binding = evidence.binding.snapshot();
    if plan
        .summary()
        .desired
        .services
        .iter()
        .any(|service| !binding.registration.services.contains(&service.name))
    {
        return Err(conflict());
    }
    let mut changes = Vec::new();
    for change in &plan.summary().impact.services {
        let source_change = match (&change.before, &change.after) {
            (None, Some(_)) => ComposeSourceChange::Add,
            (Some(_), Some(_)) => ComposeSourceChange::Change,
            (Some(_), None) => ComposeSourceChange::Remove,
            (None, None) => return Err(conflict()),
        };
        let member_indices = binding
            .member_indices
            .iter()
            .copied()
            .filter(|index| {
                matches!(&binding.engine.containers[*index].ownership,
                    ComposeOwnership::Compose { service, .. } if *service == change.name)
            })
            .collect();
        changes.push(ComposeLifecycleChange {
            service: change.name.clone(),
            source_change,
            member_indices,
        });
    }
    let proposal = BoundComposeLifecycleProposal {
        artifact_sha256: hex::encode(Sha256::digest(plan.artifact())),
        evidence,
        plan,
        changes,
        state: AtomicU8::new(0),
    };
    if proposal.changes.len() > 16
        || proposal
            .changes
            .iter()
            .map(|change| change.member_indices.len())
            .sum::<usize>()
            > 16
        || proposal.retained_descriptor_count() > RETAINED_FDS
    {
        return Err(unavailable());
    }
    check_lifecycle_report(&proposal.summary(), REPORT_BYTES)?;
    proposal.evidence.binding.window.poll(cancelled)?;
    Ok(proposal)
}
fn check_lifecycle_report(summary: &ComposeLifecycleSummary<'_>, max: usize) -> Result<()> {
    serde_json::to_writer(&mut Count { bytes: 0, max }, summary).map_err(|_| unavailable())
}
/// Fixed-policy collection and reviewable planning, without an executable operation.
pub async fn inspect_compose_lifecycle_proposal(
    catalog_path: &Path,
    stack: &str,
    desired: &ComposeProject,
    cancelled: &dyn Fn() -> bool,
) -> Result<BoundComposeLifecycleProposal> {
    if !rustix::process::geteuid().is_root() {
        return Err(Error(ErrorCode::Forbidden));
    }
    inspect_bound_compose_json(catalog_path, stack, cancelled)
        .await?
        .propose_lifecycle(desired, cancelled)
        .await
}

#[cfg(test)]
pub(crate) mod tests;
