use super::*;
use limeos_domain::{
    ContainerSnapshot, ExecutionState, JobState, Principal, RestartJob, RestartRequest,
};
use limeos_executor_protocol::{Receipt, Request};
use std::time::{Duration, Instant};

/// Least time between two receipt lookups for one held job.
const RECOVERY_PAUSE: Duration = Duration::from_secs(30);

/// Per-job counts of a repeated failure, so a job that fails on every pass
/// is logged at its 1st, 2nd, 4th, 8th... failure, as the supervisor logs a
/// failing task: the first is always logged, and a job stuck for days costs
/// a few lines, not one per pass.
#[derive(Default)]
pub(super) struct Repeats(std::collections::HashMap<String, u64>);
/// Jobs counted at once; past it the counts start again (each job's next
/// failure is then logged as a first).
const REPEATS_TRACKED: usize = 4096;
impl Repeats {
    /// Counts a failure of job `id`; the count, when this one is logged.
    pub(super) fn failed(&mut self, id: &str) -> Option<u64> {
        if self.0.len() >= REPEATS_TRACKED && !self.0.contains_key(id) {
            self.0.clear();
        }
        let count = self.0.entry(id.to_owned()).or_default();
        *count += 1;
        count.is_power_of_two().then_some(*count)
    }
    /// Job `id` went through: a later failure is a first again.
    pub(super) fn passed(&mut self, id: &str) {
        self.0.remove(id);
    }
    /// Keeps only the jobs `listed` (the others settled or went away).
    fn retain(&mut self, listed: impl Fn(&str) -> bool) {
        self.0.retain(|id, _| listed(id));
    }
}

/// Fair recovery polling of held (dispatched, unsettled) jobs. Each pass lists
/// the next page of held jobs after `cursor`, in ID order, and a short page
/// starts over, so every held job is listed within a bounded number of passes
/// however many are held: none can be starved by older ones. `polled` keeps
/// each job to one lookup per `RECOVERY_PAUSE`; an entry lives only that long,
/// so memory is bounded by the jobs polled in one pause.
#[derive(Default)]
pub(super) struct Rotation {
    cursor: String,
    polled: std::collections::HashMap<String, Instant>,
    /// Repeated failures of these jobs, for the log.
    pub(super) failures: Repeats,
}
impl Rotation {
    /// Where the next listing starts.
    pub(super) fn cursor(&self) -> &str {
        &self.cursor
    }
    /// Move past this pass's held jobs (`held`, in listing order), at `now`.
    pub(super) fn advance<'a>(&mut self, held: impl IntoIterator<Item = &'a str>, now: Instant) {
        let held: Vec<&str> = held.into_iter().collect();
        self.cursor = match held.last() {
            Some(last) if held.len() >= limeos_persistence::RECOVERY_BATCH => (*last).to_owned(),
            _ => String::new(),
        };
        self.polled
            .retain(|_, at| now.saturating_duration_since(*at) < RECOVERY_PAUSE);
    }
    /// Whether to look up this held job at `now`; if so, the lookup is
    /// recorded.
    pub(super) fn due(&mut self, id: &str, now: Instant) -> bool {
        if self
            .polled
            .get(id)
            .is_some_and(|at| now.saturating_duration_since(*at) < RECOVERY_PAUSE)
        {
            return false;
        }
        self.polled.insert(id.to_owned(), now);
        true
    }
}

/// A failed call to the container executor, and whether the request may
/// have reached it.
pub(super) struct Failure {
    pub(super) error: Error,
    /// False only when the connection could not be established at all, so
    /// not one byte of the request was written: the executor cannot have
    /// received it. Any failure after the connection exists (writing,
    /// waiting, reading, a refusal, a deadline) leaves it true.
    pub(super) sent: bool,
}

