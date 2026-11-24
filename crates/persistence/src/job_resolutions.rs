//! Operator resolution of `needs_intervention` container and stack jobs.
//!
//! Three durable steps, each its own call so a crash between them is safe.
//!
//! 1. `request_resolution` records who, what and on which evidence. The job and
//!    its locks are untouched.
//! 2. Core asks the executor to resolve its own rows (`ResolveAction`).
//! 3. `complete_resolution` stores the executor's confirmation, sets the job
//!    `resolved` and releases the locks in one transaction.
//!
//! After a restart `pending_resolutions` lists requests that still need step 2
//! (idempotent at the executor) and step 3. Original job, plan, receipt and
//! result rows are never modified.
//!
//! Recovery keeps polling a held job while a request is open, and whichever
//! settles the job first wins: recovery records only what the executor's
//! receipt or closure proves, which is at least as strong as an operator's
//! declaration, and holding recovery back would not exclude the race anyway
//! (a lookup already on its way when the request is recorded still lands
//! after it). A request whose job has left the held states can then never
//! complete, so `supersede_resolutions` (or `supersede_resolution`, when a
//! completion loses the race) closes it: the row is deleted, as a withdrawn
//! one is, and a `job_resolution_superseded` event records what the job
//! settled as, so the request no longer pins the job's history and the
//! operator sees it settled rather than pending. Retry listings name only
//! requests whose job is still held.
use super::*;
use limeos_domain::{
    ActionPlan, ContainerAction, ContainerReceipt, ContainerSnapshot, DeclaredOutcome,
    JobResolution, NeedsAttentionJob, ResolutionEvidence, ResolutionMember, ResolutionRequest,
    ResolveKind, ResolveReceipt, StackReceipt,
};

/// What core needs to gather evidence and to address the executor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolutionSubject {
    pub job: String,
    pub state: JobState,
    pub kind: ResolveKind,
    pub operation: ContainerAction,
    /// Digest of the stored plan; the executor keys receipts by it.
    pub plan_digest: String,
    /// The plan's targets as `expected`; inspect each, then
    /// `ResolutionEvidence::observe(operation, ..)`.
    pub targets: Vec<ContainerSnapshot>,
    /// The plan's expiry: the executor judges a missing receipt against its
    /// prune watermark with it (`limeos_domain::ReceiptAbsence`).
    pub expires_at: i64,
    /// The stored plan itself, which `ResolveAction` carries so the executor
    /// can tie `expires_at` to `plan_digest`.
    pub plan: ActionPlan,
}

/// A recorded request together with the executor addressing for step 2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingResolution {
    pub request: ResolutionRequest,
    pub kind: ResolveKind,
    pub plan_digest: String,
    /// The stored plan, sent with `ResolveAction`.
    pub plan: ActionPlan,
    /// The plan's operation and targets, to measure the evidence again.
    pub operation: ContainerAction,
    pub targets: Vec<ContainerSnapshot>,
    /// Core holds a record a `never_recorded` closure for the job would
    /// contradict (`jobs::record_disputes`): such a closure is not evidence,
    /// and every declaration rests on the operator's measurement. Sent with
    /// `ResolveAction` so the executor applies the same rule.
    pub disputed: bool,
}

/// The results table of a job of `kind`.
fn results_of(kind: ResolveKind) -> &'static str {
    match kind {
        ResolveKind::Container => "container_results",
        ResolveKind::Stack => "stack_results",
    }
}

fn subject_of(id: &str, state: &str, intent: &str) -> Result<ResolutionSubject> {
    let state = parse::<JobState>(&format!("\"{state}\""))?;
    let (kind, operation, plan_digest, targets, expires_at, plan) = match parse::<Intent>(intent)? {
        Intent::ContainerRestart { plan }
        | Intent::ContainerStart { plan }
        | Intent::ContainerStop { plan } => (
            ResolveKind::Container,
            plan.operation,
            limeos_identity::digest(&json(&plan)?),
            vec![plan.expected.clone()],
            plan.expires_at,
            ActionPlan::Container(plan),
        ),
        Intent::StackRestart { plan }
        | Intent::StackStart { plan }
        | Intent::StackStop { plan } => (
            ResolveKind::Stack,
            plan.operation,
            limeos_identity::digest(&json(&plan)?),
            plan.targets().into_iter().cloned().collect(),
            plan.expires_at,
            ActionPlan::Stack(plan),
        ),
        _ => return Err(Error(ErrorCode::Forbidden)),
    };
    Ok(ResolutionSubject {
        job: id.into(),
        state,
        kind,
        operation,
        plan_digest,
        targets,
        expires_at,
        plan,
    })
}

