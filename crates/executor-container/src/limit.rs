//! The executor's own bound on effects, and its record of them (ADR 0013).
//!
//! Core is trusted within the ceiling, so a core that is compromised can ask
//! for every effect the ceiling grants. The executor sees every effect, so it
//! bounds them alone: each managed container may be charged at most
//! [`BURST`] effects in any ten minutes and [`DAILY`] in any day, whatever
//! the operation (restart, start and stop share one budget) and whether it
//! comes alone or as a member of a stack action. A stack action is admitted
//! whole or refused whole: every target must have room, and each target is
//! charged one effect. There is no budget shared between containers, so
//! effects on one container never refuse an effect on another (the trade-off
//! core's login limiter made, `docs/p01-operations.md`).
//!
//! Only new effects are charged. A replayed action (its receipt held) is
//! answered from the receipt before the budget is consulted; receipt lookups,
//! closures, verification and resolution never reach it.
//!
//! The source of truth is the action log, `effect_log`, a table of the
//! receipt store: one `effect` row per container charged, written in the
//! transaction that prepares the action, so a crash cannot leave an effect
//! uncharged or a charge without its receipt; and a `refused` row for a
//! refusal. The budget is a count of a container's effect rows inside each
//! window, whatever their outcome, so it survives a restart of the executor
//! and needs no counter of its own. A charge is never refunded: a member
//! never sent (its stack stopped first, or its plan expired between prepare
//! and the Engine call, which core can aim at since it writes the plan's
//! expiry) still counts. So every effect row was admitted by the budget, and
//! the budget bounds every row the log can hold (below).
//!
//! Atomicity. The check (`within_budget`) and the charge (the prepare
//! transaction) are not one SQL transaction. They are atomic because every
//! execute holds the receipt store's mutex for its whole call (the role takes
//! it with `try_lock`, `bins/executor/src/container_requests.rs`), the
//! receipt store takes `&mut self`, and no `.await` lies between the check
//! and the commit that charges. A change that puts an await between them must
//! move the check into the prepare transaction.
//!
//! Clock. Rows carry the wall clock (`CLOCK_REALTIME`), the clock that
//! validates plans and the only one that means the same after a reboot. A
//! row counts while `now - window < at <= now + window`. A row dated after
//! `now` (the clock stepped back) still counts while it is within one window
//! of `now`, so a small backward step holds a charge longer, the strict
//! direction; a row further in the future (a clock that ran far ahead and was
//! corrected) no longer counts, so a large backward step gives the budget
//! back at most once, as a forward step of more than the window does. Without
//! that bound, ten charges taken while a clock without a battery ran three
//! years ahead would refuse the container for three years once it was
//! corrected. Core cannot set the clock; each step changes at most one
//! window's worth of budget.
//!
//! Failure. A budget that cannot be read refuses the effect
//! (`StateNotDurable`), like any store failure before prepare. A refusal is
//! recorded best effort: the refusal stands whether or not its row is written.
//!
//! Bounds. Rows are kept newest first up to [`LOG_ROWS`], removed oldest
//! first in the inserting transaction by sequence number, never by the clock.
//! Refusal rows are coalesced: none for a container while one of its refusal
//! rows is within [`REFUSAL_INTERVAL`] of `now` either way, carrying the
//! number of refusals since the previous one this process saw. So, the clock
//! steady, a container has at most [`DAILY`] effect rows (of any outcome) and
//! 25 refusal rows in any day, and [`MANAGED`] containers fewer than
//! `LOG_ROWS` together: no row a daily window counts is ever removed, and a
//! flood of refused requests costs at most one write per container an hour.
//! Every class of row the log can hold is in that bound. Each clock step can
//! add at most one more window's worth. `effect_days` keeps per-day totals
//! for [`DAYS_KEPT`] days, so the volume of past days survives after their
//! rows were pushed out.
use super::*;
use rusqlite::Transaction;

/// A rolling window and the effects one container may receive in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub seconds: i64,
    pub effects: u32,
}