/// Whether an error from an authorization read is a definite refusal of the
/// principal's authority: only then is a queued job rejected. Anything else,
/// above all a store that gave no answer (`Unavailable`: not run,
/// `StateNotDurable`: in doubt, `Overloaded`), leaves the job queued for the
/// next tick.
pub(super) fn refused(error: Error) -> bool {
    matches!(
        error.0,
        ErrorCode::Forbidden
            | ErrorCode::Expired
            | ErrorCode::NotFound
            | ErrorCode::Unauthenticated
    )
}

impl Core {
    /// Runs claim `f` for job `id`. A claim whose outcome is unknown (the
    /// call was in doubt, or the store could not say its commit is durable)
    /// is noted in `claims_in_doubt`; no execute is sent for it now, and the
    /// job's next sighting decides (`claim_in_doubt`). Any other failure
    /// leaves the job as it was.
    pub(super) async fn claim<T: Send + 'static>(
        &self,
        id: &str,
        f: impl FnOnce(&mut limeos_persistence::Store) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let outcome = self.db.try_call(f).await;
        if matches!(
            outcome,
            Err(limeos_persistence::Unanswered::InDoubt)
                | Ok(Err(Error(ErrorCode::StateNotDurable)))
        ) {
            tracing::warn!(
                "a job claim did not answer in time and may still commit; no execute was sent, and the next dispatcher pass sends it if the claim committed"
            );
            self.in_doubt().insert(id.to_owned());
        }
        outcome?
    }
    fn in_doubt(&self) -> std::sync::MutexGuard<'_, std::collections::HashSet<String>> {
        self.claims_in_doubt
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    /// Whether job `id`, now in `state`, holds a claim this process gave up
    /// on that committed after all: it is `running` and no execute was sent
    /// for it, so the dispatcher sends it now (`claim_resumed` first). The
    /// writer runs calls in order, so the listing that showed this state was
    /// answered after the claim had finished. A job still `queued` never got
    /// the claim; its note goes.
    pub(super) fn claim_in_doubt(&self, id: &str, state: JobState) -> bool {
        let mut noted = self.in_doubt();
        if state == JobState::Running {
            return noted.contains(id);
        }
        noted.remove(id);
        false
    }
    /// The execute for a claim found committed is about to be sent: forget
    /// the note first, so it is sent once whatever happens next.
    pub(super) fn claim_resumed(&self, id: &str) {
        self.in_doubt().remove(id);
    }
    pub(super) async fn executor(&self, request: Request, deadline: Duration) -> Result<Receipt> {
        self.exchange(request, deadline).await.map_err(|f| f.error)
    }
    /// As `executor`, telling a request that never left core apart from one
    /// that may have (`Failure::sent`).
    pub(super) async fn exchange(
        &self,
        request: Request,
        deadline: Duration,
    ) -> std::result::Result<Receipt, Failure> {
        // Set once the connection exists, before the first byte is written.
        let mut sent = false;
        let outcome = tokio::time::timeout(deadline, async {
            let mut stream = UnixStream::connect(self.container_socket.as_ref())
                .await
                .map_err(|_| Error(ErrorCode::Unavailable))?;
            sent = true;
            limeos_contracts::write_frame(&mut stream, &request).await?;
            let response: Receipt = limeos_contracts::read_frame(&mut stream).await?;
            if response.version != VERSION {
                return Err(Error(ErrorCode::Unavailable));
            }
            if let Some(error) = response.error {
                return Err(Error(error));
            }
            if !response.ready {
                return Err(Error(ErrorCode::Unavailable));
            }
            // Before any caller acts on a receipt answer: a store epoch other
            // than the one last seen marks every held job, so a closure in
            // this very answer is judged against it (`observe_executor_epoch`).
            if let Some(epoch) = response.epoch.clone() {
                self.db
                    .call(move |s| s.observe_executor_epoch(&epoch, now()))
                    .await?;
            }
            Ok(response)
        })
        .await
        .unwrap_or(Err(Error(ErrorCode::Unavailable)));
        outcome.map_err(|error| Failure { error, sent })
    }
    /// The receipt store's current epoch, recorded (`observe_executor_epoch`)
    /// before it is returned. Asked immediately before a claim, which
    /// dispatches the job under it.
    pub(super) async fn receipt_epoch(&self) -> Result<String> {
        self.executor(
            Request::ReceiptEpoch { version: VERSION },
            limeos_contracts::RPC_DEADLINE,
        )
        .await?
        .epoch
        .ok_or(Error(ErrorCode::Unavailable))
    }
    pub(super) async fn inspect(&self, resource: &str) -> Result<ContainerSnapshot> {
        if !limeos_domain::opaque_id(resource.strip_prefix("container:").unwrap_or_default()) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let value = self
            .executor(
                Request::Inspect {
                    version: VERSION,
                    resource: resource.into(),
                },
                limeos_contracts::RPC_DEADLINE,
            )
            .await?
            .inspection
            .ok_or(Error(ErrorCode::Unavailable))?;
        value.validate()?;
        if value.resource != resource {
            return Err(Error(ErrorCode::Conflict));
        }
        Ok(value)
    }
    async fn logs(
        &self,
        resource: String,
        options: limeos_domain::LogOptions,
    ) -> Result<limeos_domain::ContainerLogs> {
        options.validate()?;
        if !limeos_domain::opaque_id(resource.strip_prefix("container:").unwrap_or_default()) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let value = self
            .executor(
                Request::ContainerLogs {
                    version: VERSION,
                    resource: resource.clone(),
                    options,
                },
                limeos_contracts::RPC_DEADLINE,
            )
            .await?
            .logs
            .ok_or(Error(ErrorCode::Unavailable))?;
        if value.resource != resource || value.text.len() > limeos_domain::CONTAINER_LOG_BYTES {
            return Err(Error(ErrorCode::Unavailable));
        }
        Ok(value)
    }
    pub(super) async fn human_logs(
        &self,
        token: String,
        resource: String,
        options: limeos_domain::LogOptions,
    ) -> Result<limeos_domain::ContainerLogs> {
        let principal = self.session(token.clone(), None).await?.principal;
        self.authorize_resource(principal, resource.clone()).await?;
        let value = self.logs(resource.clone(), options).await?;
        let digest = limeos_identity::digest(&token);
        self.db
            .call(move |s| {
                let current = s.authenticate(&digest, None, now())?;
                limeos_policy::authorize(
                    &current,
                    &s.grants(&current)?,
                    &Scope {
                        operation: limeos_domain::Operation::ContainerManage,
                        resource,
                    },
                )?;
                Ok(value)
            })
            .await
    }
    pub(super) async fn task_logs(
        &self,
        token: String,
        uid: u32,
        task: String,
        resource: String,
        options: limeos_domain::LogOptions,
    ) -> Result<limeos_domain::ContainerLogs> {
        self.task_principal(&token, uid, &task, &resource).await?;
        let value = self.logs(resource.clone(), options).await?;
        let digest = limeos_identity::digest(&token);
        self.db
            .call(move |s| {
                s.check_task(
                    &digest,
                    uid,
                    &task,
                    &Scope {
                        operation: limeos_domain::Operation::ContainerManage,
                        resource,
                    },
                    now(),
                )?;
                Ok(value)
            })
            .await
    }
    pub(super) async fn authorize_resource(
        &self,
        principal: Principal,
        resource: String,
    ) -> Result<()> {
        self.db
            .call(move |s| {
                limeos_policy::authorize(
                    &principal,
                    &s.grants(&principal)?,
                    &Scope {
                        operation: limeos_domain::Operation::ContainerManage,
                        resource,
                    },
                )
            })
            .await
    }
    pub(super) async fn propose(
        &self,
        token: String,
        csrf: String,
        resource: String,
    ) -> Result<limeos_domain::PlannedRestart> {
        self.propose_action(
            token,
            csrf,
            limeos_domain::ContainerAction::Restart,
            resource,
        )
        .await
    }
    pub(super) async fn propose_action(
        &self,
        token: String,
        csrf: String,
        operation: limeos_domain::ContainerAction,
        resource: String,
    ) -> Result<limeos_domain::PlannedContainerAction> {
        let principal = self
            .session(token.clone(), Some(csrf.clone()))
            .await?
            .principal;
        self.authorize_resource(principal.clone(), resource.clone())
            .await?;
        let snapshot = self.inspect(&resource).await?;
        let digest = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, Some(&csrf), now())?;
                s.plan_container(&principal, operation, &snapshot, now())
            })
            .await
    }
    pub(super) async fn submit(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::QueueRestartInput,
    ) -> Result<RestartJob> {
        let principal = self
            .session(token.clone(), Some(csrf.clone()))
            .await?
            .principal;
        // Reject changed/unowned plans before selected inspection.
        let p = principal.clone();
        let proposal = input.proposal.clone();
        let key = input.idempotency_key.clone();
        let approval = input.approval.clone();
        let replay = self
            .db
            .call(move |s| s.replay_container(&p, &key, &proposal, &approval))
            .await?;
        if let Some(job) = replay {
            return Ok(job);
        }
        let snapshot = self.inspect(&input.proposal.plan.expected.resource).await?;
        let digest = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, Some(&csrf), now())?;
                s.queue_container(
                    &principal,
                    &input.idempotency_key,
                    &input.proposal,
                    &input.approval,
                    &snapshot,
                    now(),
                )
            })
            .await
    }
    pub(super) async fn task_principal(
        &self,
        token: &str,
        uid: u32,
        task: &str,
        resource: &str,
    ) -> Result<Principal> {
        if !limeos_domain::opaque_id(token) {
            return Err(Error(ErrorCode::Expired));
        }
        let digest = limeos_identity::digest(token);
        let task = task.to_owned();
        let resource = resource.to_owned();
        self.db
            .call(move |s| {
                s.check_task(
                    &digest,
                    uid,
                    &task,
                    &Scope {
                        operation: limeos_domain::Operation::ContainerManage,
                        resource,
                    },
                    now(),
                )
            })
            .await
    }
    pub(super) async fn propose_task(
        &self,
        token: String,
        uid: u32,
        task: String,
        operation: limeos_domain::ContainerAction,
        resource: String,
    ) -> Result<limeos_domain::PlannedContainerAction> {
        let principal = self.task_principal(&token, uid, &task, &resource).await?;
        let snapshot = self.inspect(&resource).await?;
        let digest = limeos_identity::digest(&token);
        self.db
            .call(move |s| {
                let current = s.check_task(&digest, uid, &task, &snapshot.scope(), now())?;
                if current != principal {
                    return Err(Error(ErrorCode::Expired));
                }
                s.plan_container(&current, operation, &snapshot, now())
            })
            .await
    }
    pub(super) async fn submit_task(
        &self,
        token: String,
        uid: u32,
        task: String,
        input: limeos_contracts::QueueRestartInput,
    ) -> Result<RestartJob> {
        self.task_principal(&token, uid, &task, &input.proposal.plan.expected.resource)
            .await?;
        let digest = limeos_identity::digest(&token);
        let checked_task = task.clone();
        let supplied = input.clone();
        let checked_digest = digest.clone();
        let replay = self
            .db
            .call(move |s| {
                let p = s.check_task(
                    &checked_digest,
                    uid,
                    &checked_task,
                    &supplied.proposal.plan.expected.scope(),
                    now(),
                )?;
                s.replay_container(
                    &p,
                    &supplied.idempotency_key,
                    &supplied.proposal,
                    &supplied.approval,
                )
            })
            .await?;
        if let Some(job) = replay {
            return Ok(job);
        }
        let snapshot = self.inspect(&input.proposal.plan.expected.resource).await?;
        self.db
            .call(move |s| {
                let p = s.check_task(&digest, uid, &task, &snapshot.scope(), now())?;
                s.queue_container(
                    &p,
                    &input.idempotency_key,
                    &input.proposal,
                    &input.approval,
                    &snapshot,
                    now(),
                )
            })
            .await
    }
    async fn mark(&self, job: &RestartJob, state: JobState) -> Result<()> {
        let id = job.id.clone();
        let from = job.state;
        if from == state {
            return Ok(());
        }
        self.db
            .call(move |s| s.transition(&id, from, state, now()))
            .await
    }
    pub(super) async fn dispatch_one(
        &self,
        principal: Principal,
        mut job: RestartJob,
    ) -> Result<()> {
        let digest = limeos_identity::digest(
            &serde_json::to_string(&job.plan).map_err(|_| Error(ErrorCode::InvalidInput))?,
        );
        // A claim that committed after this process gave up on it: no
        // execute was sent, so it is sent now rather than looked up.
        let resumed = self.claim_in_doubt(&job.id, job.state);
        let recovering = job.state != JobState::Queued && !resumed;
        let (mut absence, mut closed) = (None, false);
        let receipt = if !recovering {
            let epoch = if resumed {
                // The store epoch as it is now; an epoch changed since the
                // claim marks the job (`observe_executor_epoch`).
                let epoch = self.receipt_epoch().await?;
                self.claim_resumed(&job.id);
                epoch
            } else {
                // Revocation and expiration are checked immediately before
                // durable dispatch intent. A resource lock conflict, or a store
                // that gives no answer, leaves this job queued.
                let authorized = match self
                    .authorize_resource(principal.clone(), job.plan.expected.resource.clone())
                    .await
                {
                    Ok(()) => true,
                    Err(error) if refused(error) => false,
                    Err(error) => return Err(error),
                };
                if !authorized
                    || job.plan.grant_revision != principal.grant_revision
                    || job.plan.validate(now()).is_err()
                {
                    return self.mark(&job, JobState::PreconditionChanged).await;
                }
                let current = self.inspect(&job.plan.expected.resource).await?;
                if job.plan.check_current(&current, now()).is_err() {
                    return self.mark(&job, JobState::PreconditionChanged).await;
                }
                // The store epoch this job is dispatched under, recorded by the
                // claim before it takes the job (`claim_container`).
                let epoch = self.receipt_epoch().await?;
                let (p, id, under) = (principal, job.id.clone(), epoch.clone());
                job = self
                    .claim(&job.id, move |s| {
                        s.claim_container(&p, &id, &current, &under, now())
                    })
                    .await?;
                epoch
            };
            // Core death from here onward requires receipt reconciliation; it
            // cannot put the job back into the dispatch queue.
            match self
                .exchange(
                    Request::ExecuteContainer {
                        version: VERSION,
                        request: RestartRequest {
                            action: job.id.clone(),
                            plan_digest: digest.clone(),
                            plan: job.plan.clone(),
                        },
                    },
                    Duration::from_secs(38),
                )
                .await
            {
                Ok(value) => {
                    note_epoch(&epoch, value.epoch.as_deref());
                    value.restart
                }
                Err(failure) if !failure.sent || refused_before_any_row(failure.error) => {
                    // Never sent, or refused before the executor recorded
                    // anything: nothing ran or can run for this job.
                    note_execute_failure(failure);
                    let id = job.id.clone();
                    return self
                        .db
                        .call(move |s| s.record_unsent_execute(&id, now()))
                        .await;
                }
                Err(failure) => {
                    note_execute_failure(failure);
                    self.mark(&job, JobState::NeedsIntervention).await?;
                    return Ok(());
                }
            }
        } else {
            // Recovery is a receipt lookup, never an effect request. Core lost
            // the execute answer, so the lookup closes an action no execute
            // recorded: a closure of a never-recorded action comes back as a
            // `precondition_changed` receipt and releases the job below.
            let value = self
                .executor(
                    Request::RestartReceipt {
                        version: VERSION,
                        action: job.id.clone(),
                        digest: digest.clone(),
                        plan: job.plan.clone(),
                        close_if_absent: true,
                    },
                    limeos_contracts::RPC_DEADLINE,
                )
                .await?;
            (absence, closed) = (value.absence, value.closed);
            value.restart
        };
        let Some(mut receipt) = receipt else {
            if recovering {
                // A lookup that succeeded with no receipt to act on. An
                // absence that is not closed proves nothing about an execute
                // still on its way: held, and asked again. Otherwise the
                // executor may have run and pruned the action (`no_longer_held`):
                // held for an operator, and the event records why.
                if absence == Some(limeos_domain::ReceiptAbsence::NeverRecorded) && !closed {
                    return self.mark(&job, JobState::NeedsIntervention).await;
                }
                let id = job.id.clone();
                return self
                    .db
                    .call(move |s| s.record_receipt_missing(&id, now()))
                    .await;
            }
            self.mark(&job, JobState::NeedsIntervention).await?;
            return Ok(());
        };
        // Reject mismatched IPC evidence before it can unlock a core resource.
        if receipt.action != job.id
            || receipt.plan_digest != digest
            || receipt.before != job.plan.expected
            || receipt.operation != job.plan.operation
        {
            self.mark(&job, JobState::NeedsIntervention).await?;
            return Err(Error(ErrorCode::Conflict));
        }
        if closed {
            // A closure of an action the executor never recorded: released
            // like a refusal, unless it contradicts a result core holds or a
            // dispatch under an earlier store epoch; then the job is held
            // (`record_container_closure`).
            return self
                .db
                .call(move |s| s.record_container_closure(&job, &receipt, now()))
                .await
                .map(|_| ());
        }
        let mut after = None;
        if matches!(
            receipt.state,
            ExecutionState::EffectAccepted | ExecutionState::Verified
        ) {
            if matches!(job.state, JobState::Running | JobState::OutcomeUnknown) {
                self.mark(&job, JobState::Verifying).await?;
                job.state = JobState::Verifying;
            }
            if let Ok(current) = self.inspect(&job.plan.expected.resource).await
                && limeos_domain::verify_container(receipt.operation, &receipt.before, &current)
                    .is_ok()
            {
                if receipt.state == ExecutionState::EffectAccepted
                    && let Ok(value) = self
                        .executor(
                            Request::VerifyRestart {
                                version: VERSION,
                                action: job.id.clone(),
                                digest,
                                after: current.clone(),
                            },
                            limeos_contracts::RPC_DEADLINE,
                        )
                        .await
                    && let Some(verified) = value.restart
                {
                    receipt = verified;
                }
                after = Some(current);
            }
        }
        self.db
            .call(move |s| s.record_container_result(&job, &receipt, after.as_ref(), now()))
            .await?;
        Ok(())
    }
    pub(super) async fn dispatcher(&self) {
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut state = Dispatcher::default();
        loop {
            // The first tick is immediate, which completes startup leftovers.
            tick.tick().await;
            self.dispatcher_pass(&mut state).await;
        }
    }
    /// One tick of the dispatcher: reports, resolution retries, stack jobs,
    /// storage target and mount jobs, then container jobs.
    pub(super) async fn dispatcher_pass(&self, state: &mut Dispatcher) {
        let Dispatcher {
            recovery,
            target_recovery,
            mount_recovery,
            stack_recovery,
            resolution_retry,
            audit,
            retention_failures,
            store_stalled,
            target_failures,
            mount_failures,
        } = state;
        // A stuck writer makes this loop's own store calls fail at their
        // deadline, so this runs at least every `CALL_DEADLINE` or so.
        *store_stalled = report_store(*store_stalled, self.db.writer_activity());
        *audit = report_audit(*audit, self.db.audit_usage());
        *retention_failures = report_retention(*retention_failures, self.db.retention_failure());
        self.complete_pending_resolutions(resolution_retry, 5).await;
        self.dispatch_stack_candidates(stack_recovery).await;
        if let Ok(jobs) = self.db.call(|s| s.storage_target_candidates()).await {
            target_recovery.retain(|id, _| jobs.iter().any(|(_, job)| &job.id == id));
            target_failures.retain(|id| jobs.iter().any(|(_, job)| job.id == id));
            for (principal, job) in jobs {
                if job.state != JobState::Queued {
                    if target_recovery.get(&job.id).is_some_and(|at| {
                        monotonic().saturating_duration_since(*at) < Duration::from_secs(30)
                    }) {
                        continue;
                    }
                    target_recovery.insert(job.id.clone(), monotonic());
                }
                let id = job.id.clone();
                match self.dispatch_target(principal, job).await {
                    Ok(()) => target_failures.passed(&id),
                    Err(e) => {
                        if let Some(failures) = target_failures.failed(&id) {
                            tracing::warn!(code=?e.0, failures, "storage target job remains pending or requires reconciliation");
                        }
                    }
                }
            }
        }

        if let Ok(jobs) = self.db.call(|store| store.storage_mount_candidates()).await {
            mount_recovery.retain(|id, _| jobs.iter().any(|(_, job)| &job.id == id));
            mount_failures.retain(|id| jobs.iter().any(|(_, job)| job.id == id));
            for (principal, job) in jobs {
                if job.state != JobState::Queued {
                    if mount_recovery.get(&job.id).is_some_and(|at| {
                        monotonic().saturating_duration_since(*at) < Duration::from_secs(30)
                    }) {
                        continue;
                    }
                    mount_recovery.insert(job.id.clone(), monotonic());
                }
                let id = job.id.clone();
                match self.dispatch_mount(principal, job).await {
                    Ok(()) => mount_failures.passed(&id),
                    Err(error) => {
                        if let Some(failures) = mount_failures.failed(&id) {
                            tracing::warn!(code=?error.0, failures, "storage mount job remains pending or requires reconciliation");
                        }
                    }
                }
            }
        }
        let cursor = recovery.cursor().to_owned();
        let Ok(jobs) = self
            .db
            .call(move |s| s.container_candidates_after(&cursor))
            .await
        else {
            return;
        };
        recovery.advance(
            jobs.iter()
                .filter(|(_, job)| job.state != JobState::Queued)
                .map(|(_, job)| job.id.as_str()),
            monotonic(),
        );
        for (principal, job) in jobs {
            if job.state != JobState::Queued && !recovery.due(&job.id, monotonic()) {
                continue;
            }
            let id = job.id.clone();
            match self.dispatch_one(principal, job).await {
                Ok(()) => recovery.failures.passed(&id),
                Err(e) => {
                    if let Some(failures) = recovery.failures.failed(&id) {
                        tracing::warn!(code=?e.0, failures, "container job remains pending or requires reconciliation");
                    }
                }
            }
        }
    }
}