/// The job states a resolution applies to: every state that holds locks.
const HELD: &str = "('running','verifying','outcome_unknown','needs_intervention')";

/// Supersede the open requests whose job has left `HELD` and that match
/// `filter` (an SQL tail; `?1` binds `job`).
fn supersede(
    tx: &Transaction<'_>,
    filter: &str,
    job: Option<&str>,
    now: i64,
) -> Result<Vec<(String, JobState)>> {
    let found: Vec<(String, String, String)> = {
        let mut stmt = tx
            .prepare(&format!(
                "SELECT r.job,r.resolved_by,j.state FROM job_resolutions r JOIN jobs j ON j.id=r.job WHERE r.confirmation IS NULL AND j.state NOT IN {HELD} {filter}"
            ))
            .map_err(durable)?;
        let row = |r: &rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?, r.get(2)?));
        match job {
            Some(job) => stmt.query_map([job], row),
            None => stmt.query_map([], row),
        }
        .map_err(durable)?
        .collect::<std::result::Result<_, _>>()
        .map_err(durable)?
    };
    let mut superseded = Vec::with_capacity(found.len());
    for (job, resolved_by, state) in found {
        let state = parse::<JobState>(&format!("\"{state}\""))?;
        if tx
            .execute(
                "DELETE FROM job_resolutions WHERE job=? AND confirmation IS NULL",
                [&job],
            )
            .map_err(durable)?
            != 1
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        event(
            tx,
            Some(&resolved_by),
            Some(&job),
            EventKind::JobResolutionSuperseded { state },
            now,
        )?;
        superseded.push((job, state));
    }
    Ok(superseded)
}

type Row = (String, String, String, String, String, i64, String, i64);
const REQUEST_COLUMNS: &str =
    "job,resolved_by,declared,reason,evidence,requested,evidence_digest,rowid";

fn request_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
    ))
}
fn request_of(row: Row) -> Result<ResolutionRequest> {
    let (job, resolved_by, declared, reason, evidence, requested_at, evidence_digest, _) = row;
    let request = ResolutionRequest {
        job,
        resolved_by,
        declared: parse(&format!("\"{declared}\""))?,
        reason,
        evidence: parse(&evidence)?,
        evidence_digest,
        requested_at,
    };
    // Stored rows are re-validated: a damaged request is never acted on.
    request
        .validate()
        .map_err(|_| Error(ErrorCode::StateNotDurable))?;
    Ok(request)
}

