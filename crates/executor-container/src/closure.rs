//! Closing an action the receipt store holds no row for.
//!
//! Core asks for a receipt when it lost the answer to an execute request. If
//! the store holds no row, that absence proves nothing about an execute request
//! still on its way: one that reaches the store later would prepare and run
//! after core had released the job and its locks. Waiting for the plan to
//! expire is no barrier either, because the clock that judges expiry can step
//! back. So a lookup that is told to (`close_if_absent`, and every
//! `ResolveAction`) records a closure instead: a terminal `precondition_changed`
//! row for the action and plan digest, written in one transaction under the
//! store lock every execute takes (the caller's `&mut self`, and the executor's
//! single store mutex). Whichever of a lookup and an execute takes the lock
//! first wins: an execute that prepared first leaves a row the lookup reports,
//! and a closure recorded first is found by the execute's own replay check,
//! before it reads any clock or contacts the Engine. From then on the action ID
//! is taken.
//!
//! A closure reuses the refusal row shape (`refuse_stack`): nothing was sent,
//! nothing holds a lock, and every reader that already handles a refusal
//! handles a closure. What the store knew when it closed is kept in the row's
//! `closed` column, because the two cases must stay apart:
//!
//! - `never_recorded`: no receipt for the action was ever written, so nothing
//!   was ever sent, and now nothing can be. The closure receipt is the answer,
//!   and a late execute gets it back like any stored receipt.
//! - `no_longer_held`: a receipt may have existed and been pruned, so the action
//!   may have run. The answer stays that absence (core keeps the job for an
//!   operator), and a late execute is refused with `Expired` rather than shown
//!   a receipt that claims nothing was sent.
//!
//! A closure row is not an action's receipt for any other purpose: `verify`
//! and `verify_stack` find nothing in it to verify, and `ResolveAction`
//! answers it as the absence it records. Retention prunes it like any settled
//! row (`retention`).
//!
//! Writing a closure needs no per-target authorization beyond the write
//! authority the protocol ceiling already demands of the request: it grants
//! nothing, and only takes an action ID out of use. Requiring the target's
//! ceiling would leave a job for a container since removed from the ceiling
//! impossible to settle.
use super::*;
use crate::stack::Insert;
use limeos_domain::{
    ActionPlan, ContainerPlan, ReceiptAbsence, ResolveKind, StackMemberReceipt, StackPlan,
    aggregate_state,
};

/// `closed` is set only on a `precondition_changed` row, to what the store
/// knew when it closed the action.
pub(crate) const CLOSED_CHECK: &str = "CHECK(closed IS NULL OR (closed IN ('never_recorded','no_longer_held') AND state='precondition_changed'))";

/// The error a closure receipt records: the action's authority was withdrawn
/// before any execute recorded it.
pub const CLOSED_ERROR: ErrorCode = ErrorCode::Expired;

/// What a receipt lookup (`RestartReceipt`, `StackReceipt`) answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup<R> {
    /// A receipt to act on: written by an execute, or (`closed`) by a closure
    /// of an action the store never recorded, so nothing was ever sent.
    Receipt { receipt: R, closed: bool },
    /// No receipt to act on, and why. `closed`: a closure now refuses every
    /// execute for the action; otherwise one may still arrive and run.
    Absent {
        absence: ReceiptAbsence,
        closed: bool,
    },
}

fn closed_answer<R>(absence: ReceiptAbsence, receipt: R) -> Lookup<R> {
    match absence {
        ReceiptAbsence::NeverRecorded => Lookup::Receipt {
            receipt,
            closed: true,
        },
        ReceiptAbsence::NoLongerHeld => Lookup::Absent {
            absence,
            closed: true,
        },
    }
}

/// An execute that found a closure row: a `no_longer_held` closure claims
/// nothing about what was sent, so it is never returned as a receipt.
pub(crate) fn refuse_if_unproven(closure: Option<ReceiptAbsence>) -> Result<()> {
    if closure == Some(ReceiptAbsence::NoLongerHeld) {
        return Err(Error(ErrorCode::Expired));
    }
    Ok(())
}

