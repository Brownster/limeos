use axum::{
    Router,
    body::to_bytes,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use limeos_contracts::{ErrorEnvelope, Health, Login, SessionView};
use limeos_domain::{Error, ErrorCode, Result};
use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;
// Proposed routes that no router mounts yet, hence unused outside their tests;
// see docs/deferred-code.md.
#[cfg(feature = "deferred")]
#[allow(dead_code)]
mod automation;
mod automation_history;
mod automation_policy;
mod automation_schedules;
mod compose;
#[cfg(feature = "deferred")]
#[allow(dead_code)]
mod deployment;
mod identity_enablement;
mod job_resolution;
#[cfg(test)]
mod operation_tests;
mod operations;
#[cfg(test)]
mod read_tests;
mod reads;
mod serve;
pub use serve::{Limits, serve, serve_with};
#[cfg(test)]
mod serve_tests;
#[cfg(test)]
mod stack_operation_tests;
mod stack_operations;
mod stacks;
mod storage;
mod storage_mounts;
#[cfg(test)]
mod test_backend;
#[cfg(test)]
use test_backend::TestBackend;

pub struct IssuedSession {
    pub token: String,
    pub view: SessionView,
}
/// How far a password-gated call got with the password itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasswordCheck {
    /// Refused before any comparison: storage, a busy worker, a closed
    /// session. Nothing about the password was learned.
    NotReached,
    /// Compared and wrong, including a name with no account (compared
    /// against a dummy hash, so the work is the same).
    Rejected,
    /// The password of an existing account matched. Later steps may still
    /// fail; that does not undo the match.
    Accepted,
}
/// Handed to a password-gated backend call, which marks the moment it starts
/// a comparison: once a worker is admitted, right before the password goes
/// to it. A request cancelled before that mark is refunded; after it, the
/// charge stands, since the work was spent and refunding would let a client
/// start unlimited comparisons by cancelling each one.
#[derive(Clone, Default)]
pub struct PasswordAttempt(Arc<std::sync::atomic::AtomicBool>);
impl PasswordAttempt {
    pub fn start_comparison(&self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    pub fn comparison_started(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}
/// A password-gated call's result together with how far its check got.
pub struct PasswordChecked<T> {
    pub check: PasswordCheck,
    pub result: Result<T>,
}
impl<T> PasswordChecked<T> {
    pub fn new(check: PasswordCheck, result: Result<T>) -> Self {
        Self { check, result }
    }
    /// A refusal before any comparison.
    pub fn not_reached(error: Error) -> Self {
        Self::new(PasswordCheck::NotReached, Err(error))
    }
    /// A wrong password, or no such account.
    pub fn rejected() -> Self {
        Self::new(
            PasswordCheck::Rejected,
            Err(Error(ErrorCode::Unauthenticated)),
        )
    }
}
/// Everything the routes ask of core. No method has a default body: a route
/// wired here that core does not implement fails to compile, rather than
/// answering 503 at run time. Tests implement `TestBackend` instead.
pub trait Backend: Clone + Send + Sync + 'static {
    fn observe_health_policy(
        &self,
        token: String,
        deadline: Instant,
    ) -> impl Future<Output = Result<limeos_domain::automation::HealthPolicyView>> + Send;
    fn replace_observe_health_policy(
        &self,
        token: String,
        csrf: String,
        input: limeos_domain::automation::ReplaceHealthPolicy,
        deadline: Instant,
    ) -> impl Future<Output = Result<limeos_domain::automation::HealthPolicyView>> + Send;
    fn observe_health_alerts(
        &self,
        token: String,
        after: Option<String>,
        deadline: Instant,
    ) -> impl Future<
        Output = Result<
            limeos_domain::automation::AutomationPage<limeos_domain::automation::AlertState>,
        >,
    > + Send;
    fn observe_health_notifications(
        &self,
        token: String,
        after: Option<String>,
        deadline: Instant,
    ) -> impl Future<
        Output = Result<
            limeos_domain::automation::AutomationPage<limeos_domain::automation::Notification>,
        >,
    > + Send;
    fn acknowledge_observe_health_notification(
        &self,
        token: String,
        csrf: String,
        id: String,
        input: limeos_domain::automation::AcknowledgeNotification,
        deadline: Instant,
    ) -> impl Future<Output = Result<limeos_domain::automation::Notification>> + Send;
    /// Current stored session/grants and the shared stack cache only.
    fn stack_inventory(
        &self,
        token: String,
    ) -> impl Future<Output = Result<limeos_domain::deployment::discovery::StackInventory>> + Send;
    fn observe_health_schedules(
        &self,
        token: String,
        after: Option<String>,
        deadline: Instant,
    ) -> impl Future<
        Output = Result<
            limeos_domain::automation::AutomationPage<limeos_domain::automation::ScheduleView>,
        >,
    > + Send;
    fn create_observe_health_schedule(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::ObserveHealthScheduleInput,
        deadline: Instant,
    ) -> impl Future<Output = Result<limeos_domain::automation::ScheduleView>> + Send;
    fn pause_observe_health_schedule(
        &self,
        token: String,
        csrf: String,
        id: String,
        input: limeos_domain::automation::PauseSchedule,
        deadline: Instant,
    ) -> impl Future<Output = Result<limeos_domain::automation::ScheduleView>> + Send;
    fn enable_observe_health(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::ObserveHealthEnable,
        deadline: Instant,
        attempt: PasswordAttempt,
    ) -> impl Future<Output = PasswordChecked<limeos_contracts::ObserveHealthEnabled>> + Send;
    fn storage_mount_catalog(
        &self,
        token: String,
    ) -> impl Future<Output = Result<limeos_domain::StorageMountCatalog>> + Send;
    fn plan_storage_mount(
        &self,
        token: String,
        csrf: String,
        selection: limeos_domain::StorageMountSelection,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageMount>> + Send;
    fn storage_mount_plan(
        &self,
        token: String,
        id: String,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageMount>> + Send;
    fn approve_storage_mount(
        &self,
        token: String,
        csrf: String,
        id: String,
        digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send;
    fn queue_storage_mount(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::QueueStorageMountInput,
    ) -> impl Future<Output = Result<limeos_domain::StorageMountJob>> + Send;
    fn storage_mount_job(
        &self,
        token: String,
        id: String,
    ) -> impl Future<Output = Result<limeos_domain::StorageMountJob>> + Send;
    fn storage_mount_jobs(
        &self,
        token: String,
    ) -> impl Future<Output = Result<Vec<limeos_domain::StorageMountJob>>> + Send;
    fn cancel_storage_mount(
        &self,
        token: String,
        csrf: String,
        id: String,
    ) -> impl Future<Output = Result<()>> + Send;
    fn cancel_storage_mount_job(
        &self,
        token: String,
        csrf: String,
        id: String,
    ) -> impl Future<Output = Result<()>> + Send;
    fn container_storage_inventory(
        &self,
        token: String,
    ) -> impl Future<Output = Result<limeos_domain::ContainerStorageInventoryView>> + Send;
    fn plan_storage_targets(
        &self,
        token: String,
        csrf: String,
        contract: limeos_domain::StorageContract,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageTargets>> + Send;
    fn storage_target_plan(
        &self,
        token: String,
        id: String,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageTargets>> + Send;
    fn approve_storage_targets(
        &self,
        token: String,
        csrf: String,
        id: String,
        digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send;
    fn queue_storage_targets(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::QueueStorageTargetsInput,
    ) -> impl Future<Output = Result<limeos_domain::StorageTargetJob>> + Send;
    fn storage_target_job(
        &self,
        token: String,
        id: String,
    ) -> impl Future<Output = Result<limeos_domain::StorageTargetJob>> + Send;
    fn cancel_storage_targets(
        &self,
        token: String,
        csrf: String,
        id: String,
    ) -> impl Future<Output = Result<()>> + Send;
    fn cancel_storage_target_job(
        &self,
        token: String,
        csrf: String,
        id: String,
    ) -> impl Future<Output = Result<()>> + Send;
    fn storage_inventory(
        &self,
        token: String,
    ) -> impl Future<Output = Result<limeos_domain::StorageInventoryView>> + Send;
    fn plan_storage(
        &self,
        token: String,
        csrf: String,
        input: limeos_domain::StorageSetupInput,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageSetup>> + Send;
    fn storage_plan(
        &self,
        token: String,
        id: String,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageSetup>> + Send;
    fn approve_storage(
        &self,
        token: String,
        csrf: String,
        id: String,
        digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send;
    fn cancel_storage(
        &self,
        token: String,
        csrf: String,
        id: String,
    ) -> impl Future<Output = Result<()>> + Send;
    fn plan_compose(
        &self,
        token: String,
        csrf: String,
        selection: limeos_domain::ComposeSelection,
    ) -> impl Future<Output = Result<limeos_domain::PlannedCompose>> + Send;
    fn compose_plan(
        &self,
        token: String,
        id: String,
    ) -> impl Future<Output = Result<limeos_domain::PlannedCompose>> + Send;
    fn approve_compose(
        &self,
        token: String,
        csrf: String,
        id: String,
        digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send;
    fn cancel_compose(
        &self,
        token: String,
        csrf: String,
        id: String,
    ) -> impl Future<Output = Result<()>> + Send;
    fn login(
        &self,
        request: Login,
        attempt: PasswordAttempt,
    ) -> impl Future<Output = PasswordChecked<IssuedSession>> + Send;
    fn session(
        &self,
        token: String,
        csrf: Option<String>,
    ) -> impl Future<Output = Result<SessionView>> + Send;
    fn logout(&self, token: String, csrf: String) -> impl Future<Output = Result<()>> + Send;
    fn health(&self) -> impl Future<Output = Result<Health>> + Send;
    /// Whether `token` names a live session, for how much health shows. It
    /// must answer promptly and add no work to the store's queue (core reads
    /// its query-only connection); false when unsure.
    fn signed_in(&self, token: String) -> impl Future<Output = bool> + Send;
    fn observations(
        &self,
        token: String,
    ) -> impl Future<Output = Result<limeos_contracts::Overview>> + Send;
    fn history(
        &self,
        token: String,
        range: limeos_contracts::HistoryRange,
    ) -> impl Future<Output = Result<limeos_contracts::MetricHistory>> + Send;
    fn changes(&self) -> tokio::sync::watch::Receiver<u64>;
    fn plan_restart(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::RestartInput,
    ) -> impl Future<Output = Result<limeos_domain::PlannedRestart>> + Send;
    fn plan_container(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::ContainerInput,
    ) -> impl Future<Output = Result<limeos_domain::PlannedContainerAction>> + Send;
    fn container_logs(
        &self,
        token: String,
        resource: String,
        options: limeos_domain::LogOptions,
    ) -> impl Future<Output = Result<limeos_domain::ContainerLogs>> + Send;
    fn approve_container(
        &self,
        token: String,
        csrf: String,
        id: String,
        digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send;
    fn queue_container(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::QueueRestartInput,
    ) -> impl Future<Output = Result<limeos_domain::RestartJob>> + Send;
    fn restart_progress(
        &self,
        token: String,
        id: String,
        after: i64,
    ) -> impl Future<Output = Result<limeos_contracts::JobProgress>> + Send;
    fn container_jobs(
        &self,
        token: String,
    ) -> impl Future<Output = Result<Vec<limeos_domain::RestartJob>>> + Send;
    fn cancel_restart(
        &self,
        token: String,
        csrf: String,
        id: String,
        plan: bool,
    ) -> impl Future<Output = Result<()>> + Send;
    fn plan_stack(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::StackInput,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStackAction>> + Send;
    fn approve_stack(
        &self,
        token: String,
        csrf: String,
        id: String,
        digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send;
    fn queue_stack(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::QueueStackInput,
    ) -> impl Future<Output = Result<limeos_domain::StackJob>> + Send;
    fn stack_job_progress(
        &self,
        token: String,
        id: String,
        after: i64,
    ) -> impl Future<Output = Result<limeos_contracts::StackJobProgress>> + Send;
    fn stack_jobs(
        &self,
        token: String,
    ) -> impl Future<Output = Result<Vec<limeos_domain::StackJob>>> + Send;
    fn cancel_stack_action(
        &self,
        token: String,
        csrf: String,
        id: String,
        plan: bool,
    ) -> impl Future<Output = Result<()>> + Send;
    /// Administrator session only. Fresh read-only inspection of the job's
    /// targets; never an Engine effect.
    fn job_resolution(
        &self,
        token: String,
        id: String,
    ) -> impl Future<Output = Result<limeos_contracts::ResolutionView>> + Send;
    /// Administrator session only. Read-only: every principal's container and
    /// stack jobs that hold locks until resolved, at most 64.
    fn needs_attention_jobs(
        &self,
        token: String,
    ) -> impl Future<Output = Result<Vec<limeos_domain::NeedsAttentionJob>>> + Send;
    /// Administrator session, CSRF and the administrator's password.
    fn resolve_job(
        &self,
        token: String,
        csrf: String,
        id: String,
        input: limeos_contracts::ResolveJobInput,
        attempt: PasswordAttempt,
    ) -> impl Future<Output = PasswordChecked<limeos_contracts::ResolveJobResponse>> + Send;
}
#[derive(Clone)]
struct App<B> {
    backend: B,
    origin: Arc<str>,
    limiter: Arc<Mutex<Limiter>>,
    readers: Arc<Semaphore>,
    /// Open observation streams per session token, within `readers`.
    streams: Arc<Mutex<HashMap<String, usize>>>,
}
/// Whose password is being tried. Every peer is the local proxy, so the
/// address says nothing; forwarded headers are never trusted either.
#[derive(Clone, Copy)]
enum Attempt<'a> {
    /// The username a login names, whether or not such an account exists.
    /// Already checked by `Login::well_formed`, so at most 64 bytes.
    Login(&'a str),
    /// The principal of an authenticated session re-entering its password.
    Principal(&'a str),
}
/// An attempt's budget key: the full name, never a hash of it.
#[derive(Clone, PartialEq, Eq, Hash)]
enum AttemptKey {
    Login(String),
    Principal(String),
}
impl Attempt<'_> {
    fn key(self) -> AttemptKey {
        match self {
            Self::Login(name) => AttemptKey::Login(name.into()),
            Self::Principal(id) => AttemptKey::Principal(id.into()),
        }
    }
}
/// Failed password attempts allowed per name and window.
const ATTEMPTS: u32 = 10;
const ATTEMPT_WINDOW: Duration = Duration::from_secs(60);
/// Names with a live budget. Worst case, with 64-byte usernames and principal
/// ids: about 56 bytes per table bucket (key, window, count) for at most
/// 4,096 buckets, plus one heap allocation of at most 80 bytes per entry,
/// under 0.5 MiB in all. In practice far fewer are live: an entry lasts one
/// window, wrong passwords for names with no account are capped by core at
/// 120 a minute, and refunded entries are removed at once.
const ATTEMPT_CAPACITY: usize = 2048;
/// Exact per-name budgets of failed password attempts: ten per window for
/// each username and each principal, with no sharing between names. When
/// the table is full of live entries an unseen name is refused (fail
/// closed). The aggregate cost of names with no account is bounded in core.
#[derive(Default)]
struct Limiter {
    budgets: HashMap<AttemptKey, (Instant, u32)>,
}
/// The name and window one charge was taken from. A refund applies only to
/// that window: a check charged just before the window rolled must not erase
/// a failure counted in the next one.
struct Ticket {
    key: AttemptKey,
    window: Instant,
}
impl Limiter {
    /// Charge one attempt before the password work; `None` when the name's
    /// budget is spent or the table is full. Charging first keeps concurrent
    /// attempts inside the budget.
    fn charge(&mut self, attempt: Attempt, now: Instant) -> Option<Ticket> {
        self.budgets
            .retain(|_, (window, _)| now.saturating_duration_since(*window) < ATTEMPT_WINDOW);
        let key = attempt.key();
        if !self.budgets.contains_key(&key) && self.budgets.len() >= ATTEMPT_CAPACITY {
            return None;
        }
        let (window, count) = self.budgets.entry(key.clone()).or_insert((now, 0));
        if *count >= ATTEMPTS {
            return None;
        }
        *count += 1;
        Some(Ticket {
            key,
            window: *window,
        })
    }
    fn refund(&mut self, ticket: Ticket) {
        if let Some((window, count)) = self.budgets.get_mut(&ticket.key) {
            if *window == ticket.window {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.budgets.remove(&ticket.key);
                }
            }
        }
    }
}
/// Run one password check under the failed-attempt budget. Callers validate
/// origin, credentials headers and the body first, so junk is never charged.
///
/// The charge is refunded when no comparison happened, or when the password
/// was correct and the response already shows that (anything but 401). An
/// accepted password whose call still answers 401 keeps its charge: a refund
/// there would let the budget tell it apart from a wrong one. A request
/// dropped before it completes is settled by `Charge` instead.
async fn password_attempt<B, T, F>(
    app: &App<B>,
    attempt: Attempt<'_>,
    check: impl FnOnce(PasswordAttempt) -> F,
) -> Result<T>
where
    F: Future<Output = PasswordChecked<T>>,
{
    let ticket = app
        .limiter
        .lock()
        .ok()
        .and_then(|mut limiter| limiter.charge(attempt, Instant::now()));
    let Some(ticket) = ticket else {
        return Err(Error(ErrorCode::Overloaded));
    };
    let handle = PasswordAttempt::default();
    let charge = Charge {
        limiter: &app.limiter,
        ticket: Some(ticket),
        attempt: handle.clone(),
    };
    let PasswordChecked { check, result } = check(handle).await;
    charge.settle(match check {
        PasswordCheck::NotReached => true,
        PasswordCheck::Rejected => false,
        PasswordCheck::Accepted => !matches!(result, Err(Error(ErrorCode::Unauthenticated))),
    });
    result
}
/// One charged attempt, settled exactly once: by `settle` when the check
/// completes, or on drop when the request is cancelled first. A cancelled
/// request is refunded only if its comparison had not started.
struct Charge<'a> {
    limiter: &'a Mutex<Limiter>,
    ticket: Option<Ticket>,
    attempt: PasswordAttempt,
}
impl Charge<'_> {
    fn settle(mut self, refund: bool) {
        if let Some(ticket) = self.ticket.take() {
            if refund {
                self.refund(ticket);
            }
        }
    }
    fn refund(&self, ticket: Ticket) {
        if let Ok(mut limiter) = self.limiter.lock() {
            limiter.refund(ticket);
        }
    }
}
impl Drop for Charge<'_> {
    fn drop(&mut self) {
        if let Some(ticket) = self.ticket.take() {
            if !self.attempt.comparison_started() {
                self.refund(ticket);
            }
        }
    }
}
pub fn router<B: Backend>(backend: B, origin: String) -> Router {
    Router::new()
        .route("/api/v1/deployment/stacks", get(stacks::inventory::<B>))
        .route("/api/v1/health", get(health::<B>))
        .route(
            "/api/v1/storage/mounts/targets",
            get(storage_mounts::catalog::<B>),
        )
        .route(
            "/api/v1/storage/mounts/plans",
            post(storage_mounts::plan::<B>),
        )
        .route(
            "/api/v1/storage/mounts/plans/{id}",
            get(storage_mounts::read::<B>),
        )
        .route(
            "/api/v1/storage/mounts/plans/{id}/approval",
            post(storage_mounts::approve::<B>),
        )
        .route(
            "/api/v1/storage/mounts/plans/{id}/cancel",
            post(storage_mounts::cancel::<B>),
        )
        .route(
            "/api/v1/storage/mounts/jobs",
            get(storage_mounts::jobs::<B>).post(storage_mounts::queue::<B>),
        )
        .route(
            "/api/v1/storage/mounts/jobs/{id}",
            get(storage_mounts::job::<B>),
        )
        .route(
            "/api/v1/storage/mounts/jobs/{id}/cancel",
            post(storage_mounts::cancel_job::<B>),
        )
        .route(
            "/api/v1/storage/targets/plans",
            post(storage::target_plan::<B>),
        )
        .route(
            "/api/v1/storage/targets/plans/{id}",
            get(storage::target_read::<B>),
        )
        .route(
            "/api/v1/storage/targets/plans/{id}/approval",
            post(storage::target_approve::<B>),
        )
        .route(
            "/api/v1/storage/targets/plans/{id}/cancel",
            post(storage::target_cancel::<B>),
        )
        .route(
            "/api/v1/storage/targets/jobs",
            post(storage::target_queue::<B>),
        )
        .route(
            "/api/v1/storage/targets/jobs/{id}",
            get(storage::target_job::<B>),
        )
        .route(
            "/api/v1/storage/targets/jobs/{id}/cancel",
            post(storage::target_cancel_job::<B>),
        )
        .route("/api/v1/storage/inventory", get(storage::inventory::<B>))
        .route(
            "/api/v1/storage/container-dependencies",
            get(storage::container_dependencies::<B>),
        )
        .route("/api/v1/storage/plans", post(storage::plan::<B>))
        .route("/api/v1/storage/plans/{id}", get(storage::read::<B>))
        .route(
            "/api/v1/storage/plans/{id}/approval",
            post(storage::approve::<B>),
        )
        .route(
            "/api/v1/storage/plans/{id}/cancel",
            post(storage::cancel::<B>),
        )
        .route("/api/v1/auth/login", post(login::<B>))
        .route(
            "/api/v1/auth/capabilities/observe-health/enable",
            post(identity_enablement::enable::<B>),
        )
        .route("/api/v1/auth/session", get(session::<B>))
        .route("/api/v1/auth/logout", post(logout::<B>))
        .route("/api/v1/overview", get(reads::overview::<B>))
        .route("/api/v1/resources", get(reads::resources::<B>))
        .route("/api/v1/system/history", get(reads::history::<B>))
        .route("/api/v1/observations/stream", get(reads::stream::<B>))
        .route("/api/v1/compose/plans", post(compose::plan::<B>))
        .route("/api/v1/compose/plans/{id}", get(compose::read::<B>))
        .route(
            "/api/v1/compose/plans/{id}/approval",
            post(compose::approve::<B>),
        )
        .route(
            "/api/v1/compose/plans/{id}/cancel",
            post(compose::cancel::<B>),
        )
        .route(
            "/api/v1/container/restart/plans",
            post(operations::plan::<B>),
        )
        .route(
            "/api/v1/container/plans",
            post(operations::container_plan::<B>),
        )
        .route(
            "/api/v1/container/plans/{id}/approval",
            post(operations::approve::<B>),
        )
        .route(
            "/api/v1/container/plans/{id}/cancel",
            post(operations::cancel_plan::<B>),
        )
        .route(
            "/api/v1/container/jobs",
            post(operations::container_queue::<B>).get(operations::jobs::<B>),
        )
        .route(
            "/api/v1/container/jobs/{id}",
            get(operations::progress::<B>),
        )
        .route(
            "/api/v1/container/jobs/{id}/cancel",
            post(operations::cancel_job::<B>),
        )
        .route("/api/v1/containers/{id}/logs", get(operations::logs::<B>))
        .route(
            "/api/v1/container/restart/plans/{id}/approval",
            post(operations::approve::<B>),
        )
        .route(
            "/api/v1/container/restart/plans/{id}/cancel",
            post(operations::cancel_plan::<B>),
        )
        .route(
            "/api/v1/container/restart/jobs",
            post(operations::queue::<B>).get(operations::jobs::<B>),
        )
        .route(
            "/api/v1/container/restart/jobs/{id}",
            get(operations::progress::<B>),
        )
        .route(
            "/api/v1/container/restart/jobs/{id}/cancel",
            post(operations::cancel_job::<B>),
        )
        .route("/api/v1/stack/plans", post(stack_operations::plan::<B>))
        .route(
            "/api/v1/stack/plans/{id}/approval",
            post(stack_operations::approve::<B>),
        )
        .route(
            "/api/v1/stack/plans/{id}/cancel",
            post(stack_operations::cancel_plan::<B>),
        )
        .route(
            "/api/v1/stack/jobs",
            post(stack_operations::queue::<B>).get(stack_operations::jobs::<B>),
        )
        .route(
            "/api/v1/stack/jobs/{id}",
            get(stack_operations::progress::<B>),
        )
        .route(
            "/api/v1/stack/jobs/{id}/cancel",
            post(stack_operations::cancel_job::<B>),
        )
        .route(
            "/api/v1/jobs/needs-attention",
            get(job_resolution::needs_attention::<B>),
        )
        .route(
            "/api/v1/jobs/{id}/resolution",
            get(job_resolution::read::<B>).post(job_resolution::resolve::<B>),
        )
        .merge(automation_schedules::routes::<B>())
        .merge(automation_history::routes::<B>())
        .merge(automation_policy::routes::<B>())
        .fallback(|| async { failure(Error(ErrorCode::NotFound)) })
        .layer(middleware::from_fn(limits))
        .with_state(App {
            backend,
            origin: origin.into(),
            limiter: Arc::new(Mutex::new(Limiter::default())),
            readers: Arc::new(Semaphore::new(8)),
            streams: Default::default(),
        })
}
fn failure(error: Error) -> Response {
    let status = match error.0 {
        ErrorCode::InvalidInput => StatusCode::BAD_REQUEST,
        ErrorCode::Unauthenticated => StatusCode::UNAUTHORIZED,
        ErrorCode::Forbidden | ErrorCode::Expired => StatusCode::FORBIDDEN,
        ErrorCode::Conflict => StatusCode::CONFLICT,
        ErrorCode::NotFound => StatusCode::NOT_FOUND,
        ErrorCode::Overloaded => StatusCode::TOO_MANY_REQUESTS,
        ErrorCode::StorageMountLimit => StatusCode::CONFLICT,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    // Identifier correlates responses without including private request material.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    (
        status,
        axum::Json(ErrorEnvelope {
            code: error.0,
            message: error.message().into(),
            retry: matches!(error.0, ErrorCode::Overloaded | ErrorCode::Unavailable),
            audit_id: format!("http-{id}"),
        }),
    )
        .into_response()
}
/// How long a handler may take before it is answered with `Unavailable`;
/// an event stream's body is not counted, only producing its response.
pub const HANDLER_DEADLINE: Duration = Duration::from_secs(10);
async fn limits(request: Request, next: Next) -> Response {
    let header_bytes: usize = request
        .headers()
        .iter()
        .map(|(k, v)| k.as_str().len() + v.len())
        .sum();
    if header_bytes > 8192 {
        return failure(Error(ErrorCode::InvalidInput));
    }
    let mut response = match tokio::time::timeout(HANDLER_DEADLINE, next.run(request)).await {
        Ok(r) => r,
        Err(_) => failure(Error(ErrorCode::Unavailable)),
    };
    let headers = response.headers_mut();
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response
}
fn origin<B>(app: &App<B>, headers: &HeaderMap) -> Result<()> {
    if headers.get("origin").and_then(|v| v.to_str().ok()) != Some(&app.origin) {
        return Err(Error(ErrorCode::Forbidden));
    }
    if headers
        .get("sec-fetch-site")
        .is_some_and(|v| v != "same-origin" && v != "none")
    {
        return Err(Error(ErrorCode::Forbidden));
    }
    Ok(())
}
fn cookie(headers: &HeaderMap) -> Result<String> {
    let mut found = None;
    for value in headers.get_all("cookie") {
        for item in value
            .to_str()
            .map_err(|_| Error(ErrorCode::Unauthenticated))?
            .split(';')
        {
            if let Some(token) = item.trim().strip_prefix("__Host-limeos=") {
                if found.is_some()
                    || token.len() != 64
                    || !token.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err(Error(ErrorCode::Unauthenticated));
                }
                found = Some(token.to_owned());
            }
        }
    }
    found.ok_or(Error(ErrorCode::Unauthenticated))
}
/// Unauthenticated: anyone who can reach the proxy learns whether core is
/// ready and which subsystems are degraded. What is wrong with them, and
/// since when, is kept for a signed-in caller (and `limeosctl status`).
/// With the store itself degraded no session is looked up at all: health
/// answers at once, without detail, and adds nothing to the stuck store.
async fn health<B: Backend>(State(app): State<App<B>>, headers: HeaderMap) -> Response {
    let mut health = match app.backend.health().await {
        Ok(health) => health,
        Err(e) => return failure(e),
    };
    let store_degraded = health
        .degraded
        .iter()
        .any(|d| d.subsystem == limeos_contracts::Subsystem::Store);
    if !health.degraded.is_empty() && (store_degraded || !signed_in(&app, &headers).await) {
        health.degraded = health
            .degraded
            .into_iter()
            .map(limeos_contracts::DegradedSubsystem::redacted)
            .collect();
    }
    axum::Json(health).into_response()
}
/// How long health waits to learn whether its caller is signed in.
const SIGNED_IN_BOUND: Duration = Duration::from_millis(500);
/// Whether the request carries a live session. Bounded by
/// `SIGNED_IN_BOUND`, so a slow store cannot hold the health check: it then
/// answers as for anyone.
async fn signed_in<B: Backend>(app: &App<B>, headers: &HeaderMap) -> bool {
    let Ok(token) = cookie(headers) else {
        return false;
    };
    matches!(
        tokio::time::timeout(SIGNED_IN_BOUND, app.backend.signed_in(token)).await,
        Ok(true)
    )
}
async fn login<B: Backend>(State(app): State<App<B>>, request: Request) -> Response {
    if let Err(e) = origin(&app, request.headers()) {
        return failure(e);
    }
    if request
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .map(|h| h.split(';').next().unwrap_or("").trim())
        != Some("application/json")
    {
        return failure(Error(ErrorCode::InvalidInput));
    }
    let body = match to_bytes(request.into_body(), 16 * 1024).await {
        Ok(b) => b,
        Err(_) => return failure(Error(ErrorCode::InvalidInput)),
    };
    let input: Login = match serde_json::from_slice(&body) {
        Ok(i) => i,
        Err(_) => return failure(Error(ErrorCode::InvalidInput)),
    };
    // Malformed credentials are refused as core refuses them, uncharged.
    if !input.well_formed() {
        return failure(Error(ErrorCode::Unauthenticated));
    }
    let username = input.username.clone();
    match password_attempt(&app, Attempt::Login(&username), |attempt| {
        app.backend.login(input, attempt)
    })
    .await
    {
        Ok(issued) => {
            let value = format!(
                "__Host-limeos={}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=28800",
                issued.token
            );
            match HeaderValue::from_str(&value) {
                Ok(header) => ([("set-cookie", header)], axum::Json(issued.view)).into_response(),
                Err(_) => failure(Error(ErrorCode::Unavailable)),
            }
        }
        Err(e) => failure(e),
    }
}
async fn session<B: Backend>(State(app): State<App<B>>, headers: HeaderMap) -> Response {
    let token = match cookie(&headers) {
        Ok(t) => t,
        Err(e) => return failure(e),
    };
    match app.backend.session(token, None).await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => failure(e),
    }
}
async fn logout<B: Backend>(State(app): State<App<B>>, headers: HeaderMap) -> Response {
    if let Err(e) = origin(&app, &headers) {
        return failure(e);
    }
    let token = match cookie(&headers) {
        Ok(t) => t,
        Err(e) => return failure(e),
    };
    let Some(csrf) = headers
        .get("x-csrf-token")
        .and_then(|h| h.to_str().ok())
        .filter(|s| s.len() == 64)
    else {
        return failure(Error(ErrorCode::Forbidden));
    };
    match app.backend.logout(token, csrf.into()).await {
        Ok(()) => (
            StatusCode::NO_CONTENT,
            [(
                "set-cookie",
                "__Host-limeos=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0",
            )],
        )
            .into_response(),
        Err(e) => failure(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use limeos_contracts::VERSION;
    use tower::ServiceExt;
    #[derive(Clone)]
    struct Fake;
    impl TestBackend for Fake {
        async fn login(&self, _: Login, _: PasswordAttempt) -> PasswordChecked<IssuedSession> {
            PasswordChecked::rejected()
        }
        async fn session(&self, _: String, _: Option<String>) -> Result<SessionView> {
            Err(Error(ErrorCode::Unauthenticated))
        }
        async fn logout(&self, _: String, _: String) -> Result<()> {
            panic!("unvalidated mutation")
        }
        async fn health(&self) -> Result<Health> {
            Ok(Health {
                version: VERSION,
                generation: 1,
                schema_version: 1,
                ready: true,
                degraded: Vec::new(),
            })
        }
    }
    /// A core whose store is degraded, and whose session lookups would each
    /// put work on that store: health must not make any.
    #[derive(Clone, Default)]
    struct StoreStuck {
        lookups: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl TestBackend for StoreStuck {
        async fn login(&self, _: Login, _: PasswordAttempt) -> PasswordChecked<IssuedSession> {
            PasswordChecked::rejected()
        }
        async fn session(&self, _: String, _: Option<String>) -> Result<SessionView> {
            self.lookups
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(Error(ErrorCode::Unavailable))
        }
        async fn signed_in(&self, _: String) -> bool {
            self.lookups
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            true
        }
        async fn logout(&self, _: String, _: String) -> Result<()> {
            panic!("unvalidated mutation")
        }
        async fn health(&self) -> Result<Health> {
            Ok(Health {
                version: VERSION,
                generation: 1,
                schema_version: 1,
                ready: false,
                degraded: vec![limeos_contracts::DegradedSubsystem {
                    subsystem: limeos_contracts::Subsystem::Store,
                    detail: Some(limeos_contracts::DegradedDetail {
                        code: ErrorCode::Unavailable,
                        since: 1,
                        count: 64,
                    }),
                }],
            })
        }
    }
    #[tokio::test]
    async fn health_with_the_store_degraded_answers_without_looking_up_a_session() {
        let backend = StoreStuck::default();
        let app = router(backend.clone(), "https://localhost".into());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/health")
                    .header("cookie", format!("__Host-limeos={}", "m".repeat(64)))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let health: Health = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            health.degraded,
            vec![limeos_contracts::DegradedSubsystem {
                subsystem: limeos_contracts::Subsystem::Store,
                detail: None,
            }]
        );
        assert_eq!(backend.lookups.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn no_default_login_cross_site_or_get_mutation() {
        let app = router(Fake, "https://localhost".into());
        for (method, path, status) in [
            ("GET", "/api/v1/auth/logout", 405),
            ("POST", "/api/v1/auth/logout", 403),
            ("POST", "/api/v1/auth/login", 403),
            ("GET", "/api/v1/auth/session", 401),
        ] {
            let r = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(r.status().as_u16(), status);
        }
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/auth/login")
            .header("origin", "https://localhost")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"username":"admin","password":"pihealth"}"#))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("pihealth"));
    }
    /// Accepts exactly `admin` / `right`; every other attempt is a failure.
    /// Like core, names with no account get 120 comparisons, then none.
    #[derive(Clone, Default)]
    struct Accepting {
        unknown: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl TestBackend for Accepting {
        async fn login(&self, input: Login, _: PasswordAttempt) -> PasswordChecked<IssuedSession> {
            if input.username != "admin"
                && self
                    .unknown
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                    >= 120
            {
                return PasswordChecked::not_reached(Error(ErrorCode::Overloaded));
            }
            // Verified, then the session could not be stored.
            if input.username == "admin" && input.password == "right-but-down" {
                return PasswordChecked::new(
                    PasswordCheck::Accepted,
                    Err(Error(ErrorCode::Unavailable)),
                );
            }
            if input.username != "admin" || input.password != "right" {
                return PasswordChecked::rejected();
            }
            PasswordChecked::new(
                PasswordCheck::Accepted,
                Ok(IssuedSession {
                    token: "a".repeat(64),
                    view: SessionView {
                        principal: limeos_domain::Principal {
                            id: "p".repeat(64),
                            role: limeos_domain::Role::Administrator,
                            grant_revision: 1,
                        },
                        csrf_token: "c".repeat(64),
                        expires_at: 999,
                    },
                }),
            )
        }
        async fn session(&self, _: String, _: Option<String>) -> Result<SessionView> {
            Err(Error(ErrorCode::Unauthenticated))
        }
        async fn logout(&self, _: String, _: String) -> Result<()> {
            panic!("unvalidated mutation")
        }
        async fn health(&self) -> Result<Health> {
            Err(Error(ErrorCode::Unavailable))
        }
    }
    fn login_request(content_type: Option<&str>, body: String) -> Request {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/api/v1/auth/login")
            .header("origin", "https://localhost");
        if let Some(value) = content_type {
            builder = builder.header("content-type", value);
        }
        builder.body(Body::from(body)).unwrap()
    }
    fn attempt(username: &str, password: &str) -> Request {
        login_request(
            Some("application/json"),
            serde_json::json!({"username":username,"password":password}).to_string(),
        )
    }
    async fn status(app: &Router, request: Request) -> StatusCode {
        app.clone().oneshot(request).await.unwrap().status()
    }
    #[tokio::test]
    async fn junk_requests_do_not_spend_the_login_budget() {
        let app = router(Accepting::default(), "https://localhost".into());
        for _ in 0..12 {
            for (content_type, body) in [
                (Some("application/json"), "{bad".to_owned()),
                (Some("application/json"), "x".repeat(20 * 1024)),
                (
                    Some("application/json"),
                    r#"{"username":"admin"}"#.to_owned(),
                ),
                (
                    Some("text/plain"),
                    r#"{"username":"admin","password":"x"}"#.to_owned(),
                ),
                (None, String::new()),
            ] {
                assert_eq!(
                    status(&app, login_request(content_type, body)).await,
                    StatusCode::BAD_REQUEST
                );
            }
        }
        assert_eq!(
            status(&app, attempt("admin", "right")).await,
            StatusCode::OK
        );
    }
    #[tokio::test]
    async fn failed_passwords_are_limited_per_username_and_successes_are_free() {
        let app = router(Accepting::default(), "https://localhost".into());
        for _ in 0..10 {
            assert_eq!(
                status(&app, attempt("alice", "wrong")).await,
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(
            status(&app, attempt("alice", "wrong")).await,
            StatusCode::TOO_MANY_REQUESTS
        );
        // Another account, existing or not, keeps its own budget.
        assert_eq!(
            status(&app, attempt("nobody", "wrong")).await,
            StatusCode::UNAUTHORIZED
        );
        for _ in 0..15 {
            assert_eq!(
                status(&app, attempt("admin", "right")).await,
                StatusCode::OK
            );
        }
        for _ in 0..9 {
            assert_eq!(
                status(&app, attempt("admin", "wrong")).await,
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(
            status(&app, attempt("admin", "right")).await,
            StatusCode::OK
        );
        assert_eq!(
            status(&app, attempt("admin", "wrong")).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status(&app, attempt("admin", "right")).await,
            StatusCode::TOO_MANY_REQUESTS
        );
    }
    #[tokio::test]
    async fn malformed_credentials_are_refused_without_spending_the_budget() {
        let app = router(Accepting::default(), "https://localhost".into());
        for _ in 0..12 {
            for (username, password) in [
                ("admin", String::new()),
                ("admin", "x".repeat(1025)),
                ("bad name", "wrong".into()),
                ("", "wrong".into()),
                (&"a".repeat(65), "wrong".into()),
            ] {
                assert_eq!(
                    status(&app, attempt(username, &password)).await,
                    StatusCode::UNAUTHORIZED
                );
            }
        }
        for _ in 0..9 {
            assert_eq!(
                status(&app, attempt("admin", "wrong")).await,
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(
            status(&app, attempt("admin", "right")).await,
            StatusCode::OK
        );
    }
    #[tokio::test]
    async fn a_correct_password_is_free_even_when_the_login_then_fails() {
        let app = router(Accepting::default(), "https://localhost".into());
        for _ in 0..15 {
            assert_eq!(
                status(&app, attempt("admin", "right-but-down")).await,
                StatusCode::SERVICE_UNAVAILABLE
            );
        }
        assert_eq!(
            status(&app, attempt("admin", "right")).await,
            StatusCode::OK
        );
    }
    #[test]
    fn a_refund_from_an_earlier_window_cannot_erase_a_new_failure() {
        let mut limiter = Limiter::default();
        let alice = Attempt::Login("alice");
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        assert!(limiter.charge(alice, at(0)).is_some());
        // Charged at 59.9 s; its check finishes only after the window rolled.
        let late = limiter.charge(alice, at(59_900)).unwrap();
        assert!(limiter.charge(alice, at(60_000)).is_some());
        limiter.refund(late);
        // The new window holds its one failure: nine more, then refused.
        for _ in 0..9 {
            assert!(limiter.charge(alice, at(61_000)).is_some());
        }
        assert!(limiter.charge(alice, at(61_000)).is_none());
        // A refund inside its own window still applies.
        let ticket = limiter.charge(alice, at(130_000)).unwrap();
        limiter.refund(ticket);
        for _ in 0..10 {
            assert!(limiter.charge(alice, at(130_001)).is_some());
        }
        assert!(limiter.charge(alice, at(130_001)).is_none());
    }
    fn app() -> App<Accepting> {
        App {
            backend: Accepting::default(),
            origin: "https://localhost".into(),
            limiter: Arc::new(Mutex::new(Limiter::default())),
            readers: Arc::new(Semaphore::new(8)),
            streams: Default::default(),
        }
    }
    fn live(app: &App<Accepting>) -> usize {
        app.limiter.lock().unwrap().budgets.len()
    }
    fn count(app: &App<Accepting>, name: &str) -> Option<u32> {
        let key = AttemptKey::Login(name.into());
        app.limiter.lock().unwrap().budgets.get(&key).map(|e| e.1)
    }
    #[test]
    fn requests_cancelled_before_a_comparison_leave_no_entry() {
        use futures_util::FutureExt;
        let app = app();
        // Rotating names, each abandoned during the account lookup, cannot
        // fill the table.
        for i in 0..5000 {
            let name = format!("user{i}");
            let pending = password_attempt(&app, Attempt::Login(&name), |_| {
                std::future::pending::<PasswordChecked<()>>()
            });
            assert!(pending.now_or_never().is_none());
        }
        assert_eq!(live(&app), 0);
        assert!(
            app.limiter
                .lock()
                .unwrap()
                .charge(Attempt::Login("admin"), Instant::now())
                .is_some()
        );
    }
    #[test]
    fn a_request_cancelled_during_its_comparison_keeps_the_charge() {
        use futures_util::FutureExt;
        let app = app();
        for _ in 0..10 {
            let pending = password_attempt(&app, Attempt::Login("alice"), |attempt| async move {
                attempt.start_comparison();
                std::future::pending::<PasswordChecked<()>>().await
            });
            assert!(pending.now_or_never().is_none());
        }
        assert_eq!(count(&app, "alice"), Some(10));
        let refused = password_attempt(&app, Attempt::Login("alice"), |_| async {
            PasswordChecked::<()>::rejected()
        });
        assert_eq!(
            refused.now_or_never().unwrap().err(),
            Some(Error(ErrorCode::Overloaded))
        );
    }
    #[test]
    fn each_charge_is_settled_exactly_once() {
        use futures_util::FutureExt;
        let app = app();
        let complete = |check: PasswordCheck, result: Result<()>| {
            password_attempt(&app, Attempt::Login("alice"), move |_| async move {
                PasswordChecked::new(check, result)
            })
            .now_or_never()
            .unwrap()
        };
        // Two kept charges, then one refunded completion: the count does not
        // move, and the completed charge's drop refunds nothing more.
        complete(
            PasswordCheck::Rejected,
            Err(Error(ErrorCode::Unauthenticated)),
        )
        .unwrap_err();
        complete(
            PasswordCheck::Rejected,
            Err(Error(ErrorCode::Unauthenticated)),
        )
        .unwrap_err();
        assert_eq!(count(&app, "alice"), Some(2));
        complete(PasswordCheck::Accepted, Ok(())).unwrap();
        complete(PasswordCheck::NotReached, Err(Error(ErrorCode::Overloaded))).unwrap_err();
        assert_eq!(count(&app, "alice"), Some(2));
        // A kept charge whose comparison never started is not refunded on drop.
        let ticket = app
            .limiter
            .lock()
            .unwrap()
            .charge(Attempt::Login("alice"), Instant::now());
        Charge {
            limiter: &app.limiter,
            ticket,
            attempt: PasswordAttempt::default(),
        }
        .settle(false);
        assert_eq!(count(&app, "alice"), Some(3));
    }
    #[test]
    fn two_names_never_share_a_budget() {
        let mut limiter = Limiter::default();
        let now = Instant::now();
        for _ in 0..10 {
            assert!(limiter.charge(Attempt::Login("alice"), now).is_some());
        }
        assert!(limiter.charge(Attempt::Login("alice"), now).is_none());
        // Every other name, and the same text as a principal, is untouched.
        for i in 0..1000 {
            let name = format!("user{i}");
            assert!(limiter.charge(Attempt::Login(&name), now).is_some());
        }
        assert!(limiter.charge(Attempt::Principal("alice"), now).is_some());
        assert!(limiter.charge(Attempt::Login("alice"), now).is_none());
    }
    #[test]
    fn a_full_map_refuses_an_unseen_name_and_recovers_when_windows_end() {
        let mut limiter = Limiter::default();
        let start = Instant::now();
        for i in 0..ATTEMPT_CAPACITY {
            let name = format!("user{i}");
            assert!(limiter.charge(Attempt::Login(&name), start).is_some());
        }
        assert_eq!(limiter.budgets.len(), ATTEMPT_CAPACITY);
        // Fail closed for a name not yet in the map; known names keep going.
        assert!(limiter.charge(Attempt::Login("late"), start).is_none());
        assert!(limiter.charge(Attempt::Login("user0"), start).is_some());
        assert_eq!(limiter.budgets.len(), ATTEMPT_CAPACITY);
        // Windows end and their entries go.
        let later = start + ATTEMPT_WINDOW;
        assert!(limiter.charge(Attempt::Login("late"), later).is_some());
        assert_eq!(limiter.budgets.len(), 1);
        // An entry refunded to zero is removed at once.
        let ticket = limiter.charge(Attempt::Login("brief"), later).unwrap();
        limiter.refund(ticket);
        assert_eq!(limiter.budgets.len(), 1);
    }
    /// Holds every check open until released, counting those that arrive.
    #[derive(Clone)]
    struct Held {
        arrived: Arc<std::sync::atomic::AtomicUsize>,
        release: Arc<tokio::sync::Notify>,
    }
    impl TestBackend for Held {
        async fn login(&self, _: Login, _: PasswordAttempt) -> PasswordChecked<IssuedSession> {
            self.arrived
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.release.notified().await;
            PasswordChecked::rejected()
        }
        async fn session(&self, _: String, _: Option<String>) -> Result<SessionView> {
            Err(Error(ErrorCode::Unauthenticated))
        }
        async fn logout(&self, _: String, _: String) -> Result<()> {
            panic!("unvalidated mutation")
        }
        async fn health(&self) -> Result<Health> {
            Err(Error(ErrorCode::Unavailable))
        }
    }
    #[tokio::test]
    async fn concurrent_attempts_stay_within_the_per_name_budget() {
        let held = Held {
            arrived: Arc::default(),
            release: Arc::default(),
        };
        let app = router(held.clone(), "https://localhost".into());
        let tasks: Vec<_> = (0..30)
            .map(|_| tokio::spawn(app.clone().oneshot(attempt("alice", "wrong"))))
            .collect();
        // Every attempt is either inside a check or already refused.
        while held.arrived.load(std::sync::atomic::Ordering::SeqCst) < 10 {
            tokio::task::yield_now().await;
        }
        held.release.notify_waiters();
        let mut statuses = Vec::new();
        for task in tasks {
            statuses.push(task.await.unwrap().unwrap().status());
        }
        let count = |s| statuses.iter().filter(|x| **x == s).count();
        assert_eq!(count(StatusCode::UNAUTHORIZED), 10);
        assert_eq!(count(StatusCode::TOO_MANY_REQUESTS), 20);
        assert_eq!(held.arrived.load(std::sync::atomic::Ordering::SeqCst), 10);
    }
    #[tokio::test]
    async fn failures_across_other_usernames_never_lock_out_an_untouched_one() {
        let app = router(Accepting::default(), "https://localhost".into());
        // A flood of rotating names with no account: 120 compared, the rest
        // refused by core without a comparison, which the limiter refunds.
        for i in 0..3000 {
            let expected = if i < 120 {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::TOO_MANY_REQUESTS
            };
            assert_eq!(
                status(&app, attempt(&format!("user{i}"), "wrong")).await,
                expected
            );
        }
        assert_eq!(
            status(&app, attempt("admin", "wrong")).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status(&app, attempt("admin", "right")).await,
            StatusCode::OK
        );
    }
}