impl Store {
    /// Read-only: the job's targets and executor addressing. Administrator only.
    pub fn resolution_subject(
        &self,
        principal: &Principal,
        job: &str,
    ) -> Result<ResolutionSubject> {
        self.grants(principal)?;
        limeos_policy::authorize_job_resolution(principal)?;
        let (state, intent): (String, String) = self
            .conn
            .query_row("SELECT state,intent FROM jobs WHERE id=?", [job], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()
            .map_err(durable)?
            .ok_or(Error(ErrorCode::NotFound))?;
        subject_of(job, &state, &intent)
    }

    /// Every principal's container and stack jobs that hold locks until an
    /// administrator settles them: `needs_intervention`, plus `running` or
    /// `verifying` left by an earlier core generation (the same test as
    /// `request_resolution`). Oldest first, at most 64. Read-only; administrator
    /// only, so everyone else keeps seeing just their own jobs.
    pub fn needs_attention_jobs(&self, principal: &Principal) -> Result<Vec<NeedsAttentionJob>> {
        self.grants(principal)?;
        limeos_policy::authorize_job_resolution(principal)?;
        let mut stmt = self
            .conn
            .prepare(
                "SELECT j.id,u.username,j.intent,j.state,COALESCE((SELECT min(created) FROM events WHERE job=j.id),0) \
                 FROM jobs j JOIN users u ON u.id=j.principal \
                 WHERE (j.state='needs_intervention' OR (j.state IN ('running','verifying') AND j.generation<?)) \
                 AND json_extract(j.intent,'$.operation') IN ('container_restart','container_start','container_stop','stack_restart','stack_start','stack_stop') \
                 ORDER BY j.rowid LIMIT 64",
            )
            .map_err(durable)?;
        stmt.query_map([self.generation], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })
        .map_err(durable)?
        .map(|row| {
            let (job, owner, intent, state, created_at) = row.map_err(durable)?;
            let state = parse::<JobState>(&format!("\"{state}\""))?;
            let (kind, operation, resource, targets) = match parse::<Intent>(&intent)? {
                Intent::ContainerRestart { plan }
                | Intent::ContainerStart { plan }
                | Intent::ContainerStop { plan } => (
                    ResolveKind::Container,
                    plan.operation,
                    plan.expected.resource.clone(),
                    vec![plan.expected.resource],
                ),
                Intent::StackRestart { plan }
                | Intent::StackStart { plan }
                | Intent::StackStop { plan } => (
                    ResolveKind::Stack,
                    plan.operation,
                    plan.expected.resource.clone(),
                    plan.targets()
                        .into_iter()
                        .map(|t| t.resource.clone())
                        .collect(),
                ),
                _ => return Err(Error(ErrorCode::StateNotDurable)),
            };
            Ok(NeedsAttentionJob {
                job,
                owner,
                kind,
                operation,
                state,
                resource,
                targets,
                created_at,
            })
        })
        .collect()
    }

    /// The session's administrator and stored password hash, for the fresh
    /// re-authentication that precedes a resolution. Core verifies the password;
    /// nothing here proves it. The session, its CSRF binding and the role are
    /// checked now.
    pub fn resolution_reauthentication(
        &self,
        session: &str,
        csrf: &str,
        now: i64,
    ) -> Result<LoginRecord> {
        let principal = identity_store::authenticate(&self.conn, session, Some(csrf), now)?;
        identity_store::current_grants(&self.conn, &principal)?;
        limeos_policy::authorize_job_resolution(&principal)?;
        let password_hash = self
            .conn
            .query_row(
                "SELECT password_hash FROM users WHERE id=?",
                [&principal.id],
                |r| r.get(0),
            )
            .map_err(durable)?;
        Ok(LoginRecord {
            principal,
            password_hash,
        })
    }

