//! Whole-stack start, stop and restart receipts.
//!
//! The stack row and one `actions` row per target are written together BEFORE
//! any Engine call, so the existing per-container unique lock also excludes a
//! single-container action on any target. Each later change updates the member
//! row and the stack aggregate in one transaction. Recovery reads receipts
//! (closing an action no execute recorded, see `closure`) and verifies by
//! fresh inspection; no effect is ever resent.
use super::*;
use crate::resolution::RESOLVED;
use limeos_domain::{
    StackMemberReceipt, StackReceipt, StackRequest, StackSnapshot, aggregate_state,
    verify_stack_member,
};

const ACTIVE: &str = "('prepared','effect_accepted','outcome_unknown')";
/// Last-resort cap; retention keeps normal operation far below it.
const MAX_STACK_ROWS: u32 = 8192;

/// The first write of a stack receipt: its plan's expiry and the validating
/// clock. With `close`, the row is a closure (`closure`).
#[derive(Clone, Copy)]
pub(crate) struct Insert {
    pub(crate) expires_at: i64,
    pub(crate) now: i64,
    pub(crate) close: bool,
}

/// The member's own protected row: a container receipt under a derived action.
pub fn member_action(action: &str, resource: &str) -> String {
    limeos_identity::digest(&format!("stack-member:{action}:{resource}"))
}
fn member_receipt(stack: &StackReceipt, member: &StackMemberReceipt) -> Result<ContainerReceipt> {
    let before = stack
        .before
        .members
        .iter()
        .find(|m| m.resource == member.resource)
        .ok_or(Error(ErrorCode::StateNotDurable))?
        .clone();
    Ok(ContainerReceipt {
        action: member_action(&stack.action, &member.resource),
        plan_digest: stack.plan_digest.clone(),
        operation: stack.operation,
        before,
        state: member.state,
        error: member.error,
    })
}
fn unique(error: rusqlite::Error) -> Error {
    match error {
        rusqlite::Error::SqliteFailure(code, _)
            if code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
        {
            Error(ErrorCode::Conflict)
        }
        other => durable(other),
    }
}