/// Room for a person restarting one container while debugging, and for a
/// stack action on top: one effect a minute on average for ten minutes.
pub const BURST: Window = Window {
    seconds: 600,
    effects: 10,
};
/// Six such sessions a day on one container, or a nightly schedule and a bad
/// evening besides. A compromised core gets 60 restarts of a container a day
/// (about ten minutes of downtime at ten seconds a restart) instead of one
/// every few seconds.
pub const DAILY: Window = Window {
    seconds: 86_400,
    effects: 60,
};
/// The windows every effect is charged against.
pub const WINDOWS: [Window; 2] = [BURST, DAILY];
/// Action-log rows kept.
pub const LOG_ROWS: i64 = 8192;
/// Least time between two refusal rows for one container.
pub const REFUSAL_INTERVAL: i64 = 3600;
/// Days of totals kept in `effect_days`.
pub const DAYS_KEPT: i64 = 400;
/// The most containers a ceiling lists (`ContainerCeiling::authorize`); every
/// row names one, since a stack's targets must be listed too.
pub const MANAGED: i64 = 64;
/// Refusal rows one container can have in any day: one per interval, and
/// one more where a day's boundary falls between two.
pub const REFUSAL_ROWS: i64 = DAILY.seconds / REFUSAL_INTERVAL + 1;
/// Refusals counted but not yet written, at most this many containers.
const SUPPRESSED: usize = 256;
// A day of every row every listed container can have (effect rows of any
// outcome, all admitted by the budget, and refusal rows) fits in the log, so
// the daily window never loses a row it counts.
const _: () = assert!(MANAGED * (DAILY.effects as i64 + REFUSAL_ROWS) < LOG_ROWS);

/// The schema step that adds the log (receipt schema 9).
pub(crate) const SCHEMA: &str = "CREATE TABLE effect_log(seq INTEGER PRIMARY KEY,at INTEGER NOT NULL,kind TEXT NOT NULL CHECK(kind IN ('effect','refused')),resource TEXT NOT NULL,operation TEXT NOT NULL CHECK(operation IN ('restart','start','stop')),stack TEXT,action TEXT NOT NULL,peer INTEGER NOT NULL,principal TEXT NOT NULL,outcome TEXT CHECK(outcome IN ('effect_accepted','outcome_unknown','not_sent','rate_limited')),refusals INTEGER NOT NULL DEFAULT 0); CREATE INDEX effect_log_resource ON effect_log(resource,kind,at); CREATE TABLE effect_days(day INTEGER PRIMARY KEY,charges INTEGER NOT NULL,refusals INTEGER NOT NULL);";
/// Every column, for the open's schema check.
pub(crate) const COLUMNS: &str = "SELECT seq,at,kind,resource,operation,stack,action,peer,principal,outcome,refusals FROM effect_log LIMIT 0";

/// Who asked for an effect and what it is: the same for every container one
/// action charges.
pub(crate) struct Entry<'a> {
    pub(crate) action: &'a str,
    pub(crate) operation: ContainerAction,
    /// The stack resource, for a member of a stack action.
    pub(crate) stack: Option<&'a str>,
    /// The peer's UID (SO_PEERCRED), which the ceiling has matched to core.
    pub(crate) peer: u32,
    /// The plan's principal: what core says, checked for format only.
    pub(crate) principal: &'a str,
}

/// One action-log row, as the executor journals it after the request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionRecord {
    pub seq: i64,
    pub at: i64,
    /// `effect` or `refused`.
    pub kind: &'static str,
    pub resource: String,
    pub operation: ContainerAction,
    pub stack: Option<String>,
    pub action: String,
    pub peer: u32,
    pub principal: String,
    /// `effect_accepted`, `outcome_unknown` or `not_sent` for an effect;
    /// `rate_limited` for a refusal.
    pub outcome: &'static str,
    /// Refusals this row stands for (0 for an effect).
    pub refusals: u32,
}
impl ActionRecord {
    fn new(entry: &Entry<'_>, seq: i64, at: i64, resource: &str) -> Self {
        Self {
            seq,
            at,
            kind: "effect",
            resource: resource.into(),
            operation: entry.operation,
            stack: entry.stack.map(Into::into),
            action: entry.action.into(),
            peer: entry.peer,
            principal: entry.principal.into(),
            outcome: "outcome_unknown",
            refusals: 0,
        }
    }
}

/// What a finished effect's row records.
pub(crate) fn outcome(state: ExecutionState) -> Option<&'static str> {
    match state {
        ExecutionState::EffectAccepted | ExecutionState::Verified => Some("effect_accepted"),
        ExecutionState::OutcomeUnknown => Some("outcome_unknown"),
        ExecutionState::PreconditionChanged => Some("not_sent"),
        _ => None,
    }
}

