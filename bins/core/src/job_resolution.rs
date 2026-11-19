//! Operator resolution of `needs_intervention` container and stack jobs.
//!
//! Evidence is a fresh read-only `Inspect` of exactly the job's targets; no
//! start, stop or restart is ever sent. A decision runs in the crash-safe order
//! request (durable) -> `ResolveAction` at the executor -> completion (durable).
//! Only a definite executor refusal withdraws the request; a lost answer leaves
//! it pending for the retry loop, because the executor may already have freed
//! its side and a changed request would then be refused there for good.
use super::*;
use limeos_contracts::{
    ResolutionTargetView, ResolutionView, ResolveJobInput, ResolveJobResponse, ResolveStatus,
};
use limeos_domain::{
    DeclaredOutcome, JobResolution, JobState, Principal, ResolutionEvidence, validate_reason,
};
use limeos_executor_protocol::Request;
use limeos_persistence::{PendingResolution, ResolutionSubject};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

/// What the retry or completion of one recorded request came to.
enum Finish {
    Done(JobResolution),
    /// The executor definitely refused and the request was withdrawn.
    Refused(ErrorCode),
    /// Nothing was settled. The request stays recorded and held.
    Pending(ErrorCode),
}

fn awaiting(state: JobState) -> bool {
    matches!(
        state,
        JobState::NeedsIntervention | JobState::Running | JobState::Verifying
    )
}
const PENDING_MESSAGE: &str = "The request is recorded and the job stays held. The executor could not confirm; core retries automatically. Repeat the same request or check again shortly.";

fn view_of(
    subject: &ResolutionSubject,
    evidence: Option<&ResolutionEvidence>,
    members: Option<Vec<limeos_domain::ResolutionMember>>,
    pending: Option<limeos_domain::ResolutionRequest>,
    resolution: Option<JobResolution>,
) -> Result<ResolutionView> {
    let mut targets = Vec::new();
    let mut allowed = Vec::new();
    if let Some(evidence) = evidence {
        for measured in &evidence.targets {
            let expected = subject
                .targets
                .iter()
                .find(|t| t.resource.strip_prefix("container:") == Some(&measured.container))
                .ok_or(Error(ErrorCode::StateNotDurable))?;
            targets.push(ResolutionTargetView {
                expected: expected.clone(),
                measured: measured.clone(),
            });
        }
        for declared in [DeclaredOutcome::VerifiedDone, DeclaredOutcome::NotDone] {
            if evidence.blocking(declared).is_empty() {
                allowed.push(declared);
            }
        }
        allowed.push(DeclaredOutcome::AcceptedDivergence);
    }
    Ok(ResolutionView {
        job: subject.job.clone(),
        state: subject.state,
        kind: subject.kind,
        operation: subject.operation,
        targets,
        evidence_digest: evidence.map(|e| e.digest()).transpose()?,
        allowed,
        members,
        pending,
        resolution,
    })
}

impl Core {
    /// Fresh, read-only measurement of exactly the job's targets. A container
    /// the Engine no longer has is evidence too; any other failure is not.
    async fn gather_evidence(&self, subject: &ResolutionSubject) -> Result<ResolutionEvidence> {
        let mut current = Vec::with_capacity(subject.targets.len());
        for target in &subject.targets {
            current.push(match self.inspect(&target.resource).await {
                Ok(snapshot) => Some(snapshot),
                Err(Error(ErrorCode::NotFound)) => None,
                Err(e) => return Err(e),
            });
        }
        ResolutionEvidence::observe(
            subject.operation,
            &subject.targets.iter().collect::<Vec<_>>(),
            &current,
        )
    }

    /// Evidence and any existing record for one job. Administrator only; the
    /// caller has already authenticated `principal`.
    pub(super) async fn resolution_view(
        &self,
        principal: Principal,
        job: String,
    ) -> Result<ResolutionView> {
        if !limeos_domain::opaque_id(&job) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let id = job.clone();
        let (subject, members, pending, resolution) = self
            .db
            .call(move |s| {
                let subject = s.resolution_subject(&principal, &id)?;
                Ok((
                    subject,
                    s.resolution_members(&principal, &id)?,
                    s.pending_resolutions()?
                        .into_iter()
                        .find(|p| p.request.job == id)
                        .map(|p| p.request),
                    s.job_resolution(&principal, &id)?,
                ))
            })
            .await?;
        let evidence = if resolution.is_none() && awaiting(subject.state) {
            Some(self.gather_evidence(&subject).await?)
        } else {
            None
        };
        view_of(&subject, evidence.as_ref(), members, pending, resolution)
    }