impl ReceiptStore {
    /// What the store knew when it closed the action, if a lookup closed it.
    /// Pure lookup.
    pub(crate) fn closure(
        &self,
        kind: ResolveKind,
        action: &str,
    ) -> Result<Option<ReceiptAbsence>> {
        let table = match kind {
            ResolveKind::Container => "actions",
            ResolveKind::Stack => "stack_actions",
        };
        let row: Option<(String, Option<String>)> = self
            .conn
            .query_row(
                &format!("SELECT state,closed FROM {table} WHERE action=?"),
                [action],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(durable)?;
        match row {
            Some((state, Some(closed))) => {
                if state != ExecutionState::PreconditionChanged.as_str() {
                    return Err(Error(ErrorCode::StateNotDurable));
                }
                ReceiptAbsence::parse(&closed)
                    .map(Some)
                    .ok_or(Error(ErrorCode::StateNotDurable))
            }
            _ => Ok(None),
        }
    }

    /// Receipt lookup for a single-container action, asked with the plan whose
    /// digest is `digest` (`verified_expiry`). With no row, `close` records a
    /// closure (`now` is the executor clock, for retention only); without it
    /// nothing is written and the answer is the bare absence.
    #[allow(clippy::too_many_arguments)]
    pub fn lookup(
        &mut self,
        ceiling: &ContainerCeiling,
        peer_uid: u32,
        action: &str,
        digest: &str,
        plan: &ContainerPlan,
        close: bool,
        now: i64,
    ) -> Result<Lookup<RestartReceipt>> {
        let expires_at = verified_expiry(
            ResolveKind::Container,
            digest,
            &ActionPlan::Container(plan.clone()),
        )?;
        if let Some(receipt) = self.receipt(action, digest)? {
            if let Some(absence) = self.closure(ResolveKind::Container, action)? {
                return Ok(closed_answer(absence, receipt));
            }
            ceiling.authorize(peer_uid, &receipt.before.resource, receipt.operation)?;
            return Ok(Lookup::Receipt {
                receipt,
                closed: false,
            });
        }
        if !close {
            return Ok(Lookup::Absent {
                absence: self.absence_at(ResolveKind::Container, action, expires_at)?,
                closed: false,
            });
        }
        let (absence, receipt) = self.close_container(action, digest, plan, now)?;
        Ok(closed_answer(absence, receipt))
    }

    /// As `lookup`, for a whole-stack action. A row an execute wrote is
    /// authorized from its own receipt and reconciled (`reconcile_stack`).
    #[allow(clippy::too_many_arguments)]
    pub fn lookup_stack(
        &mut self,
        ceiling: &ContainerCeiling,
        peer_uid: u32,
        action: &str,
        digest: &str,
        plan: &StackPlan,
        close: bool,
        now: i64,
    ) -> Result<Lookup<StackReceipt>> {
        let expires_at =
            verified_expiry(ResolveKind::Stack, digest, &ActionPlan::Stack(plan.clone()))?;
        if let Some(found) = self.stack_receipt(action, digest)? {
            if let Some(absence) = self.closure(ResolveKind::Stack, action)? {
                return Ok(closed_answer(absence, found));
            }
            // Authorize from the pure lookup before any bookkeeping write.
            ceiling.authorize_stack(peer_uid, &found.before, found.operation)?;
            let receipt = self
                .reconcile_stack(action, digest)?
                .ok_or(Error(ErrorCode::StateNotDurable))?;
            return Ok(Lookup::Receipt {
                receipt,
                closed: false,
            });
        }
        if !close {
            return Ok(Lookup::Absent {
                absence: self.absence_at(ResolveKind::Stack, action, expires_at)?,
                closed: false,
            });
        }
        let (absence, receipt) = self.close_stack(action, digest, plan, now)?;
        Ok(closed_answer(absence, receipt))
    }

    /// Record a closure for a container action the store holds no row for.
    /// `plan` must already be verified against `digest`.
    pub(crate) fn close_container(
        &mut self,
        action: &str,
        digest: &str,
        plan: &ContainerPlan,
        now: i64,
    ) -> Result<(ReceiptAbsence, RestartReceipt)> {
        let receipt = RestartReceipt {
            action: action.into(),
            plan_digest: digest.into(),
            operation: plan.operation,
            before: plan.expected.clone(),
            state: ExecutionState::PreconditionChanged,
            error: Some(CLOSED_ERROR),
        };
        let absence = self
            .insert_action(&receipt, plan.expires_at, now, true)?
            .ok_or(Error(ErrorCode::StateNotDurable))?;
        Ok((absence, receipt))
    }

    /// Record a closure for a stack action the store holds no row for: the
    /// stack row and one member row per target, as a refusal is stored, so
    /// lookups cross-check it and retention removes it whole.
    pub(crate) fn close_stack(
        &mut self,
        action: &str,
        digest: &str,
        plan: &StackPlan,
        now: i64,
    ) -> Result<(ReceiptAbsence, StackReceipt)> {
        let members: Vec<_> = plan
            .targets()
            .iter()
            .map(|t| StackMemberReceipt {
                resource: t.resource.clone(),
                state: ExecutionState::PreconditionChanged,
                error: Some(CLOSED_ERROR),
            })
            .collect();
        let receipt = StackReceipt {
            action: action.into(),
            plan_digest: digest.into(),
            operation: plan.operation,
            before: plan.expected.clone(),
            state: aggregate_state(&members),
            members,
            error: Some(CLOSED_ERROR),
        };
        receipt.validate()?;
        let absence = self
            .write_stack(
                &receipt,
                &[],
                Some(Insert {
                    expires_at: plan.expires_at,
                    now,
                    close: true,
                }),
            )?
            .ok_or(Error(ErrorCode::StateNotDurable))?;
        Ok((absence, receipt))
    }
}

#[cfg(test)]
mod tests;