/// The containers among `resources` with no room left in some window at
/// `now`, each with when it has room again by this clock: when enough of the
/// charges it counts have left every window it is spent in. Pure lookup.
pub(crate) fn spent(
    conn: &Connection,
    windows: &[Window],
    resources: &[&str],
    now: i64,
) -> Result<Vec<(String, i64)>> {
    let mut spent = Vec::new();
    for resource in resources {
        let mut frees = None;
        for window in windows {
            let (from, to) = (
                now.saturating_sub(window.seconds),
                now.saturating_add(window.seconds),
            );
            let used: u32 = conn
                .query_row(
                    "SELECT count(*) FROM effect_log WHERE resource=?1 AND kind='effect' AND at>?2 AND at<=?3",
                    params![resource, from, to],
                    |r| r.get(0),
                )
                .map_err(durable)?;
            if used >= window.effects {
                // Room comes back when the charge that brings the count
                // below the limit leaves the window.
                let leaving: i64 = conn
                    .query_row(
                        "SELECT at FROM effect_log WHERE resource=?1 AND kind='effect' AND at>?2 AND at<=?3 ORDER BY at LIMIT 1 OFFSET ?4",
                        params![resource, from, to, used - window.effects],
                        |r| r.get(0),
                    )
                    .map_err(durable)?;
                let at = leaving.saturating_add(window.seconds).max(now);
                frees = Some(frees.map_or(at, |f: i64| f.max(at)));
            }
        }
        if let Some(frees) = frees {
            spent.push(((*resource).to_owned(), frees));
        }
    }
    Ok(spent)
}

/// Charge one effect per resource, in the transaction that prepares the
/// action; the rows' sequence numbers, in `resources` order.
pub(crate) fn charge(
    tx: &Transaction<'_>,
    entry: &Entry<'_>,
    resources: &[&str],
    now: i64,
) -> Result<Vec<i64>> {
    let mut seqs = Vec::with_capacity(resources.len());
    for resource in resources {
        tx.execute(
            "INSERT INTO effect_log(at,kind,resource,operation,stack,action,peer,principal) VALUES(?1,'effect',?2,?3,?4,?5,?6,?7)",
            params![
                now,
                resource,
                entry.operation.as_str(),
                entry.stack,
                entry.action,
                entry.peer,
                entry.principal
            ],
        )
        .map_err(durable)?;
        seqs.push(tx.last_insert_rowid());
    }
    tally(tx, now, resources.len() as i64, 0)?;
    trim(tx)?;
    Ok(seqs)
}

/// Record a charged effect's outcome, once: a row already finished, or gone,
/// is left as it is.
pub(crate) fn finish(tx: &Transaction<'_>, seq: i64, state: ExecutionState) -> Result<()> {
    if let Some(outcome) = outcome(state) {
        tx.execute(
            "UPDATE effect_log SET outcome=?2 WHERE seq=?1 AND kind='effect' AND outcome IS NULL",
            params![seq, outcome],
        )
        .map_err(durable)?;
    }
    Ok(())
}

/// Add to the day's totals and keep the newest `DAYS_KEPT` days. A day's
/// `charges` counts every container charged that day, sent or not; its
/// `refusals` counts the refusals its refusal rows stand for, booked to the
/// day each row is written (a coalesced row carries refusals from up to an
/// hour before it).
fn tally(tx: &Transaction<'_>, now: i64, charges: i64, refusals: i64) -> Result<()> {
    tx.execute(
        "INSERT INTO effect_days(day,charges,refusals) VALUES(?1,?2,?3) ON CONFLICT(day) DO UPDATE SET charges=charges+excluded.charges,refusals=refusals+excluded.refusals",
        params![now.div_euclid(86_400), charges, refusals],
    )
    .map_err(durable)?;
    tx.execute(
        "DELETE FROM effect_days WHERE day<(SELECT day FROM effect_days ORDER BY day DESC LIMIT 1 OFFSET ?1)",
        [DAYS_KEPT - 1],
    )
    .map_err(durable)?;
    Ok(())
}

/// Keep the newest `LOG_ROWS` rows. The newest row is never removed, so
/// sequence numbers only grow.
fn trim(tx: &Transaction<'_>) -> Result<()> {
    tx.execute(
        "DELETE FROM effect_log WHERE seq<=(SELECT max(seq) FROM effect_log)-?1",
        [LOG_ROWS],
    )
    .map_err(durable)?;
    Ok(())
}

