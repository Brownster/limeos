//! Operator resolution of a stuck action (`ResolveAction`).
//!
//! Core has already recorded an administrator's request. Here the executor
//! settles its own side: it re-inspects the action's targets read-only, refuses
//! if the result no longer hashes to the evidence the administrator saw, and
//! moves the action row (and a stack's active member rows) to `resolved` in one
//! transaction. `resolved` is outside both unique-index `WHERE` lists, so the
//! resource frees. The stored receipt is kept as recorded until retention
//! prunes the settled row (`retention`). No Engine start, stop or restart is
//! ever issued from this path.
//!
//! A terminal receipt (`verified`, `precondition_changed`, a stack's
//! `partly_done`) holds no lock, but core may still hold the job: it crashed
//! before recording the result, or its own evidence check failed. Such a row is
//! resolvable too, with the same authorization and fresh evidence, provided the
//! declaration agrees with the recorded outcome
//! (`limeos_domain::declarations_for_recorded`); otherwise the answer is a
//! `ResolveRefusal` naming the outcome and what it allows.
use super::*;
use limeos_domain::{
    ActionPlan, DeclaredOutcome, ReceiptAbsence, ResolutionEvidence, ResolveKind, ResolveReceipt,
    ResolveRefusal, StackReceipt, declarations_for_recorded,
};

pub const RESOLVED: &str = "resolved";
const ACTIVE: &str = "('prepared','effect_accepted','outcome_unknown')";

/// What `resolve_action` came to, short of an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveOutcome {
    /// The row is `resolved`; returned for the original call and every repeat
    /// while retention keeps the row.
    Resolved(ResolveReceipt),
    /// The store held no receipt of an execute for the action, so it has
    /// nothing to free. It has closed the action (`closure`): no execute for it
    /// can run any more. Core decides what the absence means
    /// (`ReceiptAbsence`). A `never_recorded` closure is answered so only for
    /// a declaration a `precondition_changed` receipt allows
    /// (`closed_outcome`); otherwise it is `Refused`.
    Absent(ReceiptAbsence),
    /// The receipt is terminal and its recorded outcome contradicts the
    /// declaration. Nothing changed.
    Refused(ResolveRefusal),
}

/// What a closure (found or just written) answers a resolution. A
/// `never_recorded` closure proves nothing was ever sent, as a refusal an
/// execute wrote does, so it allows exactly what a `precondition_changed`
/// receipt allows (`declarations_for_recorded`), and any other declaration is
/// `Refused`. A `no_longer_held` closure claims nothing about what ran: every
/// declaration is left to the operator and core's fresh measurement.
pub(crate) fn closed_outcome(absence: ReceiptAbsence, declared: DeclaredOutcome) -> ResolveOutcome {
    let recorded = ExecutionState::PreconditionChanged;
    match (absence, declarations_for_recorded(recorded)) {
        (ReceiptAbsence::NeverRecorded, Some(allowed)) if !allowed.contains(&declared) => {
            ResolveOutcome::Refused(ResolveRefusal {
                recorded,
                declared,
                allowed: allowed.to_vec(),
            })
        }
        _ => ResolveOutcome::Absent(absence),
    }
}