    /// Decide one job. `principal` is a current administrator whose
    /// re-authentication the caller has already settled.
    pub(super) async fn resolve_as(
        &self,
        principal: Principal,
        job: String,
        declared: DeclaredOutcome,
        reason: String,
        shown_digest: String,
    ) -> Result<ResolveJobResponse> {
        validate_reason(declared, &reason)?;
        let fresh = self.resolution_view(principal.clone(), job.clone()).await?;
        if fresh.resolution.is_some() || !awaiting(fresh.state) {
            return Err(Error(ErrorCode::Conflict));
        }
        let respond = |status, message: &str, blocking: Vec<String>| ResolveJobResponse {
            status,
            message: message.into(),
            resolution: None,
            view: Some(fresh.clone()),
            blocking,
        };
        if fresh.evidence_digest.as_deref() != Some(shown_digest.as_str()) {
            return Ok(respond(
                ResolveStatus::StaleEvidence,
                "The state changed since the evidence was shown. Review the new evidence and decide again.",
                Vec::new(),
            ));
        }
        let blocking: Vec<String> = fresh
            .targets
            .iter()
            .filter(|t| match declared {
                DeclaredOutcome::VerifiedDone => !t.measured.intended_state_reached,
                DeclaredOutcome::NotDone => !t.measured.matches_expected,
                DeclaredOutcome::AcceptedDivergence => false,
            })
            .map(|t| t.measured.container.clone())
            .collect();
        if !blocking.is_empty() {
            return Ok(respond(
                ResolveStatus::UnsupportedDeclaration,
                "The measured state does not support that declaration.",
                blocking,
            ));
        }
        // Rebuild the exact evidence the digest names, from what was measured.
        let evidence = ResolutionEvidence {
            targets: fresh.targets.iter().map(|t| t.measured.clone()).collect(),
        };
        let (p, id, why, digest) = (principal.clone(), job.clone(), reason, shown_digest);
        let pending = self
            .db
            .call(move |s| s.request_resolution(&p, &id, declared, &why, &evidence, &digest, now()))
            .await?;
        // Detached: if the HTTP deadline drops this future the executor step and
        // completion still run, and the retry loop covers a crash.
        let (this, actor) = (self.clone(), principal.clone());
        let finish = tokio::spawn(async move { this.finish_resolution(&pending, &actor).await })
            .await
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        match finish {
            Finish::Done(resolution) => Ok(ResolveJobResponse {
                status: ResolveStatus::Resolved,
                message: "Resolved. The job no longer holds its resources.".into(),
                resolution: Some(resolution),
                view: None,
                blocking: Vec::new(),
            }),
            // Withdrawn, so a new decision is possible: show what changed.
            Finish::Refused(ErrorCode::Conflict) => Ok(ResolveJobResponse {
                status: ResolveStatus::StaleEvidence,
                message: "The executor measured something different. Nothing was changed; review the evidence and decide again.".into(),
                resolution: None,
                view: self.resolution_view(principal, job).await.ok(),
                blocking: Vec::new(),
            }),
            Finish::Refused(_) => Err(Error(ErrorCode::Conflict)),
            Finish::Pending(_) => Ok(ResolveJobResponse {
                status: ResolveStatus::Pending,
                message: PENDING_MESSAGE.into(),
                resolution: None,
                view: None,
                blocking: Vec::new(),
            }),
        }
    }

    /// Step 2 and 3 for one recorded request. Idempotent at the executor and in
    /// the store, so it is safe to repeat after any interruption.
    async fn finish_resolution(&self, pending: &PendingResolution, actor: &Principal) -> Finish {
        let request = &pending.request;
        let budget = Duration::from_secs(10 + 3 * request.evidence.targets.len() as u64);
        let confirmation = self
            .executor(
                Request::ResolveAction {
                    version: VERSION,
                    kind: pending.kind,
                    action: request.job.clone(),
                    digest: pending.plan_digest.clone(),
                    evidence_digest: request.evidence_digest.clone(),
                    declared: request.declared,
                    resolved_by: request.resolved_by.clone(),
                },
                budget,
            )
            .await;
        match confirmation {
            Ok(receipt) => {
                // An answer without a confirmation settles nothing.
                let Some(confirmation) = receipt.resolution else {
                    return Finish::Pending(ErrorCode::Unavailable);
                };
                let job = request.job.clone();
                match self
                    .db
                    .call(move |s| s.complete_resolution(&job, &confirmation, now()))
                    .await
                {
                    Ok(resolution) => Finish::Done(resolution),
                    Err(e) => Finish::Pending(e.0),
                }
            }
            // Only these are a definite "no": the executor answered and did not
            // free anything. Everything else, including a lost connection, may
            // have happened and stays recorded.
            Err(Error(
                code @ (ErrorCode::Conflict | ErrorCode::NotFound | ErrorCode::Forbidden),
            )) => {
                let (actor, job) = (actor.clone(), request.job.clone());
                match self
                    .db
                    .call(move |s| s.withdraw_resolution(&actor, &job, now()))
                    .await
                {
                    // Already withdrawn or completed elsewhere is settled enough.
                    Ok(()) | Err(Error(ErrorCode::NotFound)) => Finish::Refused(code),
                    Err(e) => Finish::Pending(e.0),
                }
            }
            Err(e) => Finish::Pending(e.0),
        }
    }

