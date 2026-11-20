//! Bounded retention of settled receipts.
//!
//! A receipt is terminal when no later transition can change it and it holds
//! no resource lock: `verified`, `precondition_changed` and `resolved` for a
//! container action, and also `partly_done` for a stack. `prepared`,
//! `effect_accepted` and `outcome_unknown` hold the lock that crash recovery
//! and verification rely on and are never removed here. A stack and its member
//! rows go together, and only when every member is terminal too.
//!
//! A `resolved` row is the operator's recorded confirmation, which core needs
//! only until it has completed its own side. Kept forever, each resolution
//! would turn prunable history into a permanent row until the last-resort caps
//! stop all execution. A core that crashed before completing retries
//! `ResolveAction`: while the row is kept it gets the stored confirmation;
//! once it is pruned, the executor closes the action and answers
//! `NoLongerHeld` (the watermark covers it), and core then completes only if
//! its own fresh measurement still matches the approved evidence.
//!
//! Replay. A receipt is kept until its plan has been expired for at least
//! `RETAIN_AFTER_EXPIRY`. Every request carries its plan, and a pruned row's
//! plan expires at or below the watermark, so a repeated action id whose
//! receipt was pruned is refused by the execute barrier (below) before any
//! clock is read; an expired plan is refused with `Expired` besides.
//!
//! Closures. A closure row (`closure`) is a `precondition_changed` refusal
//! like any other: settled, lock-free, pruned under these rules and retired
//! into the watermark. It stores its plan's `expires_at`, so it is kept until
//! that plan has been expired for `RETAIN_AFTER_EXPIRY` by the clock that
//! prunes, and while kept it refuses its action whatever the clock says. Only
//! eligibility reads the clock: a backward step delays pruning, and a forward
//! step can prune a closure early only for a plan that clock already holds
//! expired by the full margin.
//!
//! The execute barrier. Once a closure is pruned, a delayed execute request
//! for its action finds no row, and after a backward clock step to before the
//! plan's expiry both expiry checks pass. So an execute for an action the
//! store holds no row for is refused with `BeyondReceiptHistory`, before it
//! prepares or contacts the Engine, whenever the absence would be answered
//! `NoLongerHeld` (`ReceiptStore::admit`): its plan expires at or below the
//! watermark, where any receipt for it may have been pruned, so the store can
//! no longer tell whether the action was closed or already ran. The rule is
//! the absence rule, non-strict at the watermark, and reads no clock. Its
//! availability cost: after a backward step of more than the retention margin
//! (plus the plan TTL), freshly approved plans also expire at or below the
//! watermark and are refused until the clock passes it again. That is the
//! safe direction; the executor logs each such refusal.
//!
//! Pruning runs inside the transaction that inserts a new receipt, removes at
//! most `PRUNE_BATCH` rows, oldest plan expiry first, and writes nothing when
//! nothing is over budget. The row and file caps stay as last-resort stops.
//!
//! Absence. The same transaction raises the prune watermark, the greatest
//! `expires_at` among every row ever pruned (or, for a store created since
//! schema 7, its creation floor if greater: `initial_watermark`); it never
//! decreases. A row stores
//! its plan's `expires_at`, and a stack's member rows store the stack's, so a
//! pruned receipt always has `expires_at <= watermark`. A lookup that finds no
//! row for a plan expiring strictly after the watermark therefore proves that
//! no receipt was ever written (`ReceiptAbsence::NeverRecorded`); at or below
//! it a receipt may have been pruned (`NoLongerHeld`). The expiry comes from
//! the plan itself, which the request carries and whose digest must equal the
//! plan digest it names (`verified_expiry`), never from a bare number. This holds whatever the
//! order of pruning and whatever the clock does; oldest-expiry-first only keeps
//! the watermark low, at least an hour behind the clock that pruned, so a young
//! plan is answered `NeverRecorded` at once.
//!
//! Rows written before schema 7 are `legacy`: schema 6 back-filled their
//! `expires_at` from the upgrade clock, which need not be at or above their
//! plan's real expiry. A pruned legacy row leaves its action ID in
//! `pruned_legacy`, which answers `NoLongerHeld` for it regardless of the
//! watermark. There is at most one tombstone per row that existed at the
//! upgrade, each smaller than the row it replaces, so the table is bounded.
//! A store opened at schema 6 may already have pruned without a watermark;
//! its watermark starts at the maximum, so it never claims `NeverRecorded`.
use super::*;
use limeos_domain::{ActionPlan, PLAN_TTL_SECONDS, ReceiptAbsence, ResolveKind};
use rusqlite::Transaction;

