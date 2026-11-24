//! Protected effect receipts. Persistence is not consumer or launch authority.
pub mod mounts;

#[cfg(test)]
use crate::journal::{LOCK, ProcessLock};
use crate::{
    Failure, Result,
    journal::{self, Receipt},
};
use limeos_domain::{
    MountEffectQuiescence, MountExecutionState, MountHelperIdentity, STORAGE_MOUNT_RECEIPT_BYTES,
    StorageMountEffectPlan, StorageMountReceipt,
};
#[cfg(test)]
use rustix::fs::{FlockOperation, Mode, OFlags};
#[cfg(test)]
use std::path::Path;

const SCHEMA: &str = "BEGIN IMMEDIATE; CREATE TABLE receipts(action TEXT PRIMARY KEY,digest TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','effect_accepted','verified','precondition_changed','outcome_unknown')),body TEXT NOT NULL) STRICT; CREATE TABLE locks(resource TEXT PRIMARY KEY,action TEXT NOT NULL REFERENCES receipts(action)) STRICT; PRAGMA user_version=1; COMMIT;";

/// The mounts journal (`mounts.sqlite`), the shared storage receipt journal
/// (`journal`) with the mounts' launch cursor. Only trusted runtime
/// orchestration writes it, after consuming genuine admission/supervision
/// owners; wire receipts are stored facts, never a public request to grant
/// execution or termination proof.
pub type MountJournal = journal::Journal<StorageMountReceipt, Option<FreshLaunch>>;
/// The receipt this open's own `begin` recorded, until it binds a helper:
/// the only receipt `arm` and `bind_helper` accept. Never constructed from
/// a stored or imported receipt.
pub struct FreshLaunch {
    action: String,
    digest: String,
    armed: bool,
}
impl Receipt for StorageMountReceipt {
    const DATABASE: &'static str = "mounts.sqlite";
    const SCHEMA: &'static str = SCHEMA;
    const BODY_BYTES: usize = STORAGE_MOUNT_RECEIPT_BYTES;
    const UNSAFE_DIRECTORY: Failure = Failure::StateNotDurable;
    fn action(&self) -> &str {
        &self.plan.action
    }
    fn digest(&self) -> &str {
        &self.digest
    }
    fn state_name(&self) -> &'static str {
        name(self.state)
    }
    fn holds_claims(&self) -> bool {
        !terminal(self.state)
    }
    fn claims(&self) -> Vec<String> {
        self.plan.resources()
    }
    fn plan_digest(&self) -> Result<String> {
        Ok(limeos_identity::digest(&encoded(&self.plan)?))
    }
    fn admit(&self) -> bool {
        self.validate().is_ok()
    }
    fn same_subject(&self, next: &Self) -> bool {
        self.plan == next.plan && self.digest == next.digest
    }
}
/// One-use release authority, minted by a fresh durable helper-binding commit.
/// Serialized receipts, replay lookups and a matching digest cannot construct it.
/// ```compile_fail
/// use limeos_executor_storage::effects::MountLaunchPermit;
/// let imported: MountLaunchPermit = serde_json::from_str("{}").unwrap();
/// ```
/// ```compile_fail
/// use limeos_executor_storage::effects::MountLaunchPermit;
/// fn consume(_: MountLaunchPermit) {}
/// fn replay(permit: MountLaunchPermit) { consume(permit); consume(permit); }
/// ```
pub struct MountLaunchPermit {
    action: String,
    digest: String,
    helper: MountHelperIdentity,
    expires_at: i64,
}
impl MountLaunchPermit {
    pub fn action(&self) -> &str {
        &self.action
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn helper(&self) -> &MountHelperIdentity {
        &self.helper
    }
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
    pub fn check_binding(
        &self,
        action: &str,
        digest: &str,
        helper: &MountHelperIdentity,
        at: i64,
    ) -> Result<()> {
        if self.action != action
            || self.digest != digest
            || self.helper != *helper
            || at >= self.expires_at
        {
            return Err(Failure::Conflict);
        }
        Ok(())
    }
}
fn encoded(value: &impl serde::Serialize) -> Result<String> {
    journal::encoded_within(value, STORAGE_MOUNT_RECEIPT_BYTES)
}
impl MountJournal {
    pub fn lookup(
        &self,
        action: &str,
        digest: Option<&str>,
    ) -> Result<Option<StorageMountReceipt>> {
        self.stored(action, digest)
    }
    /// Durable receipt and all root claims precede any possible helper launch.
    pub fn begin(&mut self, plan: &StorageMountEffectPlan, at: i64) -> Result<StorageMountReceipt> {
        // Replay or failure cannot preserve a prior publication's release cursor.
        self.cursor = None;
        plan.validate(plan.created_at)
            .map_err(|_| Failure::InvalidPlan)?;
        let digest = limeos_identity::digest(&encoded(plan)?);
        if let Some(existing) = self.lookup(&plan.action, Some(&digest))? {
            return Ok(existing);
        }
        plan.validate(at).map_err(|_| Failure::InvalidPlan)?;
        let receipt = StorageMountReceipt {
            plan: plan.clone(),
            digest,
            state: MountExecutionState::Prepared,
            quiescence: MountEffectQuiescence::NotLaunched {},
            after: None,
        };
        self.insert(&receipt)?;
        self.cursor = Some(FreshLaunch {
            action: receipt.plan.action.clone(),
            digest: receipt.digest.clone(),
            armed: false,
        });
        Ok(receipt)
    }
    /// Mark uncertainty durably before creation/release of any gated helper.
    pub fn arm(&mut self, receipt: &StorageMountReceipt) -> Result<StorageMountReceipt> {
        if receipt.state != MountExecutionState::Prepared
            || receipt.quiescence != (MountEffectQuiescence::NotLaunched {})
            || !self.fresh_matches(receipt, false)
        {
            return Err(Failure::Conflict);
        }
        let mut next = receipt.clone();
        next.quiescence = MountEffectQuiescence::Pending { helper: None };
        if let Err(error) = self.record(receipt, &next) {
            self.cursor = None;
            return Err(error);
        }
        self.cursor.as_mut().ok_or(Failure::Conflict)?.armed = true;
        Ok(next)
    }
    /// A gated supervisor supplies this actual identity before releasing mount.
    pub fn bind_helper(
        &mut self,
        receipt: &StorageMountReceipt,
        helper: MountHelperIdentity,
    ) -> Result<(StorageMountReceipt, MountLaunchPermit)> {
        if receipt.state != MountExecutionState::Prepared
            || receipt.quiescence != (MountEffectQuiescence::Pending { helper: None })
            || !self.fresh_matches(receipt, true)
        {
            return Err(Failure::Conflict);
        }
        let mut next = receipt.clone();
        next.quiescence = MountEffectQuiescence::Pending {
            helper: Some(helper.clone()),
        };
        if let Err(error) = self.record(receipt, &next) {
            self.cursor = None;
            return Err(error);
        }
        self.cursor.take();
        let permit = MountLaunchPermit {
            action: next.plan.action.clone(),
            digest: next.digest.clone(),
            helper,
            expires_at: next.plan.expires_at,
        };
        Ok((next, permit))
    }
    fn fresh_matches(&self, receipt: &StorageMountReceipt, armed: bool) -> bool {
        self.cursor.as_ref().is_some_and(|fresh| {
            fresh.action == receipt.plan.action
                && fresh.digest == receipt.digest
                && fresh.armed == armed
        })
    }
    /// Refine only a known accepted effect with already recorded owned-helper
    /// termination. The composition root supplies a separate genuine fresh
    /// mounted observation; this never launches, retries or refreshes expiry.
    pub fn verify_accepted(
        &mut self,
        receipt: &StorageMountReceipt,
        independent: &limeos_domain::StorageMountedSnapshot,
    ) -> Result<StorageMountReceipt> {
        if receipt.state != MountExecutionState::EffectAccepted
            || !receipt.quiescence.succeeded()
            || receipt.after.is_none()
        {
            return Err(Failure::Conflict);
        }
        limeos_domain::verify_storage_mount(&receipt.plan, receipt, independent)
            .map_err(|_| Failure::Conflict)?;
        let mut verified = receipt.clone();
        verified.state = MountExecutionState::Verified;
        self.record(receipt, &verified)?;
        Ok(verified)
    }
    /// Persist trusted orchestration's verified facts. This is not an RPC and
    /// must never be called with client-supplied receipt/quiescence bodies.
    pub fn record(
        &mut self,
        before: &StorageMountReceipt,
        next: &StorageMountReceipt,
    ) -> Result<()> {
        let result = self.record_inner(before, next);
        if result.is_err() {
            self.cursor = None;
        }
        result
    }
    fn record_inner(
        &mut self,
        before: &StorageMountReceipt,
        next: &StorageMountReceipt,
    ) -> Result<()> {
        before.validate().map_err(|_| Failure::InvalidPlan)?;
        next.validate().map_err(|_| Failure::InvalidPlan)?;
        self.replace(before, next, transition)
    }
}
fn terminal(state: MountExecutionState) -> bool {
    matches!(
        state,
        MountExecutionState::Verified | MountExecutionState::PreconditionChanged
    )
}
fn name(state: MountExecutionState) -> &'static str {
    match state {
        MountExecutionState::Prepared => "prepared",
        MountExecutionState::EffectAccepted => "effect_accepted",
        MountExecutionState::Verified => "verified",
        MountExecutionState::PreconditionChanged => "precondition_changed",
        MountExecutionState::OutcomeUnknown => "outcome_unknown",
    }
}
fn transition(before: &StorageMountReceipt, next: &StorageMountReceipt) -> bool {
    let quiescence = match (&before.quiescence, &next.quiescence) {
        (a, b) if a == b => true,
        (
            MountEffectQuiescence::NotLaunched {},
            MountEffectQuiescence::Pending { helper: None },
        ) => true,
        (
            MountEffectQuiescence::Pending { helper: None },
            MountEffectQuiescence::Pending { helper: Some(_) },
        ) => true,
        (
            MountEffectQuiescence::Pending { helper: Some(a) },
            MountEffectQuiescence::Reaped { helper: b, .. },
        ) => a == b,
        _ => false,
    };
    let state = matches!(
        (before.state, next.state),
        (
            MountExecutionState::Prepared,
            MountExecutionState::Prepared
                | MountExecutionState::OutcomeUnknown
                | MountExecutionState::EffectAccepted
                | MountExecutionState::PreconditionChanged,
        ) | (
            MountExecutionState::OutcomeUnknown,
            MountExecutionState::OutcomeUnknown
                | MountExecutionState::EffectAccepted
                | MountExecutionState::Verified
                | MountExecutionState::PreconditionChanged,
        ) | (
            MountExecutionState::EffectAccepted,
            MountExecutionState::EffectAccepted
                | MountExecutionState::Verified
                | MountExecutionState::OutcomeUnknown,
        )
    );
    quiescence && state
}

#[cfg(test)]
mod tests;