    /// Retry every recorded request that is not yet complete: run once at
    /// startup, then on the dispatcher tick with a per-job pause. `min_age`
    /// leaves a request alone while its own handler is still working on it.
    pub(super) async fn complete_pending_resolutions(
        &self,
        retry: &mut HashMap<String, Instant>,
        min_age: i64,
    ) {
        let Ok(pending) = self.db.call(|s| s.pending_resolutions()).await else {
            return;
        };
        retry.retain(|job, _| pending.iter().any(|p| &p.request.job == job));
        for pending in pending {
            let job = pending.request.job.clone();
            if now() - pending.request.requested_at < min_age
                || retry
                    .get(&job)
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(30))
            {
                continue;
            }
            retry.insert(job.clone(), Instant::now());
            let id = pending.request.resolved_by.clone();
            // The administrator as they are now: a demoted one cannot withdraw.
            let actor = match self.db.call(move |s| s.resolution_principal(&id)).await {
                Ok(actor) => actor,
                Err(e) => {
                    tracing::warn!(code=?e.0,"job resolution request has no administrator");
                    continue;
                }
            };
            match self.finish_resolution(&pending, &actor).await {
                Finish::Done(_) => tracing::info!("pending job resolution completed"),
                Finish::Refused(code) => {
                    tracing::warn!(
                        ?code,
                        "executor refused a pending job resolution; withdrawn"
                    )
                }
                Finish::Pending(code) => {
                    tracing::warn!(?code, "job resolution stays pending")
                }
            }
        }
    }

    /// HTTP read: current administrator session, no CSRF (a GET).
    pub(super) async fn human_resolution_view(
        &self,
        token: String,
        job: String,
    ) -> Result<ResolutionView> {
        let digest = limeos_identity::digest(&token);
        let principal = self
            .db
            .call(move |s| {
                let principal = s.authenticate(&digest, None, now())?;
                limeos_policy::authorize_job_resolution(&principal)?;
                Ok(principal)
            })
            .await?;
        self.resolution_view(principal, job).await
    }

    /// HTTP read: current administrator session, no CSRF (a GET). Persistence
    /// authorizes again, so a role change since the session was made is seen.
    pub(super) async fn human_needs_attention(
        &self,
        token: String,
    ) -> Result<Vec<limeos_domain::NeedsAttentionJob>> {
        let digest = limeos_identity::digest(&token);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, None, now())?;
                s.needs_attention_jobs(&principal)
            })
            .await
    }

    /// HTTP decision: administrator session with its CSRF binding and a fresh
    /// password check. The password is verified before any executor contact.
    pub(super) async fn human_resolve(
        &self,
        token: String,
        csrf: String,
        job: String,
        input: ResolveJobInput,
    ) -> Result<ResolveJobResponse> {
        if !limeos_contracts::password_well_formed(&input.password) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let (session, csrf) = (
            limeos_identity::digest(&token),
            limeos_identity::digest(&csrf),
        );
        let (s, c) = (session.clone(), csrf.clone());
        let record = self
            .db
            .call(move |store| store.resolution_reauthentication(&s, &c, now()))
            .await?;
        let verified = self
            .password(limeos_contracts::PasswordRequest::Verify {
                password: input.password,
                encoded: record.password_hash.clone(),
            })
            .await?;
        if !verified.valid {
            return Err(Error(ErrorCode::Unauthenticated));
        }
        // The password check is slow; the same session must still be the same
        // administrator with the same grants afterwards.
        let principal = self
            .db
            .call(move |store| {
                let principal = store.authenticate(&session, Some(&csrf), now())?;
                limeos_policy::authorize_job_resolution(&principal)?;
                Ok(principal)
            })
            .await?;
        if principal != record.principal {
            return Err(Error(ErrorCode::Expired));
        }
        self.resolve_as(
            principal,
            job,
            input.declared,
            input.reason,
            input.evidence_digest,
        )
        .await
    }

    /// The root peer names an administrator; that user's current role decides.
    async fn rpc_administrator(&self, username: String) -> Result<Principal> {
        if !limeos_domain::identifier(&username) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let record = self
            .db
            .call(move |s| s.login_record(&username))
            .await?
            .ok_or(Error(ErrorCode::NotFound))?;
        limeos_policy::authorize_job_resolution(&record.principal)?;
        Ok(record.principal)
    }
    pub(super) async fn rpc_resolution_view(
        &self,
        admin: String,
        job: String,
    ) -> Result<ResolutionView> {
        let principal = self.rpc_administrator(admin).await?;
        self.resolution_view(principal, job).await
    }
    pub(super) async fn rpc_resolve(
        &self,
        admin: String,
        job: String,
        declared: DeclaredOutcome,
        reason: String,
        evidence_digest: String,
    ) -> Result<ResolveJobResponse> {
        // The reason rules apply to what the operator wrote, then the channel is
        // appended so the stored record shows how it was entered.
        validate_reason(declared, &reason)?;
        let principal = self.rpc_administrator(admin).await?;
        self.resolve_as(
            principal,
            job,
            declared,
            format!("{reason} [limeosctl, root peer]"),
            evidence_digest,
        )
        .await
    }
}

#[cfg(test)]
mod tests;
