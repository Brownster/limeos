use limeos_api::{Backend, IssuedSession, PasswordAttempt, PasswordCheck, PasswordChecked};
use limeos_contracts::{
    CoreRequest, CoreResponse, Enrollment, ErrorEnvelope, Health, Login, SessionView, VERSION,
};
use limeos_domain::{Error, ErrorCode, Result, Scope};
use limeos_persistence::{
    Database,
    config::{self, ConfigRead},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    net::{TcpListener, UnixListener, UnixStream},
    sync::Semaphore,
};
// Proposed observation runner that nothing calls yet, hence unused outside its
// tests; see docs/deferred-code.md.
#[cfg(feature = "deferred")]
#[allow(dead_code)]
mod automation;
mod automation_history;
mod automation_runtime;
mod automation_schedules;
pub mod automation_store;
mod compose;
pub mod health_collection;
mod identity_enablement;
mod job_resolution;
mod operations;
mod password_admission;
mod reader_health;
use password_admission::PasswordWorkers;
mod stack_operations;
mod stacks;
mod storage;
mod storage_mounts;
mod storage_targets;
mod supervision;

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}
/// Work for a password worker. A verification carries the request's attempt
/// handle, so it cannot be issued without marking when its comparison starts.
enum PasswordJob<'a> {
    Hash(String),
    Verify {
        attempt: &'a PasswordAttempt,
        password: String,
        encoded: String,
    },
}
#[derive(Clone)]
struct Core {
    automation_status: automation_runtime::Status,
    reader_health: reader_health::Status,
    db: Database,
    password_workers: Arc<PasswordWorkers>,
    dummy_hash: Arc<str>,
    observations: limeos_observations::Cache,
    telemetry: Option<limeos_observations::telemetry::Telemetry>,
    container_socket: Arc<PathBuf>,
    compose_catalog: Option<Arc<limeos_domain::ComposeCatalog>>,
    storage_socket: Arc<PathBuf>,
    storage_target_socket: Arc<PathBuf>,
    storage_mount_socket: Arc<PathBuf>,
    storage_readers: Arc<Semaphore>,
    /// Jobs whose claim this process gave up on with its outcome unknown
    /// (the store call was in doubt), so no execute request was sent for
    /// them. The dispatcher's next sighting decides: still queued, the claim
    /// never committed; running, it did, and the execute is sent then.
    claims_in_doubt: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    stacks: limeos_observations::stacks::Cache,
}
impl Core {
    /// Login with how far the password check got recorded in `check`.
    async fn login_checked(
        &self,
        input: Login,
        attempt: &PasswordAttempt,
        check: &mut PasswordCheck,
    ) -> Result<IssuedSession> {
        if !input.well_formed() {
            return Err(Error(ErrorCode::Unauthenticated));
        }
        let record = self
            .db
            .call(move |s| s.login_record(&input.username))
            .await?;
        // Names with no account share one comparison budget, so a flood of
        // made-up names cannot hold the workers; real accounts never use it.
        if record.is_none()
            && !self
                .password_workers
                .take_unknown_name(std::time::Instant::now())
        {
            return Err(Error(ErrorCode::Overloaded));
        }
        let encoded = record
            .as_ref()
            .map(|r| r.password_hash.clone())
            .unwrap_or_else(|| self.dummy_hash.to_string());
        let response = self
            .password(PasswordJob::Verify {
                attempt,
                password: input.password,
                encoded,
            })
            .await?;
        let (valid, upgraded) = (response.valid, response.upgraded);
        // A match against the dummy hash of a missing account is no match.
        *check = if valid && record.is_some() {
            PasswordCheck::Accepted
        } else {
            PasswordCheck::Rejected
        };
        if !valid {
            return Err(Error(ErrorCode::Unauthenticated));
        }
        let record = record.ok_or(Error(ErrorCode::Unauthenticated))?;
        let principal = record.principal.clone();
        let token = limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?;
        // CSRF token is deterministically derived from the session secret, and only
        // its digest is persisted. It can be recovered after a browser reconnect.
        let csrf = limeos_identity::digest(&format!("csrf:{token}"));
        let digest = limeos_identity::digest(&token);
        let csrf_digest = limeos_identity::digest(&csrf);
        let expires_at = self
            .db
            .call(move |s| {
                s.create_session(&record, upgraded.as_deref(), &digest, &csrf_digest, now())
            })
            .await?;
        Ok(IssuedSession {
            token,
            view: SessionView {
                principal,
                csrf_token: csrf,
                expires_at,
            },
        })
    }
    async fn password(&self, job: PasswordJob<'_>) -> Result<limeos_contracts::PasswordResponse> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let _admission = self.password_workers.admit().await?;
        // Admitted: from here a cancelled request has spent a comparison.
        let request = match job {
            PasswordJob::Hash(password) => limeos_contracts::PasswordRequest::Hash { password },
            PasswordJob::Verify {
                attempt,
                password,
                encoded,
            } => {
                attempt.start_comparison();
                limeos_contracts::PasswordRequest::Verify { password, encoded }
            }
        };
        let executable = std::env::current_exe().map_err(|_| Error(ErrorCode::Unavailable))?;
        let directory = executable.parent().ok_or(Error(ErrorCode::Unavailable))?;
        #[cfg(test)]
        let directory = if directory.ends_with("deps") {
            directory.parent().ok_or(Error(ErrorCode::Unavailable))?
        } else {
            directory
        };
        let mut child = tokio::process::Command::new(directory.join("limeos-password-worker"))
            .env_clear()
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        let bytes = serde_json::to_vec(&request).map_err(|_| Error(ErrorCode::InvalidInput))?;
        tokio::time::timeout(std::time::Duration::from_secs(8), async {
            let mut input = child.stdin.take().ok_or(Error(ErrorCode::Unavailable))?;
            input
                .write_all(&bytes)
                .await
                .map_err(|_| Error(ErrorCode::Unavailable))?;
            drop(input);
            let mut output = Vec::new();
            child
                .stdout
                .take()
                .ok_or(Error(ErrorCode::Unavailable))?
                .take(2049)
                .read_to_end(&mut output)
                .await
                .map_err(|_| Error(ErrorCode::Unavailable))?;
            if output.len() > 2048
                || !child
                    .wait()
                    .await
                    .map_err(|_| Error(ErrorCode::Unavailable))?
                    .success()
            {
                return Err(Error(ErrorCode::Unavailable));
            }
            serde_json::from_slice(&output).map_err(|_| Error(ErrorCode::Unavailable))
        })
        .await
        .map_err(|_| Error(ErrorCode::Unavailable))?
    }
    async fn hash(&self, password: String) -> Result<String> {
        if !(12..=1024).contains(&password.len()) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        self.password(PasswordJob::Hash(password))
            .await?
            .upgraded
            .ok_or(Error(ErrorCode::Unavailable))
    }
    async fn enroll(&self, input: Enrollment) -> Result<()> {
        if !limeos_domain::identifier(&input.username) || input.token.len() != 64 {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let hash = self.hash(input.password).await?;
        let digest = limeos_identity::digest(&input.token);
        self.db
            .call(move |s| s.enroll(&digest, &input.username, &hash, now()))
            .await
    }
    /// Hashes are validated and stored as-is; nothing is rehashed until the user
    /// next logs in.
    async fn import_legacy_users(
        &self,
        users: Vec<limeos_contracts::LegacyUser>,
    ) -> Result<(u32, Vec<String>)> {
        let users: Vec<_> = users
            .into_iter()
            .map(|u| (u.username, u.password_hash))
            .collect();
        self.db
            .call(move |s| s.import_legacy_users(&users, now()))
            .await
    }
}
impl Backend for Core {
    async fn observe_health_policy(
        &self,
        token: String,
        deadline: std::time::Instant,
    ) -> Result<limeos_domain::automation::HealthPolicyView> {
        self.human_health_policy(token, deadline).await
    }
    async fn replace_observe_health_policy(
        &self,
        token: String,
        csrf: String,
        input: limeos_domain::automation::ReplaceHealthPolicy,
        deadline: std::time::Instant,
    ) -> Result<limeos_domain::automation::HealthPolicyView> {
        self.human_health_policy_replace(token, csrf, input, deadline)
            .await
    }
    async fn observe_health_alerts(
        &self,
        token: String,
        after: Option<String>,
        deadline: std::time::Instant,
    ) -> Result<limeos_domain::automation::AutomationPage<limeos_domain::automation::AlertState>>
    {
        self.human_alert_page(token, after, deadline).await
    }
    async fn observe_health_notifications(
        &self,
        token: String,
        after: Option<String>,
        deadline: std::time::Instant,
    ) -> Result<limeos_domain::automation::AutomationPage<limeos_domain::automation::Notification>>
    {
        self.human_notification_page(token, after, deadline).await
    }
    async fn acknowledge_observe_health_notification(
        &self,
        token: String,
        csrf: String,
        id: String,
        input: limeos_domain::automation::AcknowledgeNotification,
        deadline: std::time::Instant,
    ) -> Result<limeos_domain::automation::Notification> {
        self.human_notification_acknowledge(token, csrf, id, input, deadline)
            .await
    }
    async fn stack_inventory(
        &self,
        token: String,
    ) -> Result<limeos_domain::deployment::discovery::StackInventory> {
        self.human_stack_inventory(token).await
    }
    async fn observe_health_schedules(
        &self,
        token: String,
        after: Option<String>,
        deadline: std::time::Instant,
    ) -> Result<limeos_domain::automation::AutomationPage<limeos_domain::automation::ScheduleView>>
    {
        self.human_schedule_page(token, after, deadline).await
    }
    async fn create_observe_health_schedule(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::ObserveHealthScheduleInput,
        deadline: std::time::Instant,
    ) -> Result<limeos_domain::automation::ScheduleView> {
        self.human_schedule_create(token, csrf, input, deadline)
            .await
    }
    async fn pause_observe_health_schedule(
        &self,
        token: String,
        csrf: String,
        id: String,
        input: limeos_domain::automation::PauseSchedule,
        deadline: std::time::Instant,
    ) -> Result<limeos_domain::automation::ScheduleView> {
        self.human_schedule_pause(token, csrf, id, input, deadline)
            .await
    }
    async fn enable_observe_health(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::ObserveHealthEnable,
        deadline: std::time::Instant,
        attempt: PasswordAttempt,
    ) -> PasswordChecked<limeos_contracts::ObserveHealthEnabled> {
        let mut check = PasswordCheck::NotReached;
        let result = self
            .human_enable_observe_health_checked(token, csrf, input, deadline, &attempt, &mut check)
            .await;
        PasswordChecked::new(check, result)
    }
    async fn storage_mount_catalog(
        &self,
        token: String,
    ) -> Result<limeos_domain::StorageMountCatalog> {
        self.human_mount_catalog(token).await
    }
    async fn plan_storage_mount(
        &self,
        token: String,
        csrf: String,
        selection: limeos_domain::StorageMountSelection,
    ) -> Result<limeos_domain::PlannedStorageMount> {
        self.human_plan_mount(token, csrf, selection).await
    }
    async fn storage_mount_plan(
        &self,
        token: String,
        id: String,
    ) -> Result<limeos_domain::PlannedStorageMount> {
        let principal = self.storage_principal(token, None).await?;
        self.db
            .call(move |store| store.storage_mount_plan(&principal, &id))
            .await
    }
    async fn approve_storage_mount(
        &self,
        token: String,
        csrf: String,
        id: String,
        digest: String,
    ) -> Result<limeos_domain::PlanApproval> {
        self.human_approve_mount(token, csrf, id, digest).await
    }
    async fn queue_storage_mount(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::QueueStorageMountInput,
    ) -> Result<limeos_domain::StorageMountJob> {
        self.human_queue_mount(token, csrf, input).await
    }
    async fn storage_mount_job(
        &self,
        token: String,
        id: String,
    ) -> Result<limeos_domain::StorageMountJob> {
        let principal = self.storage_principal(token, None).await?;
        self.db
            .call(move |store| store.storage_mount_job(&principal, &id))
            .await
    }
    async fn storage_mount_jobs(
        &self,
        token: String,
    ) -> Result<Vec<limeos_domain::StorageMountJob>> {
        let principal = self.storage_principal(token, None).await?;
        self.db
            .call(move |store| store.storage_mount_jobs(&principal))
            .await
    }
    async fn cancel_storage_mount(&self, token: String, csrf: String, id: String) -> Result<()> {
        let principal = self.storage_principal(token, Some(csrf)).await?;
        self.db
            .call(move |store| store.cancel_storage_mount(&principal, &id, now()))
            .await
    }
    async fn cancel_storage_mount_job(
        &self,
        token: String,
        csrf: String,
        id: String,
    ) -> Result<()> {
        let principal = self.storage_principal(token, Some(csrf)).await?;
        self.db
            .call(move |store| store.cancel_storage_mount_job(&principal, &id, now()))
            .await
    }

