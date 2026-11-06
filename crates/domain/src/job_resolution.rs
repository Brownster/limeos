//! Operator resolution of `needs_intervention` jobs.
//!
//! Resolution is bookkeeping plus read-only verification. It records what an
//! administrator concluded about an uncertain job, on fresh evidence, and
//! releases the job's locks. It never starts, stops or restarts anything and
//! never edits the original job, plan, receipt or member results.
use crate::{ContainerAction, ContainerSnapshot, Error, ErrorCode, Result, opaque_id};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

pub const MAX_RESOLUTION_REASON: usize = 500;
pub const MIN_DIVERGENCE_REASON: usize = 20;
const MAX_TARGETS: usize = crate::MAX_STACK_MEMBERS;

/// What the administrator declares about the job. Only `AcceptedDivergence`
/// rests on the operator's word; the other two are checked against evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum DeclaredOutcome {
    /// The intended end state is present on every target.
    VerifiedDone,
    /// Nothing took effect: every target still equals the plan's `expected`.
    NotDone,
    /// The present state differs from both before and intended; accepted.
    AcceptedDivergence,
}
impl DeclaredOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::VerifiedDone => "verified_done",
            Self::NotDone => "not_done",
            Self::AcceptedDivergence => "accepted_divergence",
        }
    }
}

/// Which executor table holds the action: one container or a whole stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ResolveKind {
    Container,
    Stack,
}

/// One stuck container or stack job in the administrator-wide attention list.
/// Read-only: it names the job and its owner, never evidence or a decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct NeedsAttentionJob {
    pub job: String,
    /// Username of the principal that created the job.
    pub owner: String,
    pub kind: ResolveKind,
    pub operation: ContainerAction,
    /// `needs_intervention`, or `running`/`verifying` left by an earlier core.
    pub state: crate::JobState,
    /// The plan's headline resource: `container:<id>` or `stack:<name>`.
    pub resource: String,
    /// Resources the action would touch, as `container:<id>`.
    pub targets: Vec<String>,
    /// Unix seconds of the job's first recorded event; 0 when none survives.
    #[ts(type = "number")]
    pub created_at: i64,
}

/// One target's fresh, read-only measurement against the plan. A container
/// that no longer exists is recorded with empty `image` and `started_at`,
/// `running` false and both flags false (see `is_absent`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ResolutionTarget {
    /// Full container ID, without the `container:` prefix.
    pub container: String,
    pub running: bool,
    pub started_at: String,
    pub image: String,
    /// ID, start time, image and running state all equal the plan's `expected`.
    pub matches_expected: bool,
    /// The plan's action is proven by this target's post-state.
    pub intended_state_reached: bool,
}
impl ResolutionTarget {
    pub fn is_absent(&self) -> bool {
        self.image.is_empty()
    }
}

