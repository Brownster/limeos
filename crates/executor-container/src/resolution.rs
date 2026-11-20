//! Operator resolution of a stuck action (`ResolveAction`).
//!
//! Core has already recorded an administrator's request. Here the executor
//! settles its own side: it re-inspects the action's targets read-only, refuses
//! if the result no longer hashes to the evidence the administrator saw, and
//! moves the action row (and a stack's active member rows) to `resolved` in one
//! transaction. `resolved` is outside both unique-index `WHERE` lists, so the
//! resource frees. The stored receipt is kept as recorded. No Engine start,
//! stop or restart is ever issued from this path.
use super::*;
use limeos_domain::{
    DeclaredOutcome, ResolutionEvidence, ResolveKind, ResolveReceipt, StackReceipt,
};

pub const RESOLVED: &str = "resolved";
const ACTIVE: &str = "('prepared','effect_accepted','outcome_unknown')";

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
    /// `Conflict`. A missing row is `NotFound` (core completes the resolution
    /// itself once the plan is past `receipt_absence_proves_unsent`, since a
    /// pruned row held nothing); a row that is not holding the
    /// resource (already verified or never sent) is `Conflict`. A changed
    /// target (digest mismatch) or a declaration the evidence does not support
    /// is `Conflict`. Only read-only inspections reach the Engine.
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
    ) -> Result<ResolveReceipt> {
        if ![action, digest, evidence_digest, resolved_by]
            .into_iter()
            .all(opaque_id)
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let confirmation = ResolveReceipt {
            kind,
            action: action.into(),
            evidence_digest: evidence_digest.into(),
            declared,
            resolved_by: resolved_by.into(),
        };
        // Authorize from the pure lookup, as for every other action.
        let (operation, before) = match kind {
            ResolveKind::Container => {
                let receipt = self
                    .receipt(action, digest)?
                    .ok_or(Error(ErrorCode::NotFound))?;
                ceiling.authorize(peer_uid, &receipt.before.resource, receipt.operation)?;
                (receipt.operation, vec![receipt.before])
            }
            ResolveKind::Stack => {
                let receipt: StackReceipt = self
                    .stack_receipt(action, digest)?
                    .ok_or(Error(ErrorCode::NotFound))?;
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
                (receipt.operation, targets)
            }
        };
        if let Some(stored) = self.resolution(kind, action)? {
            return if stored == confirmation {
                Ok(stored)
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
        if !["prepared", "effect_accepted", "outcome_unknown"].contains(&state.as_str()) {
            return Err(Error(ErrorCode::Conflict));
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
                &format!("UPDATE {table} SET state='resolved',resolution=? WHERE action=? AND digest=? AND state IN {ACTIVE}"),
                params![body, action, digest],
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
        Ok(confirmation)
    }
}

#[cfg(test)]
mod tests;
