use schemars::schema_for;
use std::path::Path;
use ts_rs::TS;
fn lift_definitions(
    value: &mut serde_json::Value,
    definitions: &mut serde_json::Map<String, serde_json::Value>,
) {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(serde_json::Value::Object(defs)) = object.remove("$defs") {
                for (key, mut schema) in defs {
                    lift_definitions(&mut schema, definitions);
                    if let Some(previous) = definitions.insert(key, schema.clone()) {
                        assert_eq!(previous, schema, "schema name collision");
                    }
                }
            }
            if let Some(serde_json::Value::String(reference)) = object.get_mut("$ref") {
                if let Some(name) = reference.strip_prefix("#/$defs/") {
                    *reference = format!("#/components/schemas/{name}");
                }
            }
            for child in object.values_mut() {
                lift_definitions(child, definitions);
            }
        }
        serde_json::Value::Array(array) => {
            for child in array {
                lift_definitions(child, definitions);
            }
        }
        _ => {}
    }
}
fn main() {
    let dest = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "contracts/generated".into());
    let dir = Path::new(&dest);
    std::fs::create_dir_all(dir).expect("schema output directory");
    macro_rules! emit {
        ($name:literal,$ty:ty) => {
            std::fs::write(
                dir.join(concat!($name, ".schema.json")),
                serde_json::to_string_pretty(&schema_for!($ty)).unwrap() + "\n",
            )
            .unwrap();
        };
    }
    emit!("core-request", limeos_contracts::CoreRequest);
    emit!(
        "observe-health-enable",
        limeos_contracts::ObserveHealthEnable
    );
    emit!(
        "observe-health-enabled",
        limeos_contracts::ObserveHealthEnabled
    );
    emit!(
        "observe-health-schedule-input",
        limeos_contracts::ObserveHealthScheduleInput
    );
    emit!(
        "schedule-pause-input",
        limeos_domain::automation::PauseSchedule
    );
    emit!(
        "automation-schedule",
        limeos_domain::automation::ScheduleView
    );
    emit!(
        "automation-schedule-page",
        limeos_domain::automation::AutomationPage<limeos_domain::automation::ScheduleView>
    );
    emit!("automation-alert", limeos_domain::automation::AlertState);
    emit!(
        "health-policy-definition",
        limeos_domain::automation::HealthPolicyDefinition
    );
    emit!("health-policy", limeos_domain::automation::HealthPolicyView);
    emit!(
        "health-policy-replacement",
        limeos_domain::automation::ReplaceHealthPolicy
    );
    emit!(
        "automation-alert-page",
        limeos_domain::automation::AutomationPage<limeos_domain::automation::AlertState>
    );
    emit!(
        "automation-notification",
        limeos_domain::automation::Notification
    );
    emit!(
        "automation-notification-page",
        limeos_domain::automation::AutomationPage<limeos_domain::automation::Notification>
    );
    emit!(
        "automation-acknowledgment",
        limeos_domain::automation::AcknowledgeNotification
    );
    emit!("core-response", limeos_contracts::CoreResponse);
    emit!("executor-request", limeos_executor_protocol::Request);
    emit!("executor-ceiling", limeos_executor_protocol::Ceiling);
    emit!("intent", limeos_domain::Intent);
    emit!("event", limeos_domain::Event);
    emit!("overview", limeos_contracts::Overview);
    emit!(
        "stack-inventory",
        limeos_domain::deployment::discovery::StackInventory
    );
    emit!(
        "stack-cohort",
        limeos_domain::deployment::discovery::DiscoveryCohort
    );
    emit!("resource-page", limeos_contracts::ResourcePage);
    emit!("metric-history", limeos_contracts::MetricHistory);
    emit!("restart-plan", limeos_domain::PlannedRestart);
    emit!("restart-job", limeos_domain::RestartJob);
    emit!("executor-receipt", limeos_executor_protocol::Receipt);
    emit!("restart-input", limeos_contracts::RestartInput);
    emit!("approval-input", limeos_contracts::ApprovalInput);
    emit!("queue-restart-input", limeos_contracts::QueueRestartInput);
    emit!("job-progress", limeos_contracts::JobProgress);
    emit!("container-input", limeos_contracts::ContainerInput);
    emit!("container-plan", limeos_domain::PlannedContainerAction);
    emit!("container-job", limeos_domain::ContainerJob);
    emit!("stack-input", limeos_contracts::StackInput);
    emit!("stack-plan", limeos_domain::PlannedStackAction);
    emit!("stack-job", limeos_domain::StackJob);
    emit!("queue-stack-input", limeos_contracts::QueueStackInput);
    emit!("stack-job-progress", limeos_contracts::StackJobProgress);
    emit!("resolution-view", limeos_contracts::ResolutionView);
    emit!(
        "needs-attention-jobs",
        Vec<limeos_domain::NeedsAttentionJob>
    );
    emit!("resolve-job-input", limeos_contracts::ResolveJobInput);
    emit!("resolve-job-response", limeos_contracts::ResolveJobResponse);
    emit!("container-logs", limeos_domain::ContainerLogs);
    emit!("compose-catalog", limeos_domain::ComposeCatalog);
    emit!("compose-selection", limeos_domain::ComposeSelection);
    emit!("compose-plan", limeos_domain::PlannedCompose);
    emit!("storage-contract", limeos_domain::StorageContract);
    emit!("storage-inventory", limeos_domain::StorageInventoryView);
    emit!("storage-setup-input", limeos_domain::StorageSetupInput);
    emit!("storage-setup-plan", limeos_domain::PlannedStorageSetup);
    emit!("storage-target-plan", limeos_domain::PlannedStorageTargets);
    emit!("storage-target-job", limeos_domain::StorageTargetJob);
    emit!(
        "storage-mount-selection",
        limeos_domain::StorageMountSelection
    );
    emit!("storage-mount-catalog", limeos_domain::StorageMountCatalog);
    emit!("storage-mount-plan", limeos_domain::PlannedStorageMount);
    emit!("storage-mount-job", limeos_domain::StorageMountJob);
    emit!(
        "storage-mount-ceiling",
        limeos_executor_protocol::StorageMountCeiling
    );
    emit!(
        "queue-storage-mount-input",
        limeos_contracts::QueueStorageMountInput
    );
    emit!(
        "storage-target-ceiling",
        limeos_executor_protocol::StorageTargetCeiling
    );
    emit!(
        "queue-storage-targets-input",
        limeos_contracts::QueueStorageTargetsInput
    );
    emit!(
        "storage-mount-wait-plan",
        limeos_domain::StorageMountWaitPlan
    );
    emit!("pools-config", limeos_domain::PoolsConfig);
    emit!("pool-preview", limeos_domain::PoolPreview);
    emit!("snapraid-config", limeos_domain::SnapraidConfig);
    emit!("snapraid-preview", limeos_domain::SnapraidPreview);
    emit!("snapraid-sync-preview", limeos_domain::SnapraidSyncPreview);
    emit!(
        "snapraid-diff-evidence",
        limeos_domain::SnapraidDiffEvidence
    );
    emit!("storage-read-failure", limeos_domain::StorageReadFailure);
    emit!(
        "container-storage-inventory",
        limeos_domain::ContainerStorageInventoryView
    );
    let mut types = String::from("// Generated by limeos-schema. Do not edit.\n");
    macro_rules! ts {
        ($ty:ty) => {
            push_typescript::<$ty>(&mut types);
        };
    }
    ts!(limeos_domain::ErrorCode);
    ts!(limeos_domain::Role);
    ts!(limeos_domain::Operation);
    ts!(limeos_domain::Principal);
    ts!(limeos_domain::JobState);
    ts!(limeos_domain::CustodyBinding);
    ts!(limeos_domain::BackupPreparationBinding);
    ts!(limeos_domain::BackupPreparationJobPlan);
    ts!(limeos_domain::PlannedBackupPreparation);
    ts!(limeos_domain::BackupPreparationJob);
    ts!(limeos_domain::automation::OccurrenceKey);
    ts!(limeos_domain::Intent);
    ts!(limeos_domain::ContainerSnapshot);
    ts!(limeos_domain::ContainerAction);
    ts!(limeos_domain::LogOptions);
    ts!(limeos_domain::ContainerLogs);
    ts!(limeos_domain::ComposeSelection);
    ts!(limeos_domain::ComposeFileName);
    ts!(limeos_domain::ComposeFile);
    ts!(limeos_domain::ComposeUser);
    ts!(limeos_domain::PortProtocol);
    ts!(limeos_domain::ComposePort);
    ts!(limeos_domain::ComposeMount);
    ts!(limeos_domain::ComposeDevice);
    ts!(limeos_domain::ComposeRestart);
    ts!(limeos_domain::ComposeService);
    ts!(limeos_domain::ComposeProject);
    ts!(limeos_domain::ComposePrivilege);
    ts!(limeos_domain::ComposeTemplate);
    ts!(limeos_domain::ComposeStack);
    ts!(limeos_domain::ComposeCatalog);
    ts!(limeos_domain::ComposeServiceChange);
    ts!(limeos_domain::ComposeImpact);
    ts!(limeos_domain::ComposePlan);
    ts!(limeos_domain::PlannedCompose);
    ts!(limeos_domain::StorageSchemaVersion);
    ts!(limeos_domain::StorageProfile);
    ts!(limeos_domain::StorageRole);
    ts!(limeos_domain::StorageFilesystem);
    ts!(limeos_domain::StorageMediaIdentity);
    ts!(limeos_domain::StorageLocations);
    ts!(limeos_domain::StorageDevice);
    ts!(limeos_domain::StorageContract);
    ts!(limeos_domain::StorageMountWaitPlan);
    ts!(limeos_domain::StorageBlockDevice);
    ts!(limeos_domain::StorageObservedMount);
    ts!(limeos_domain::StorageObservedDevice);
    ts!(limeos_domain::StorageFstabEntry);
    ts!(limeos_domain::StorageInventory);
    ts!(limeos_domain::StorageInventoryView);
    ts!(limeos_domain::ContainerStorageSource);
    ts!(limeos_domain::ContainerStorageMount);
    ts!(limeos_domain::ContainerStorageConsumer);
    ts!(limeos_domain::ContainerStorageInventory);
    ts!(limeos_domain::ContainerStorageInventoryView);
    ts!(limeos_domain::DeclaredContainerStorageDependency);
    ts!(limeos_domain::StorageSetupInput);
    ts!(limeos_domain::StorageSetupPlan);
    ts!(limeos_domain::PlannedStorageSetup);
    ts!(limeos_domain::DirectoryIdentity);
    ts!(limeos_domain::TargetEvidence);
    ts!(limeos_domain::StorageTargetSnapshot);
    ts!(limeos_domain::TargetPreparationPlan);
    ts!(limeos_domain::PreparationState);
    ts!(limeos_domain::TargetPreparationReceipt);
    ts!(limeos_domain::StorageTargetPlan);
    ts!(limeos_domain::PlannedStorageTargets);
    ts!(limeos_domain::StorageTargetJob);
    ts!(limeos_executor_protocol::StorageTargetCeiling);
    ts!(limeos_executor_protocol::ManagedStorageTarget);
    ts!(limeos_contracts::QueueStorageTargetsInput);
    ts!(limeos_domain::StorageMountSelection);
    ts!(limeos_domain::StorageMountCatalog);
    ts!(limeos_domain::StorageMountInspection);
    ts!(limeos_domain::StorageMountEffectPlan);
    ts!(limeos_domain::StorageMountPlan);
    ts!(limeos_domain::PlannedStorageMount);
    ts!(limeos_domain::StorageMountJob);
    ts!(limeos_domain::MountExecutionState);
    ts!(limeos_domain::StorageMountedSnapshot);
    ts!(limeos_domain::MountHelperIdentity);
    ts!(limeos_domain::MountHelperOutcome);
    ts!(limeos_domain::MountEffectQuiescence);
    ts!(limeos_domain::StorageMountReceipt);
    ts!(limeos_executor_protocol::ManagedStorageMount);
    ts!(limeos_executor_protocol::StorageMountCeiling);
    ts!(limeos_contracts::QueueStorageMountInput);
    ts!(limeos_domain::StoragePlanningFindingCode);
    ts!(limeos_domain::StoragePlanningFinding);
    ts!(limeos_domain::PoolCreatePolicy);
    ts!(limeos_domain::PoolPreset);
    ts!(limeos_domain::PoolFileCache);
    ts!(limeos_domain::PoolGetattr);
    ts!(limeos_domain::PoolOptions);
    ts!(limeos_domain::PoolConfig);
    ts!(limeos_domain::PoolsConfig);
    ts!(limeos_domain::PoolDependency);
    ts!(limeos_domain::PoolPreview);
    ts!(limeos_domain::SnapraidRole);
    ts!(limeos_domain::SnapraidDrive);
    ts!(limeos_domain::SnapraidSettings);
    ts!(limeos_domain::SnapraidThresholds);
    ts!(limeos_domain::SnapraidScrub);
    ts!(limeos_domain::SnapraidSchedule);
    ts!(limeos_domain::SnapraidConfig);
    ts!(limeos_domain::ProtectionPathKind);
    ts!(limeos_domain::ProtectionPathRequirement);
    ts!(limeos_domain::SnapraidPreview);
    ts!(limeos_domain::ProtectionPathState);
    ts!(limeos_domain::ProtectionPathObservation);
    ts!(limeos_domain::ProtectionSourceEvidence);
    ts!(limeos_domain::StorageReadTermination);
    ts!(limeos_domain::StorageReadFailureReason);
    ts!(limeos_domain::StorageReadFailure);
    ts!(limeos_domain::SnapraidDiff);
    ts!(limeos_domain::SnapraidDiffOutcome);
    ts!(limeos_domain::SnapraidDiffEvidence);
    ts!(limeos_domain::SnapraidSyncPreview);
    ts!(limeos_domain::RestartPlan);
    ts!(limeos_domain::PlannedRestart);
    ts!(limeos_domain::PlanApproval);
    ts!(limeos_domain::RestartJob);
    ts!(limeos_domain::RestartRequest);
    ts!(limeos_domain::RestartReceipt);
    ts!(limeos_domain::ExecutionState);
    ts!(limeos_domain::EventKind);
    ts!(limeos_domain::Event);
    ts!(limeos_contracts::Login);
    ts!(limeos_contracts::ObserveHealthEnable);
    ts!(limeos_contracts::ObserveHealthEnabled);
    ts!(limeos_contracts::ObserveHealthScheduleInput);
    ts!(limeos_domain::automation::ScheduledOperation);
    ts!(limeos_domain::automation::MissedRunPolicy);
    ts!(limeos_domain::automation::MaintenanceWindow);
    ts!(limeos_domain::automation::ScheduleDefinition);
    ts!(limeos_domain::automation::Schedule);
    ts!(limeos_domain::automation::PauseSchedule);
    ts!(limeos_domain::automation::ScheduleStatus);
    ts!(limeos_domain::automation::ObservationPhase);
    ts!(limeos_domain::automation::ObservationRuntimeView);
    ts!(limeos_domain::automation::OccurrenceOutcome);
    ts!(limeos_domain::automation::OccurrenceView);
    ts!(limeos_domain::automation::ScheduleView);
    ts!(limeos_domain::automation::AlertKind);
    ts!(limeos_domain::automation::Severity);
    ts!(limeos_domain::automation::SignalStatus);
    ts!(limeos_domain::automation::AlertSignal);
    ts!(limeos_domain::automation::IncidentState);
    ts!(limeos_domain::automation::AlertState);
    ts!(limeos_domain::automation::AlertEventKind);
    ts!(limeos_domain::automation::HealthPolicySilence);
    ts!(limeos_domain::automation::HealthPolicyDefinition);
    ts!(limeos_domain::automation::ContainerHealthMember);
    ts!(limeos_domain::automation::ContainerHealthCohort);
    ts!(limeos_domain::automation::HealthPolicyView);
    ts!(limeos_domain::automation::ReplaceHealthPolicy);
    ts!(limeos_domain::automation::AlertEvent);
    ts!(limeos_domain::automation::DeliveryChannel);
    ts!(limeos_domain::automation::DeliveryState);
    ts!(limeos_domain::automation::Notification);
    ts!(limeos_domain::automation::AcknowledgeNotification);
    ts!(limeos_domain::automation::AutomationPage<limeos_domain::automation::ScheduleView>);
    ts!(limeos_contracts::Enrollment);
    ts!(limeos_contracts::LegacyUser);
    ts!(limeos_contracts::Health);
    ts!(limeos_contracts::SessionView);
    ts!(limeos_contracts::ErrorEnvelope);
    ts!(limeos_contracts::CoreRequest);
    ts!(limeos_contracts::CoreResponse);
    ts!(limeos_contracts::Source);
    ts!(limeos_contracts::Availability);
    ts!(limeos_contracts::Freshness);
    ts!(limeos_contracts::HostMetrics);
    ts!(limeos_contracts::ResourceKind);
    ts!(limeos_contracts::Resource);
    ts!(limeos_contracts::ObservationBatch);
    ts!(limeos_contracts::Overview);
    ts!(limeos_contracts::ResourcePage);
    ts!(limeos_contracts::HistoryRange);
    ts!(limeos_contracts::MetricPoint);
    ts!(limeos_contracts::MetricHistory);
    ts!(limeos_contracts::RestartInput);
    ts!(limeos_contracts::ContainerInput);
    ts!(limeos_contracts::ApprovalInput);
    ts!(limeos_contracts::QueueRestartInput);
    ts!(limeos_contracts::JobProgress);
    ts!(limeos_domain::StackSnapshot);
    ts!(limeos_domain::StackPlan);
    ts!(limeos_domain::PlannedStackAction);
    ts!(limeos_domain::StackJob);
    ts!(limeos_domain::StackRequest);
    ts!(limeos_domain::StackMemberReceipt);
    ts!(limeos_domain::StackReceipt);
    ts!(limeos_contracts::StackInput);
    ts!(limeos_contracts::QueueStackInput);
    ts!(limeos_contracts::StackJobProgress);
    ts!(limeos_domain::DeclaredOutcome);
    ts!(limeos_domain::ResolveKind);
    ts!(limeos_domain::ResolutionTarget);
    ts!(limeos_domain::ResolutionEvidence);
    ts!(limeos_domain::ResolutionMember);
    ts!(limeos_domain::ResolutionRequest);
    ts!(limeos_domain::ResolveReceipt);
    ts!(limeos_domain::JobResolution);
    ts!(limeos_contracts::ResolutionTargetView);
    ts!(limeos_contracts::ResolutionView);
    ts!(limeos_domain::NeedsAttentionJob);
    ts!(limeos_contracts::ResolveJobInput);
    ts!(limeos_contracts::ResolveStatus);
    ts!(limeos_contracts::ResolveJobResponse);
    ts!(limeos_executor_protocol::Request);
    ts!(limeos_domain::deployment::discovery::StackInventory);
    ts!(limeos_domain::deployment::discovery::SourceStatus);
    ts!(limeos_domain::deployment::discovery::SourceState);
    ts!(limeos_domain::deployment::discovery::RuntimeStatus);
    ts!(limeos_domain::deployment::discovery::Counts);
    ts!(limeos_domain::deployment::discovery::MemberView);
    ts!(limeos_domain::deployment::discovery::ServiceView);
    ts!(limeos_domain::deployment::discovery::StackIssue);
    ts!(limeos_domain::deployment::discovery::StackView);
    ts!(limeos_domain::deployment::discovery::RegistrationRelation);
    ts!(limeos_domain::deployment::discovery::OtherProjectView);
    ts!(limeos_domain::deployment::discovery::ProjectIdentity);
    ts!(limeos_domain::deployment::discovery::FileStatus);
    ts!(limeos_domain::deployment::discovery::UnsupportedSource);
    ts!(limeos_domain::deployment::discovery::CatalogFacts);
    ts!(limeos_domain::deployment::discovery::EngineFacts);
    ts!(limeos_domain::deployment::discovery::Declaration);
    ts!(limeos_domain::deployment::discovery::Membership);
    ts!(limeos_domain::deployment::discovery::DiscoveryCohort);
    ts!(limeos_executor_protocol::Ceiling);
    ts!(limeos_executor_protocol::Receipt);
    types.push_str("export type RestartPlan = ContainerPlan;\nexport type PlannedRestart = PlannedContainerAction;\nexport type RestartJob = ContainerJob;\nexport type RestartRequest = ContainerRequest;\nexport type RestartReceipt = ContainerReceipt;\n");
    std::fs::write(dir.join("types.ts"), types).unwrap();
    let mut openapi = serde_json::json!({"openapi":"3.1.0","info":{"title":"LimeOS foundation API","version":"1"},"paths":{
        "/api/v1/health":{"get":{"responses":{"200":{"description":"Core readiness","content":{"application/json":{"schema":schema_for!(limeos_contracts::Health)}}}}}},
        "/api/v1/auth/login":{"post":{"requestBody":{"required":true,"content":{"application/json":{"schema":schema_for!(limeos_contracts::Login)}}},"responses":{"200":{"description":"Secure session cookie and CSRF token","content":{"application/json":{"schema":schema_for!(limeos_contracts::SessionView)}}},"401":{"description":"Authentication failed"}}}},
        "/api/v1/auth/session":{"get":{"responses":{"200":{"description":"Current session","content":{"application/json":{"schema":schema_for!(limeos_contracts::SessionView)}}}}}},
        "/api/v1/auth/logout":{"post":{"responses":{"204":{"description":"Session revoked"}}}}
    }});
    openapi["paths"]["/api/v1/deployment/stacks"] = serde_json::json!({"get":{
        "security":[{"session":[]}],
        "description":"Read the shared stack cache. Requires current HealthRead grants for deployment:stacks and every disclosed stack, project and container. A partial grant refuses the complete view. Does not launch a probe or authorize stack actions.",
        "responses":{"200":{"description":"Bounded stack/member observation with original source clocks","content":{"application/json":{"schema":schema_for!(limeos_domain::deployment::discovery::StackInventory)}}},"401":{"description":"Session expired or revoked"},"403":{"description":"Complete inventory is outside current grants"},"503":{"description":"Bounded inventory is unavailable"}}
    }});
    openapi["paths"]["/api/v1/auth/capabilities/observe-health/enable"] = serde_json::json!({"post":{
        "security":[{"session":[]}],
        "description":"Reverify the current user's password (1–1024 UTF-8 bytes) and enable only that user's exact ObserveHealth management grant. Current IdentityManage and host-read grants are required. A changed grant invalidates prior sessions/tasks and requires a new login; an existing exact grant is a write-free no-op. Does not create or resume schedules.",
        "parameters":[
            {"name":"Origin","in":"header","required":true,"schema":{"type":"string"}},
            {"name":"X-CSRF-Token","in":"header","required":true,"schema":{"type":"string","minLength":64,"maxLength":64}}
        ],
        "requestBody":{"required":true,"content":{"application/json":{"schema":schema_for!(limeos_contracts::ObserveHealthEnable)}}},
        "responses":{"200":{"description":"Fixed self-enablement outcome; clear session cookie when reauthenticate is true","content":{"application/json":{"schema":schema_for!(limeos_contracts::ObserveHealthEnabled)}}}}
    }});
    openapi["paths"]["/api/v1/automation/schedules"] = serde_json::json!({
        "get": {
            "security":[{"session":[]}],
            "description":"Read at most 32 owned ObserveHealth schedules within 64 KiB. Scheduled includes bounded historical partial/unavailable runtime diagnostics while the owned Core producer runs. AwaitingProducer is not running; AuthoritySuspended requires explicit current-authority resume. Reads do not revise authority, advance watermarks or invoke a collector.",
            "parameters":[{"name":"after","in":"query","schema":{"type":"string","pattern":"^[a-f0-9]{64}$"}}],
            "responses":{"200":{"description":"Owned schedule metadata and current readiness","content":{"application/json":{"schema":schema_for!(limeos_domain::automation::AutomationPage<limeos_domain::automation::ScheduleView>)}}}}
        },
        "post": {
            "security":[{"session":[]}],
            "description":"Create fixed ObserveHealth metadata with current literal management and host-read grants. Owner, operation and anchor are server-owned. The owned Core producer later admits due container observations under current authority; this request commits metadata only. A lost create response must be reconciled through the owned list; do not automatically retry.",
            "parameters":[
                {"name":"Origin","in":"header","required":true,"schema":{"type":"string"}},
                {"name":"X-CSRF-Token","in":"header","required":true,"schema":{"type":"string","minLength":64,"maxLength":64}}
            ],
            "requestBody":{"required":true,"content":{"application/json":{"schema":schema_for!(limeos_contracts::ObserveHealthScheduleInput)}}},
            "responses":{"201":{"description":"Durable schedule metadata with current partial observation registration","content":{"application/json":{"schema":schema_for!(limeos_domain::automation::ScheduleView)}}}}
        }
    });
    openapi["paths"]["/api/v1/automation/schedules/{id}/pause"] = serde_json::json!({"post":{
        "security":[{"session":[]}],
        "description":"Current-owner revision CAS. paused=false explicitly resumes/rebinds current authority and resets the server anchor only for an actual management change. Cancel only queued observation jobs atomically; no dispatch or effect. Stale retries conflict.",
        "parameters":[
            {"name":"Origin","in":"header","required":true,"schema":{"type":"string"}},
            {"name":"X-CSRF-Token","in":"header","required":true,"schema":{"type":"string","minLength":64,"maxLength":64}}
        ],
        "requestBody":{"required":true,"content":{"application/json":{"schema":schema_for!(limeos_domain::automation::PauseSchedule)}}},
        "responses":{"200":{"description":"Committed schedule metadata","content":{"application/json":{"schema":schema_for!(limeos_domain::automation::ScheduleView)}}}}
    }});
    openapi["paths"]["/api/v1/automation/alerts"] = serde_json::json!({"get":{
        "security":[{"session":[]}],
        "description":"Read at most 32 owned stored alert rows within 64 KiB, with current host and per-resource HealthRead grants. No collection or publication. Missing/incomplete observation history remains partial; empty history is not proof of health.",
        "parameters":[{"name":"after","in":"query","schema":{"type":"string","minLength":1,"maxLength":256}}],
        "responses":{"200":{"description":"Owned alert history","content":{"application/json":{"schema":schema_for!(limeos_domain::automation::AutomationPage<limeos_domain::automation::AlertState>)}}}}
    }});
    openapi["paths"]["/api/v1/automation/health-policy"] = serde_json::json!({
        "get":{"security":[{"session":[]}],"description":"Read current owned policy with host-read and narrow health-automation management grants. Implicit revision zero creates no rows. No collection or delivery.","responses":{"200":{"description":"Current owned configuration","content":{"application/json":{"schema":schema_for!(limeos_domain::automation::HealthPolicyView)}}}}},
        "post":{"security":[{"session":[]}],"description":"Replace owned policy by expected revision with current session, Origin, CSRF, host-read and narrow health-automation management grants. Preserves literal mounts and silence metadata; no credential or resource authority. Lost replies may have committed: reconcile by GET before any explicit retry. Registered automatic container checks remain globally partial.","parameters":[{"name":"Origin","in":"header","required":true,"schema":{"type":"string"}},{"name":"X-CSRF-Token","in":"header","required":true,"schema":{"type":"string","minLength":64,"maxLength":64}}],"requestBody":{"required":true,"content":{"application/json":{"schema":schema_for!(limeos_domain::automation::ReplaceHealthPolicy)}}},"responses":{"200":{"description":"Committed explicit policy revision","content":{"application/json":{"schema":schema_for!(limeos_domain::automation::HealthPolicyView)}}}}}
    });
    openapi["paths"]["/api/v1/automation/notifications"] = serde_json::json!({"get":{
        "security":[{"session":[]}],
        "description":"Read at most 32 owned in-app notifications within 64 KiB under current host and per-resource HealthRead grants. Does not publish, send or acknowledge. Partial state remains explicit.",
        "parameters":[{"name":"after","in":"query","schema":{"type":"string","pattern":"^[a-f0-9]{64}$"}}],
        "responses":{"200":{"description":"Owned in-app notification history","content":{"application/json":{"schema":schema_for!(limeos_domain::automation::AutomationPage<limeos_domain::automation::Notification>)}}}}
    }});
    openapi["paths"]["/api/v1/automation/notifications/{id}/acknowledgment"] = serde_json::json!({"post":{
        "security":[{"session":[]}],
        "description":"Acknowledge an owned delivered in-app row with current session/CSRF and resource grants. Revision CAS, idempotent acknowledgment replay and existing audit apply. An abandoned/timed-out reply may have committed; reconcile with GET, without automatically repeating the POST. No send or publication.",
        "parameters":[
            {"name":"id","in":"path","required":true,"schema":{"type":"string","pattern":"^[a-f0-9]{64}$"}},
            {"name":"Origin","in":"header","required":true,"schema":{"type":"string"}},
            {"name":"X-CSRF-Token","in":"header","required":true,"schema":{"type":"string","minLength":64,"maxLength":64}}
        ],
        "requestBody":{"required":true,"content":{"application/json":{"schema":schema_for!(limeos_domain::automation::AcknowledgeNotification)}}},
        "responses":{"200":{"description":"Committed acknowledgment","content":{"application/json":{"schema":schema_for!(limeos_domain::automation::Notification)}}}}
    }});
    for (path, schema) in [
        (
            "/api/v1/overview",
            serde_json::to_value(schema_for!(limeos_contracts::Overview)).unwrap(),
        ),
        (
            "/api/v1/resources",
            serde_json::to_value(schema_for!(limeos_contracts::ResourcePage)).unwrap(),
        ),
        (
            "/api/v1/system/history",
            serde_json::to_value(schema_for!(limeos_contracts::MetricHistory)).unwrap(),
        ),
    ] {
        openapi["paths"][path] = serde_json::json!({"get":{"security":[{"session":[]}],"responses":{"200":{"description":"Scoped cached observations","content":{"application/json":{"schema":schema}}}}}});
    }
    openapi["paths"]["/api/v1/resources"]["get"]["parameters"] = serde_json::json!([
        {"name":"offset","in":"query","schema":{"type":"integer","minimum":0,"maximum":1024,"default":0}},
        {"name":"limit","in":"query","schema":{"type":"integer","minimum":1,"maximum":50,"default":20}},
        {"name":"revision","in":"query","schema":{"type":"integer","minimum":0}},
        {"name":"kind","in":"query","schema":schema_for!(limeos_contracts::ResourceKind)}
    ]);
    openapi["paths"]["/api/v1/system/history"]["get"]["parameters"] = serde_json::json!([{ "name":"range","in":"query","required":true,"schema":schema_for!(limeos_contracts::HistoryRange)}]);
    openapi["paths"]["/api/v1/storage/container-dependencies"] = serde_json::json!({"get":{"security":[{"session":[]}],"description":"Fresh bounded Docker mount declarations. Requires a human administrator and the current storage grant; physical, pool, protection and share checks remain necessary before storage effects.","responses":{"200":{"description":"Digest-bound container storage inventory","content":{"application/json":{"schema":schema_for!(limeos_domain::ContainerStorageInventoryView)}}},"403":{"description":"Storage authority is required"},"409":{"description":"Collection changed or expired"},"503":{"description":"Complete live evidence is unavailable"}}}});
    openapi["paths"]["/api/v1/observations/stream"] = serde_json::json!({"get":{"security":[{"session":[]}],"description":"Full authorized snapshot on connect and reconnect. Maximum eight readers, 25 seconds and 128 KiB per stream. Identity rechecked every five seconds.","parameters":[{"name":"Last-Event-ID","in":"header","schema":{"type":"string","pattern":"^[0-9]{1,20}$"}}],"responses":{"200":{"description":"snapshot events; id is the current revision","content":{"text/event-stream":{"schema":{"type":"string"}}}}}}});
    for (path, input, output, status) in [
        (
            "/api/v1/storage/mounts/plans",
            serde_json::to_value(schema_for!(limeos_domain::StorageMountSelection)).unwrap(),
            serde_json::to_value(schema_for!(limeos_domain::PlannedStorageMount)).unwrap(),
            "201",
        ),
        (
            "/api/v1/storage/mounts/plans/{id}/approval",
            serde_json::to_value(schema_for!(limeos_contracts::ApprovalInput)).unwrap(),
            serde_json::to_value(schema_for!(limeos_domain::PlanApproval)).unwrap(),
            "200",
        ),
        (
            "/api/v1/storage/mounts/jobs",
            serde_json::to_value(schema_for!(limeos_contracts::QueueStorageMountInput)).unwrap(),
            serde_json::to_value(schema_for!(limeos_domain::StorageMountJob)).unwrap(),
            "202",
        ),
        (
            "/api/v1/storage/targets/plans",
            serde_json::to_value(schema_for!(limeos_domain::StorageContract)).unwrap(),
            serde_json::to_value(schema_for!(limeos_domain::PlannedStorageTargets)).unwrap(),
            "201",
        ),
        (
            "/api/v1/storage/targets/plans/{id}/approval",
            serde_json::to_value(schema_for!(limeos_contracts::ApprovalInput)).unwrap(),
            serde_json::to_value(schema_for!(limeos_domain::PlanApproval)).unwrap(),
            "200",
        ),
        (
            "/api/v1/storage/targets/jobs",
            serde_json::to_value(schema_for!(limeos_contracts::QueueStorageTargetsInput)).unwrap(),
            serde_json::to_value(schema_for!(limeos_domain::StorageTargetJob)).unwrap(),
            "202",
        ),
        (
            "/api/v1/storage/plans",
            serde_json::to_value(schema_for!(limeos_domain::StorageSetupInput)).unwrap(),
            serde_json::to_value(schema_for!(limeos_domain::PlannedStorageSetup)).unwrap(),
            "201",
        ),
        (
            "/api/v1/storage/plans/{id}/approval",
            serde_json::to_value(schema_for!(limeos_contracts::ApprovalInput)).unwrap(),
            serde_json::to_value(schema_for!(limeos_domain::PlanApproval)).unwrap(),
            "200",
        ),
        (
            "/api/v1/compose/plans",
            serde_json::to_value(schema_for!(limeos_domain::ComposeSelection)).unwrap(),
            serde_json::to_value(schema_for!(limeos_domain::PlannedCompose)).unwrap(),
            "201",
        ),
        (
            "/api/v1/compose/plans/{id}/approval",
            serde_json::to_value(schema_for!(limeos_contracts::ApprovalInput)).unwrap(),
            serde_json::to_value(schema_for!(limeos_domain::PlanApproval)).unwrap(),
            "200",
        ),
        (
            "/api/v1/container/restart/plans",
            serde_json::to_value(schema_for!(limeos_contracts::RestartInput)).unwrap(),
            serde_json::to_value(schema_for!(limeos_domain::PlannedRestart)).unwrap(),
            "201",
        ),
        (
            "/api/v1/container/restart/plans/{id}/approval",
            serde_json::to_value(schema_for!(limeos_contracts::ApprovalInput)).unwrap(),
            serde_json::to_value(schema_for!(limeos_domain::PlanApproval)).unwrap(),
            "200",
        ),
        (
            "/api/v1/container/restart/jobs",
            serde_json::to_value(schema_for!(limeos_contracts::QueueRestartInput)).unwrap(),
            serde_json::to_value(schema_for!(limeos_domain::RestartJob)).unwrap(),
            "202",
        ),
        (
            "/api/v1/stack/plans",
            serde_json::to_value(schema_for!(limeos_contracts::StackInput)).unwrap(),
            serde_json::to_value(schema_for!(limeos_domain::PlannedStackAction)).unwrap(),
            "201",
        ),
        (
            "/api/v1/stack/plans/{id}/approval",
            serde_json::to_value(schema_for!(limeos_contracts::ApprovalInput)).unwrap(),
            serde_json::to_value(schema_for!(limeos_domain::PlanApproval)).unwrap(),
            "200",
        ),
        (
            "/api/v1/stack/jobs",
            serde_json::to_value(schema_for!(limeos_contracts::QueueStackInput)).unwrap(),
            serde_json::to_value(schema_for!(limeos_domain::StackJob)).unwrap(),
            "202",
        ),
    ] {
        let mut operation = serde_json::json!({"security":[{"session":[]}],"parameters":[
            {"name":"Origin","in":"header","required":true,"schema":{"type":"string"}},
            {"name":"X-CSRF-Token","in":"header","required":true,"schema":{"type":"string","minLength":64,"maxLength":64}}
        ],"requestBody":{"required":true,"content":{"application/json":{"schema":input}}},"responses":{}});
        operation["responses"][status] = serde_json::json!({"description":"Durable plan, approval or queued job","content":{"application/json":{"schema":output}}});
        openapi["paths"][path] = serde_json::json!({"post":operation});
    }
    openapi["paths"]["/api/v1/container/restart/jobs/{id}"] = serde_json::json!({"get":{"security":[{"session":[]}],"parameters":[{"name":"after","in":"query","schema":{"type":"integer","minimum":0,"default":0}}],"responses":{"200":{"description":"Owner-scoped durable progress, at most 128 events","content":{"application/json":{"schema":schema_for!(limeos_contracts::JobProgress)}}}}}});
    openapi["paths"]["/api/v1/stack/jobs/{id}"] = serde_json::json!({"get":{"security":[{"session":[]}],"parameters":[{"name":"after","in":"query","schema":{"type":"integer","minimum":0,"default":0}}],"responses":{"200":{"description":"Owner-scoped durable progress, at most 128 events, with per-member executor outcomes once recorded","content":{"application/json":{"schema":schema_for!(limeos_contracts::StackJobProgress)}}}}}});
    openapi["paths"]["/api/v1/jobs/needs-attention"] = serde_json::json!({"get":{"security":[{"session":[]}],
        "description":"Administrator session only; a token, API key or any Authorization header is refused with 403, and an operator or viewer session gets 403. Read-only: every principal's container and stack jobs that hold locks until resolved (needs_intervention, plus running or verifying left behind by an earlier core generation), oldest first, at most 64, with the owner's username. Evidence and decisions stay on /api/v1/jobs/{id}/resolution.",
        "responses":{"200":{"description":"Stuck jobs of all principals","content":{"application/json":{"schema":schema_for!(Vec<limeos_domain::NeedsAttentionJob>)}}}}}});
    openapi["paths"]["/api/v1/jobs/{id}/resolution"] = serde_json::json!({
        "get":{"security":[{"session":[]}],
            "description":"Administrator session only; a token, API key or any Authorization header is refused with 403. Fresh read-only inspection of exactly the job's targets (never a start, stop or restart): plan against measurement per target, the receipt's per-member outcomes, the evidence digest to send back, and any pending request or completed record. 503 when the executor cannot be read.",
            "responses":{"200":{"description":"Evidence and any existing record","content":{"application/json":{"schema":schema_for!(limeos_contracts::ResolutionView)}}}}},
        "post":{"security":[{"session":[]}],
            "description":"Administrator session with CSRF, the Origin rule and the administrator's own password (fresh re-authentication; wrong password is 401 unauthenticated, attempts share the login rate limit, 429). Records the decision, asks the executor to resolve its own rows, then releases the job, in that order. 200 resolved; 202 pending (recorded, job still held, core retries, repeating the same POST is safe); 409 with status stale_evidence (re-fetch) or unsupported_declaration (blocking lists the differing containers); other 409 when another request is open or the job is not awaiting resolution; 403 for a non-administrator.",
            "parameters":[
                {"name":"Origin","in":"header","required":true,"schema":{"type":"string"}},
                {"name":"X-CSRF-Token","in":"header","required":true,"schema":{"type":"string","minLength":64,"maxLength":64}}
            ],
            "requestBody":{"required":true,"content":{"application/json":{"schema":schema_for!(limeos_contracts::ResolveJobInput)}}},
            "responses":{
                "200":{"description":"Resolved","content":{"application/json":{"schema":schema_for!(limeos_contracts::ResolveJobResponse)}}},
                "202":{"description":"Recorded and held; the executor could not confirm","content":{"application/json":{"schema":schema_for!(limeos_contracts::ResolveJobResponse)}}}}
        }
    });
    openapi["paths"]["/api/v1/jobs/{id}/resolution"]["post"]["responses"]["409"] = serde_json::json!({
        "description":"stale_evidence or unsupported_declaration (this body), or a structured safe error",
        "content":{"application/json":{"schema":schema_for!(limeos_contracts::ResolveJobResponse)}}
    });
    openapi["paths"]["/api/v1/stack/jobs"]["get"] = serde_json::json!({"security":[{"session":[]}],"responses":{"200":{"description":"Latest 32 owned, currently authorized whole-stack jobs","content":{"application/json":{"schema":schema_for!(Vec<limeos_domain::StackJob>)}}}}});
    openapi["paths"]["/api/v1/stack/plans"]["post"]["description"] = serde_json::json!(
        "Whole-stack start, stop or restart of a registered Compose project with fresh sources and no inventory issue. Binds the exact full member IDs and states, excluding one-off containers. Never up, down or recreate; no Compose file is read or written."
    );
    openapi["paths"]["/api/v1/compose/plans/{id}"] = serde_json::json!({"get":{"security":[{"session":[]}],"responses":{"200":{"description":"Current owner-scoped preview; no host effects or expiry refresh","content":{"application/json":{"schema":schema_for!(limeos_domain::PlannedCompose)}}}}}});
    openapi["paths"]["/api/v1/compose/plans"]["post"]["description"] = serde_json::json!(
        "Select from the protected catalog and persist a non-executable preview. Deployment requires a new P04 operation and fresh approval."
    );
    openapi["paths"]["/api/v1/container/restart/jobs"]["get"] = serde_json::json!({"security":[{"session":[]}],"responses":{"200":{"description":"Latest 32 owned, currently authorized jobs","content":{"application/json":{"schema":schema_for!(Vec<limeos_domain::RestartJob>)}}}}});
    for path in [
        "/api/v1/compose/plans/{id}/cancel",
        "/api/v1/storage/targets/plans/{id}/cancel",
        "/api/v1/storage/mounts/plans/{id}/cancel",
        "/api/v1/storage/mounts/jobs/{id}/cancel",
        "/api/v1/storage/targets/jobs/{id}/cancel",
        "/api/v1/storage/plans/{id}/cancel",
        "/api/v1/container/restart/plans/{id}/cancel",
        "/api/v1/container/restart/jobs/{id}/cancel",
        "/api/v1/stack/plans/{id}/cancel",
        "/api/v1/stack/jobs/{id}/cancel",
    ] {
        openapi["paths"][path] = serde_json::json!({"post":{"security":[{"session":[]}],"parameters":[
            {"name":"Origin","in":"header","required":true,"schema":{"type":"string"}},
            {"name":"X-CSRF-Token","in":"header","required":true,"schema":{"type":"string","minLength":64,"maxLength":64}}
        ],"responses":{"204":{"description":"Canceled before dispatch; in-flight cancellation conflicts"}}}});
    }
    openapi["paths"]["/api/v1/storage/mounts/targets"] = serde_json::json!({"get":{"security":[{"session":[]}],"description":"Bounded root-managed ext4 target configuration; current storage authority checked before and after read. This is not live readiness or execution authority.","responses":{"200":{"description":"Managed target choices","content":{"application/json":{"schema":schema_for!(limeos_domain::StorageMountCatalog)}}}}}});
    openapi["paths"]["/api/v1/storage/mounts/plans"]["post"]["description"] = serde_json::json!(
        "Create a human-approved nonpersistent ext4 mount proposal by protected managed target ID. Fresh physical/consumer evidence and independent executor policy are required; caller-supplied paths, contracts and options are refused."
    );
    openapi["paths"]["/api/v1/storage/mounts/plans/{id}"] = serde_json::json!({"get":{"security":[{"session":[]}],"responses":{"200":{"description":"Owner-scoped saved proposal; database read only, no hardware or renewed expiry","content":{"application/json":{"schema":schema_for!(limeos_domain::PlannedStorageMount)}}}}}});
    openapi["paths"]["/api/v1/storage/mounts/jobs"]["get"] = serde_json::json!({"security":[{"session":[]}],"responses":{"200":{"description":"Latest 32 currently authorized owned mount jobs; database only","content":{"application/json":{"schema":schema_for!(Vec<limeos_domain::StorageMountJob>)}}}}});
    openapi["paths"]["/api/v1/storage/mounts/jobs/{id}"] = serde_json::json!({"get":{"security":[{"session":[]}],"responses":{"200":{"description":"Owned durable mount status; database only, no dispatch","content":{"application/json":{"schema":schema_for!(limeos_domain::StorageMountJob)}}}}}});
    openapi["paths"]["/api/v1/storage/inventory"] = serde_json::json!({"get":{"security":[{"session":[]}],"description":"Fresh, root-reader evidence under a four-second deadline. Administrator storage grant required; fstab credentials stay private.","responses":{"200":{"description":"Disk/mount/fstab identity and revision","content":{"application/json":{"schema":schema_for!(limeos_domain::StorageInventoryView)}}}}}});
    openapi["paths"]["/api/v1/storage/plans/{id}"] = serde_json::json!({"get":{"security":[{"session":[]}],"responses":{"200":{"description":"Owner-scoped durable preview; no inspection or expiry refresh","content":{"application/json":{"schema":schema_for!(limeos_domain::PlannedStorageSetup)}}}}}});
    openapi["paths"]["/api/v1/storage/plans"]["post"]["description"] = serde_json::json!(
        "Persist guided assignments and a generated managed fstab section against fresh evidence. Preview only; storage execution requires a new operation version and new approval."
    );
    openapi["paths"]["/api/v1/storage/targets/plans"]["post"]["description"] = serde_json::json!(
        "Create a version-2 executable proposal to prepare empty mount directories only. Fresh protected target and device evidence plus an independent exact UUID/path ceiling are required. Existing preview approvals do not authorize this operation."
    );
    for (path, schema, description) in [
        (
            "/api/v1/storage/targets/plans/{id}",
            serde_json::to_value(schema_for!(limeos_domain::PlannedStorageTargets)).unwrap(),
            "Owner-scoped durable target proposal; read only, no expiry refresh or inspection",
        ),
        (
            "/api/v1/storage/targets/jobs/{id}",
            serde_json::to_value(schema_for!(limeos_domain::StorageTargetJob)).unwrap(),
            "Owner-scoped durable target job; read only, no dispatch or reconciliation",
        ),
    ] {
        openapi["paths"][path] = serde_json::json!({"get":{"security":[{"session":[]}],"responses":{"200":{"description":description,"content":{"application/json":{"schema":schema}}}}}});
    }
    for suffix in [
        "/plans",
        "/plans/{id}/approval",
        "/plans/{id}/cancel",
        "/jobs",
        "/jobs/{id}",
        "/jobs/{id}/cancel",
    ] {
        let legacy = format!("/api/v1/container/restart{suffix}");
        let generic = format!("/api/v1/container{suffix}");
        openapi["paths"][&generic] = openapi["paths"][&legacy].clone();
    }
    openapi["paths"]["/api/v1/container/plans"]["post"]["requestBody"]["content"]["application/json"]
        ["schema"] = serde_json::to_value(schema_for!(limeos_contracts::ContainerInput)).unwrap();
    openapi["paths"]["/api/v1/containers/{id}/logs"] = serde_json::json!({"get":{"security":[{"session":[]}],"description":"Current management grant and independent log ceiling required. Non-following tail; at most 24 KiB after framing/control/credential filtering.","parameters":[{"name":"tail","in":"query","schema":{"type":"integer","minimum":1,"maximum":200,"default":100}}],"responses":{"200":{"description":"Bounded selected-container log text","content":{"application/json":{"schema":schema_for!(limeos_domain::ContainerLogs)}}}}}});
    for (path, item) in openapi["paths"].as_object_mut().unwrap() {
        if path.contains("{id}") {
            for method in item.as_object_mut().unwrap().values_mut() {
                if method.get("parameters").is_none() {
                    method["parameters"] = serde_json::json!([]);
                }
                method["parameters"].as_array_mut().unwrap().push(serde_json::json!({"name":"id","in":"path","required":true,"schema":{"type":"string","pattern":"^[a-f0-9]{64}$"}}));
            }
        }
    }
    for path in openapi["paths"].as_object_mut().unwrap().values_mut() {
        for operation in path.as_object_mut().unwrap().values_mut() {
            for code in ["400", "401", "403", "409", "429", "503"] {
                // A route may document its own body for a code (resolution 409).
                if operation["responses"].get(code).is_some() {
                    continue;
                }
                operation["responses"][code] = serde_json::json!({
                    "description":"Structured safe error", "content":{"application/json":{"schema":schema_for!(limeos_contracts::ErrorEnvelope)}}
                });
            }
        }
    }
    openapi["paths"]["/api/v1/auth/login"]["post"]["parameters"] = serde_json::json!([
        {"name":"Origin","in":"header","required":true,"schema":{"type":"string"}}
    ]);
    openapi["paths"]["/api/v1/auth/logout"]["post"]["parameters"] = serde_json::json!([
        {"name":"Origin","in":"header","required":true,"schema":{"type":"string"}},
        {"name":"X-CSRF-Token","in":"header","required":true,"schema":{"type":"string","minLength":64,"maxLength":64}}
    ]);
    let mut definitions = serde_json::Map::new();
    lift_definitions(&mut openapi, &mut definitions);
    openapi["components"] = serde_json::json!({"schemas":definitions,"securitySchemes":{"session":{"type":"apiKey","in":"cookie","name":"__Host-limeos"}}});
    for path in ["/api/v1/auth/session", "/api/v1/auth/logout"] {
        let method = if path.ends_with("logout") {
            "post"
        } else {
            "get"
        };
        openapi["paths"][path][method]["security"] = serde_json::json!([{"session":[]}]);
    }
    std::fs::write(
        dir.join("authority-v1.sql"),
        include_str!("../../../crates/persistence/src/schema.sql"),
    )
    .unwrap();
    std::fs::write(
        dir.join("authority-v2.sql"),
        concat!(
            include_str!("../../../crates/persistence/src/schema.sql"),
            include_str!("../../../crates/persistence/src/migration-v2.sql")
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("authority-v3.sql"),
        concat!(
            include_str!("../../../crates/persistence/src/schema.sql"),
            include_str!("../../../crates/persistence/src/migration-v2.sql"),
            include_str!("../../../crates/persistence/src/migration-v3.sql")
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("authority-v4.sql"),
        concat!(
            include_str!("../../../crates/persistence/src/schema.sql"),
            include_str!("../../../crates/persistence/src/migration-v2.sql"),
            include_str!("../../../crates/persistence/src/migration-v3.sql"),
            include_str!("../../../crates/persistence/src/migration-v4.sql")
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("authority-v5.sql"),
        format!(
            "{}{}{}{}{}",
            include_str!("../../../crates/persistence/src/schema.sql"),
            include_str!("../../../crates/persistence/src/migration-v2.sql"),
            include_str!("../../../crates/persistence/src/migration-v3.sql"),
            include_str!("../../../crates/persistence/src/migration-v4.sql"),
            include_str!("../../../crates/persistence/src/migration-v5.sql")
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("authority-v6.sql"),
        format!(
            "{}{}{}{}{}{}",
            include_str!("../../../crates/persistence/src/schema.sql"),
            include_str!("../../../crates/persistence/src/migration-v2.sql"),
            include_str!("../../../crates/persistence/src/migration-v3.sql"),
            include_str!("../../../crates/persistence/src/migration-v4.sql"),
            include_str!("../../../crates/persistence/src/migration-v5.sql"),
            include_str!("../../../crates/persistence/src/migration-v6.sql")
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("authority-v7.sql"),
        concat!(
            include_str!("../../../crates/persistence/src/schema.sql"),
            include_str!("../../../crates/persistence/src/migration-v2.sql"),
            include_str!("../../../crates/persistence/src/migration-v3.sql"),
            include_str!("../../../crates/persistence/src/migration-v4.sql"),
            include_str!("../../../crates/persistence/src/migration-v5.sql"),
            include_str!("../../../crates/persistence/src/migration-v6.sql"),
            include_str!("../../../crates/persistence/src/migration-v7.sql")
        ),
    )
    .unwrap();
    let authority = std::fs::read_to_string(dir.join("authority-v7.sql")).unwrap();
    std::fs::write(
        dir.join("authority-v8.sql"),
        format!(
            "{}{}",
            authority,
            include_str!("../../../crates/persistence/src/migration-v8.sql")
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("authority-v9.sql"),
        format!(
            "{}{}",
            std::fs::read_to_string(dir.join("authority-v8.sql")).unwrap(),
            include_str!("../../../crates/persistence/src/migration-v9.sql")
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("authority-v10.sql"),
        format!(
            "{}{}",
            std::fs::read_to_string(dir.join("authority-v9.sql")).unwrap(),
            include_str!("../../../crates/persistence/src/migration-v10.sql")
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("authority-v13.sql"),
        format!(
            "{}{}{}{}",
            std::fs::read_to_string(dir.join("authority-v10.sql")).unwrap(),
            include_str!("../../../crates/persistence/src/migration-v11.sql"),
            include_str!("../../../crates/persistence/src/migration-v12.sql"),
            include_str!("../../../crates/persistence/src/migration-v13.sql")
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("authority-v14.sql"),
        format!(
            "{}{}",
            std::fs::read_to_string(dir.join("authority-v13.sql")).unwrap(),
            include_str!("../../../crates/persistence/src/migration-v14.sql")
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("authority-v15.sql"),
        format!(
            "{}{}",
            std::fs::read_to_string(dir.join("authority-v14.sql")).unwrap(),
            include_str!("../../../crates/persistence/src/migration-v15.sql")
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("authority-v16.sql"),
        format!(
            "{}{}",
            std::fs::read_to_string(dir.join("authority-v15.sql")).unwrap(),
            include_str!("../../../crates/persistence/src/migration-v16.sql")
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("openapi.json"),
        serde_json::to_string_pretty(&openapi).unwrap() + "\n",
    )
    .unwrap();
}

fn push_typescript<T: TS>(output: &mut String) {
    output.push_str("export ");
    for line in T::decl().lines() {
        output.push_str(line.trim_end());
        output.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiline_schedule_declarations_have_no_trailing_whitespace() {
        let mut output = String::new();
        push_typescript::<limeos_domain::automation::Schedule>(&mut output);
        assert!(output.starts_with("export type Schedule = "));
        assert!(output.contains("settled_through"));
        assert!(output.ends_with('\n'));
        assert!(output.lines().all(|line| line == line.trim_end()));
    }
}