/// Evidence for exactly the job's targets, sorted by container ID. Core and
/// executor build it with the same function so their digests agree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ResolutionEvidence {
    pub targets: Vec<ResolutionTarget>,
}
impl ResolutionEvidence {
    /// `expected` are the plan's targets for `operation`; `current[i]` is the
    /// fresh inspection of `expected[i]`, `None` when the container is gone.
    pub fn observe(
        operation: ContainerAction,
        expected: &[&ContainerSnapshot],
        current: &[Option<ContainerSnapshot>],
    ) -> Result<Self> {
        if expected.len() != current.len() {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let mut targets = Vec::with_capacity(expected.len());
        for (before, now) in expected.iter().zip(current) {
            let container = before
                .resource
                .strip_prefix("container:")
                .ok_or(Error(ErrorCode::InvalidInput))?
                .to_owned();
            targets.push(match now {
                Some(now) if now.resource == before.resource => ResolutionTarget {
                    container,
                    running: now.running,
                    started_at: now.started_at.clone(),
                    image: now.image.clone(),
                    matches_expected: now == *before,
                    intended_state_reached: crate::verify_container(operation, before, now).is_ok(),
                },
                Some(_) => return Err(Error(ErrorCode::InvalidInput)),
                None => ResolutionTarget {
                    container,
                    running: false,
                    started_at: String::new(),
                    image: String::new(),
                    matches_expected: false,
                    intended_state_reached: false,
                },
            });
        }
        targets.sort_by(|a, b| a.container.cmp(&b.container));
        let evidence = Self { targets };
        evidence.validate()?;
        Ok(evidence)
    }
    pub fn validate(&self) -> Result<()> {
        if self.targets.is_empty()
            || self.targets.len() > MAX_TARGETS
            || self
                .targets
                .windows(2)
                .any(|w| w[0].container >= w[1].container)
            || self.targets.iter().any(|t| {
                !opaque_id(&t.container)
                    || t.started_at.len() > 64
                    || t.image.len() > 128
                    || (t.is_absent()
                        && (t.running || t.matches_expected || t.intended_state_reached))
            })
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        Ok(())
    }
    /// Stable digest binding a decision to the evidence the administrator saw.
    pub fn digest(&self) -> Result<String> {
        self.validate()?;
        let body = serde_json::to_string(self).map_err(|_| Error(ErrorCode::InvalidInput))?;
        Ok(hex::encode(Sha256::digest(
            format!("limeos-resolution-evidence-v1:{body}").as_bytes(),
        )))
    }
    /// Targets that make `declared` unavailable (the per-target diff). Empty
    /// means the declaration is allowed.
    pub fn blocking(&self, declared: DeclaredOutcome) -> Vec<&ResolutionTarget> {
        self.targets
            .iter()
            .filter(|t| match declared {
                DeclaredOutcome::VerifiedDone => !t.intended_state_reached,
                DeclaredOutcome::NotDone => !t.matches_expected,
                DeclaredOutcome::AcceptedDivergence => false,
            })
            .collect()
    }
    /// `Conflict` when the evidence disagrees with the declaration.
    pub fn check(&self, declared: DeclaredOutcome) -> Result<()> {
        self.validate()?;
        if self.blocking(declared).is_empty() {
            Ok(())
        } else {
            Err(Error(ErrorCode::Conflict))
        }
    }
}

/// The reason is mandatory for every outcome and longer for divergence.
pub fn validate_reason(declared: DeclaredOutcome, reason: &str) -> Result<()> {
    let length = reason.trim().chars().count();
    if reason.trim().len() != reason.len()
        || reason.chars().any(char::is_control)
        || length == 0
        || reason.chars().count() > MAX_RESOLUTION_REASON
        || (declared == DeclaredOutcome::AcceptedDivergence && length < MIN_DIVERGENCE_REASON)
    {
        return Err(Error(ErrorCode::InvalidInput));
    }
    Ok(())
}

/// One target's outcome as the executor recorded it in the job's receipt. For
/// a container job there is exactly one; for a stack, one per member.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ResolutionMember {
    /// Full container ID, without the `container:` prefix.
    pub container: String,
    pub state: crate::ExecutionState,
    pub error: Option<ErrorCode>,
}

/// A recorded request (step 1). The job stays locked until the executor has
/// confirmed and core has completed it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ResolutionRequest {
    pub job: String,
    /// Administrator principal ID.
    pub resolved_by: String,
    pub declared: DeclaredOutcome,
    pub reason: String,
    pub evidence: ResolutionEvidence,
    pub evidence_digest: String,
    #[ts(type = "number")]
    pub requested_at: i64,
}
impl ResolutionRequest {
    pub fn validate(&self) -> Result<()> {
        validate_reason(self.declared, &self.reason)?;
        if !opaque_id(&self.job)
            || !opaque_id(&self.resolved_by)
            || self.evidence.digest()? != self.evidence_digest
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        self.evidence.check(self.declared)
    }
}

/// The executor's confirmation that its row is `resolved` and its unique-index
/// slot is free. Returned for the original request and for every repeat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ResolveReceipt {
    pub kind: ResolveKind,
    pub action: String,
    pub evidence_digest: String,
    pub declared: DeclaredOutcome,
    pub resolved_by: String,
}