    /// The current principal for a user ID, so a retry acts with today's role
    /// and grant revision. Internal; no actor.
    pub fn resolution_principal(&self, id: &str) -> Result<Principal> {
        let (role, grant_revision): (String, i64) = self
            .conn
            .query_row(
                "SELECT role,grant_revision FROM users WHERE id=?",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(durable)?
            .ok_or(Error(ErrorCode::NotFound))?;
        Ok(Principal {
            id: id.into(),
            role: parse(&role)?,
            grant_revision,
        })
    }

    /// The executor's recorded per-target outcomes for the job, whoever owns
    /// it. `None` before any receipt exists. Administrator only.
    pub fn resolution_members(
        &self,
        principal: &Principal,
        job: &str,
    ) -> Result<Option<Vec<ResolutionMember>>> {
        let subject = self.resolution_subject(principal, job)?;
        // Receipt resources were validated with their receipt; a spelling
        // that is not a container resource is shown whole, never trimmed.
        let id = |resource: &str| {
            limeos_domain::container_resource_id(resource)
                .unwrap_or(resource)
                .to_owned()
        };
        let table = results_of(subject.kind);
        let receipt: Option<String> = self
            .conn
            .query_row(
                &format!("SELECT receipt FROM {table} WHERE job=?"),
                [job],
                |r| r.get(0),
            )
            .optional()
            .map_err(durable)?;
        receipt
            .map(|body| {
                Ok(match subject.kind {
                    ResolveKind::Container => {
                        let r: ContainerReceipt = parse(&body)?;
                        vec![ResolutionMember {
                            container: id(&r.before.resource),
                            state: r.state,
                            error: r.error,
                        }]
                    }
                    ResolveKind::Stack => parse::<StackReceipt>(&body)?
                        .members
                        .into_iter()
                        .map(|m| ResolutionMember {
                            container: id(&m.resource),
                            state: m.state,
                            error: m.error,
                        })
                        .collect(),
                })
            })
            .transpose()
    }

    /// Step 1. `evidence` is the fresh executor measurement; `shown_digest` is
    /// the digest the administrator was shown. A mismatch is `Conflict` (the
    /// page re-fetches). `Conflict` also when another request is open or
    /// complete for the job, the job is not awaiting resolution, or the
    /// evidence does not support `declared`. An identical repeat by the same
    /// administrator returns the open request unchanged.
    #[allow(clippy::too_many_arguments)]
    pub fn request_resolution(
        &mut self,
        principal: &Principal,
        job: &str,
        declared: DeclaredOutcome,
        reason: &str,
        evidence: &ResolutionEvidence,
        shown_digest: &str,
        now: i64,
    ) -> Result<PendingResolution> {
        self.grants(principal)?;
        limeos_policy::authorize_job_resolution(principal)?;
        limeos_domain::validate_reason(declared, reason)?;
        let digest = evidence.digest()?;
        if digest != shown_digest {
            return Err(Error(ErrorCode::Conflict));
        }
        let generation = self.generation;
        self.write(|tx| {
            let (state, intent, job_generation): (String, String, i64) = tx
                .query_row(
                    "SELECT state,intent,generation FROM jobs WHERE id=?",
                    [job],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(durable)?
                .ok_or(Error(ErrorCode::NotFound))?;
            let subject = subject_of(job, &state, &intent)?;
            // Running/verifying is only uncertain when the core that owned it
            // is gone; a live job is still owned by its worker.
            if !(state == "needs_intervention"
                || (matches!(state.as_str(), "running" | "verifying")
                    && job_generation < generation))
            {
                return Err(Error(ErrorCode::Conflict));
            }
            let mut expected: Vec<&str> = subject
                .targets
                .iter()
                .filter_map(|t| limeos_domain::container_resource_id(&t.resource))
                .collect();
            expected.sort_unstable();
            if evidence
                .targets
                .iter()
                .map(|t| t.container.as_str())
                .ne(expected.iter().copied())
            {
                return Err(Error(ErrorCode::InvalidInput));
            }
            evidence.check(declared)?;
            let existing: Option<Row> = tx
                .query_row(
                    &format!("SELECT {REQUEST_COLUMNS} FROM job_resolutions WHERE job=?"),
                    [job],
                    request_row,
                )
                .optional()
                .map_err(durable)?;
            let request = ResolutionRequest {
                job: job.into(),
                resolved_by: principal.id.clone(),
                declared,
                reason: reason.into(),
                evidence: evidence.clone(),
                evidence_digest: digest,
                requested_at: now,
            };
            if let Some(row) = existing {
                let open = request_of(row)?;
                // The request time is part of the first write only.
                return if (&open.job, &open.resolved_by, open.declared, &open.reason)
                    == (&request.job, &request.resolved_by, declared, &request.reason)
                    && open.evidence_digest == request.evidence_digest
                {
                    Ok(PendingResolution {
                        request: open,
                        disputed: super::jobs::record_disputes(tx, job, results_of(subject.kind))?,
                        kind: subject.kind,
                        plan_digest: subject.plan_digest,
                        plan: subject.plan,
                        operation: subject.operation,
                        targets: subject.targets,
                    })
                } else {
                    Err(Error(ErrorCode::Conflict))
                };
            }
            request.validate()?;
            tx.execute(
                "INSERT INTO job_resolutions(job,resolved_by,declared,reason,evidence,evidence_digest,requested) VALUES(?,?,?,?,?,?,?)",
                params![
                    job,
                    principal.id,
                    declared.as_str(),
                    reason,
                    json(evidence)?,
                    request.evidence_digest,
                    now
                ],
            )
            .map_err(|_| Error(ErrorCode::Conflict))?;
            event(
                tx,
                Some(&principal.id),
                Some(job),
                EventKind::JobResolutionRequested { declared },
                now,
            )?;
            Ok(PendingResolution {
                request,
                disputed: super::jobs::record_disputes(tx, job, results_of(subject.kind))?,
                kind: subject.kind,
                plan_digest: subject.plan_digest,
                plan: subject.plan,
                operation: subject.operation,
                targets: subject.targets,
            })
        })
    }

    /// Drop an open request that the executor refused (`Conflict`, `NotFound`,
    /// `Forbidden`), so the administrator can look again. Never call this after
    /// a transport failure: the executor may already have confirmed, and a
    /// changed request would then be refused there for good. A completed
    /// resolution cannot be withdrawn.
    pub fn withdraw_resolution(
        &mut self,
        principal: &Principal,
        job: &str,
        now: i64,
    ) -> Result<()> {
        self.grants(principal)?;
        limeos_policy::authorize_job_resolution(principal)?;
        self.write(|tx| {
            let open = tx
                .execute(
                    "DELETE FROM job_resolutions WHERE job=? AND confirmation IS NULL",
                    [job],
                )
                .map_err(durable)?;
            if open != 1 {
                return Err(Error(ErrorCode::NotFound));
            }
            event(
                tx,
                Some(&principal.id),
                Some(job),
                EventKind::JobResolutionWithdrawn,
                now,
            )
        })
    }

    /// The first page of `pending_resolutions_after`.
    pub fn pending_resolutions(&self) -> Result<Vec<PendingResolution>> {
        self.pending_resolutions_after("")
    }

    /// Open requests whose job is still held, so they still need the executor
    /// (step 2) or completion (step 3): those whose job ID sorts after
    /// `cursor`, in job ID order, at most `RECOVERY_BATCH`. A caller that
    /// pages through them with the cursor reaches every one, however many are
    /// open. A request whose job has settled is never listed: it is
    /// superseded (`supersede_resolutions`). Internal, no actor.
    pub fn pending_resolutions_after(&self, cursor: &str) -> Result<Vec<PendingResolution>> {
        self.open_requests(
            &format!("r.job>?1 ORDER BY r.job LIMIT {}", crate::RECOVERY_BATCH),
            cursor,
        )
    }

    /// The open request for one job, if its job is still held.
    pub fn pending_resolution(&self, job: &str) -> Result<Option<PendingResolution>> {
        Ok(self.open_requests("r.job=?1", job)?.into_iter().next())
    }

    fn open_requests(&self, filter: &str, value: &str) -> Result<Vec<PendingResolution>> {
        let columns = REQUEST_COLUMNS
            .split(',')
            .map(|c| format!("r.{c}"))
            .collect::<Vec<_>>()
            .join(",");
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {columns} FROM job_resolutions r JOIN jobs j ON j.id=r.job WHERE r.confirmation IS NULL AND j.state IN {HELD} AND {filter}"
            ))
            .map_err(durable)?;
        let rows = stmt
            .query_map([value], request_row)
            .map_err(durable)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(durable)?;
        rows.into_iter()
            .map(|row| {
                let request = request_of(row)?;
                let (state, intent): (String, String) = self
                    .conn
                    .query_row(
                        "SELECT state,intent FROM jobs WHERE id=?",
                        [&request.job],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .map_err(durable)?;
                let subject = subject_of(&request.job, &state, &intent)?;
                Ok(PendingResolution {
                    disputed: super::jobs::record_disputes(
                        &self.conn,
                        &request.job,
                        results_of(subject.kind),
                    )?,
                    request,
                    kind: subject.kind,
                    plan_digest: subject.plan_digest,
                    plan: subject.plan,
                    operation: subject.operation,
                    targets: subject.targets,
                })
            })
            .collect()
    }

    /// Close every open request (at most `RECOVERY_BATCH` per call, oldest
    /// first) whose job has left the held states, as superseded: the row is
    /// deleted and one `job_resolution_superseded` event, attributed to the
    /// requesting administrator, records the state the job settled as.
    /// Returns each job and that state. Internal, no actor.
    pub fn supersede_resolutions(&mut self, now: i64) -> Result<Vec<(String, JobState)>> {
        self.write(|tx| {
            supersede(
                tx,
                &format!("ORDER BY r.rowid LIMIT {}", crate::RECOVERY_BATCH),
                None,
                now,
            )
        })
    }

    /// As `supersede_resolutions`, for one job: after a completion found the
    /// job no longer held. `None` when the job is still held or no request is
    /// open.
    pub fn supersede_resolution(&mut self, job: &str, now: i64) -> Result<Option<JobState>> {
        self.write(|tx| {
            Ok(supersede(tx, "AND r.job=?1", Some(job), now)?
                .into_iter()
                .next()
                .map(|(_, state)| state))
        })
    }

    /// Step 3. Only with the executor's confirmation of exactly the recorded
    /// request. Sets the job `resolved`, which releases its locks, and writes the
    /// audit event, all in one transaction. A repeat with the same confirmation
    /// returns the stored record.
    pub fn complete_resolution(
        &mut self,
        job: &str,
        confirmation: &ResolveReceipt,
        now: i64,
    ) -> Result<JobResolution> {
        self.write(|tx| {
            let row: Row = tx
                .query_row(
                    &format!("SELECT {REQUEST_COLUMNS} FROM job_resolutions WHERE job=?"),
                    [job],
                    request_row,
                )
                .optional()
                .map_err(durable)?
                .ok_or(Error(ErrorCode::NotFound))?;
            let request = request_of(row)?;
            let (state, intent): (String, String) = tx
                .query_row("SELECT state,intent FROM jobs WHERE id=?", [job], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .map_err(durable)?;
            let subject = subject_of(job, &state, &intent)?;
            if confirmation.kind != subject.kind
                || confirmation.action != job
                || confirmation.evidence_digest != request.evidence_digest
                || confirmation.declared != request.declared
                || confirmation.resolved_by != request.resolved_by
            {
                return Err(Error(ErrorCode::Conflict));
            }
            let (stored, completed): (Option<String>, Option<i64>) = tx
                .query_row(
                    "SELECT confirmation,completed FROM job_resolutions WHERE job=?",
                    [job],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(durable)?;
            let record = |resolved_at| JobResolution {
                job: request.job.clone(),
                resolved_by: request.resolved_by.clone(),
                declared: request.declared,
                reason: request.reason.clone(),
                evidence: request.evidence.clone(),
                evidence_digest: request.evidence_digest.clone(),
                requested_at: request.requested_at,
                resolved_at,
            };
            if let (Some(_), Some(at)) = (stored, completed) {
                return if state == "resolved" {
                    Ok(record(at))
                } else {
                    Err(Error(ErrorCode::StateNotDurable))
                };
            }
            tx.execute(
                "UPDATE job_resolutions SET confirmation=?,completed=? WHERE job=? AND confirmation IS NULL",
                params![json(confirmation)?, now, job],
            )
            .map_err(durable)?;
            // The schema trigger releases the locks as the state leaves the
            // lock set; the guard rejects any state but an uncertain one.
            if tx
                .execute(
                    "UPDATE jobs SET state='resolved',settled=? WHERE id=? AND state IN ('running','verifying','needs_intervention')",
                    params![now, job],
                )
                .map_err(|_| Error(ErrorCode::Conflict))?
                != 1
            {
                return Err(Error(ErrorCode::Conflict));
            }
            let locks: i64 = tx
                .query_row(
                    "SELECT count(*) FROM resource_locks WHERE job=?",
                    [job],
                    |r| r.get(0),
                )
                .map_err(durable)?;
            if locks != 0 {
                return Err(Error(ErrorCode::StateNotDurable));
            }
            event(
                tx,
                Some(&request.resolved_by),
                Some(job),
                EventKind::JobResolved {
                    declared: request.declared,
                },
                now,
            )?;
            Ok(record(now))
        })
    }

    /// The completed audit record, if the job was resolved. Administrator only.
    pub fn job_resolution(
        &self,
        principal: &Principal,
        job: &str,
    ) -> Result<Option<JobResolution>> {
        self.grants(principal)?;
        limeos_policy::authorize_job_resolution(principal)?;
        let row: Option<(Row, i64)> = self
            .conn
            .query_row(
                &format!("SELECT {REQUEST_COLUMNS},completed FROM job_resolutions WHERE job=? AND confirmation IS NOT NULL"),
                [job],
                |r| Ok((request_row(r)?, r.get(8)?)),
            )
            .optional()
            .map_err(durable)?;
        row.map(|(row, resolved_at)| {
            let r = request_of(row)?;
            Ok(JobResolution {
                job: r.job,
                resolved_by: r.resolved_by,
                declared: r.declared,
                reason: r.reason,
                evidence: r.evidence,
                evidence_digest: r.evidence_digest,
                requested_at: r.requested_at,
                resolved_at,
            })
        })
        .transpose()
    }
}

#[cfg(test)]
mod tests;
