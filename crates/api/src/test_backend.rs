//! A test-only `Backend` with a default for every method, so each fake
//! overrides only what its test drives. `Backend` itself has no defaults: a
//! route core does not implement fails to compile instead of answering 503.
use super::*;

/// A live session as core reports it, for fakes whose credentials are all
/// valid: the gate asks for it only before releasing an input or load
/// refusal (`gate::authenticated_refusal`), never for authority, which
/// stays in the call that does the work.
pub(crate) fn live_session() -> Result<SessionView> {
    Ok(SessionView {
        principal: limeos_domain::Principal {
            id: "fixture".into(),
            role: limeos_domain::Role::Viewer,
            grant_revision: 1,
        },
        csrf_token: "c".repeat(64),
        expires_at: i64::MAX,
    })
}

pub(crate) trait TestBackend: Clone + Send + Sync + 'static {
    /// Where every method a fake does not override lands, with its name and
    /// the credentials the route handed it. Unavailable by default.
    fn fallback(&self, _method: &'static str, _token: Option<&str>, _csrf: Option<&str>) -> Error {
        Error(ErrorCode::Unavailable)
    }
    fn observe_health_policy(
        &self,
        token: String,
        _deadline: Instant,
    ) -> impl Future<Output = Result<limeos_domain::automation::HealthPolicyView>> + Send {
        let error = self.fallback("observe_health_policy", Some(&token), None);
        async move { Err(error) }
    }
    fn replace_observe_health_policy(
        &self,
        token: String,
        csrf: String,
        _input: limeos_domain::automation::ReplaceHealthPolicy,
        _deadline: Instant,
    ) -> impl Future<Output = Result<limeos_domain::automation::HealthPolicyView>> + Send {
        let error = self.fallback("replace_observe_health_policy", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn observe_health_alerts(
        &self,
        token: String,
        _after: Option<String>,
        _deadline: Instant,
    ) -> impl Future<
        Output = Result<
            limeos_domain::automation::AutomationPage<limeos_domain::automation::AlertState>,
        >,
    > + Send {
        let error = self.fallback("observe_health_alerts", Some(&token), None);
        async move { Err(error) }
    }
    fn observe_health_notifications(
        &self,
        token: String,
        _after: Option<String>,
        _deadline: Instant,
    ) -> impl Future<
        Output = Result<
            limeos_domain::automation::AutomationPage<limeos_domain::automation::Notification>,
        >,
    > + Send {
        let error = self.fallback("observe_health_notifications", Some(&token), None);
        async move { Err(error) }
    }
    fn acknowledge_observe_health_notification(
        &self,
        token: String,
        csrf: String,
        _id: String,
        _input: limeos_domain::automation::AcknowledgeNotification,
        _deadline: Instant,
    ) -> impl Future<Output = Result<limeos_domain::automation::Notification>> + Send {
        let error = self.fallback(
            "acknowledge_observe_health_notification",
            Some(&token),
            Some(&csrf),
        );
        async move { Err(error) }
    }
    fn stack_inventory(
        &self,
        token: String,
    ) -> impl Future<Output = Result<limeos_domain::deployment::discovery::StackInventory>> + Send
    {
        let error = self.fallback("stack_inventory", Some(&token), None);
        async move { Err(error) }
    }
    fn observe_health_schedules(
        &self,
        token: String,
        _after: Option<String>,
        _deadline: Instant,
    ) -> impl Future<
        Output = Result<
            limeos_domain::automation::AutomationPage<limeos_domain::automation::ScheduleView>,
        >,
    > + Send {
        let error = self.fallback("observe_health_schedules", Some(&token), None);
        async move { Err(error) }
    }
    fn create_observe_health_schedule(
        &self,
        token: String,
        csrf: String,
        _input: limeos_contracts::ObserveHealthScheduleInput,
        _deadline: Instant,
    ) -> impl Future<Output = Result<limeos_domain::automation::ScheduleView>> + Send {
        let error = self.fallback("create_observe_health_schedule", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn pause_observe_health_schedule(
        &self,
        token: String,
        csrf: String,
        _id: String,
        _input: limeos_domain::automation::PauseSchedule,
        _deadline: Instant,
    ) -> impl Future<Output = Result<limeos_domain::automation::ScheduleView>> + Send {
        let error = self.fallback("pause_observe_health_schedule", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn enable_observe_health(
        &self,
        token: String,
        csrf: String,
        _input: limeos_contracts::ObserveHealthEnable,
        _deadline: Instant,
        _attempt: PasswordAttempt,
    ) -> impl Future<Output = PasswordChecked<limeos_contracts::ObserveHealthEnabled>> + Send {
        let error = self.fallback("enable_observe_health", Some(&token), Some(&csrf));
        async move { PasswordChecked::not_reached(error) }
    }
    fn storage_mount_catalog(
        &self,
        token: String,
    ) -> impl Future<Output = Result<limeos_domain::StorageMountCatalog>> + Send {
        let error = self.fallback("storage_mount_catalog", Some(&token), None);
        async move { Err(error) }
    }
    fn plan_storage_mount(
        &self,
        token: String,
        csrf: String,
        _selection: limeos_domain::StorageMountSelection,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageMount>> + Send {
        let error = self.fallback("plan_storage_mount", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn storage_mount_plan(
        &self,
        token: String,
        _id: String,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageMount>> + Send {
        let error = self.fallback("storage_mount_plan", Some(&token), None);
        async move { Err(error) }
    }
    fn approve_storage_mount(
        &self,
        token: String,
        csrf: String,
        _id: String,
        _digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send {
        let error = self.fallback("approve_storage_mount", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn queue_storage_mount(
        &self,
        token: String,
        csrf: String,
        _input: limeos_contracts::QueueStorageMountInput,
    ) -> impl Future<Output = Result<limeos_domain::StorageMountJob>> + Send {
        let error = self.fallback("queue_storage_mount", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn storage_mount_job(
        &self,
        token: String,
        _id: String,
    ) -> impl Future<Output = Result<limeos_domain::StorageMountJob>> + Send {
        let error = self.fallback("storage_mount_job", Some(&token), None);
        async move { Err(error) }
    }
    fn storage_mount_jobs(
        &self,
        token: String,
    ) -> impl Future<Output = Result<Vec<limeos_domain::StorageMountJob>>> + Send {
        let error = self.fallback("storage_mount_jobs", Some(&token), None);
        async move { Err(error) }
    }
    fn cancel_storage_mount(
        &self,
        token: String,
        csrf: String,
        _id: String,
    ) -> impl Future<Output = Result<()>> + Send {
        let error = self.fallback("cancel_storage_mount", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn cancel_storage_mount_job(
        &self,
        token: String,
        csrf: String,
        _id: String,
    ) -> impl Future<Output = Result<()>> + Send {
        let error = self.fallback("cancel_storage_mount_job", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn container_storage_inventory(
        &self,
        token: String,
    ) -> impl Future<Output = Result<limeos_domain::ContainerStorageInventoryView>> + Send {
        let error = self.fallback("container_storage_inventory", Some(&token), None);
        async move { Err(error) }
    }
    fn plan_storage_targets(
        &self,
        token: String,
        csrf: String,
        _contract: limeos_domain::StorageContract,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageTargets>> + Send {
        let error = self.fallback("plan_storage_targets", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn storage_target_plan(
        &self,
        token: String,
        _id: String,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageTargets>> + Send {
        let error = self.fallback("storage_target_plan", Some(&token), None);
        async move { Err(error) }
    }
    fn approve_storage_targets(
        &self,
        token: String,
        csrf: String,
        _id: String,
        _digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send {
        let error = self.fallback("approve_storage_targets", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn queue_storage_targets(
        &self,
        token: String,
        csrf: String,
        _input: limeos_contracts::QueueStorageTargetsInput,
    ) -> impl Future<Output = Result<limeos_domain::StorageTargetJob>> + Send {
        let error = self.fallback("queue_storage_targets", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn storage_target_job(
        &self,
        token: String,
        _id: String,
    ) -> impl Future<Output = Result<limeos_domain::StorageTargetJob>> + Send {
        let error = self.fallback("storage_target_job", Some(&token), None);
        async move { Err(error) }
    }
    fn cancel_storage_targets(
        &self,
        token: String,
        csrf: String,
        _id: String,
    ) -> impl Future<Output = Result<()>> + Send {
        let error = self.fallback("cancel_storage_targets", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn cancel_storage_target_job(
        &self,
        token: String,
        csrf: String,
        _id: String,
    ) -> impl Future<Output = Result<()>> + Send {
        let error = self.fallback("cancel_storage_target_job", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn storage_inventory(
        &self,
        token: String,
    ) -> impl Future<Output = Result<limeos_domain::StorageInventoryView>> + Send {
        let error = self.fallback("storage_inventory", Some(&token), None);
        async move { Err(error) }
    }
    fn plan_storage(
        &self,
        token: String,
        csrf: String,
        _input: limeos_domain::StorageSetupInput,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageSetup>> + Send {
        let error = self.fallback("plan_storage", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn storage_plan(
        &self,
        token: String,
        _id: String,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageSetup>> + Send {
        let error = self.fallback("storage_plan", Some(&token), None);
        async move { Err(error) }
    }
    fn approve_storage(
        &self,
        token: String,
        csrf: String,
        _id: String,
        _digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send {
        let error = self.fallback("approve_storage", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn cancel_storage(
        &self,
        token: String,
        csrf: String,
        _id: String,
    ) -> impl Future<Output = Result<()>> + Send {
        let error = self.fallback("cancel_storage", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn plan_compose(
        &self,
        token: String,
        csrf: String,
        _selection: limeos_domain::ComposeSelection,
    ) -> impl Future<Output = Result<limeos_domain::PlannedCompose>> + Send {
        let error = self.fallback("plan_compose", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn compose_plan(
        &self,
        token: String,
        _id: String,
    ) -> impl Future<Output = Result<limeos_domain::PlannedCompose>> + Send {
        let error = self.fallback("compose_plan", Some(&token), None);
        async move { Err(error) }
    }
    fn approve_compose(
        &self,
        token: String,
        csrf: String,
        _id: String,
        _digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send {
        let error = self.fallback("approve_compose", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn cancel_compose(
        &self,
        token: String,
        csrf: String,
        _id: String,
    ) -> impl Future<Output = Result<()>> + Send {
        let error = self.fallback("cancel_compose", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
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
    fn signed_in(&self, token: String) -> impl Future<Output = bool> + Send {
        async move { self.session(token, None).await.is_ok() }
    }
    fn observations(
        &self,
        token: String,
    ) -> impl Future<Output = Result<limeos_contracts::Overview>> + Send {
        let error = self.fallback("observations", Some(&token), None);
        async move { Err(error) }
    }
    fn history(
        &self,
        token: String,
        _range: limeos_contracts::HistoryRange,
    ) -> impl Future<Output = Result<limeos_contracts::MetricHistory>> + Send {
        let error = self.fallback("history", Some(&token), None);
        async move { Err(error) }
    }
    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        tokio::sync::watch::channel(0).1
    }
    fn plan_restart(
        &self,
        token: String,
        csrf: String,
        _input: limeos_contracts::RestartInput,
    ) -> impl Future<Output = Result<limeos_domain::PlannedRestart>> + Send {
        let error = self.fallback("plan_restart", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn plan_container(
        &self,
        token: String,
        csrf: String,
        _input: limeos_contracts::ContainerInput,
    ) -> impl Future<Output = Result<limeos_domain::PlannedContainerAction>> + Send {
        let error = self.fallback("plan_container", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn container_logs(
        &self,
        token: String,
        _resource: String,
        _options: limeos_domain::LogOptions,
    ) -> impl Future<Output = Result<limeos_domain::ContainerLogs>> + Send {
        let error = self.fallback("container_logs", Some(&token), None);
        async move { Err(error) }
    }
    fn approve_container(
        &self,
        token: String,
        csrf: String,
        _id: String,
        _digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send {
        let error = self.fallback("approve_container", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn queue_container(
        &self,
        token: String,
        csrf: String,
        _input: limeos_contracts::QueueRestartInput,
    ) -> impl Future<Output = Result<limeos_domain::RestartJob>> + Send {
        let error = self.fallback("queue_container", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn restart_progress(
        &self,
        token: String,
        _id: String,
        _after: i64,
    ) -> impl Future<Output = Result<limeos_contracts::JobProgress>> + Send {
        let error = self.fallback("restart_progress", Some(&token), None);
        async move { Err(error) }
    }
    fn container_jobs(
        &self,
        token: String,
    ) -> impl Future<Output = Result<Vec<limeos_domain::RestartJob>>> + Send {
        let error = self.fallback("container_jobs", Some(&token), None);
        async move { Err(error) }
    }
    fn cancel_restart(
        &self,
        token: String,
        csrf: String,
        _id: String,
        _plan: bool,
    ) -> impl Future<Output = Result<()>> + Send {
        let error = self.fallback("cancel_restart", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn plan_stack(
        &self,
        token: String,
        csrf: String,
        _input: limeos_contracts::StackInput,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStackAction>> + Send {
        let error = self.fallback("plan_stack", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn approve_stack(
        &self,
        token: String,
        csrf: String,
        _id: String,
        _digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send {
        let error = self.fallback("approve_stack", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn queue_stack(
        &self,
        token: String,
        csrf: String,
        _input: limeos_contracts::QueueStackInput,
    ) -> impl Future<Output = Result<limeos_domain::StackJob>> + Send {
        let error = self.fallback("queue_stack", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn stack_job_progress(
        &self,
        token: String,
        _id: String,
        _after: i64,
    ) -> impl Future<Output = Result<limeos_contracts::StackJobProgress>> + Send {
        let error = self.fallback("stack_job_progress", Some(&token), None);
        async move { Err(error) }
    }
    fn stack_jobs(
        &self,
        token: String,
    ) -> impl Future<Output = Result<Vec<limeos_domain::StackJob>>> + Send {
        let error = self.fallback("stack_jobs", Some(&token), None);
        async move { Err(error) }
    }
    fn cancel_stack_action(
        &self,
        token: String,
        csrf: String,
        _id: String,
        _plan: bool,
    ) -> impl Future<Output = Result<()>> + Send {
        let error = self.fallback("cancel_stack_action", Some(&token), Some(&csrf));
        async move { Err(error) }
    }
    fn job_resolution(
        &self,
        token: String,
        _id: String,
    ) -> impl Future<Output = Result<limeos_contracts::ResolutionView>> + Send {
        let error = self.fallback("job_resolution", Some(&token), None);
        async move { Err(error) }
    }
    fn needs_attention_jobs(
        &self,
        token: String,
    ) -> impl Future<Output = Result<Vec<limeos_domain::NeedsAttentionJob>>> + Send {
        let error = self.fallback("needs_attention_jobs", Some(&token), None);
        async move { Err(error) }
    }
    fn resolve_job(
        &self,
        token: String,
        csrf: String,
        _id: String,
        _input: limeos_contracts::ResolveJobInput,
        _attempt: PasswordAttempt,
    ) -> impl Future<Output = PasswordChecked<limeos_contracts::ResolveJobResponse>> + Send {
        let error = self.fallback("resolve_job", Some(&token), Some(&csrf));
        async move { PasswordChecked::not_reached(error) }
    }
}
impl<T: TestBackend> Backend for T {
    fn observe_health_policy(
        &self,
        token: String,
        deadline: Instant,
    ) -> impl Future<Output = Result<limeos_domain::automation::HealthPolicyView>> + Send {
        TestBackend::observe_health_policy(self, token, deadline)
    }
    fn replace_observe_health_policy(
        &self,
        token: String,
        csrf: String,
        input: limeos_domain::automation::ReplaceHealthPolicy,
        deadline: Instant,
    ) -> impl Future<Output = Result<limeos_domain::automation::HealthPolicyView>> + Send {
        TestBackend::replace_observe_health_policy(self, token, csrf, input, deadline)
    }
    fn observe_health_alerts(
        &self,
        token: String,
        after: Option<String>,
        deadline: Instant,
    ) -> impl Future<
        Output = Result<
            limeos_domain::automation::AutomationPage<limeos_domain::automation::AlertState>,
        >,
    > + Send {
        TestBackend::observe_health_alerts(self, token, after, deadline)
    }
    fn observe_health_notifications(
        &self,
        token: String,
        after: Option<String>,
        deadline: Instant,
    ) -> impl Future<
        Output = Result<
            limeos_domain::automation::AutomationPage<limeos_domain::automation::Notification>,
        >,
    > + Send {
        TestBackend::observe_health_notifications(self, token, after, deadline)
    }
    fn acknowledge_observe_health_notification(
        &self,
        token: String,
        csrf: String,
        id: String,
        input: limeos_domain::automation::AcknowledgeNotification,
        deadline: Instant,
    ) -> impl Future<Output = Result<limeos_domain::automation::Notification>> + Send {
        TestBackend::acknowledge_observe_health_notification(self, token, csrf, id, input, deadline)
    }
    fn stack_inventory(
        &self,
        token: String,
    ) -> impl Future<Output = Result<limeos_domain::deployment::discovery::StackInventory>> + Send
    {
        TestBackend::stack_inventory(self, token)
    }
    fn observe_health_schedules(
        &self,
        token: String,
        after: Option<String>,
        deadline: Instant,
    ) -> impl Future<
        Output = Result<
            limeos_domain::automation::AutomationPage<limeos_domain::automation::ScheduleView>,
        >,
    > + Send {
        TestBackend::observe_health_schedules(self, token, after, deadline)
    }
    fn create_observe_health_schedule(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::ObserveHealthScheduleInput,
        deadline: Instant,
    ) -> impl Future<Output = Result<limeos_domain::automation::ScheduleView>> + Send {
        TestBackend::create_observe_health_schedule(self, token, csrf, input, deadline)
    }
    fn pause_observe_health_schedule(
        &self,
        token: String,
        csrf: String,
        id: String,
        input: limeos_domain::automation::PauseSchedule,
        deadline: Instant,
    ) -> impl Future<Output = Result<limeos_domain::automation::ScheduleView>> + Send {
        TestBackend::pause_observe_health_schedule(self, token, csrf, id, input, deadline)
    }
    fn enable_observe_health(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::ObserveHealthEnable,
        deadline: Instant,
        attempt: PasswordAttempt,
    ) -> impl Future<Output = PasswordChecked<limeos_contracts::ObserveHealthEnabled>> + Send {
        TestBackend::enable_observe_health(self, token, csrf, input, deadline, attempt)
    }
    fn storage_mount_catalog(
        &self,
        token: String,
    ) -> impl Future<Output = Result<limeos_domain::StorageMountCatalog>> + Send {
        TestBackend::storage_mount_catalog(self, token)
    }
    fn plan_storage_mount(
        &self,
        token: String,
        csrf: String,
        selection: limeos_domain::StorageMountSelection,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageMount>> + Send {
        TestBackend::plan_storage_mount(self, token, csrf, selection)
    }
    fn storage_mount_plan(
        &self,
        token: String,
        id: String,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageMount>> + Send {
        TestBackend::storage_mount_plan(self, token, id)
    }
    fn approve_storage_mount(
        &self,
        token: String,
        csrf: String,
        id: String,
        digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send {
        TestBackend::approve_storage_mount(self, token, csrf, id, digest)
    }
    fn queue_storage_mount(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::QueueStorageMountInput,
    ) -> impl Future<Output = Result<limeos_domain::StorageMountJob>> + Send {
        TestBackend::queue_storage_mount(self, token, csrf, input)
    }
    fn storage_mount_job(
        &self,
        token: String,
        id: String,
    ) -> impl Future<Output = Result<limeos_domain::StorageMountJob>> + Send {
        TestBackend::storage_mount_job(self, token, id)
    }
    fn storage_mount_jobs(
        &self,
        token: String,
    ) -> impl Future<Output = Result<Vec<limeos_domain::StorageMountJob>>> + Send {
        TestBackend::storage_mount_jobs(self, token)
    }
    fn cancel_storage_mount(
        &self,
        token: String,
        csrf: String,
        id: String,
    ) -> impl Future<Output = Result<()>> + Send {
        TestBackend::cancel_storage_mount(self, token, csrf, id)
    }
    fn cancel_storage_mount_job(
        &self,
        token: String,
        csrf: String,
        id: String,
    ) -> impl Future<Output = Result<()>> + Send {
        TestBackend::cancel_storage_mount_job(self, token, csrf, id)
    }
    fn container_storage_inventory(
        &self,
        token: String,
    ) -> impl Future<Output = Result<limeos_domain::ContainerStorageInventoryView>> + Send {
        TestBackend::container_storage_inventory(self, token)
    }
    fn plan_storage_targets(
        &self,
        token: String,
        csrf: String,
        contract: limeos_domain::StorageContract,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageTargets>> + Send {
        TestBackend::plan_storage_targets(self, token, csrf, contract)
    }
    fn storage_target_plan(
        &self,
        token: String,
        id: String,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageTargets>> + Send {
        TestBackend::storage_target_plan(self, token, id)
    }
    fn approve_storage_targets(
        &self,
        token: String,
        csrf: String,
        id: String,
        digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send {
        TestBackend::approve_storage_targets(self, token, csrf, id, digest)
    }
    fn queue_storage_targets(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::QueueStorageTargetsInput,
    ) -> impl Future<Output = Result<limeos_domain::StorageTargetJob>> + Send {
        TestBackend::queue_storage_targets(self, token, csrf, input)
    }
    fn storage_target_job(
        &self,
        token: String,
        id: String,
    ) -> impl Future<Output = Result<limeos_domain::StorageTargetJob>> + Send {
        TestBackend::storage_target_job(self, token, id)
    }
    fn cancel_storage_targets(
        &self,
        token: String,
        csrf: String,
        id: String,
    ) -> impl Future<Output = Result<()>> + Send {
        TestBackend::cancel_storage_targets(self, token, csrf, id)
    }
    fn cancel_storage_target_job(
        &self,
        token: String,
        csrf: String,
        id: String,
    ) -> impl Future<Output = Result<()>> + Send {
        TestBackend::cancel_storage_target_job(self, token, csrf, id)
    }
    fn storage_inventory(
        &self,
        token: String,
    ) -> impl Future<Output = Result<limeos_domain::StorageInventoryView>> + Send {
        TestBackend::storage_inventory(self, token)
    }
    fn plan_storage(
        &self,
        token: String,
        csrf: String,
        input: limeos_domain::StorageSetupInput,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageSetup>> + Send {
        TestBackend::plan_storage(self, token, csrf, input)
    }
    fn storage_plan(
        &self,
        token: String,
        id: String,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageSetup>> + Send {
        TestBackend::storage_plan(self, token, id)
    }
    fn approve_storage(
        &self,
        token: String,
        csrf: String,
        id: String,
        digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send {
        TestBackend::approve_storage(self, token, csrf, id, digest)
    }
    fn cancel_storage(
        &self,
        token: String,
        csrf: String,
        id: String,
    ) -> impl Future<Output = Result<()>> + Send {
        TestBackend::cancel_storage(self, token, csrf, id)
    }
    fn plan_compose(
        &self,
        token: String,
        csrf: String,
        selection: limeos_domain::ComposeSelection,
    ) -> impl Future<Output = Result<limeos_domain::PlannedCompose>> + Send {
        TestBackend::plan_compose(self, token, csrf, selection)
    }
    fn compose_plan(
        &self,
        token: String,
        id: String,
    ) -> impl Future<Output = Result<limeos_domain::PlannedCompose>> + Send {
        TestBackend::compose_plan(self, token, id)
    }
    fn approve_compose(
        &self,
        token: String,
        csrf: String,
        id: String,
        digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send {
        TestBackend::approve_compose(self, token, csrf, id, digest)
    }
    fn cancel_compose(
        &self,
        token: String,
        csrf: String,
        id: String,
    ) -> impl Future<Output = Result<()>> + Send {
        TestBackend::cancel_compose(self, token, csrf, id)
    }
    fn login(
        &self,
        request: Login,
        attempt: PasswordAttempt,
    ) -> impl Future<Output = PasswordChecked<IssuedSession>> + Send {
        TestBackend::login(self, request, attempt)
    }
    fn session(
        &self,
        token: String,
        csrf: Option<String>,
    ) -> impl Future<Output = Result<SessionView>> + Send {
        TestBackend::session(self, token, csrf)
    }
    fn logout(&self, token: String, csrf: String) -> impl Future<Output = Result<()>> + Send {
        TestBackend::logout(self, token, csrf)
    }
    fn health(&self) -> impl Future<Output = Result<Health>> + Send {
        TestBackend::health(self)
    }
    fn signed_in(&self, token: String) -> impl Future<Output = bool> + Send {
        TestBackend::signed_in(self, token)
    }
    fn observations(
        &self,
        token: String,
    ) -> impl Future<Output = Result<limeos_contracts::Overview>> + Send {
        TestBackend::observations(self, token)
    }
    fn history(
        &self,
        token: String,
        range: limeos_contracts::HistoryRange,
    ) -> impl Future<Output = Result<limeos_contracts::MetricHistory>> + Send {
        TestBackend::history(self, token, range)
    }
    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        TestBackend::changes(self)
    }
    fn plan_restart(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::RestartInput,
    ) -> impl Future<Output = Result<limeos_domain::PlannedRestart>> + Send {
        TestBackend::plan_restart(self, token, csrf, input)
    }
    fn plan_container(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::ContainerInput,
    ) -> impl Future<Output = Result<limeos_domain::PlannedContainerAction>> + Send {
        TestBackend::plan_container(self, token, csrf, input)
    }
    fn container_logs(
        &self,
        token: String,
        resource: String,
        options: limeos_domain::LogOptions,
    ) -> impl Future<Output = Result<limeos_domain::ContainerLogs>> + Send {
        TestBackend::container_logs(self, token, resource, options)
    }
    fn approve_container(
        &self,
        token: String,
        csrf: String,
        id: String,
        digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send {
        TestBackend::approve_container(self, token, csrf, id, digest)
    }
    fn queue_container(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::QueueRestartInput,
    ) -> impl Future<Output = Result<limeos_domain::RestartJob>> + Send {
        TestBackend::queue_container(self, token, csrf, input)
    }
    fn restart_progress(
        &self,
        token: String,
        id: String,
        after: i64,
    ) -> impl Future<Output = Result<limeos_contracts::JobProgress>> + Send {
        TestBackend::restart_progress(self, token, id, after)
    }
    fn container_jobs(
        &self,
        token: String,
    ) -> impl Future<Output = Result<Vec<limeos_domain::RestartJob>>> + Send {
        TestBackend::container_jobs(self, token)
    }
    fn cancel_restart(
        &self,
        token: String,
        csrf: String,
        id: String,
        plan: bool,
    ) -> impl Future<Output = Result<()>> + Send {
        TestBackend::cancel_restart(self, token, csrf, id, plan)
    }
    fn plan_stack(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::StackInput,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStackAction>> + Send {
        TestBackend::plan_stack(self, token, csrf, input)
    }
    fn approve_stack(
        &self,
        token: String,
        csrf: String,
        id: String,
        digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send {
        TestBackend::approve_stack(self, token, csrf, id, digest)
    }
    fn queue_stack(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::QueueStackInput,
    ) -> impl Future<Output = Result<limeos_domain::StackJob>> + Send {
        TestBackend::queue_stack(self, token, csrf, input)
    }
    fn stack_job_progress(
        &self,
        token: String,
        id: String,
        after: i64,
    ) -> impl Future<Output = Result<limeos_contracts::StackJobProgress>> + Send {
        TestBackend::stack_job_progress(self, token, id, after)
    }
    fn stack_jobs(
        &self,
        token: String,
    ) -> impl Future<Output = Result<Vec<limeos_domain::StackJob>>> + Send {
        TestBackend::stack_jobs(self, token)
    }
    fn cancel_stack_action(
        &self,
        token: String,
        csrf: String,
        id: String,
        plan: bool,
    ) -> impl Future<Output = Result<()>> + Send {
        TestBackend::cancel_stack_action(self, token, csrf, id, plan)
    }
    fn job_resolution(
        &self,
        token: String,
        id: String,
    ) -> impl Future<Output = Result<limeos_contracts::ResolutionView>> + Send {
        TestBackend::job_resolution(self, token, id)
    }
    fn needs_attention_jobs(
        &self,
        token: String,
    ) -> impl Future<Output = Result<Vec<limeos_domain::NeedsAttentionJob>>> + Send {
        TestBackend::needs_attention_jobs(self, token)
    }
    fn resolve_job(
        &self,
        token: String,
        csrf: String,
        id: String,
        input: limeos_contracts::ResolveJobInput,
        attempt: PasswordAttempt,
    ) -> impl Future<Output = PasswordChecked<limeos_contracts::ResolveJobResponse>> + Send {
        TestBackend::resolve_job(self, token, csrf, id, input, attempt)
    }
}