    async fn container_storage_inventory(
        &self,
        token: String,
    ) -> Result<limeos_domain::ContainerStorageInventoryView> {
        self.human_container_storage(token).await
    }
    async fn plan_storage_targets(
        &self,
        token: String,
        csrf: String,
        contract: limeos_domain::StorageContract,
    ) -> Result<limeos_domain::PlannedStorageTargets> {
        self.human_plan_targets(token, csrf, contract).await
    }
    async fn storage_target_plan(
        &self,
        token: String,
        id: String,
    ) -> Result<limeos_domain::PlannedStorageTargets> {
        let p = self.storage_principal(token, None).await?;
        self.db.call(move |s| s.storage_target_plan(&p, &id)).await
    }
    async fn approve_storage_targets(
        &self,
        token: String,
        csrf: String,
        id: String,
        digest: String,
    ) -> Result<limeos_domain::PlanApproval> {
        self.human_approve_targets(token, csrf, id, digest).await
    }
    async fn queue_storage_targets(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::QueueStorageTargetsInput,
    ) -> Result<limeos_domain::StorageTargetJob> {
        self.human_queue_targets(token, csrf, input).await
    }
    async fn storage_target_job(
        &self,
        token: String,
        id: String,
    ) -> Result<limeos_domain::StorageTargetJob> {
        let p = self.storage_principal(token, None).await?;
        self.db.call(move |s| s.storage_target_job(&p, &id)).await
    }
    async fn cancel_storage_targets(&self, token: String, csrf: String, id: String) -> Result<()> {
        let p = self.storage_principal(token, Some(csrf)).await?;
        self.db
            .call(move |s| s.cancel_storage_targets(&p, &id, now()))
            .await
    }
    async fn cancel_storage_target_job(
        &self,
        token: String,
        csrf: String,
        id: String,
    ) -> Result<()> {
        let p = self.storage_principal(token, Some(csrf)).await?;
        self.db
            .call(move |s| s.cancel_storage_target_job(&p, &id, now()))
            .await
    }
    async fn storage_inventory(
        &self,
        token: String,
    ) -> Result<limeos_domain::StorageInventoryView> {
        self.human_storage_inventory(token).await
    }
    async fn plan_storage(
        &self,
        token: String,
        csrf: String,
        input: limeos_domain::StorageSetupInput,
    ) -> Result<limeos_domain::PlannedStorageSetup> {
        self.human_plan_storage(token, csrf, input).await
    }
    async fn storage_plan(
        &self,
        token: String,
        id: String,
    ) -> Result<limeos_domain::PlannedStorageSetup> {
        let principal = self.storage_principal(token, None).await?;
        self.db
            .call(move |s| s.storage_plan(&principal, &id, now()))
            .await
    }
    async fn approve_storage(
        &self,
        token: String,
        csrf: String,
        id: String,
        digest: String,
    ) -> Result<limeos_domain::PlanApproval> {
        self.human_approve_storage(token, csrf, id, digest).await
    }
    async fn cancel_storage(&self, token: String, csrf: String, id: String) -> Result<()> {
        let principal = self.storage_principal(token, Some(csrf)).await?;
        self.db
            .call(move |s| s.cancel_storage(&principal, &id, now()))
            .await
    }
    async fn login(
        &self,
        input: Login,
        attempt: PasswordAttempt,
    ) -> PasswordChecked<IssuedSession> {
        let mut check = PasswordCheck::NotReached;
        let result = self.login_checked(input, &attempt, &mut check).await;
        PasswordChecked::new(check, result)
    }
    async fn session(&self, token: String, csrf: Option<String>) -> Result<SessionView> {
        let csrf_token = limeos_identity::digest(&format!("csrf:{token}"));
        let digest = limeos_identity::digest(&token);
        let csrf = csrf.map(|s| limeos_identity::digest(&s));
        let principal = self
            .db
            .call(move |s| s.authenticate(&digest, csrf.as_deref(), now()))
            .await?;
        // Return the durable expiry, never extend sessions on a status read.
        let digest = limeos_identity::digest(&token);
        let expires_at = self.db.call(move |s| s.session_expiry(&digest)).await?;
        Ok(SessionView {
            principal,
            csrf_token,
            expires_at,
        })
    }
    /// On the store's query-only connection, never the writer's queue, so
    /// an anonymous caller with a made-up cookie adds no store work and a
    /// stuck writer or a full queue cannot hold health or the gate. The same
    /// checks as `session` (`authenticate` with the CSRF digest), without
    /// the expiry read; `Unavailable` when the reader cannot answer within
    /// `SIGNED_IN_BOUND`.
    async fn signed_in(&self, token: String, csrf: Option<String>) -> Result<()> {
        let csrf = csrf.map(|s| limeos_identity::digest(&s));
        self.db
            .session_confirm(
                limeos_identity::digest(&token),
                csrf,
                now(),
                SIGNED_IN_BOUND,
            )
            .await
    }
    async fn logout(&self, token: String, csrf: String) -> Result<()> {
        // One call on the revocation lane checks the session and its CSRF and
        // removes it, so logout works with the queue, disk or audit table full.
        let revocation = limeos_persistence::Revocation::Session {
            digest: limeos_identity::digest(&token),
            csrf: limeos_identity::digest(&csrf),
        };
        self.db.revoke(revocation, now()).await
    }
    async fn health(&self) -> Result<Health> {
        let store = self.store_health().await?;
        Ok(Health {
            version: VERSION,
            generation: self.db.generation,
            schema_version: limeos_persistence::SCHEMA_VERSION,
            ready: store.is_none(),
            degraded: self
                .automation_status
                .health()
                .degraded()
                .into_iter()
                .chain(self.reader_health.degraded())
                .chain(store)
                .collect(),
        })
    }
    async fn observations(&self, token: String) -> Result<limeos_contracts::Overview> {
        let digest = limeos_identity::digest(&token);
        let (principal, grants) = self
            .db
            .call(move |s| {
                let p = s.authenticate(&digest, None, now())?;
                let g = s.grants(&p)?;
                Ok((p, g))
            })
            .await?;
        let allowed = |resource: &str| {
            limeos_policy::authorize(
                &principal,
                &grants,
                &Scope {
                    operation: limeos_domain::Operation::HealthRead,
                    resource: resource.into(),
                },
            )
            .is_ok()
        };
        let mut value = self.observations.snapshot();
        let host = allowed("host:system");
        if !host {
            value.host = None;
        }
        value.resources.retain(|r| allowed(&r.id));
        value.sources.retain(|s| {
            (matches!(
                s.source,
                limeos_contracts::Source::Host | limeos_contracts::Source::History
            ) && host)
                || value.resources.iter().any(|r| r.source == s.source)
                || grants.iter().any(|g| {
                    g.operation == limeos_domain::Operation::HealthRead && g.resource == "*"
                })
        });
        if !host && value.resources.is_empty() && value.sources.is_empty() {
            return Err(Error(ErrorCode::Forbidden));
        }
        self.observations.demand();
        Ok(value)
    }
    async fn history(
        &self,
        token: String,
        range: limeos_contracts::HistoryRange,
    ) -> Result<limeos_contracts::MetricHistory> {
        let digest = limeos_identity::digest(&token);
        self.db
            .call(move |s| {
                let p = s.authenticate(&digest, None, now())?;
                limeos_policy::authorize(
                    &p,
                    &s.grants(&p)?,
                    &Scope {
                        operation: limeos_domain::Operation::HealthRead,
                        resource: "host:system".into(),
                    },
                )
            })
            .await?;
        self.telemetry
            .as_ref()
            .ok_or(Error(ErrorCode::Unavailable))?
            .query(range, now())
            .await
    }
    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.observations.subscribe()
    }
    async fn plan_compose(
        &self,
        token: String,
        csrf: String,
        selection: limeos_domain::ComposeSelection,
    ) -> Result<limeos_domain::PlannedCompose> {
        // Authenticated before the catalog is consulted, so an anonymous
        // caller cannot tell whether one is installed.
        let catalog = self.catalog();
        let digest = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, Some(&csrf), now())?;
                let catalog = catalog?;
                s.plan_compose(&principal, &catalog, &selection, now())
            })
            .await
    }
    async fn compose_plan(
        &self,
        token: String,
        id: String,
    ) -> Result<limeos_domain::PlannedCompose> {
        let catalog = self.catalog();
        let digest = limeos_identity::digest(&token);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, None, now())?;
                let catalog = catalog?;
                let preview = s.compose_plan(&principal, &catalog, &id, now())?;
                let scopes = limeos_persistence::compose_scopes(&catalog, &preview.plan.selection)?;
                Ok(
                    s.resolve_compose_session(&digest, &catalog, &preview, &scopes)?
                        .into_preview(),
                )
            })
            .await
    }
    async fn approve_compose(
        &self,
        token: String,
        csrf: String,
        id: String,
        plan_digest: String,
    ) -> Result<limeos_domain::PlanApproval> {
        let catalog = self.catalog();
        let digest = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, Some(&csrf), now())?;
                let catalog = catalog?;
                s.approve_compose(&principal, &catalog, &id, &plan_digest, now())
            })
            .await
    }
    async fn cancel_compose(&self, token: String, csrf: String, id: String) -> Result<()> {
        let catalog = self.catalog();
        let digest = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, Some(&csrf), now())?;
                let catalog = catalog?;
                s.cancel_compose(&principal, &catalog, &id, now())
            })
            .await
    }
    async fn plan_restart(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::RestartInput,
    ) -> Result<limeos_domain::PlannedRestart> {
        self.propose(token, csrf, input.resource).await
    }
    async fn plan_container(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::ContainerInput,
    ) -> Result<limeos_domain::PlannedContainerAction> {
        self.propose_action(token, csrf, input.operation, input.resource)
            .await
    }
    async fn container_logs(
        &self,
        token: String,
        resource: String,
        options: limeos_domain::LogOptions,
    ) -> Result<limeos_domain::ContainerLogs> {
        self.human_logs(token, resource, options).await
    }
    async fn approve_container(
        &self,
        token: String,
        csrf: String,
        id: String,
        plan_digest: String,
    ) -> Result<limeos_domain::PlanApproval> {
        let digest = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, Some(&csrf), now())?;
                s.approve_container(&principal, &id, &plan_digest, now())
            })
            .await
    }
    async fn queue_container(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::QueueRestartInput,
    ) -> Result<limeos_domain::RestartJob> {
        self.submit(token, csrf, input).await
    }
    async fn restart_progress(
        &self,
        token: String,
        id: String,
        after: i64,
    ) -> Result<limeos_contracts::JobProgress> {
        let principal = self.session(token, None).await?.principal;
        self.db
            .call(move |s| {
                Ok(limeos_contracts::JobProgress {
                    job: s.container_job(&principal, &id)?,
                    events: s.container_events(&principal, &id, after)?,
                })
            })
            .await
    }
    async fn container_jobs(&self, token: String) -> Result<Vec<limeos_domain::RestartJob>> {
        let principal = self.session(token, None).await?.principal;
        self.db.call(move |s| s.container_jobs(&principal)).await
    }
    async fn cancel_restart(
        &self,
        token: String,
        csrf: String,
        id: String,
        plan: bool,
    ) -> Result<()> {
        let digest = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, Some(&csrf), now())?;
                if plan {
                    s.cancel_container_plan(&principal, &id, now())
                } else {
                    s.cancel_container_job(&principal, &id, now())
                }
            })
            .await
    }
    async fn plan_stack(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::StackInput,
    ) -> Result<limeos_domain::PlannedStackAction> {
        self.propose_stack(token, csrf, input.project, input.operation)
            .await
    }
    async fn approve_stack(
        &self,
        token: String,
        csrf: String,
        id: String,
        plan_digest: String,
    ) -> Result<limeos_domain::PlanApproval> {
        self.approve_stack_plan(token, csrf, id, plan_digest).await
    }
    async fn queue_stack(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::QueueStackInput,
    ) -> Result<limeos_domain::StackJob> {
        self.submit_stack(token, csrf, input).await
    }
    async fn stack_job_progress(
        &self,
        token: String,
        id: String,
        after: i64,
    ) -> Result<limeos_contracts::StackJobProgress> {
        self.stack_progress(token, id, after).await
    }
    async fn stack_jobs(&self, token: String) -> Result<Vec<limeos_domain::StackJob>> {
        self.stack_job_list(token).await
    }
    async fn cancel_stack_action(
        &self,
        token: String,
        csrf: String,
        id: String,
        plan: bool,
    ) -> Result<()> {
        self.cancel_stack(token, csrf, id, plan).await
    }
    async fn job_resolution(
        &self,
        token: String,
        id: String,
    ) -> Result<limeos_contracts::ResolutionView> {
        self.human_resolution_view(token, id).await
    }
    async fn needs_attention_jobs(
        &self,
        token: String,
    ) -> Result<Vec<limeos_domain::NeedsAttentionJob>> {
        self.human_needs_attention(token).await
    }
    async fn resolve_job(
        &self,
        token: String,
        csrf: String,
        id: String,
        input: limeos_contracts::ResolveJobInput,
        attempt: PasswordAttempt,
    ) -> PasswordChecked<limeos_contracts::ResolveJobResponse> {
        let mut check = PasswordCheck::NotReached;
        let result = self
            .human_resolve_checked(token, csrf, id, input, &attempt, &mut check)
            .await;
        PasswordChecked::new(check, result)
    }
}
async fn handle_rpc(core: Core, mut stream: UnixStream) -> Result<()> {
    let peer = stream
        .peer_cred()
        .map_err(|_| Error(ErrorCode::Forbidden))?;
    let result: Result<CoreResponse> = async {
        let request: CoreRequest = limeos_contracts::read_frame(&mut stream).await?;
        let response = match request {
            CoreRequest::Health { version } if version == VERSION => {
                CoreResponse::Health(core.health().await?)
            }
            CoreRequest::IssueBootstrap { version } if version == VERSION && peer.uid() == 0 => {
                let token = limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?;
                let digest = limeos_identity::digest(&token);
                let expires_at = core
                    .db
                    .call(move |s| s.issue_bootstrap(&digest, now()))
                    .await?;
                CoreResponse::Bootstrap { token, expires_at }
            }
            CoreRequest::Enroll {
                version,
                enrollment,
            } if version == VERSION && peer.uid() == 0 => {
                core.enroll(enrollment).await?;
                CoreResponse::Enrolled
            }
            CoreRequest::ImportLegacyUsers { version, users }
                if version == VERSION && peer.uid() == 0 =>
            {
                let (imported, skipped_existing) = core.import_legacy_users(users).await?;
                CoreResponse::LegacyUsersImported {
                    imported,
                    skipped_existing,
                }
            }
            CoreRequest::ReadResolution {
                version,
                admin,
                job,
            } if version == VERSION && peer.uid() == 0 => {
                CoreResponse::Resolution(core.rpc_resolution_view(admin, job).await?)
            }
            CoreRequest::ResolveJob {
                version,
                admin,
                job,
                declared,
                reason,
                evidence_digest,
            } if version == VERSION && peer.uid() == 0 => CoreResponse::Resolved(
                core.rpc_resolve(admin, job, declared, reason, evidence_digest)
                    .await?,
            ),
            CoreRequest::CheckTask {
                version,
                token,
                task,
                operation,
                resource,
            } if version == VERSION => {
                let digest = limeos_identity::digest(&token);
                let uid = peer.uid();
                core.db
                    .call(move |s| {
                        s.check_task(
                            &digest,
                            uid,
                            &task,
                            &Scope {
                                operation,
                                resource,
                            },
                            now(),
                        )
                    })
                    .await?;
                CoreResponse::Authorized
            }
            CoreRequest::ProposeRestart {
                version,
                token,
                task,
                resource,
            } if version == VERSION => {
                let proposal = core
                    .propose_task(
                        token,
                        peer.uid(),
                        task,
                        limeos_domain::ContainerAction::Restart,
                        resource,
                    )
                    .await?;
                CoreResponse::RestartPlan(proposal)
            }
            CoreRequest::QueueRestart {
                version,
                token,
                task,
                input,
            } if version == VERSION => {
                if input.proposal.plan.operation != limeos_domain::ContainerAction::Restart {
                    return Err(Error(ErrorCode::InvalidInput));
                }
                let job = core.submit_task(token, peer.uid(), task, *input).await?;
                CoreResponse::RestartJob(job)
            }
            CoreRequest::ProposeContainer {
                version,
                token,
                task,
                resource,
                operation,
            } if version == VERSION => CoreResponse::ContainerPlan(
                core.propose_task(token, peer.uid(), task, operation, resource)
                    .await?,
            ),
            CoreRequest::QueueContainer {
                version,
                token,
                task,
                input,
            } if version == VERSION => {
                CoreResponse::ContainerJob(core.submit_task(token, peer.uid(), task, *input).await?)
            }
            CoreRequest::ReadContainerLogs {
                version,
                token,
                task,
                resource,
                options,
            } if version == VERSION => CoreResponse::ContainerLogs(
                core.task_logs(token, peer.uid(), task, resource, options)
                    .await?,
            ),
            CoreRequest::ProposeCompose {
                version,
                token,
                task,
                selection,
            } if version == VERSION => CoreResponse::ComposePlan(
                core.propose_compose_task(token, peer.uid(), task, selection)
                    .await?,
            ),
            _ => return Err(Error(ErrorCode::Forbidden)),
        };
        Ok(response)
    }
    .await;
    let response = result.unwrap_or_else(|error| {
        CoreResponse::Error(ErrorEnvelope {
            code: error.0,
            message: error.message().into(),
            retry: matches!(error.0, ErrorCode::Overloaded | ErrorCode::Unavailable),
            audit_id: format!("rpc-{}", now()),
        })
    });
    limeos_contracts::write_frame(&mut stream, &response).await
}
async fn rpc_server(core: Core, listener: UnixListener) -> std::io::Result<()> {
    let permits = Arc::new(Semaphore::new(16));
    let mut failures = 0u64;
    // Sent from this loop, so systemd restarts core if it stops being run.
    // It does not depend on the store's writer: a stuck writer is almost
    // always the device (a hung fsync on a failing SD card) or a lock held
    // from outside, which a new process meets again, so restarting would
    // loop, and a thread in uninterruptible sleep would keep the old process
    // from exiting. Store calls fail at their deadline instead, and health
    // reports the store degraded and core not ready.
    let mut watchdog = limeos_contracts::Watchdog::from_env();
    loop {
        let accepted = tokio::select! {
            accepted = listener.accept() => accepted,
            () = watchdog.due() => {
                if let Err(error) = watchdog.beat() {
                    tracing::warn!(%error, "watchdog keep-alive could not be sent");
                }
                continue;
            }
        };
        let stream = match accepted {
            Ok((stream, _)) => {
                failures = 0;
                stream
            }
            // Descriptor exhaustion or an aborted peer: the listener still works.
            Err(error) if limeos_contracts::accept_retryable(&error) => {
                failures += 1;
                if failures.is_power_of_two() {
                    tracing::warn!(%error, failures, "local RPC accept failed; retrying");
                }
                tokio::time::sleep(limeos_contracts::ACCEPT_PAUSE).await;
                continue;
            }
            Err(error) => return Err(error),
        };
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            continue;
        };
        let core = core.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let _ = tokio::time::timeout(limeos_contracts::RPC_DEADLINE, handle_rpc(core, stream))
                .await;
        });
    }
}
fn options() -> Result<(PathBuf, PathBuf, PathBuf)> {
    let mut state = PathBuf::from("/var/lib/limeos/core");
    let mut config = PathBuf::from("/etc/limeos/core.json");
    let mut socket = PathBuf::from("/run/limeos-core/core.sock");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = args.next().ok_or(Error(ErrorCode::InvalidInput))?;
        match arg.as_str() {
            "--state-dir" => state = value.into(),
            "--config" => config = value.into(),
            "--socket" => socket = value.into(),
            _ => return Err(Error(ErrorCode::InvalidInput)),
        }
    }
    Ok((state, config, socket))
}
fn bind_socket(path: &Path) -> Result<UnixListener> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        use std::os::unix::fs::FileTypeExt;
        if !meta.file_type().is_socket() || std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Err(Error(ErrorCode::Conflict));
        }
        std::fs::remove_file(path).map_err(|_| Error(ErrorCode::Unavailable))?;
    }
    let listener = UnixListener::bind(path).map_err(|_| Error(ErrorCode::Unavailable))?;
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o660))
        .map_err(|_| Error(ErrorCode::Unavailable))?;
    Ok(listener)
}
/// An item the store's writer is still on after this is not legitimate work
/// (each is a sub-second commit): the store is reported degraded.
const STORE_STALL: Duration = Duration::from_secs(10);
/// How long health waits for the writer to answer a probe; it never waits
/// out the full call deadline.
const STORE_PROBE: Duration = Duration::from_secs(2);
/// How long health's signed-in check may take on the query-only connection.
const SIGNED_IN_BOUND: Duration = Duration::from_millis(250);
impl Core {
    /// The store as a degraded subsystem, if its writer is stuck. Read from
    /// the writer's activity, which needs no queue, and from a probe bounded
    /// by `STORE_PROBE`, so health answers even when every call is stuck.
    /// A closed writer or a full queue is an error, as before.
    async fn store_health(&self) -> Result<Option<limeos_contracts::DegradedSubsystem>> {
        let activity = self.db.writer_activity();
        let stalled = activity.busy_for.filter(|busy| *busy >= STORE_STALL);
        if stalled.is_none() {
            if let Ok(answer) = tokio::time::timeout(STORE_PROBE, self.db.call(|_| Ok(()))).await {
                return answer.map(|()| None);
            }
        }
        let activity = self.db.writer_activity();
        let busy = activity.busy_for.unwrap_or(STORE_PROBE);
        Ok(Some(limeos_contracts::DegradedSubsystem {
            subsystem: limeos_contracts::Subsystem::Store,
            detail: Some(limeos_contracts::DegradedDetail {
                code: ErrorCode::Unavailable,
                since: now() - i64::try_from(busy.as_secs()).unwrap_or(i64::MAX / 2),
                count: u32::try_from(activity.queued).unwrap_or(u32::MAX),
            }),
        }))
    }
}
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    tracing_subscriber::fmt().json().with_target(false).init();
    if let Err(error) = run().await {
        tracing::error!(code=?error.0,message=error.message(),"core stopped");
        std::process::exit(1);
    }
}
async fn run() -> Result<()> {
    let (state, config_path, socket) = options()?;
    let config = match config::read(&config_path)? {
        ConfigRead::Valid(c) => c,
        ConfigRead::Missing => config::Config::default(),
        ConfigRead::Corrupt => return Err(Error(ErrorCode::CorruptConfiguration)),
    };
    std::fs::create_dir_all(&state).map_err(|_| Error(ErrorCode::StateNotDurable))?;
    std::fs::set_permissions(&state, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .map_err(|_| Error(ErrorCode::StateNotDurable))?;
    let store = state.join("core.sqlite");
    // A schema upgrade inside `open` builds indexes over up to 64 MiB of
    // audit history, which on SD storage has no stated bound: systemd's
    // start timeout is extended while it runs (up to a cap), rather than
    // killing core mid-upgrade to roll back and repeat it.
    let opening = limeos_contracts::StartExtension::from_env(
        "Opening the store; a schema upgrade can take several minutes",
    );
    let db = Database::open(&store);
    drop(opening);
    let db = db.inspect_err(|_| {
        if let Some(space) = limeos_persistence::upgrade_space(&store) {
            tracing::error!(
                from = space.from,
                to = limeos_persistence::SCHEMA_VERSION,
                needed = space.needed,
                available = space.available,
                "the database schema upgrade needs more free space in the state directory; free space there and restart"
            );
        }
    })?;
    let telemetry =
        limeos_observations::telemetry::Telemetry::open(&state.join("metrics.sqlite")).ok();
    let observations = limeos_observations::start_core(
        config.host_socket.clone().into(),
        config.container_socket.clone().into(),
        telemetry.clone(),
    );
    let mut core = Core {
        automation_status: automation_runtime::Status::default(),
        reader_health: Default::default(),
        stacks: limeos_observations::stacks::start_proxy(config.storage_socket.clone().into()),
        db,
        password_workers: Arc::new(PasswordWorkers::new(2)),
        dummy_hash: "".into(),
        observations,
        telemetry,
        container_socket: Arc::new(config.container_socket.into()),
        storage_socket: Arc::new(config.storage_socket.into()),
        storage_target_socket: Arc::new(config.storage_target_socket.into()),
        storage_mount_socket: Arc::new(config.storage_mount_socket.into()),
        storage_readers: Arc::new(Semaphore::new(1)),
        claims_in_doubt: Default::default(),
        compose_catalog: config::read_compose_catalog(
            &config_path.with_file_name("compose-catalog.json"),
        )?
        .map(Arc::new),
    };
    core.dummy_hash = core
        .hash(limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?)
        .await?
        .into();
    let http = TcpListener::bind(&config.listen)
        .await
        .map_err(|_| Error(ErrorCode::Unavailable))?;
    let rpc = bind_socket(&socket)?;
    tracing::info!(generation = core.db.generation, "core ready");
    // Type=notify: the store is open, the listeners are bound and serving
    // starts below, so systemd counts core started (and starts its watchdog
    // clock) only now.
    if let Err(error) = limeos_contracts::notify_ready() {
        tracing::warn!(%error, "could not tell systemd that core is ready");
    }
    let bridge = automation_store::ObservationStoreBridge::new(core.db.clone());
    let reader = health_collection::HealthReader::new(
        (*core.storage_socket).clone(),
        core.storage_readers.clone(),
    );
    tokio::select! {
        result=services(core,http,rpc,config.origin,bridge,reader,supervision::Backoff::STANDARD)=>result,
        _=tokio::signal::ctrl_c()=>Ok(()),
        _=async {if let Ok(mut signal)=tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()){signal.recv().await;}}=>Ok(()),
    }
}
/// Everything core runs once started. Only a listener that can no longer
/// accept ends it (systemd then restarts core). The dispatcher handles each
/// job's errors itself and never returns; the observation runtime is
/// restarted after a failure and reported in `Health::degraded` meanwhile;
/// the storage reader's health is asked for and reported there too.
async fn services(
    core: Core,
    http: TcpListener,
    rpc: UnixListener,
    origin: String,
    bridge: automation_store::ObservationStoreBridge,
    reader: health_collection::HealthReader,
    backoff: supervision::Backoff,
) -> Result<()> {
    let app = limeos_api::router(core.clone(), origin);
    let reader_health =
        reader_health::Watcher::new((*core.storage_socket).clone(), core.reader_health.clone());
    let dispatcher = core.clone();
    let status = core.automation_status.clone();
    let observation = supervision::supervise(
        limeos_contracts::Subsystem::ObservationRuntime,
        status.health(),
        backoff,
        move || {
            automation_runtime::ObservationRuntime::new(
                bridge.clone(),
                reader.clone(),
                status.clone(),
            )
            .run()
        },
    );
    tokio::select! {
        result=limeos_api::serve(http,app)=>result.map_err(|_|Error(ErrorCode::Unavailable)),
        result=rpc_server(core,rpc)=>result.map_err(|_|Error(ErrorCode::Unavailable)),
        _=dispatcher.dispatcher()=>Err(Error(ErrorCode::Unavailable)),
        _=observation=>Err(Error(ErrorCode::Unavailable)),
        ()=reader_health.run()=>Err(Error(ErrorCode::Unavailable)),
    }
}

#[cfg(test)]
mod compose_tests;
#[cfg(test)]
mod operation_tests;
#[cfg(test)]
mod stack_operation_tests;
#[cfg(test)]
mod storage_tests;
#[cfg(test)]
mod tests;
