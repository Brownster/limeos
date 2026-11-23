//! The container executor's receipt-store and Engine requests, request to
//! response: inspection, the receipt epoch, execute, lookup (and closure),
//! verify and resolve, for single containers and Compose stacks. The daemon's
//! connection handler (`shared_role.rs`) keeps the peer check, framing, deadlines and the
//! ceiling's `validate`, answers its own reads (observations, logs, storage
//! inventories, health), and hands every other request to [`answer`]; the
//! long-uptime soak (`bins/core/src/soak`) calls the same function with a
//! simulated Engine and clock.
use limeos_domain::{Error, ErrorCode, Result};
use limeos_executor_container::{ActionRecord, Answer, Engine, ReceiptStore, RestartCeiling};
use limeos_executor_protocol::{Receipt, Request};
use std::sync::Arc;
use tokio::sync::Mutex;

/// What the container role answers with.
pub struct ContainerRole<E> {
    pub engine: E,
    /// The receipt store, for an executor whose ceiling can write; locked
    /// with `try_lock` by every request that reads or writes it.
    pub receipts: Option<Arc<Mutex<ReceiptStore>>>,
    /// The store's epoch for this open, answered to `ReceiptEpoch` without
    /// taking the store's lock.
    pub epoch: Option<Arc<str>>,
    /// Whether this executor serves the Docker source: the Engine-facing
    /// requests are refused otherwise.
    pub docker: bool,
}

/// The container role's answer to `request` from peer `uid`, written into
/// `receipt` (which the ceiling's `validate` returned); `ceiling` is the
/// container ceiling derived from the ceiling read for this request
/// (`crate::ceiling`), and `clock` is the executor's clock. Any other
/// request is `Forbidden`.
pub async fn answer<E, C>(
    role: &ContainerRole<E>,
    ceiling: &RestartCeiling,
    uid: u32,
    request: Request,
    receipt: &mut Receipt,
    clock: &C,
) -> Result<()>
where
    E: Engine + Sync,
    C: Fn() -> i64 + Sync,
{
    // The ceiling grants writes (`validate` checked) but the store did not
    // open at start, because the ceiling then granted none: `Unavailable`
    // until a restart opens it.
    let store = || role.receipts.as_ref().ok_or(Error(ErrorCode::Unavailable));
    match request {
        Request::Inspect { resource, .. } if role.docker => {
            receipt.inspection = Some(
                tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    role.engine.inspect(&resource[10..]),
                )
                .await
                .map_err(|_| Error(ErrorCode::Unavailable))??,
            );
        }
        Request::Restart { request, .. } | Request::ExecuteContainer { request, .. }
            if role.docker =>
        {
            // Do not wait behind another effect with an expiring plan.
            let mut store = store()?
                .try_lock()
                .map_err(|_| Error(ErrorCode::Overloaded))?;
            let executed = store
                .execute(&role.engine, ceiling, uid, &request, clock)
                .await;
            journal(&mut store);
            receipt.restart = Some(executed.inspect_err(note_refusal)?);
            receipt.epoch = Some(store.epoch().to_owned());
        }
        Request::RestartReceipt {
            action,
            digest,
            plan,
            close_if_absent,
            ..
        } => {
            // The same lock every execute takes: a closure and an execute
            // for one action never interleave. Contention is `Overloaded`,
            // which the caller retries.
            let mut store = store()?
                .try_lock()
                .map_err(|_| Error(ErrorCode::Overloaded))?;
            // The plan must be the one `digest` names, row or not.
            let lookup = store.lookup(
                ceiling,
                uid,
                &action,
                &digest,
                &plan,
                close_if_absent,
                clock(),
            )?;
            copy(receipt, Answer::restart_lookup(lookup));
            receipt.epoch = Some(store.epoch().to_owned());
        }
        Request::VerifyRestart {
            action,
            digest,
            after,
            ..
        } => {
            let mut store = store()?
                .try_lock()
                .map_err(|_| Error(ErrorCode::Overloaded))?;
            receipt.restart = Some(
                store
                    .verify(&role.engine, ceiling, uid, &action, &digest, &after)
                    .await?,
            );
            receipt.epoch = Some(store.epoch().to_owned());
        }
        Request::InspectStack { project, .. } if role.docker => {
            let members = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                role.engine.project_members(&project),
            )
            .await
            .map_err(|_| Error(ErrorCode::Unavailable))??;
            receipt.stack_members = Some(limeos_domain::StackSnapshot::new(&project, members)?);
        }
        Request::ExecuteStack { request, .. } if role.docker => {
            // Do not wait behind another effect with an expiring plan.
            let mut store = store()?
                .try_lock()
                .map_err(|_| Error(ErrorCode::Overloaded))?;
            let executed = store
                .execute_stack(&role.engine, ceiling, uid, &request, clock)
                .await;
            journal(&mut store);
            receipt.stack = Some(executed.inspect_err(note_refusal)?);
            receipt.epoch = Some(store.epoch().to_owned());
        }
        Request::StackReceipt {
            action,
            digest,
            plan,
            close_if_absent,
            ..
        } => {
            // As for `RestartReceipt`: the execute path's lock.
            let mut store = store()?
                .try_lock()
                .map_err(|_| Error(ErrorCode::Overloaded))?;
            let lookup = store.lookup_stack(
                ceiling,
                uid,
                &action,
                &digest,
                &plan,
                close_if_absent,
                clock(),
            )?;
            copy(receipt, Answer::stack_lookup(lookup));
            receipt.epoch = Some(store.epoch().to_owned());
        }
        Request::VerifyStack {
            action,
            digest,
            after,
            ..
        } => {
            let mut store = store()?
                .try_lock()
                .map_err(|_| Error(ErrorCode::Overloaded))?;
            receipt.stack = Some(
                store
                    .verify_stack(&role.engine, ceiling, uid, &action, &digest, &after)
                    .await?,
            );
            receipt.epoch = Some(store.epoch().to_owned());
        }
        Request::ResolveAction {
            kind,
            action,
            digest,
            evidence_digest,
            declared,
            resolved_by,
            plan,
            disputed,
            ..
        } if role.docker => {
            let mut store = store()?
                .try_lock()
                .map_err(|_| Error(ErrorCode::Overloaded))?;
            // Read-only re-verification; this arm never reaches an Engine effect.
            let outcome = store
                .resolve_action(
                    &role.engine,
                    ceiling,
                    uid,
                    kind,
                    &action,
                    &digest,
                    &evidence_digest,
                    declared,
                    &resolved_by,
                    &plan,
                    disputed,
                    clock(),
                )
                .await?;
            copy(receipt, Answer::resolve(outcome));
            receipt.epoch = Some(store.epoch().to_owned());
        }
        Request::ReceiptEpoch { .. } => {
            receipt.epoch = Some(
                role.epoch
                    .as_deref()
                    // As for `store`: granted after start, opened at the
                    // next one.
                    .ok_or(Error(ErrorCode::Unavailable))?
                    .to_owned(),
            );
        }
        _ => return Err(Error(ErrorCode::Forbidden)),
    }
    Ok(())
}