impl ReceiptStore {
    /// Pure lookup with stack and member rows cross-checked.
    pub fn stack_receipt(&self, action: &str, digest: &str) -> Result<Option<StackReceipt>> {
        if !opaque_id(action) || !opaque_id(digest) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let row: Option<(String, String, String, String, String)> = self
            .conn
            .query_row(
                "SELECT digest,resource,state,receipt,operation FROM stack_actions WHERE action=?",
                [action],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
            .map_err(durable)?;
        let Some((stored, resource, state, body, operation)) = row else {
            return Ok(None);
        };
        if stored != digest {
            return Err(Error(ErrorCode::Conflict));
        }
        let receipt: StackReceipt =
            serde_json::from_str(&body).map_err(|_| Error(ErrorCode::StateNotDurable))?;
        if receipt.action != action
            || receipt.plan_digest != digest
            || receipt.before.resource != resource
            || (state != receipt.state.as_str() && state != RESOLVED)
            || operation != receipt.operation.as_str()
            || receipt.validate().is_err()
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        // The stack aggregate and every member row change in one transaction.
        for member in &receipt.members {
            let expected = member_receipt(&receipt, member)?;
            let found = self.receipt(&expected.action, digest)?;
            if found.as_ref() != Some(&expected) {
                return Err(Error(ErrorCode::StateNotDurable));
            }
        }
        Ok(Some(receipt))
    }

    /// Lookup for recovery. A member still `Prepared` when no call owns the
    /// lock may or may not have been sent, so it becomes `outcome_unknown`.
    /// This never contacts the Engine.
    pub fn reconcile_stack(&mut self, action: &str, digest: &str) -> Result<Option<StackReceipt>> {
        let Some(mut receipt) = self.stack_receipt(action, digest)? else {
            return Ok(None);
        };
        // Resolved rows are closed; nothing is converted or rewritten.
        if self
            .resolution(limeos_domain::ResolveKind::Stack, action)?
            .is_some()
        {
            return Ok(Some(receipt));
        }
        let pending: Vec<_> = receipt
            .members
            .iter()
            .filter(|m| m.state == ExecutionState::Prepared)
            .map(|m| m.resource.clone())
            .collect();
        if pending.is_empty() {
            return Ok(Some(receipt));
        }
        for member in &mut receipt.members {
            if member.state == ExecutionState::Prepared {
                member.state = ExecutionState::OutcomeUnknown;
                member.error = Some(ErrorCode::StateNotDurable);
            }
        }
        receipt.state = aggregate_state(&receipt.members);
        self.write_stack(&receipt, &pending, None)?;
        Ok(Some(receipt))
    }

    /// Insert (first write) or update, with member rows, in one transaction.
    /// A closing insert judges the absence in that transaction, before pruning
    /// can raise the watermark, records it in the stack row's `closed` and
    /// returns it.
    pub(crate) fn write_stack(
        &mut self,
        receipt: &StackReceipt,
        changed: &[String],
        insert: Option<Insert>,
    ) -> Result<Option<limeos_domain::ReceiptAbsence>> {
        if insert.is_some()
            && fs2::available_space(&self.directory)
                .map_err(|_| Error(ErrorCode::StateNotDurable))?
                < 8 * 1024 * 1024
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let body = encode(receipt)?;
        let tx = self.conn.transaction().map_err(durable)?;
        let mut closed = None;
        if let Some(Insert {
            expires_at,
            now,
            close,
        }) = insert
        {
            if close {
                closed = Some(retention::absence_in(
                    &tx,
                    limeos_domain::ResolveKind::Stack,
                    &receipt.action,
                    expires_at,
                )?);
            }
            retention::prune(&tx, now)?;
            let stacks: u32 = tx
                .query_row("SELECT count(*) FROM stack_actions", [], |r| r.get(0))
                .map_err(durable)?;
            let actions: u32 = tx
                .query_row("SELECT count(*) FROM actions", [], |r| r.get(0))
                .map_err(durable)?;
            if stacks >= MAX_STACK_ROWS
                || actions.saturating_add(receipt.members.len() as u32) > MAX_STACK_ROWS
            {
                return Err(Error(ErrorCode::Overloaded));
            }
            tx.execute(
                "INSERT INTO stack_actions(action,digest,resource,state,receipt,operation,expires_at,closed) VALUES(?,?,?,?,?,?,?,?)",
                params![
                    receipt.action,
                    receipt.plan_digest,
                    receipt.before.resource,
                    receipt.state.as_str(),
                    body,
                    receipt.operation.as_str(),
                    expires_at,
                    closed.map(|c: limeos_domain::ReceiptAbsence| c.as_str())
                ],
            )
            .map_err(unique)?;
            for member in &receipt.members {
                let row = member_receipt(receipt, member)?;
                tx.execute(
                    "INSERT INTO actions(action,digest,resource,state,receipt,operation,expires_at) VALUES(?,?,?,?,?,?,?)",
                    params![
                        row.action,
                        row.plan_digest,
                        row.before.resource,
                        row.state.as_str(),
                        encode(&row)?,
                        row.operation.as_str(),
                        expires_at
                    ],
                )
                .map_err(unique)?;
            }
        } else {
            for resource in changed {
                let member = receipt
                    .member(resource)
                    .ok_or(Error(ErrorCode::StateNotDurable))?;
                let row = member_receipt(receipt, member)?;
                if tx
                    .execute(
                        &format!("UPDATE actions SET state=?,receipt=? WHERE action=? AND digest=? AND state IN {ACTIVE}"),
                        params![row.state.as_str(), encode(&row)?, row.action, row.plan_digest],
                    )
                    .map_err(durable)?
                    != 1
                {
                    return Err(Error(ErrorCode::Conflict));
                }
            }
            if tx
                .execute(
                    &format!("UPDATE stack_actions SET state=?,receipt=? WHERE action=? AND digest=? AND state IN {ACTIVE}"),
                    params![receipt.state.as_str(), body, receipt.action, receipt.plan_digest],
                )
                .map_err(durable)?
                != 1
            {
                return Err(Error(ErrorCode::Conflict));
            }
        }
        tx.commit().map_err(durable)?;
        if insert.is_some() {
            File::open(&self.directory)
                .and_then(|f| f.sync_all())
                .map_err(|_| Error(ErrorCode::StateNotDurable))?;
        }
        Ok(closed)
    }

    /// Record a refusal that happened before any effect. The row is terminal,
    /// so it holds no lock, and a retry returns it instead of re-listing.
    fn refuse_stack(
        &mut self,
        request: &StackRequest,
        error: ErrorCode,
        now: i64,
    ) -> Result<StackReceipt> {
        let targets = request.plan.targets();
        let members: Vec<_> = targets
            .iter()
            .map(|t| StackMemberReceipt {
                resource: t.resource.clone(),
                state: ExecutionState::PreconditionChanged,
                error: Some(error),
            })
            .collect();
        let receipt = StackReceipt {
            action: request.action.clone(),
            plan_digest: request.plan_digest.clone(),
            operation: request.plan.operation,
            before: request.plan.expected.clone(),
            state: aggregate_state(&members),
            members,
            error: Some(error),
        };
        // Terminal rows hold no lock; member rows keep lookup cross-checks uniform.
        self.write_stack(
            &receipt,
            &[],
            Some(Insert {
                expires_at: request.plan.expires_at,
                now,
                close: false,
            }),
        )?;
        Ok(receipt)
    }

    pub async fn execute_stack(
        &mut self,
        engine: &impl Engine,
        ceiling: &ContainerCeiling,
        peer_uid: u32,
        request: &StackRequest,
        clock: impl Fn() -> i64,
    ) -> Result<StackReceipt> {
        ceiling.authorize_stack(peer_uid, &request.plan.expected, request.plan.operation)?;
        if !opaque_id(&request.action)
            || limeos_identity::digest(&encode(&request.plan)?) != request.plan_digest
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        if let Some(receipt) = self.stack_receipt(&request.action, &request.plan_digest)? {
            if receipt.operation != request.plan.operation {
                return Err(Error(ErrorCode::Conflict));
            }
            // Taken by a closure: refused whatever the clock says (`closure`).
            crate::closure::refuse_if_unproven(
                self.closure(limeos_domain::ResolveKind::Stack, &request.action)?,
            )?;
            return Ok(receipt);
        }
        request.plan.validate(clock())?;
        let project = request.plan.expected.project()?;
        let listed = tokio::time::timeout(Duration::from_secs(10), engine.project_members(project))
            .await
            .map_err(|_| Error(ErrorCode::Unavailable))??;
        // Any difference, in any member, is refused whole: no partial guess.
        let drifted = StackSnapshot::new(project, listed)
            .ok()
            .is_none_or(|actual| request.plan.check_current(&actual, clock()).is_err());
        if drifted {
            return self.refuse_stack(request, ErrorCode::Conflict, clock());
        }
        let targets = request.plan.targets();
        let members: Vec<_> = targets
            .iter()
            .map(|t| StackMemberReceipt {
                resource: t.resource.clone(),
                state: ExecutionState::Prepared,
                error: None,
            })
            .collect();
        let mut receipt = StackReceipt {
            action: request.action.clone(),
            plan_digest: request.plan_digest.clone(),
            operation: request.plan.operation,
            before: request.plan.expected.clone(),
            state: aggregate_state(&members),
            members,
            error: None,
        };
        self.write_stack(
            &receipt,
            &[],
            Some(Insert {
                expires_at: request.plan.expires_at,
                now: clock(),
                close: false,
            }),
        )?;
        // Sync may take time. Recheck expiry immediately before initiating IPC.
        if let Err(e) = request.plan.validate(clock()) {
            for member in &mut receipt.members {
                member.state = ExecutionState::PreconditionChanged;
                member.error = Some(e.0);
            }
            receipt.state = aggregate_state(&receipt.members);
            receipt.error = Some(e.0);
            let all: Vec<_> = receipt.members.iter().map(|m| m.resource.clone()).collect();
            self.write_stack(&receipt, &all, None)?;
            return Ok(receipt);
        }
        let operation = request.plan.operation;
        let budget = Duration::from_secs(
            limeos_domain::stack_definition(operation)
                .timeout_seconds
                .into(),
        );
        for index in 0..receipt.members.len() {
            let id = receipt.members[index]
                .resource
                .strip_prefix("container:")
                .ok_or(Error(ErrorCode::InvalidInput))?
                .to_owned();
            // Each earlier effect takes time. Re-inspect this one target and
            // refuse, fail-stop, if it no longer matches what the plan expected.
            let expected = request
                .plan
                .expected
                .members
                .iter()
                .find(|m| m.resource == receipt.members[index].resource)
                .ok_or(Error(ErrorCode::StateNotDurable))?;
            let current = tokio::time::timeout(Duration::from_secs(2), engine.inspect(&id)).await;
            if !matches!(current, Ok(Ok(ref value)) if value == expected) {
                let mut changed = Vec::new();
                for later in &mut receipt.members[index..] {
                    later.state = ExecutionState::PreconditionChanged;
                    later.error = Some(ErrorCode::Conflict);
                    changed.push(later.resource.clone());
                }
                receipt.error = Some(ErrorCode::Conflict);
                receipt.state = aggregate_state(&receipt.members);
                // Nothing was sent for these, so even a failed write is accurate;
                // the Prepared rows remain and recovery treats them as uncertain.
                let _ = self.write_stack(&receipt, &changed, None);
                return Ok(receipt);
            }
            let result = tokio::time::timeout(budget, async {
                match operation {
                    ContainerAction::Restart => engine.restart(&id).await,
                    ContainerAction::Start => engine.start(&id).await,
                    ContainerAction::Stop => engine.stop(&id).await,
                }
            })
            .await;
            let accepted = matches!(result, Ok(Ok(())));
            let mut changed = vec![receipt.members[index].resource.clone()];
            match result {
                Ok(Ok(())) => receipt.members[index].state = ExecutionState::EffectAccepted,
                Ok(Err(e)) => {
                    receipt.members[index].state = ExecutionState::OutcomeUnknown;
                    receipt.members[index].error = Some(e.0);
                }
                Err(_) => {
                    receipt.members[index].state = ExecutionState::OutcomeUnknown;
                    receipt.members[index].error = Some(ErrorCode::Unavailable);
                }
            }
            if !accepted {
                // Fail-stop. Later targets were never sent.
                for later in &mut receipt.members[index + 1..] {
                    later.state = ExecutionState::PreconditionChanged;
                    later.error = Some(ErrorCode::Conflict);
                    changed.push(later.resource.clone());
                }
                receipt.error = receipt.members[index].error;
            }
            receipt.state = aggregate_state(&receipt.members);
            if self.write_stack(&receipt, &changed, None).is_err() {
                // The effect may have occurred. The protected Prepared rows
                // remain the barrier to replay; never claim nothing was sent.
                receipt.members[index].state = ExecutionState::OutcomeUnknown;
                receipt.members[index].error = Some(ErrorCode::StateNotDurable);
                receipt.state = aggregate_state(&receipt.members);
                receipt.error = Some(ErrorCode::StateNotDurable);
                return Ok(receipt);
            }
            if !accepted {
                break;
            }
        }
        Ok(receipt)
    }

    /// Release locks only for members whose own fresh inspection proves the
    /// intended state. Other members keep their evidence; partial success is
    /// a result, not an error. Never replays an effect.
    pub async fn verify_stack(
        &mut self,
        engine: &impl Engine,
        ceiling: &ContainerCeiling,
        peer_uid: u32,
        action: &str,
        digest: &str,
        after: &[ContainerSnapshot],
    ) -> Result<StackReceipt> {
        let found = self
            .stack_receipt(action, digest)?
            .ok_or(Error(ErrorCode::NotFound))?;
        // Authorize from the pure lookup before any bookkeeping write.
        ceiling.authorize_stack(peer_uid, &found.before, found.operation)?;
        if self
            .resolution(limeos_domain::ResolveKind::Stack, action)?
            .is_some()
        {
            return Err(Error(ErrorCode::Conflict));
        }
        let mut receipt = self
            .reconcile_stack(action, digest)?
            .ok_or(Error(ErrorCode::NotFound))?;
        if after.len() > limeos_domain::MAX_STACK_MEMBERS {
            return Err(Error(ErrorCode::InvalidInput));
        }
        if receipt.state == ExecutionState::Verified {
            return Ok(receipt);
        }
        let mut changed = Vec::new();
        for index in 0..receipt.members.len() {
            let member = &receipt.members[index];
            if !matches!(
                member.state,
                ExecutionState::EffectAccepted | ExecutionState::OutcomeUnknown
            ) {
                continue;
            }
            let Some(proposed) = after.iter().find(|a| a.resource == member.resource) else {
                continue;
            };
            let before = receipt
                .before
                .members
                .iter()
                .find(|m| m.resource == member.resource)
                .ok_or(Error(ErrorCode::StateNotDurable))?;
            if verify_stack_member(receipt.operation, before, proposed).is_err() {
                continue;
            }
            let id = member
                .resource
                .strip_prefix("container:")
                .ok_or(Error(ErrorCode::InvalidInput))?;
            let current = tokio::time::timeout(Duration::from_secs(2), engine.inspect(id)).await;
            if !matches!(current, Ok(Ok(ref value)) if value == proposed) {
                continue;
            }
            receipt.members[index].state = ExecutionState::Verified;
            receipt.members[index].error = None;
            changed.push(receipt.members[index].resource.clone());
        }
        if changed.is_empty() {
            return Ok(receipt);
        }
        receipt.state = aggregate_state(&receipt.members);
        self.write_stack(&receipt, &changed, None)?;
        Ok(receipt)
    }
}

#[cfg(test)]
mod tests;