/// Standalone container receipts kept. Stack member rows are counted with
/// their stack instead.
pub(crate) const RETAINED_ACTIONS: u32 = 1024;
/// Stack receipts kept; each carries at most `MAX_STACK_MEMBERS` member rows.
pub(crate) const RETAINED_STACKS: u32 = 256;
/// Rows removed by one insert, at most. A stack is removed whole or not at
/// all, so a batch stops before a stack that would exceed it; one whole stack
/// always fits, so every batch over budget makes progress.
pub(crate) const PRUNE_BATCH: u32 = 64;
const _: () = assert!((limeos_domain::MAX_STACK_MEMBERS as u32) < PRUNE_BATCH);
/// A receipt outlives its plan's expiry by at least this many seconds.
pub(crate) use limeos_domain::RECEIPT_RETAIN_AFTER_EXPIRY as RETAIN_AFTER_EXPIRY;
const TERMINAL: &str = "('verified','precondition_changed','resolved')";
const STACK_TERMINAL: &str = "('verified','precondition_changed','partly_done','resolved')";
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

/// The watermark a store starts with when it gains one (schema 7); `version`
/// is the schema it was opened at and `now` the executor clock.
///
/// A store created now (version 0) knows nothing of what ran before it
/// existed, perhaps in a store since lost, so it must not answer
/// `NeverRecorded` for any plan that could predate it. Plans carry absolute
/// expiries from core's clock, on the same host, and are valid only while
/// `expires_at <= created_at + PLAN_TTL_SECONDS`; a plan created no later
/// than this store therefore expires no later than `now + PLAN_TTL_SECONDS`.
/// That is the creation floor, and the tight bound: any margin above it
/// would answer `NoLongerHeld` (and refuse at execute) plans approved after
/// the store existed. A clock that stepped back before the creation defeats
/// it, as does a store restored from an older copy; core's check of the
/// store epoch covers both (`ReceiptStore::epoch`).
///
/// Before schema 6 nothing was ever pruned. A schema 6 store may have pruned
/// rows whose expiry nobody recorded, so it never claims `NeverRecorded`.
pub(crate) fn initial_watermark(version: u32, now: i64) -> i64 {
    match version {
        0 => now.saturating_add(PLAN_TTL_SECONDS),
        6 => i64::MAX,
        _ => i64::MIN,
    }
}

/// The greatest plan expiry among every receipt ever pruned.
pub(crate) fn watermark(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT watermark FROM retention WHERE id=1", [], |r| {
        r.get(0)
    })
    .map_err(durable)
}

/// Record one pruned row: its expiry for the watermark, its ID if legacy.
fn retire(
    tx: &Transaction<'_>,
    action: &str,
    legacy: bool,
    highest: &mut Option<i64>,
    expires_at: i64,
) -> Result<()> {
    if legacy {
        tx.execute(
            "INSERT OR IGNORE INTO pruned_legacy(action) VALUES(?1)",
            [action],
        )
        .map_err(durable)?;
    }
    *highest = Some(highest.map_or(expires_at, |h| h.max(expires_at)));
    Ok(())
}