/// What the dispatcher keeps between ticks. Bounds recovery traffic and
/// memory: uncertain receipts are never retried as mutations, even after
/// their original plan has expired.
pub(super) struct Dispatcher {
    recovery: Rotation,
    target_recovery: std::collections::HashMap<String, Instant>,
    mount_recovery: std::collections::HashMap<String, Instant>,
    stack_recovery: Rotation,
    resolution_retry: Rotation,
    audit: limeos_persistence::AuditLevel,
    retention_failures: u64,
    store_stalled: bool,
    target_failures: Repeats,
    mount_failures: Repeats,
}
impl Default for Dispatcher {
    fn default() -> Self {
        Self {
            recovery: Rotation::default(),
            target_recovery: Default::default(),
            mount_recovery: Default::default(),
            stack_recovery: Rotation::default(),
            resolution_retry: Rotation::default(),
            audit: limeos_persistence::AuditLevel::Normal,
            retention_failures: 0,
            store_stalled: false,
            target_failures: Repeats::default(),
            mount_failures: Repeats::default(),
        }
    }
}

/// The execute answer came from a receipt store other than the one the job
/// was dispatched under (the executor reopened its store in between). Its
/// receipt still stands; recording the new epoch has marked the held job, so
/// a later closure for it is not accepted.
pub(super) fn note_epoch(dispatched: &str, answered: Option<&str>) {
    if answered.is_some_and(|answered| answered != dispatched) {
        tracing::warn!(
            "the executor's receipt store changed between dispatch and the execute answer"
        );
    }
}