/// The response to a refused request: not ready, with the refusal's code.
pub fn refusal(error: Error) -> Receipt {
    let mut value = Receipt::empty();
    value.ready = false;
    value.error = Some(error.0);
    value
}

/// Copies a receipt-store answer into the response, field by same-named
/// field; which fields an answer sets is decided (and tested) in
/// `limeos_executor_container::Answer`.
fn copy(receipt: &mut Receipt, answer: Answer) {
    let Answer {
        restart,
        stack,
        resolution,
        resolution_refusal,
        absence,
        closed,
    } = answer;
    receipt.restart = restart;
    receipt.stack = stack;
    receipt.resolution = resolution;
    receipt.resolution_refusal = resolution_refusal;
    receipt.absence = absence;
    receipt.closed = closed;
}

/// An execute refused by the receipt store's clock-free barrier: its plan
/// expires at or below the prune watermark (`retention`). Logged, because
/// every such refusal after a backward clock step looks alike to core.
fn note_refusal(error: &Error) {
    if error.0 == ErrorCode::BeyondReceiptHistory {
        tracing::warn!(
            "execute refused: the plan expires at or below the receipt store's prune watermark, so whether the action already ran can no longer be told; the system clock has probably stepped back, and plans are refused until it passes the watermark again"
        );
    }
}

/// The message of the journal line for each effect in the action log.
pub const EFFECT_LINE: &str = "action log: effect";
/// The message of the journal line for each refusal row in the action log.
pub const REFUSAL_LINE: &str = "action log: effect refused, the container's executor action budget is spent; core's later refusals for it within the hour are counted into its next record";

/// One journal line per action-log row the request wrote (`limit`): the
/// receipt store's table is the record core cannot touch; these lines are
/// the operator's quick view (`journalctl -u limeos-containerd`), which a
/// flood of other journal writers can rotate away.
fn journal(store: &mut ReceiptStore) {
    for record in store.take_journal() {
        line(&record);
    }
}

fn line(record: &ActionRecord) {
    let ActionRecord {
        seq,
        at,
        kind,
        resource,
        operation,
        stack,
        action,
        peer,
        principal,
        outcome,
        refusals,
    } = record;
    let (operation, stack) = (operation.as_str(), stack.as_deref().unwrap_or(""));
    if *kind == "refused" {
        tracing::warn!(
            action_log = kind,
            seq,
            at,
            resource = resource.as_str(),
            operation,
            stack,
            action = action.as_str(),
            peer,
            principal = principal.as_str(),
            outcome,
            refusals,
            "{REFUSAL_LINE}"
        );
    } else {
        tracing::info!(
            action_log = kind,
            seq,
            at,
            resource = resource.as_str(),
            operation,
            stack,
            action = action.as_str(),
            peer,
            principal = principal.as_str(),
            outcome,
            "{EFFECT_LINE}"
        );
    }
}