impl ReceiptStore {
    /// Refuse with `RateLimited` an effect on `resources` when any of them
    /// has no room left, recording the refusal (`refuse`) and when every one
    /// has room again (`take_frees_at`). Before prepare: nothing is recorded
    /// for the action. The caller charges in the prepare transaction with no
    /// await in between (see "Atomicity" above).
    pub(crate) fn within_budget(
        &mut self,
        entry: &Entry<'_>,
        resources: &[&str],
        now: i64,
    ) -> Result<()> {
        self.frees_at = None;
        let spent = spent(&self.conn, self.windows, resources, now)?;
        if spent.is_empty() {
            return Ok(());
        }
        for (resource, frees) in &spent {
            self.frees_at = Some(self.frees_at.map_or(*frees, |f| f.max(*frees)));
            self.refuse(entry, resource, now);
        }
        Err(Error(ErrorCode::RateLimited))
    }

    /// After a `RateLimited` refusal: when, by the executor's clock, every
    /// container the refused action would charge has room again.
    pub fn take_frees_at(&mut self) -> Option<i64> {
        self.frees_at.take()
    }

    /// One refusal for `resource`: written when none of its refusal rows is
    /// within `REFUSAL_INTERVAL` of `now` either way (so a clock stepped back
    /// a little delays rows rather than multiplying them, and one stepped
    /// back a long way does not silence them), with every refusal counted
    /// since the last row; otherwise counted.
    fn refuse(&mut self, entry: &Entry<'_>, resource: &str, now: i64) {
        let refusals = self
            .suppressed
            .remove(resource)
            .unwrap_or(0)
            .saturating_add(1);
        match self.write_refusal(entry, resource, refusals, now) {
            Ok(Some(record)) => self.journal(record),
            Ok(None) | Err(_) => {
                if self.suppressed.len() < SUPPRESSED {
                    self.suppressed.insert(resource.to_owned(), refusals);
                }
            }
        }
    }

    fn write_refusal(
        &mut self,
        entry: &Entry<'_>,
        resource: &str,
        refusals: u32,
        now: i64,
    ) -> Result<Option<ActionRecord>> {
        if fs2::available_space(&self.directory).map_err(|_| Error(ErrorCode::StateNotDurable))?
            < 8 * 1024 * 1024
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let tx = self.conn.transaction().map_err(durable)?;
        let recent: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM effect_log WHERE resource=?1 AND kind='refused' AND at>?2 AND at<?3)",
                params![
                    resource,
                    now.saturating_sub(REFUSAL_INTERVAL),
                    now.saturating_add(REFUSAL_INTERVAL)
                ],
                |r| r.get(0),
            )
            .map_err(durable)?;
        if recent {
            return Ok(None);
        }
        tx.execute(
            "INSERT INTO effect_log(at,kind,resource,operation,stack,action,peer,principal,outcome,refusals) VALUES(?1,'refused',?2,?3,?4,?5,?6,?7,'rate_limited',?8)",
            params![
                now,
                resource,
                entry.operation.as_str(),
                entry.stack,
                entry.action,
                entry.peer,
                entry.principal,
                refusals
            ],
        )
        .map_err(durable)?;
        let seq = tx.last_insert_rowid();
        tally(&tx, now, 0, i64::from(refusals))?;
        trim(&tx)?;
        tx.commit().map_err(durable)?;
        let mut record = ActionRecord::new(entry, seq, now, resource);
        record.kind = "refused";
        record.outcome = "rate_limited";
        record.refusals = refusals;
        Ok(Some(record))
    }

    /// Journal a charged effect once its outcome is known.
    pub(crate) fn journal_effect(
        &mut self,
        entry: &Entry<'_>,
        seq: i64,
        at: i64,
        resource: &str,
        state: ExecutionState,
    ) {
        let mut record = ActionRecord::new(entry, seq, at, resource);
        record.outcome = outcome(state).unwrap_or("outcome_unknown");
        self.journal(record);
    }

    fn journal(&mut self, record: ActionRecord) {
        // Drained after every request; a caller that never drains keeps
        // only the newest.
        if self.journal.len() >= JOURNAL {
            self.journal.remove(0);
        }
        self.journal.push(record);
    }

    /// The action-log rows written since the last call, for the executor's
    /// own journal line per effect and per recorded refusal. The table is
    /// the record; this is a convenience copy.
    pub fn take_journal(&mut self) -> Vec<ActionRecord> {
        std::mem::take(&mut self.journal)
    }

    /// Without the budget: for tests of retention at scale, never shipped.
    #[cfg(test)]
    pub(crate) fn unlimited(mut self) -> Self {
        self.windows = &[];
        self
    }
}

/// Journal records kept between drains: more than one stack's members.
const JOURNAL: usize = 64;
const _: () = assert!(limeos_domain::MAX_STACK_MEMBERS < JOURNAL);

#[cfg(test)]
mod tests;