/// The completed audit record (step 3). Additional to, never replacing, the
/// job's `outcome_unknown`/`needs_intervention` history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct JobResolution {
    pub job: String,
    pub resolved_by: String,
    pub declared: DeclaredOutcome,
    pub reason: String,
    pub evidence: ResolutionEvidence,
    pub evidence_digest: String,
    #[ts(type = "number")]
    pub requested_at: i64,
    #[ts(type = "number")]
    pub resolved_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snap(n: char, running: bool, started: &str) -> ContainerSnapshot {
        ContainerSnapshot {
            resource: format!("container:{}", n.to_string().repeat(64)),
            image: format!("sha256:{}", "1".repeat(64)),
            started_at: started.into(),
            running,
        }
    }
    const T0: &str = "2026-01-01T00:00:00.000000000Z";
    const T1: &str = "2026-01-01T00:00:05.000000000Z";
    #[test]
    fn evidence_classifies_each_target_and_digest_is_stable() {
        let a = snap('a', true, T0);
        let b = snap('b', true, T0);
        let restarted = snap('a', true, T1);
        let e = ResolutionEvidence::observe(
            ContainerAction::Restart,
            &[&b, &a],
            &[Some(b.clone()), Some(restarted.clone())],
        )
        .unwrap();
        assert_eq!(e.targets[0].container, "a".repeat(64));
        assert!(e.targets[0].intended_state_reached && !e.targets[0].matches_expected);
        assert!(!e.targets[1].intended_state_reached && e.targets[1].matches_expected);
        assert_eq!(e.digest().unwrap(), e.clone().digest().unwrap());
        let other = ResolutionEvidence::observe(
            ContainerAction::Restart,
            &[&a, &b],
            &[Some(a.clone()), Some(b.clone())],
        )
        .unwrap();
        assert_ne!(e.digest().unwrap(), other.digest().unwrap());
        assert!(e.check(DeclaredOutcome::VerifiedDone).is_err());
        assert!(e.check(DeclaredOutcome::NotDone).is_err());
        assert!(e.check(DeclaredOutcome::AcceptedDivergence).is_ok());
        assert!(other.check(DeclaredOutcome::NotDone).is_ok());
        assert_eq!(e.blocking(DeclaredOutcome::NotDone).len(), 1);
    }
    #[test]
    fn absent_container_proves_neither_outcome() {
        let a = snap('a', true, T0);
        let e = ResolutionEvidence::observe(ContainerAction::Stop, &[&a], &[None]).unwrap();
        assert!(e.targets[0].is_absent());
        assert!(e.check(DeclaredOutcome::VerifiedDone).is_err());
        assert!(e.check(DeclaredOutcome::NotDone).is_err());
        assert!(e.check(DeclaredOutcome::AcceptedDivergence).is_ok());
    }
    #[test]
    fn reason_bounds() {
        use DeclaredOutcome::*;
        assert!(validate_reason(NotDone, "x").is_ok());
        assert!(validate_reason(NotDone, "").is_err());
        assert!(validate_reason(NotDone, " padded ").is_err());
        assert!(validate_reason(NotDone, &"x".repeat(500)).is_ok());
        assert!(validate_reason(NotDone, &"x".repeat(501)).is_err());
        assert!(validate_reason(VerifiedDone, "bad\nline").is_err());
        assert!(validate_reason(AcceptedDivergence, &"x".repeat(19)).is_err());
        assert!(validate_reason(AcceptedDivergence, &"x".repeat(20)).is_ok());
    }
    #[test]
    fn only_needs_intervention_resolves() {
        use crate::JobState::*;
        for from in [
            Planned,
            WaitingApproval,
            Queued,
            Running,
            Verifying,
            Succeeded,
            Failed,
            Canceled,
            PreconditionChanged,
            OutcomeUnknown,
            Resolved,
        ] {
            assert!(!from.allows(Resolved));
        }
        assert!(NeedsIntervention.allows(Resolved));
        assert!(!Resolved.allows(Queued));
    }
}
