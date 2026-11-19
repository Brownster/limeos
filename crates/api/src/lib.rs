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
    hash::{BuildHasher, RandomState},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{net::TcpListener, sync::Semaphore};
mod automation_history;
mod automation_policy;
mod automation_schedules;
mod compose;
mod identity_enablement;
mod job_resolution;
#[cfg(test)]
mod operation_tests;
mod operations;
#[cfg(test)]
mod read_tests;
mod reads;
#[cfg(test)]
mod stack_operation_tests;
mod stack_operations;
mod stacks;
mod storage;
mod storage_mounts;

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
pub trait Backend: Clone + Send + Sync + 'static {
    fn observe_health_policy(
        &self,
        _token: String,
        _deadline: Instant,
    ) -> impl Future<Output = Result<limeos_domain::automation::HealthPolicyView>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn replace_observe_health_policy(
        &self,
        _token: String,
        _csrf: String,
        _input: limeos_domain::automation::ReplaceHealthPolicy,
        _deadline: Instant,
    ) -> impl Future<Output = Result<limeos_domain::automation::HealthPolicyView>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn observe_health_alerts(
        &self,
        _token: String,
        _after: Option<String>,
        _deadline: Instant,
    ) -> impl Future<
        Output = Result<
            limeos_domain::automation::AutomationPage<limeos_domain::automation::AlertState>,
        >,
    > + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn observe_health_notifications(
        &self,
        _token: String,
        _after: Option<String>,
        _deadline: Instant,
    ) -> impl Future<
        Output = Result<
            limeos_domain::automation::AutomationPage<limeos_domain::automation::Notification>,
        >,
    > + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn acknowledge_observe_health_notification(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
        _input: limeos_domain::automation::AcknowledgeNotification,
        _deadline: Instant,
    ) -> impl Future<Output = Result<limeos_domain::automation::Notification>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    /// Current stored session/grants and the shared stack cache only.
    fn stack_inventory(
        &self,
        _token: String,
    ) -> impl Future<Output = Result<limeos_domain::deployment::discovery::StackInventory>> + Send
    {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn observe_health_schedules(
        &self,
        _token: String,
        _after: Option<String>,
        _deadline: Instant,
    ) -> impl Future<
        Output = Result<
            limeos_domain::automation::AutomationPage<limeos_domain::automation::ScheduleView>,
        >,
    > + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn create_observe_health_schedule(
        &self,
        _token: String,
        _csrf: String,
        _input: limeos_contracts::ObserveHealthScheduleInput,
        _deadline: Instant,
    ) -> impl Future<Output = Result<limeos_domain::automation::ScheduleView>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn pause_observe_health_schedule(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
        _input: limeos_domain::automation::PauseSchedule,
        _deadline: Instant,
    ) -> impl Future<Output = Result<limeos_domain::automation::ScheduleView>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn enable_observe_health(
        &self,
        _token: String,
        _csrf: String,
        _input: limeos_contracts::ObserveHealthEnable,
        _deadline: Instant,
    ) -> impl Future<Output = PasswordChecked<limeos_contracts::ObserveHealthEnabled>> + Send {
        async { PasswordChecked::not_reached(Error(ErrorCode::Unavailable)) }
    }
    fn storage_mount_catalog(
        &self,
        _token: String,
    ) -> impl Future<Output = Result<limeos_domain::StorageMountCatalog>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn plan_storage_mount(
        &self,
        _token: String,
        _csrf: String,
        _selection: limeos_domain::StorageMountSelection,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageMount>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn storage_mount_plan(
        &self,
        _token: String,
        _id: String,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageMount>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn approve_storage_mount(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
        _digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn queue_storage_mount(
        &self,
        _token: String,
        _csrf: String,
        _input: limeos_contracts::QueueStorageMountInput,
    ) -> impl Future<Output = Result<limeos_domain::StorageMountJob>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn storage_mount_job(
        &self,
        _token: String,
        _id: String,
    ) -> impl Future<Output = Result<limeos_domain::StorageMountJob>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn storage_mount_jobs(
        &self,
        _token: String,
    ) -> impl Future<Output = Result<Vec<limeos_domain::StorageMountJob>>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn cancel_storage_mount(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
    ) -> impl Future<Output = Result<()>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn cancel_storage_mount_job(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
    ) -> impl Future<Output = Result<()>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }

    fn container_storage_inventory(
        &self,
        _token: String,
    ) -> impl Future<Output = Result<limeos_domain::ContainerStorageInventoryView>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn plan_storage_targets(
        &self,
        _token: String,
        _csrf: String,
        _contract: limeos_domain::StorageContract,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageTargets>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn storage_target_plan(
        &self,
        _token: String,
        _id: String,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageTargets>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn approve_storage_targets(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
        _digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn queue_storage_targets(
        &self,
        _token: String,
        _csrf: String,
        _input: limeos_contracts::QueueStorageTargetsInput,
    ) -> impl Future<Output = Result<limeos_domain::StorageTargetJob>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn storage_target_job(
        &self,
        _token: String,
        _id: String,
    ) -> impl Future<Output = Result<limeos_domain::StorageTargetJob>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn cancel_storage_targets(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
    ) -> impl Future<Output = Result<()>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn cancel_storage_target_job(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
    ) -> impl Future<Output = Result<()>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn storage_inventory(
        &self,
        _token: String,
    ) -> impl Future<Output = Result<limeos_domain::StorageInventoryView>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn plan_storage(
        &self,
        _token: String,
        _csrf: String,
        _input: limeos_domain::StorageSetupInput,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageSetup>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn storage_plan(
        &self,
        _token: String,
        _id: String,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageSetup>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn approve_storage(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
        _digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn cancel_storage(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
    ) -> impl Future<Output = Result<()>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn plan_compose(
        &self,
        _token: String,
        _csrf: String,
        _selection: limeos_domain::ComposeSelection,
    ) -> impl Future<Output = Result<limeos_domain::PlannedCompose>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn compose_plan(
        &self,
        _token: String,
        _id: String,
    ) -> impl Future<Output = Result<limeos_domain::PlannedCompose>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn approve_compose(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
        _digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn cancel_compose(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
    ) -> impl Future<Output = Result<()>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn login(&self, request: Login) -> impl Future<Output = PasswordChecked<IssuedSession>> + Send;
    fn session(
        &self,
        token: String,
        csrf: Option<String>,
    ) -> impl Future<Output = Result<SessionView>> + Send;
    fn logout(&self, token: String, csrf: String) -> impl Future<Output = Result<()>> + Send;
    fn health(&self) -> impl Future<Output = Result<Health>> + Send;
    fn observations(
        &self,
        _token: String,
    ) -> impl Future<Output = Result<limeos_contracts::Overview>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn history(
        &self,
        _token: String,
        _range: limeos_contracts::HistoryRange,
    ) -> impl Future<Output = Result<limeos_contracts::MetricHistory>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        tokio::sync::watch::channel(0).1
    }
    fn plan_restart(
        &self,
        _token: String,
        _csrf: String,
        _input: limeos_contracts::RestartInput,
    ) -> impl Future<Output = Result<limeos_domain::PlannedRestart>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn plan_container(
        &self,
        _token: String,
        _csrf: String,
        _input: limeos_contracts::ContainerInput,
    ) -> impl Future<Output = Result<limeos_domain::PlannedContainerAction>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn container_logs(
        &self,
        _token: String,
        _resource: String,
        _options: limeos_domain::LogOptions,
    ) -> impl Future<Output = Result<limeos_domain::ContainerLogs>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn approve_container(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
        _digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn queue_container(
        &self,
        _token: String,
        _csrf: String,
        _input: limeos_contracts::QueueRestartInput,
    ) -> impl Future<Output = Result<limeos_domain::RestartJob>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn restart_progress(
        &self,
        _token: String,
        _id: String,
        _after: i64,
    ) -> impl Future<Output = Result<limeos_contracts::JobProgress>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn container_jobs(
        &self,
        _token: String,
    ) -> impl Future<Output = Result<Vec<limeos_domain::RestartJob>>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn cancel_restart(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
        _plan: bool,
    ) -> impl Future<Output = Result<()>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn plan_stack(
        &self,
        _token: String,
        _csrf: String,
        _input: limeos_contracts::StackInput,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStackAction>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn approve_stack(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
        _digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn queue_stack(
        &self,
        _token: String,
        _csrf: String,
        _input: limeos_contracts::QueueStackInput,
    ) -> impl Future<Output = Result<limeos_domain::StackJob>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn stack_job_progress(
        &self,
        _token: String,
        _id: String,
        _after: i64,
    ) -> impl Future<Output = Result<limeos_contracts::StackJobProgress>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn stack_jobs(
        &self,
        _token: String,
    ) -> impl Future<Output = Result<Vec<limeos_domain::StackJob>>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn cancel_stack_action(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
        _plan: bool,
    ) -> impl Future<Output = Result<()>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    /// Administrator session only. Fresh read-only inspection of the job's
    /// targets; never an Engine effect.
    fn job_resolution(
        &self,
        _token: String,
        _id: String,
    ) -> impl Future<Output = Result<limeos_contracts::ResolutionView>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    /// Administrator session only. Read-only: every principal's container and
    /// stack jobs that hold locks until resolved, at most 64.
    fn needs_attention_jobs(
        &self,
        _token: String,
    ) -> impl Future<Output = Result<Vec<limeos_domain::NeedsAttentionJob>>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    /// Administrator session, CSRF and the administrator's password.
    fn resolve_job(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
        _input: limeos_contracts::ResolveJobInput,
    ) -> impl Future<Output = PasswordChecked<limeos_contracts::ResolveJobResponse>> + Send {
        async { PasswordChecked::not_reached(Error(ErrorCode::Unavailable)) }
    }
}
#[derive(Clone)]
struct App<B> {
    backend: B,
    origin: Arc<str>,
    limiter: Arc<Mutex<Limiter>>,
    readers: Arc<Semaphore>,
}
/// Whose password is being tried. Every peer is the local proxy, so the
/// address says nothing; forwarded headers are never trusted either.
#[derive(Clone, Copy, Hash)]
enum Attempt<'a> {
    /// The username a login names, whether or not such an account exists.
    Login(&'a str),
    /// The principal of an authenticated session re-entering its password.
    Principal(&'a str),
}
/// Failed password attempts per minute: ten per attempt key and 120 in all.
/// Keys are hashed with a per-process random key, so entries are fixed-size
/// whatever was submitted; the global budget bounds how many can exist.
#[derive(Default)]
struct Limiter {
    keys: RandomState,
    attempts: HashMap<u64, (Instant, u32)>,
    global: Option<(Instant, u32)>,
}
impl Limiter {
    /// Charge one attempt before the password work; false when either budget
    /// is spent. Charging first keeps concurrent attempts inside the budget.
    fn charge(&mut self, key: u64) -> bool {
        let now = Instant::now();
        let window = Duration::from_secs(60);
        self.attempts
            .retain(|_, (at, _)| now.duration_since(*at) < window);
        let global = self.global.get_or_insert((now, 0));
        if now.duration_since(global.0) >= window {
            *global = (now, 0);
        }
        if global.1 >= 120 || (!self.attempts.contains_key(&key) && self.attempts.len() >= 1024) {
            return false;
        }
        let entry = self.attempts.entry(key).or_insert((now, 0));
        if entry.1 >= 10 {
            return false;
        }
        entry.1 += 1;
        global.1 += 1;
        true
    }
    /// A correct password costs nothing.
    fn refund(&mut self, key: u64) {
        if let Some(entry) = self.attempts.get_mut(&key) {
            entry.1 = entry.1.saturating_sub(1);
        }
        if let Some(global) = &mut self.global {
            global.1 = global.1.saturating_sub(1);
        }
    }
}
/// Run one password check under the failed-attempt budget. Callers validate
/// origin, credentials headers and the body first, so junk is never charged.
///
/// The charge is refunded when no comparison happened, or when the password
/// was correct and the response already shows that (anything but 401). An
/// accepted password whose call still answers 401 keeps its charge: a refund
/// there would let the budget tell it apart from a wrong one.
async fn password_attempt<B, T>(
    app: &App<B>,
    attempt: Attempt<'_>,
    check: impl Future<Output = PasswordChecked<T>>,
) -> Result<T> {
    let key = app.limiter.lock().ok().and_then(|mut limiter| {
        let key = limiter.keys.hash_one(attempt);
        limiter.charge(key).then_some(key)
    });
    let Some(key) = key else {
        return Err(Error(ErrorCode::Overloaded));
    };
    let PasswordChecked { check, result } = check.await;
    let refund = match check {
        PasswordCheck::NotReached => true,
        PasswordCheck::Rejected => false,
        PasswordCheck::Accepted => !matches!(result, Err(Error(ErrorCode::Unauthenticated))),
    };
    if refund {
        if let Ok(mut limiter) = app.limiter.lock() {
            limiter.refund(key);
        }
    }
    result
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
async fn limits(request: Request, next: Next) -> Response {
    let header_bytes: usize = request
        .headers()
        .iter()
        .map(|(k, v)| k.as_str().len() + v.len())
        .sum();
    if header_bytes > 8192 {
        return failure(Error(ErrorCode::InvalidInput));
    }
    let mut response = match tokio::time::timeout(Duration::from_secs(10), next.run(request)).await
    {
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
async fn health<B: Backend>(State(app): State<App<B>>) -> Response {
    match app.backend.health().await {
        Ok(h) => axum::Json(h).into_response(),
        Err(e) => failure(e),
    }
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
    match password_attempt(&app, Attempt::Login(&username), app.backend.login(input)).await {
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

/// All connections have bounded parsing, occupancy, and lifetime. Hyper's timer
/// covers incomplete headers; the application deadline covers bodies and work.
pub async fn serve(listener: TcpListener, router: Router) -> std::io::Result<()> {
    let permits = Arc::new(Semaphore::new(64));
    loop {
        let (stream, _) = listener.accept().await?;
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            continue;
        };
        let app = router.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let io = hyper_util::rt::TokioIo::new(stream);
            let service = hyper_util::service::TowerToHyperService::new(app);
            let mut builder = hyper::server::conn::http1::Builder::new();
            builder
                .timer(hyper_util::rt::TokioTimer::new())
                .header_read_timeout(Duration::from_secs(5))
                .max_headers(32)
                .max_buf_size(16 * 1024)
                .keep_alive(true);
            let _ = tokio::time::timeout(
                Duration::from_secs(30),
                builder.serve_connection(io, service),
            )
            .await;
        });
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
    impl Backend for Fake {
        async fn login(&self, _: Login) -> PasswordChecked<IssuedSession> {
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
            })
        }
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
    #[derive(Clone)]
    struct Accepting;
    impl Backend for Accepting {
        async fn login(&self, input: Login) -> PasswordChecked<IssuedSession> {
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
        let app = router(Accepting, "https://localhost".into());
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
        let app = router(Accepting, "https://localhost".into());
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
        let app = router(Accepting, "https://localhost".into());
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
        let app = router(Accepting, "https://localhost".into());
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
    fn cycling_usernames_cannot_grow_the_attempt_map() {
        let mut limiter = Limiter::default();
        let mut charged = 0;
        for i in 0..5000 {
            let key = limiter.keys.hash_one(Attempt::Login(&format!("user{i}")));
            charged += usize::from(limiter.charge(key));
        }
        assert_eq!(charged, 120);
        assert!(limiter.attempts.len() <= 120);
        // A username and a principal with the same text are different keys.
        assert_ne!(
            limiter.keys.hash_one(Attempt::Login("p")),
            limiter.keys.hash_one(Attempt::Principal("p"))
        );
    }
    #[tokio::test]
    async fn a_global_backstop_bounds_failures_across_usernames() {
        let app = router(Accepting, "https://localhost".into());
        for i in 0..120 {
            assert_eq!(
                status(&app, attempt(&format!("user{i}"), "wrong")).await,
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(
            status(&app, attempt("user-next", "wrong")).await,
            StatusCode::TOO_MANY_REQUESTS
        );
    }
}