impl ReceiptStore {
    /// The stored confirmation for an action, if it was resolved. Pure lookup.
    pub fn resolution(&self, kind: ResolveKind, action: &str) -> Result<Option<ResolveReceipt>> {
        let table = match kind {
            ResolveKind::Container => "actions",
            ResolveKind::Stack => "stack_actions",
        };
        let row: Option<(String, Option<String>)> = self
            .conn
            .query_row(
                &format!("SELECT state,resolution FROM {table} WHERE action=?"),
                [action],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(durable)?;
        match row {
            Some((state, Some(body))) if state == RESOLVED => {
                let value: ResolveReceipt =
                    serde_json::from_str(&body).map_err(|_| Error(ErrorCode::StateNotDurable))?;
                if value.kind != kind || value.action != action {
                    return Err(Error(ErrorCode::StateNotDurable));
                }
                Ok(Some(value))
            }
            Some((state, resolution)) if (state == RESOLVED) != resolution.is_some() => {
                Err(Error(ErrorCode::StateNotDurable))
            }
            _ => Ok(None),
        }
    }

    /// Settle one action. Idempotent by `action`: a repeat with the same
    /// evidence, declaration and administrator returns the stored confirmation
    /// without touching the Engine; anything else on a resolved row is
    /// `Conflict`. `plan` must hash to `digest` and match `kind`, or the call
    /// is `InvalidInput`. A missing row is closed (`closure`; `now` is the
    /// executor clock, for retention only) and answered `Absent`, judged
    /// against the prune watermark with that plan's own `expires_at`; a closure
    /// row already there is answered the same way, and either is `Refused` for
    /// a declaration a never-recorded closure contradicts (`closed_outcome`).
    /// A terminal row whose recorded
    /// outcome contradicts the declaration is `Refused`. A changed target
    /// (digest mismatch) or a declaration the evidence does not support is
    /// `Conflict`. Only read-only inspections reach the Engine.
    #[allow(clippy::too_many_arguments)]
    pub async fn resolve_action(
        &mut self,
        engine: &impl Engine,
        ceiling: &ContainerCeiling,
        peer_uid: u32,
        kind: ResolveKind,
        action: &str,
        digest: &str,
        evidence_digest: &str,
        declared: DeclaredOutcome,
        resolved_by: &str,
        plan: &ActionPlan,
        now: i64,
    ) -> Result<ResolveOutcome> {
        if ![action, digest, evidence_digest, resolved_by]
            .into_iter()
            .all(opaque_id)
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        crate::retention::verified_expiry(kind, digest, plan)?;
        let confirmation = ResolveReceipt {
            kind,
            action: action.into(),
            evidence_digest: evidence_digest.into(),
            declared,
            resolved_by: resolved_by.into(),
        };
        // A closure row records an absence, answered by `closed_outcome`.
        let closed = |store: &Self| store.closure(kind, action);
        // Authorize from the pure lookup, as for every other action.
        let (operation, recorded, before) = match (kind, plan) {
            (ResolveKind::Container, ActionPlan::Container(plan)) => {
                let Some(receipt) = self.receipt(action, digest)? else {
                    let (absence, _) = self.close_container(action, digest, plan, now)?;
                    return Ok(closed_outcome(absence, declared));
                };
                if let Some(absence) = closed(self)? {
                    return Ok(closed_outcome(absence, declared));
                }
                ceiling.authorize(peer_uid, &receipt.before.resource, receipt.operation)?;
                (receipt.operation, receipt.state, vec![receipt.before])
            }
            (ResolveKind::Stack, ActionPlan::Stack(plan)) => {
                let Some(receipt): Option<StackReceipt> = self.stack_receipt(action, digest)?
                else {
                    let (absence, _) = self.close_stack(action, digest, plan, now)?;
                    return Ok(closed_outcome(absence, declared));
                };
                if let Some(absence) = closed(self)? {
                    return Ok(closed_outcome(absence, declared));
                }
                ceiling.authorize_stack(peer_uid, &receipt.before, receipt.operation)?;
                let targets = receipt
                    .members
                    .iter()
                    .map(|m| {
                        receipt
                            .before
                            .members
                            .iter()
                            .find(|b| b.resource == m.resource)
                            .cloned()
                            .ok_or(Error(ErrorCode::StateNotDurable))
                    })
                    .collect::<Result<Vec<_>>>()?;
                (receipt.operation, receipt.state, targets)
            }
            // `verified_expiry` has tied the plan's kind to `kind`.
            _ => return Err(Error(ErrorCode::InvalidInput)),
        };
        if let Some(stored) = self.resolution(kind, action)? {
            return if stored == confirmation {
                Ok(ResolveOutcome::Resolved(stored))
            } else {
                Err(Error(ErrorCode::Conflict))
            };
        }
        let table = match kind {
            ResolveKind::Container => "actions",
            ResolveKind::Stack => "stack_actions",
        };
        let state: String = self
            .conn
            .query_row(
                &format!("SELECT state FROM {table} WHERE action=?"),
                [action],
                |r| r.get(0),
            )
            .map_err(durable)?;
        if state != recorded.as_str() {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        if let Some(allowed) = declarations_for_recorded(recorded) {
            if !allowed.contains(&declared) {
                return Ok(ResolveOutcome::Refused(ResolveRefusal {
                    recorded,
                    declared,
                    allowed: allowed.to_vec(),
                }));
            }
            // A terminal stack holds no lock: none of its members may either.
            if kind == ResolveKind::Stack {
                for target in &before {
                    let member: String = self
                        .conn
                        .query_row(
                            "SELECT state FROM actions WHERE action=? AND digest=?",
                            params![member_action(action, &target.resource), digest],
                            |r| r.get(0),
                        )
                        .map_err(durable)?;
                    if ["prepared", "effect_accepted", "outcome_unknown"].contains(&member.as_str())
                    {
                        return Err(Error(ErrorCode::StateNotDurable));
                    }
                }
            }
        }
        // Fresh, read-only measurement of exactly the action's targets.
        let mut current = Vec::with_capacity(before.len());
        for target in &before {
            let id = target
                .resource
                .strip_prefix("container:")
                .ok_or(Error(ErrorCode::StateNotDurable))?;
            current.push(
                match tokio::time::timeout(Duration::from_secs(2), engine.inspect(id)).await {
                    Ok(Ok(snapshot)) => Some(snapshot),
                    Ok(Err(Error(ErrorCode::NotFound))) => None,
                    Ok(Err(e)) => return Err(e),
                    Err(_) => return Err(Error(ErrorCode::Unavailable)),
                },
            );
        }
        let evidence =
            ResolutionEvidence::observe(operation, &before.iter().collect::<Vec<_>>(), &current)?;
        if evidence.digest()? != evidence_digest {
            return Err(Error(ErrorCode::Conflict));
        }
        evidence.check(declared)?;
        let body = encode(&confirmation)?;
        let tx = self.conn.transaction().map_err(durable)?;
        if tx
            .execute(
                &format!("UPDATE {table} SET state='resolved',resolution=? WHERE action=? AND digest=? AND state=?"),
                params![body, action, digest, state],
            )
            .map_err(durable)?
            != 1
        {
            return Err(Error(ErrorCode::Conflict));
        }
        if kind == ResolveKind::Stack {
            // Member rows share the digest; those already terminal keep their state.
            for target in &before {
                tx.execute(
                    &format!("UPDATE actions SET state='resolved',resolution=? WHERE action=? AND digest=? AND state IN {ACTIVE}"),
                    params![body, member_action(action, &target.resource), digest],
                )
                .map_err(durable)?;
            }
        }
        tx.commit().map_err(durable)?;
        Ok(ResolveOutcome::Resolved(confirmation))
    }
}

#[cfg(test)]
mod tests;
