//! The host operator's listing of every principal's jobs and cancellation of
//! a queued job on its owner's behalf. Core calls these only for a root peer
//! on its control socket (`limeosctl job list`, `job cancel`); the HTTP API
//! has neither.
//!
//! Cancellation is the owner's own step, `transition(queued -> canceled)`,
//! taken through `transition_by` so the administrator the root peer named is
//! recorded beside it. The state change is a compare-and-set on `queued`, and
//! every dispatcher claim is a compare-and-set on `queued` too, both on the
//! store's single writer: whichever commits first wins and the other changes
//! nothing. A queued job holds no resource lock (locks are taken by the claim,
//! migration 7) and no executor has heard of it (core sends an execute only
//! after its claim commits), so cancelling it releases nothing and needs no
//! executor. Its plan stays consumed by the job, so it cannot be queued again,
//! and `canceled` is terminal (`JobState::allows`).
use super::*;
use limeos_domain::{
    CancelOutcome, CancelRefusal, JobFamily, MAX_OPERATOR_JOBS, OperatorCancel, OperatorJob,
    OperatorJobPage,
};

/// One listing row: id, owner, operation, state, resource, claims, created,
/// deadline, open resolution.
type ListRow = (
    String,
    String,
    Option<String>,
    String,
    String,
    i64,
    i64,
    i64,
    bool,
);

fn state_of(text: &str) -> Result<JobState> {
    parse(&format!("\"{text}\""))
}
fn family_of(operation: Option<&str>) -> Result<JobFamily> {
    operation
        .and_then(JobFamily::of_operation)
        .ok_or(Error(ErrorCode::StateNotDurable))
}

impl Store {
    /// Every principal's jobs in `states`, oldest first, at most `limit`
    /// (1 to `MAX_OPERATOR_JOBS`), with how many match in all. Read-only.
    /// No principal: core serves it to a root peer only, which can read the
    /// store's file anyway; it shows owners by username and each job's
    /// headline resource, as the administrator's attention list does, never
    /// plan bodies.
    pub fn operator_jobs(&self, states: &[JobState], limit: u32) -> Result<OperatorJobPage> {
        if states.is_empty() || states.len() > 12 || !(1..=MAX_OPERATOR_JOBS).contains(&limit) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let mut names: Vec<&str> = states.iter().map(|s| s.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        // Closed enum spellings, so quoting them inline is safe.
        let list = names
            .iter()
            .map(|n| format!("'{n}'"))
            .collect::<Vec<_>>()
            .join(",");
        let matching: i64 = self
            .conn
            .query_row(
                &format!("SELECT count(*) FROM jobs WHERE state IN ({list})"),
                [],
                |r| r.get(0),
            )
            .map_err(durable)?;
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT j.id,u.username,json_extract(j.intent,'$.operation'),j.state,j.resource,\
                 (SELECT count(*) FROM job_resources r WHERE r.job=j.id),\
                 COALESCE((SELECT e.created FROM events e WHERE e.job=j.id ORDER BY e.cursor LIMIT 1),0),\
                 j.deadline,\
                 EXISTS(SELECT 1 FROM job_resolutions r WHERE r.job=j.id AND r.confirmation IS NULL) \
                 FROM jobs j JOIN users u ON u.id=j.principal \
                 WHERE j.state IN ({list}) ORDER BY j.rowid LIMIT ?"
            ))
            .map_err(durable)?;
        let rows = stmt
            .query_map([limit], |r| -> rusqlite::Result<ListRow> {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                ))
            })
            .map_err(durable)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(durable)?;
        let jobs = rows
            .into_iter()
            .map(
                |(job, owner, operation, state, resource, claims, created_at, deadline, open)| {
                    Ok(OperatorJob {
                        family: family_of(operation.as_deref())?,
                        operation: operation.unwrap_or_default(),
                        state: state_of(&state)?,
                        job,
                        owner,
                        resource,
                        claims: u32::try_from(claims)
                            .map_err(|_| Error(ErrorCode::StateNotDurable))?,
                        created_at,
                        deadline,
                        resolution_open: open,
                    })
                },
            )
            .collect::<Result<Vec<_>>>()?;
        Ok(OperatorJobPage {
            jobs,
            matching: u32::try_from(matching).map_err(|_| Error(ErrorCode::StateNotDurable))?,
        })
    }

    fn job_family_state(&self, id: &str) -> Result<(JobFamily, JobState)> {
        let (operation, state): (Option<String>, String) = self
            .conn
            .query_row(
                "SELECT json_extract(intent,'$.operation'),state FROM jobs WHERE id=?",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(durable)?
            .ok_or(Error(ErrorCode::NotFound))?;
        Ok((family_of(operation.as_deref())?, state_of(&state)?))
    }

    /// Cancel queued job `id` on its owner's behalf. `admin` is the current
    /// administrator a root peer named; it is recorded as the actor
    /// (`job_canceled_by_administrator`) beside the owner's `canceled`
    /// transition. Whoever owns the job.
    ///
    /// Only what the owner could cancel: a `queued` job. A job in any other
    /// state is refused as `not_queued` (a dispatched one may have had its
    /// execute sent; `limeosctl job resolve` settles a held one), and an
    /// automation observation as `pause_schedule_instead`. A repeat on a
    /// canceled job writes nothing. Refusals change nothing and are answers,
    /// not errors, so the operator is told which rule applied.
    pub fn operator_cancel_job(
        &mut self,
        admin: &Principal,
        id: &str,
        now: i64,
    ) -> Result<OperatorCancel> {
        self.grants(admin)?;
        limeos_policy::authorize_job_resolution(admin)?;
        if !limeos_domain::opaque_id(id) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let (family, state) = self.job_family_state(id)?;
        let answer = |state, outcome, refusal| OperatorCancel {
            job: id.into(),
            family,
            state,
            outcome,
            refusal,
        };
        let refused = |state, why| answer(state, CancelOutcome::Refused, Some(why));
        if family == JobFamily::AutomationObservation {
            return Ok(refused(state, CancelRefusal::PauseScheduleInstead));
        }
        match state {
            JobState::Canceled => {
                return Ok(answer(state, CancelOutcome::AlreadyCanceled, None));
            }
            JobState::Queued => {}
            _ => return Ok(refused(state, CancelRefusal::NotQueued)),
        }
        match self.transition_by(
            id,
            JobState::Queued,
            JobState::Canceled,
            Some(&admin.id),
            now,
        ) {
            Ok(()) => Ok(answer(JobState::Canceled, CancelOutcome::Canceled, None)),
            // No longer queued at the compare-and-set: report what it is now.
            Err(Error(ErrorCode::Conflict)) => {
                let (_, now_state) = self.job_family_state(id)?;
                Ok(if now_state == JobState::Canceled {
                    answer(now_state, CancelOutcome::AlreadyCanceled, None)
                } else {
                    refused(now_state, CancelRefusal::NotQueued)
                })
            }
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