/// The expiry a missing receipt is judged by: that of `plan`, once its
/// recomputed digest equals `digest`, the plan digest the request names, and
/// its kind and structure agree. Anything else is `InvalidInput`, so a peer
/// cannot ask about an action with an expiry its approved plan did not have.
pub fn verified_expiry(kind: ResolveKind, digest: &str, plan: &ActionPlan) -> Result<i64> {
    let body = match plan {
        ActionPlan::Container(plan) => encode(plan)?,
        ActionPlan::Stack(plan) => encode(plan)?,
    };
    if plan.kind() != kind
        || !opaque_id(digest)
        || limeos_identity::digest(&body) != digest
        || plan.validate_shape().is_err()
    {
        return Err(Error(ErrorCode::InvalidInput));
    }
    Ok(plan.expires_at())
}

impl ReceiptStore {
    /// What the store can say about an action it holds no row for, asked with
    /// the plan whose digest is `digest` (`verified_expiry`). A row that
    /// exists is `Conflict`: look it up.
    pub fn receipt_absence(
        &self,
        action: &str,
        digest: &str,
        plan: &ActionPlan,
    ) -> Result<ReceiptAbsence> {
        let expires_at = verified_expiry(plan.kind(), digest, plan)?;
        self.absence_at(plan.kind(), action, expires_at)
    }

    /// The execute-time barrier, for an action the store holds no row for:
    /// refused with `BeyondReceiptHistory` when its plan expires at or below
    /// the watermark (or its ID is a legacy tombstone), where a receipt for
    /// it may have been pruned. Reads no clock. Pure lookup.
    pub(crate) fn admit(&self, kind: ResolveKind, action: &str, expires_at: i64) -> Result<()> {
        match self.absence_at(kind, action, expires_at)? {
            ReceiptAbsence::NeverRecorded => Ok(()),
            ReceiptAbsence::NoLongerHeld => Err(Error(ErrorCode::BeyondReceiptHistory)),
        }
    }

    /// As `receipt_absence`, with an expiry already verified against the plan.
    pub(crate) fn absence_at(
        &self,
        kind: ResolveKind,
        action: &str,
        expires_at: i64,
    ) -> Result<ReceiptAbsence> {
        absence_in(&self.conn, kind, action, expires_at)
    }
}

/// As `ReceiptStore::absence_at`, on any connection or open transaction, so a
/// closure can judge the absence in the transaction that records it.
pub(crate) fn absence_in(
    conn: &Connection,
    kind: ResolveKind,
    action: &str,
    expires_at: i64,
) -> Result<ReceiptAbsence> {
    if !opaque_id(action) {
        return Err(Error(ErrorCode::InvalidInput));
    }
    let table = match kind {
        ResolveKind::Container => "actions",
        ResolveKind::Stack => "stack_actions",
    };
    let held: bool = conn
        .query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE action=?1)"),
            [action],
            |r| r.get(0),
        )
        .map_err(durable)?;
    if held {
        return Err(Error(ErrorCode::Conflict));
    }
    let tombstone: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pruned_legacy WHERE action=?1)",
            [action],
            |r| r.get(0),
        )
        .map_err(durable)?;
    if tombstone {
        return Ok(ReceiptAbsence::NoLongerHeld);
    }
    Ok(ReceiptAbsence::judge(expires_at, watermark(conn)?))
}