/// Whether an execute refusal proves the executor recorded nothing for the
/// action: `BeyondReceiptHistory` is returned by its clock-free barrier only
/// when the store holds no row for the action, before it reads a clock,
/// contacts the Engine or writes anything (`ReceiptStore::admit`). Core sends
/// one execute per job, so with that refusal nothing ran or can run.
pub(super) fn refused_before_any_row(error: Error) -> bool {
    error.0 == ErrorCode::BeyondReceiptHistory
}

/// Logs a failed execute call. One that was never sent, or refused before
/// any row (`refused_before_any_row`), releases the job as refused; any
/// other leaves it held for reconciliation. The clock-free barrier gets its
/// own message, since after a backward clock step every new action is
/// refused the same way.
pub(super) fn note_execute_failure(failure: Failure) {
    if !failure.sent {
        tracing::warn!(
            "the container executor could not be reached; the execute request was never sent, and the job is released without effect"
        );
    } else if refused_before_any_row(failure.error) {
        tracing::warn!(
            "executor refused the action: its plan expires within pruned receipt history, so the executor cannot tell whether an action with it already ran; check the system clock (nothing was recorded or run, and the job is settled as refused)"
        );
    } else {
        tracing::warn!(code=?failure.error.0, "execute call failed; the job is held for reconciliation");
    }
}

