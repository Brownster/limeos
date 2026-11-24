//! `limeosctl job list` and `job cancel`: the host operator's view of every
//! principal's jobs and cancellation of a queued one on its owner's behalf.
//! Served on the control socket to a root peer only (`respond`), like
//! `job resolve`; the HTTP API has neither. Each request is one store call
//! on the writer; nothing here reaches an executor or runs in the background.
use super::*;
use limeos_domain::{MAX_OPERATOR_JOBS, OperatorCancel, OperatorJobFilter, OperatorJobPage};

/// Distinct states a listing may name: every `JobState`.
const MAX_STATES: usize = 12;

impl Core {
    pub(super) async fn rpc_list_jobs(
        &self,
        filter: OperatorJobFilter,
        limit: u32,
    ) -> Result<OperatorJobPage> {
        if filter.states.is_empty()
            || filter.states.len() > MAX_STATES
            || !(1..=MAX_OPERATOR_JOBS).contains(&limit)
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        self.db.call(move |s| s.operator_jobs(&filter, limit)).await
    }

    /// The root peer names an administrator (`rpc_administrator`: an existing
    /// user whose current role is administrator), recorded as the actor.
    pub(super) async fn rpc_cancel_job(
        &self,
        admin: String,
        job: String,
    ) -> Result<OperatorCancel> {
        if !limeos_domain::opaque_id(&job) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let principal = self.rpc_administrator(admin).await?;
        self.db
            .call(move |s| s.operator_cancel_job(&principal, &job, now()))
            .await
    }
}

#[cfg(test)]
mod tests;