/// Remove settled receipts beyond the budget, raising the watermark in the
/// same transaction. Returns the rows removed.
pub(crate) fn prune(tx: &Transaction<'_>, now: i64) -> Result<u32> {
    let cutoff = now.saturating_sub(RETAIN_AFTER_EXPIRY);
    let mut removed = 0u32;
    let mut highest = None;
    let stacks: u32 = tx
        .query_row("SELECT count(*) FROM stack_actions", [], |r| r.get(0))
        .map_err(durable)?;
    if stacks > RETAINED_STACKS {
        let limit = (stacks - RETAINED_STACKS).min(PRUNE_BATCH);
        // Member rows share the stack's plan digest. A digest shared by two
        // stack rows, or any member row still holding a lock, keeps the stack.
        let candidates: Vec<(String, String, i64, bool)> = {
            let mut statement = tx
                .prepare(&format!(
                    "SELECT s.action,s.digest,s.expires_at,s.legacy FROM stack_actions s WHERE s.state IN {STACK_TERMINAL} AND s.expires_at<=?1 \
                     AND (SELECT count(*) FROM stack_actions t WHERE t.digest=s.digest)=1 \
                     AND NOT EXISTS (SELECT 1 FROM actions a WHERE a.digest=s.digest AND a.state NOT IN {TERMINAL}) \
                     ORDER BY s.expires_at,s.rowid LIMIT ?2"
                ))
                .map_err(durable)?;
            statement
                .query_map(params![cutoff, limit], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .map_err(durable)?
                .collect::<rusqlite::Result<_>>()
                .map_err(durable)?
        };
        for (action, digest, expires_at, legacy) in candidates {
            let rows: u32 = tx
                .query_row(
                    &format!(
                        "SELECT count(*)+1 FROM actions WHERE digest=?1 AND state IN {TERMINAL}"
                    ),
                    [&digest],
                    |r| r.get(0),
                )
                .map_err(durable)?;
            if removed.saturating_add(rows) > PRUNE_BATCH {
                break;
            }
            // Members share the stack's expiry; each is retired on its own all
            // the same, so the watermark never rests on that invariant.
            let retired: Vec<(String, bool, i64)> = {
                let mut statement = tx
                    .prepare(&format!(
                        "SELECT action,legacy,expires_at FROM actions WHERE digest=?1 AND state IN {TERMINAL}"
                    ))
                    .map_err(durable)?;
                statement
                    .query_map([&digest], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .map_err(durable)?
                    .collect::<rusqlite::Result<_>>()
                    .map_err(durable)?
            };
            for (member, member_legacy, member_expiry) in &retired {
                retire(tx, member, *member_legacy, &mut highest, *member_expiry)?;
            }
            retire(tx, &action, legacy, &mut highest, expires_at)?;
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
    if removed < PRUNE_BATCH {
        removed = removed.saturating_add(prune_standalone(
            tx,
            cutoff,
            PRUNE_BATCH - removed,
            &mut highest,
        )?);
    }
    if let Some(highest) = highest {
        if tx
            .execute(
                "UPDATE retention SET watermark=max(watermark,?1) WHERE id=1",
                [highest],
            )
            .map_err(durable)?
            != 1
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
    }
    Ok(removed)
}

/// Standalone container receipts beyond their budget, at most `room` rows.
fn prune_standalone(
    tx: &Transaction<'_>,
    cutoff: i64,
    room: u32,
    highest: &mut Option<i64>,
) -> Result<u32> {
    let mut removed = 0u32;
    let standalone: u32 = tx
        .query_row(
            "SELECT count(*) FROM actions a WHERE NOT EXISTS (SELECT 1 FROM stack_actions s WHERE s.digest=a.digest)",
            [],
            |r| r.get(0),
        )
        .map_err(durable)?;
    if standalone > RETAINED_ACTIONS {
        let limit = (standalone - RETAINED_ACTIONS).min(room);
        let candidates: Vec<(i64, String, bool, i64)> = {
            let mut statement = tx
                .prepare(&format!(
                    "SELECT a.rowid,a.action,a.legacy,a.expires_at FROM actions a WHERE a.state IN {TERMINAL} AND a.expires_at<=?1 \
                     AND NOT EXISTS (SELECT 1 FROM stack_actions s WHERE s.digest=a.digest) \
                     ORDER BY a.expires_at,a.rowid LIMIT ?2"
                ))
                .map_err(durable)?;
            statement
                .query_map(params![cutoff, limit], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .map_err(durable)?
                .collect::<rusqlite::Result<_>>()
                .map_err(durable)?
        };
        for (rowid, action, legacy, expires_at) in candidates {
            retire(tx, &action, legacy, highest, expires_at)?;
            if tx
                .execute("DELETE FROM actions WHERE rowid=?1", [rowid])
                .map_err(durable)?
                != 1
            {
                return Err(Error(ErrorCode::StateNotDurable));
            }
            removed = removed.saturating_add(1);
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests;
