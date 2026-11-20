//! Bounded retention of settled receipts.
//!
//! A receipt is terminal when no later transition can change it and it holds
//! no resource lock: `verified` and `precondition_changed` for a container
//! action, and also `partly_done` for a stack. `prepared`, `effect_accepted` and
//! `outcome_unknown` hold the lock that crash recovery and verification rely on;
//! `resolved` rows are the operator's recorded confirmation. Neither is ever
//! removed here. A stack and its member rows go together, and only when every
//! member is terminal too.
//!
//! Replay. A receipt is kept until its plan has been expired for at least
//! `RETAIN_AFTER_EXPIRY`. Every request carries its plan, and the executor
//! refuses an expired plan with `Expired` before it inspects or prepares, so a
//! repeated action id whose receipt was pruned can no longer execute. Should the
//! clock step back by more than that margin, the plan's fresh precondition
//! (start time and running state) still refuses it, because the original effect
//! changed exactly those facts.
//!
//! Pruning runs inside the transaction that inserts a new receipt, removes at
//! most `PRUNE_BATCH` rows, oldest plan expiry first, and writes nothing when
//! nothing is over budget. The row and file caps stay as last-resort stops.
use super::*;
use limeos_domain::PLAN_TTL_SECONDS;
use rusqlite::Transaction;

/// Standalone container receipts kept. Stack member rows are counted with
/// their stack instead.
pub(crate) const RETAINED_ACTIONS: u32 = 1024;
/// Stack receipts kept; each carries at most `MAX_STACK_MEMBERS` member rows.
pub(crate) const RETAINED_STACKS: u32 = 256;
/// Rows removed by one insert, at most.
pub(crate) const PRUNE_BATCH: u32 = 64;
/// A receipt outlives its plan's expiry by at least this many seconds.
pub(crate) const RETAIN_AFTER_EXPIRY: i64 = 3600;
const TERMINAL: &str = "('verified','precondition_changed')";
const STACK_TERMINAL: &str = "('verified','precondition_changed','partly_done')";
// Retained totals stay well inside the last-resort row cap.
const _: () = assert!(
    RETAINED_ACTIONS + RETAINED_STACKS * limeos_domain::MAX_STACK_MEMBERS as u32 + 2 * PRUNE_BATCH
        < 8192
);

/// The expiry recorded for receipts written before schema 6, which did not
/// store one: any plan in flight at the upgrade expires within the plan TTL.
pub(crate) fn legacy_expiry_sql() -> String {
    format!("CAST(strftime('%s','now') AS INTEGER)+{PLAN_TTL_SECONDS}")
}

/// Remove settled receipts beyond the budget. Returns the rows removed.
pub(crate) fn prune(tx: &Transaction<'_>, now: i64) -> Result<u32> {
    let cutoff = now.saturating_sub(RETAIN_AFTER_EXPIRY);
    let mut removed = 0u32;
    let stacks: u32 = tx
        .query_row("SELECT count(*) FROM stack_actions", [], |r| r.get(0))
        .map_err(durable)?;
    if stacks > RETAINED_STACKS {
        let limit = (stacks - RETAINED_STACKS).min(PRUNE_BATCH);
        // Member rows share the stack's plan digest. A digest shared by two
        // stack rows, or any member row still active or resolved, keeps the stack.
        let candidates: Vec<(String, String)> = {
            let mut statement = tx
                .prepare(&format!(
                    "SELECT s.action,s.digest FROM stack_actions s WHERE s.state IN {STACK_TERMINAL} AND s.expires_at<=?1 \
                     AND (SELECT count(*) FROM stack_actions t WHERE t.digest=s.digest)=1 \
                     AND NOT EXISTS (SELECT 1 FROM actions a WHERE a.digest=s.digest AND a.state NOT IN {TERMINAL}) \
                     ORDER BY s.expires_at,s.rowid LIMIT ?2"
                ))
                .map_err(durable)?;
            statement
                .query_map(params![cutoff, limit], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(durable)?
                .collect::<rusqlite::Result<_>>()
                .map_err(durable)?
        };
        for (action, digest) in candidates {
            if removed >= PRUNE_BATCH {
                break;
            }
            let members = tx
                .execute(
                    &format!("DELETE FROM actions WHERE digest=?1 AND state IN {TERMINAL}"),
                    [&digest],
                )
                .map_err(durable)?;
            if tx
                .execute(
                    &format!(
                        "DELETE FROM stack_actions WHERE action=?1 AND digest=?2 AND state IN {STACK_TERMINAL}"
                    ),
                    [&action, &digest],
                )
                .map_err(durable)?
                != 1
            {
                return Err(Error(ErrorCode::StateNotDurable));
            }
            removed = removed.saturating_add(u32::try_from(members).unwrap_or(u32::MAX));
            removed = removed.saturating_add(1);
        }
    }
    if removed >= PRUNE_BATCH {
        return Ok(removed);
    }
    let standalone: u32 = tx
        .query_row(
            "SELECT count(*) FROM actions a WHERE NOT EXISTS (SELECT 1 FROM stack_actions s WHERE s.digest=a.digest)",
            [],
            |r| r.get(0),
        )
        .map_err(durable)?;
    if standalone > RETAINED_ACTIONS {
        let limit = (standalone - RETAINED_ACTIONS).min(PRUNE_BATCH - removed);
        let deleted = tx
            .execute(
                &format!(
                    "DELETE FROM actions WHERE rowid IN (SELECT a.rowid FROM actions a WHERE a.state IN {TERMINAL} AND a.expires_at<=?1 \
                     AND NOT EXISTS (SELECT 1 FROM stack_actions s WHERE s.digest=a.digest) \
                     ORDER BY a.expires_at,a.rowid LIMIT ?2)"
                ),
                params![cutoff, limit],
            )
            .map_err(durable)?;
        removed = removed.saturating_add(u32::try_from(deleted).unwrap_or(u32::MAX));
    }
    Ok(removed)
}

#[cfg(test)]
mod tests;