/// Logs when the store's writer has been on one item past `STORE_STALL`, and
/// when it moves on again.
fn report_store(stalled: bool, activity: limeos_persistence::WriterActivity) -> bool {
    let now_stalled = activity
        .busy_for
        .is_some_and(|busy| busy >= super::STORE_STALL);
    if now_stalled && !stalled {
        tracing::error!(
            busy_ms = activity
                .busy_for
                .map_or(0, |busy| u64::try_from(busy.as_millis())
                    .unwrap_or(u64::MAX)),
            queued = activity.queued,
            "the store's writer is stuck on one item (a hung disk write or a lock held from outside); store calls fail at their deadline and health reports the store degraded"
        );
    } else if stalled && !now_stalled {
        tracing::info!("the store's writer is moving again");
    }
    now_stalled
}

/// Logs each new failed retention batch (the latest, with the running
/// count). A failed batch is retried on the next pass, once a minute.
fn report_retention(seen: u64, failure: Option<limeos_persistence::RetentionFailure>) -> u64 {
    match failure {
        Some(failure) if failure.count > seen => {
            tracing::error!(
                rule = failure.rule,
                code = ?failure.code,
                failures = failure.count,
                "audit retention batch failed; it is retried on the next pass"
            );
            failure.count
        }
        _ => seen,
    }
}

/// Logs when the audit table crosses the warning (75% of the cap) or the cap
/// itself, and when it recovers. Retention keeps settled history at half the
/// cap, so a warning means pinned history is large or writes outpace pruning.
fn report_audit(
    previous: limeos_persistence::AuditLevel,
    usage: limeos_persistence::AuditUsage,
) -> limeos_persistence::AuditLevel {
    use limeos_persistence::AuditLevel;
    let level = usage.level();
    if level != previous {
        let (bytes, limit) = (usage.bytes, limeos_persistence::AuditUsage::LIMIT);
        match level {
            AuditLevel::Normal => tracing::info!(bytes, limit, "audit history back within budget"),
            AuditLevel::Warning => {
                tracing::warn!(bytes, limit, "audit history above 75% of its cap")
            }
            AuditLevel::Full => tracing::error!(
                bytes,
                limit,
                "audit history at its cap; writes other than revocation are refused"
            ),
        }
    }
    level
}
