use limeos_contracts::VERSION;
use limeos_domain::{Error, ErrorCode, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Ceiling {
    pub version: u16,
    pub core_uid: u32,
    pub allow_health: bool,
    #[serde(default)]
    pub allow_host_read: bool,
    #[serde(default)]
    pub allow_storage_read: bool,
    #[serde(default)]
    pub allow_stack_read: bool,
    #[serde(default)]
    pub allow_container_health_read: bool,
    #[serde(default)]
    pub allow_container_read: bool,
    #[serde(default)]
    pub allow_restart: bool,
    #[serde(default)]
    pub allow_start: bool,
    #[serde(default)]
    pub allow_stop: bool,
    #[serde(default)]
    pub allow_container_logs: bool,
    /// Whole-stack start/stop/restart, still bounded by the per-action flags,
    /// `managed_stacks` and every target in `managed_containers`.
    #[serde(default)]
    pub allow_stack: bool,
    #[serde(default)]
    pub managed_containers: Vec<String>,
    /// Compose project names.
    #[serde(default)]
    pub managed_stacks: Vec<String>,
}
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    ContainerHealth {
        version: u16,
        #[schemars(length(min = 64, max = 64))]
        nonce: String,
    },
    StackInventory {
        version: u16,
    },
    StorageMountCatalog {
        version: u16,
    },
    InspectStorageMount {
        version: u16,
        selection: limeos_domain::StorageMountSelection,
    },
    ExecuteStorageMount {
        version: u16,
        plan: Box<limeos_domain::StorageMountEffectPlan>,
    },
    StorageMountReceipt {
        version: u16,
        action: String,
        digest: String,
    },
    /// Refine only an existing accepted, reaped effect after fresh verification.
    /// This request cannot launch an effect or renew execution authority.
    ReconcileStorageMount {
        version: u16,
        action: String,
        digest: String,
    },
    InspectMountedStorage {
        version: u16,
        plan: Box<limeos_domain::StorageMountEffectPlan>,
    },
    ContainerStorageInventory {
        version: u16,
    },
    InspectStorageTargets {
        version: u16,
        contract: limeos_domain::StorageContract,
    },
    PrepareStorageTargets {
        version: u16,
        plan: Box<limeos_domain::TargetPreparationPlan>,
    },
    StorageTargetReceipt {
        version: u16,
        action: String,
        digest: String,
    },
    StorageInventory {
        version: u16,
    },
    Health {
        version: u16,
    },
    Observe {
        version: u16,
        source: limeos_contracts::Source,
        active: bool,
    },
    Inspect {
        version: u16,
        resource: String,
    },
    ContainerLogs {
        version: u16,
        resource: String,
        options: limeos_domain::LogOptions,
    },
    Restart {
        version: u16,
        request: limeos_domain::RestartRequest,
    },
    ExecuteContainer {
        version: u16,
        request: limeos_domain::ContainerRequest,
    },
    RestartReceipt {
        version: u16,
        action: String,
        digest: String,
    },
    VerifyRestart {
        version: u16,
        action: String,
        digest: String,
        after: limeos_domain::ContainerSnapshot,
    },
    /// Fresh Engine listing of one Compose project's ordinary members.
    InspectStack {
        version: u16,
        project: String,
    },
    ExecuteStack {
        version: u16,
        request: limeos_domain::StackRequest,
    },
    StackReceipt {
        version: u16,
        action: String,
        digest: String,
    },
    /// Fresh inspections of the plan's targets; never launches an effect.
    VerifyStack {
        version: u16,
        action: String,
        digest: String,
        after: Vec<limeos_domain::ContainerSnapshot>,
    },
    /// Operator resolution of a stuck action. The executor re-inspects the
    /// action's targets read-only and, only if the result still hashes to
    /// `evidence_digest`, moves its own rows to `resolved` so the resource
    /// frees. Idempotent by action. Never issues an Engine start, stop or
    /// restart.
    ResolveAction {
        version: u16,
        kind: limeos_domain::ResolveKind,
        action: String,
        /// Digest of the stored plan, as for the receipt lookups.
        digest: String,
        evidence_digest: String,
        declared: limeos_domain::DeclaredOutcome,
        /// The resolving administrator's principal ID, recorded in the row.
        resolved_by: String,
    },
}
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_health: Option<limeos_domain::automation::ContainerHealthCohort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_cohort: Option<limeos_domain::deployment::discovery::DiscoveryCohort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_mount_catalog: Option<limeos_domain::StorageMountCatalog>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_mount_inspection: Option<limeos_domain::StorageMountInspection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_mount: Option<limeos_domain::StorageMountReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mounted_storage: Option<limeos_domain::StorageMountedSnapshot>,
    pub version: u16,
    pub ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observations: Option<limeos_contracts::ObservationBatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspection: Option<limeos_domain::ContainerSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart: Option<limeos_domain::RestartReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack: Option<limeos_domain::StackReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_members: Option<limeos_domain::StackSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<limeos_domain::ResolveReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logs: Option<limeos_domain::ContainerLogs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorCode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<limeos_domain::StorageInventory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_targets: Option<limeos_domain::StorageTargetSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_preparation: Option<limeos_domain::TargetPreparationReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_storage: Option<limeos_domain::ContainerStorageInventory>,
}
/// Independent, root-owned authority for the one closed nonpersistent mount.
#[derive(Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageMountCeiling {
    pub version: u16,
    pub core_uid: u32,
    pub allow_mount: bool,
    pub managed_targets: Vec<ManagedStorageMount>,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ManagedStorageMount {
    pub target_id: String,
    pub filesystem_uuid: String,
    pub mountpoint: String,
}
impl StorageMountCeiling {
    pub fn configuration_valid(&self) -> bool {
        if self.managed_targets.len() > 32 {
            return false;
        }
        let targets = StorageTargetCeiling {
            version: self.version,
            core_uid: self.core_uid,
            allow_prepare_targets: false,
            managed_targets: self
                .managed_targets
                .iter()
                .map(|target| ManagedStorageTarget {
                    filesystem_uuid: target.filesystem_uuid.clone(),
                    mountpoint: target.mountpoint.clone(),
                })
                .collect(),
        };
        let mut ids = std::collections::BTreeSet::new();
        targets.configuration_valid()
            && self.managed_targets.iter().all(|target| {
                limeos_domain::StorageMountSelection {
                    target_id: target.target_id.clone(),
                }
                .validate()
                .is_ok()
                    && ids.insert(&target.target_id)
            })
    }
    pub fn authorize_peer(&self, uid: u32) -> Result<()> {
        if !self.configuration_valid() || uid != self.core_uid {
            return Err(Error(ErrorCode::Forbidden));
        }
        Ok(())
    }
    pub fn authorize_selection(
        &self,
        uid: u32,
        selection: &limeos_domain::StorageMountSelection,
    ) -> Result<&ManagedStorageMount> {
        self.authorize_peer(uid)?;
        selection.validate()?;
        self.managed_targets
            .iter()
            .find(|target| target.target_id == selection.target_id)
            .ok_or(Error(ErrorCode::Forbidden))
    }
    pub fn authorize_plan(
        &self,
        uid: u32,
        plan: &limeos_domain::StorageMountEffectPlan,
        at: i64,
    ) -> Result<()> {
        let target = self.authorize_selection(
            uid,
            &limeos_domain::StorageMountSelection {
                target_id: plan.device.id.clone(),
            },
        )?;
        if !self.allow_mount
            || target.filesystem_uuid != plan.device.filesystem_uuid
            || target.mountpoint != plan.device.mountpoint
        {
            return Err(Error(ErrorCode::Forbidden));
        }
        plan.validate(at)
    }
}

/// A separate root-owned ceiling for the dormant directory-preparation service.
/// Exact UUID/path pairs constrain the core even if its authority is compromised.
#[derive(Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageTargetCeiling {
    pub version: u16,
    pub core_uid: u32,
    pub allow_prepare_targets: bool,
    pub managed_targets: Vec<ManagedStorageTarget>,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ManagedStorageTarget {
    pub filesystem_uuid: String,
    pub mountpoint: String,
}
impl StorageTargetCeiling {
    pub fn configuration_valid(&self) -> bool {
        let mut pairs = std::collections::BTreeSet::new();
        let mut paths = std::collections::BTreeSet::new();
        self.version == VERSION
            && self.core_uid != 0
            && self.managed_targets.len() <= 32
            && self.managed_targets.iter().all(|t| {
                !t.filesystem_uuid.is_empty()
                    && t.filesystem_uuid.len() <= 128
                    && t.filesystem_uuid
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                    && t.mountpoint.starts_with("/mnt/")
                    && t.mountpoint.len() <= 512
                    && !t.mountpoint.chars().any(char::is_control)
                    && !t.mountpoint.contains('\\')
                    && t.mountpoint
                        .split('/')
                        .skip(1)
                        .all(|p| !p.is_empty() && p != "." && p != "..")
                    && pairs.insert((&t.filesystem_uuid, &t.mountpoint))
                    && paths.insert(&t.mountpoint)
            })
    }
    pub fn authorize_contract(
        &self,
        uid: u32,
        contract: &limeos_domain::StorageContract,
    ) -> Result<()> {
        if !self.configuration_valid() || uid != self.core_uid || !self.allow_prepare_targets {
            return Err(Error(ErrorCode::Forbidden));
        }
        contract.validate()?;
        if contract.devices.iter().any(|d| {
            !self
                .managed_targets
                .iter()
                .any(|t| t.filesystem_uuid == d.filesystem_uuid && t.mountpoint == d.mountpoint)
        }) {
            return Err(Error(ErrorCode::Forbidden));
        }
        Ok(())
    }
}
impl Ceiling {
    pub fn configuration_valid(&self) -> bool {
        self.version == VERSION
            && self.managed_containers.len() <= 64
            && self
                .managed_containers
                .iter()
                .all(|id| limeos_domain::opaque_id(id))
            && self.managed_stacks.len() <= 16
            && self
                .managed_stacks
                .iter()
                .all(|project| limeos_domain::stack_project(project))
            && (!self.allow_stack || (self.allow_restart || self.allow_start || self.allow_stop))
            && (!(self.can_write() || self.allow_container_logs) || self.allow_container_read)
            && (!(self.allow_storage_read
                || self.allow_stack_read
                || self.allow_container_health_read)
                || (!self.can_write()
                    && !self.allow_container_read
                    && !self.allow_host_read
                    && !self.allow_container_logs))
    }
    pub fn can_write(&self) -> bool {
        self.allow_restart || self.allow_start || self.allow_stop || self.allow_stack
    }
    pub fn validate(&self, peer_uid: u32, request: &Request) -> Result<Receipt> {
        if !self.configuration_valid() || peer_uid != self.core_uid {
            return Err(Error(ErrorCode::Forbidden));
        }
        match request {
            Request::ContainerHealth { version, nonce }
                if *version == VERSION
                    && self.allow_container_health_read
                    && limeos_domain::opaque_id(nonce) =>
            {
                Ok(Receipt::empty())
            }
            Request::StackInventory { version } if *version == VERSION && self.allow_stack_read => {
                Ok(Receipt::empty())
            }
            Request::ContainerStorageInventory { version }
                if *version == VERSION && self.allow_container_read =>
            {
                Ok(Receipt::empty())
            }
            Request::StorageInventory { version }
                if *version == VERSION && self.allow_storage_read =>
            {
                Ok(Receipt::empty())
            }
            Request::Health { version } if *version == VERSION && self.allow_health => {
                Ok(Receipt::empty())
            }
            Request::Observe {
                version, source, ..
            } if *version == VERSION
                && ((*source == limeos_contracts::Source::Host && self.allow_host_read)
                    || (*source == limeos_contracts::Source::Docker
                        && self.allow_container_read)) =>
            {
                Ok(Receipt::empty())
            }
            Request::Inspect { version, resource }
                if *version == VERSION
                    && self.allow_container_read
                    && limeos_domain::opaque_id(
                        resource.strip_prefix("container:").unwrap_or_default(),
                    ) =>
            {
                Ok(Receipt::empty())
            }
            Request::Restart { version, request } if *version == VERSION => {
                if request.plan.operation != limeos_domain::ContainerAction::Restart {
                    return Err(Error(ErrorCode::InvalidInput));
                }
                self.authorize_container(
                    peer_uid,
                    &request.plan.expected.resource,
                    request.plan.operation,
                )?;
                Ok(Receipt::empty())
            }
            Request::ContainerLogs {
                version,
                resource,
                options,
            } if *version == VERSION => {
                options.validate()?;
                let id = resource.strip_prefix("container:").unwrap_or_default();
                if !self.allow_container_logs
                    || !limeos_domain::opaque_id(id)
                    || !self.managed_containers.iter().any(|allowed| allowed == id)
                {
                    return Err(Error(ErrorCode::Forbidden));
                }
                Ok(Receipt::empty())
            }
            Request::ExecuteContainer { version, request } if *version == VERSION => {
                self.authorize_container(
                    peer_uid,
                    &request.plan.expected.resource,
                    request.plan.operation,
                )?;
                Ok(Receipt::empty())
            }
            Request::InspectStack { version, project }
                if *version == VERSION
                    && self.allow_container_read
                    && limeos_domain::stack_project(project) =>
            {
                Ok(Receipt::empty())
            }
            Request::ExecuteStack { version, request } if *version == VERSION => {
                self.authorize_stack(peer_uid, &request.plan.expected, request.plan.operation)?;
                Ok(Receipt::empty())
            }
            Request::StackReceipt {
                version,
                action,
                digest,
            }
            | Request::VerifyStack {
                version,
                action,
                digest,
                ..
            } if *version == VERSION
                && self.allow_stack
                && limeos_domain::opaque_id(action)
                && limeos_domain::opaque_id(digest) =>
            {
                if let Request::VerifyStack { after, .. } = request
                    && after.len() > limeos_domain::MAX_STACK_MEMBERS
                {
                    return Err(Error(ErrorCode::InvalidInput));
                }
                Ok(Receipt::empty())
            }
            Request::ResolveAction {
                version,
                kind,
                action,
                digest,
                evidence_digest,
                resolved_by,
                ..
            } if *version == VERSION
                && match kind {
                    limeos_domain::ResolveKind::Container => self.can_write(),
                    limeos_domain::ResolveKind::Stack => self.allow_stack,
                }
                && [action, digest, evidence_digest, resolved_by]
                    .into_iter()
                    .all(|v| limeos_domain::opaque_id(v)) =>
            {
                Ok(Receipt::empty())
            }
            Request::RestartReceipt {
                version,
                action,
                digest,
            }
            | Request::VerifyRestart {
                version,
                action,
                digest,
                ..
            } if *version == VERSION
                && self.can_write()
                && limeos_domain::opaque_id(action)
                && limeos_domain::opaque_id(digest) =>
            {
                Ok(Receipt::empty())
            }
            _ => Err(Error(ErrorCode::InvalidInput)),
        }
    }
    /// Stack authority adds to, and never replaces, the container ceiling: the
    /// project must be listed and every target must be an allowed container.
    pub fn authorize_stack(
        &self,
        peer_uid: u32,
        expected: &limeos_domain::StackSnapshot,
        operation: limeos_domain::ContainerAction,
    ) -> Result<()> {
        if !self.configuration_valid() || peer_uid != self.core_uid || !self.allow_stack {
            return Err(Error(ErrorCode::Forbidden));
        }
        expected.validate_for(operation)?;
        let project = expected.project()?;
        for target in expected.targets(operation) {
            self.authorize_container(peer_uid, &target.resource, operation)?;
        }
        if !self.managed_stacks.iter().any(|allowed| allowed == project) {
            return Err(Error(ErrorCode::Forbidden));
        }
        Ok(())
    }
    pub fn authorize_container(
        &self,
        peer_uid: u32,
        resource: &str,
        operation: limeos_domain::ContainerAction,
    ) -> Result<()> {
        let id = resource.strip_prefix("container:").unwrap_or_default();
        if self.version != VERSION
            || peer_uid != self.core_uid
            || !match operation {
                limeos_domain::ContainerAction::Restart => self.allow_restart,
                limeos_domain::ContainerAction::Start => self.allow_start,
                limeos_domain::ContainerAction::Stop => self.allow_stop,
            }
            || !limeos_domain::opaque_id(id)
            || self.managed_containers.len() > 64
            || self
                .managed_containers
                .iter()
                .any(|id| !limeos_domain::opaque_id(id))
            || !self.managed_containers.iter().any(|allowed| allowed == id)
        {
            return Err(Error(ErrorCode::Forbidden));
        }
        Ok(())
    }
}
impl Receipt {
    pub fn empty() -> Self {
        Self {
            container_health: None,
            stack_cohort: None,
            storage_mount_catalog: None,
            storage_mount_inspection: None,
            storage_mount: None,
            mounted_storage: None,
            version: VERSION,
            ready: true,
            observations: None,
            inspection: None,
            restart: None,
            stack: None,
            stack_members: None,
            resolution: None,
            logs: None,
            error: None,
            storage: None,
            storage_targets: None,
            target_preparation: None,
            container_storage: None,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stack_cache_read_is_closed_and_independent_from_other_read_and_effect_roles() {
        let mut c: Ceiling = serde_json::from_str(
            r#"{"version":1,"core_uid":1001,"allow_health":true,"allow_stack_read":true}"#,
        )
        .unwrap();
        assert!(c.configuration_valid());
        assert!(!c.can_write());
        assert!(
            c.validate(1001, &Request::StackInventory { version: VERSION })
                .is_ok()
        );
        assert!(
            c.validate(1002, &Request::StackInventory { version: VERSION })
                .is_err()
        );
        assert!(
            c.validate(1001, &Request::StackInventory { version: 2 })
                .is_err()
        );
        assert!(
            c.validate(1001, &Request::StorageInventory { version: VERSION })
                .is_err()
        );
        for field in [
            "path",
            "command",
            "endpoint",
            "principal",
            "approval",
            "resources",
        ] {
            let mut wire = serde_json::json!({"operation":"stack_inventory","version":1});
            wire[field] = serde_json::json!("caller");
            assert!(serde_json::from_value::<Request>(wire).is_err());
        }
        c.allow_host_read = true;
        assert!(!c.configuration_valid());
        c.allow_host_read = false;
        c.allow_container_read = true;
        assert!(!c.configuration_valid());
    }

    #[test]
    fn health_capture_requires_dedicated_read_ceiling_nonce_and_current_kernel_peer() {
        let mut ceiling: Ceiling = serde_json::from_str(
            r#"{"version":1,"core_uid":1001,"allow_health":true,"allow_storage_read":true}"#,
        )
        .unwrap();
        let request = Request::ContainerHealth {
            version: VERSION,
            nonce: "a".repeat(64),
        };
        assert!(ceiling.validate(1001, &request).is_err());
        ceiling.allow_container_health_read = true;
        assert!(ceiling.validate(1001, &request).is_ok());
        assert!(!ceiling.can_write());
        assert!(ceiling.validate(1002, &request).is_err());
        for nonce in ["".into(), "short".into(), "A".repeat(64), "a".repeat(65)] {
            assert!(
                ceiling
                    .validate(
                        1001,
                        &Request::ContainerHealth {
                            version: VERSION,
                            nonce
                        }
                    )
                    .is_err()
            );
        }
        assert!(
            ceiling
                .validate(
                    1001,
                    &Request::ContainerHealth {
                        version: 2,
                        nonce: "a".repeat(64)
                    }
                )
                .is_err()
        );
        for flag in [0, 1, 2, 3] {
            let mut mixed = ceiling.clone();
            match flag {
                0 => mixed.allow_host_read = true,
                1 => mixed.allow_container_read = true,
                2 => mixed.allow_container_logs = true,
                _ => {
                    mixed.allow_container_read = true;
                    mixed.allow_restart = true;
                }
            }
            assert!(!mixed.configuration_valid());
        }
        for field in [
            "endpoint", "uid", "clock", "path", "profile", "deadline", "command", "resource",
        ] {
            let mut wire = serde_json::json!({"operation":"container_health","version":1,"nonce":"a".repeat(64)});
            wire[field] = serde_json::json!("forbidden selector");
            assert!(serde_json::from_value::<Request>(wire).is_err());
        }
    }
    #[test]
    fn container_storage_read_requires_the_container_ceiling_and_accepts_no_filters() {
        let mut ceiling: Ceiling = serde_json::from_str(
            r#"{"version":1,"core_uid":1001,"allow_health":true,"allow_container_read":true}"#,
        )
        .unwrap();
        let request = Request::ContainerStorageInventory { version: VERSION };
        assert!(ceiling.validate(1001, &request).is_ok());
        assert!(ceiling.validate(1002, &request).is_err());
        assert!(
            ceiling
                .validate(1001, &Request::ContainerStorageInventory { version: 2 })
                .is_err()
        );
        assert!(!ceiling.can_write());
        ceiling.allow_container_read = false;
        ceiling.allow_host_read = true;
        assert!(ceiling.validate(1001, &request).is_err());
        for field in [
            "resource",
            "paths",
            "principal",
            "managed_containers",
            "command",
        ] {
            let mut value =
                serde_json::json!({"operation":"container_storage_inventory","version":1});
            value[field] = serde_json::json!("caller supplied");
            assert!(serde_json::from_value::<Request>(value).is_err());
        }
    }
    #[test]
    fn target_preparation_ceiling_requires_exact_uuid_and_literal_path_and_core_uid() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/storage-contracts.json"
        ))
        .unwrap();
        let contract: limeos_domain::StorageContract =
            serde_json::from_value(fixture["cases"][0]["contract"].clone()).unwrap();
        let mut ceiling = StorageTargetCeiling {
            version: VERSION,
            core_uid: 104,
            allow_prepare_targets: false,
            managed_targets: contract
                .devices
                .iter()
                .map(|d| ManagedStorageTarget {
                    filesystem_uuid: d.filesystem_uuid.clone(),
                    mountpoint: d.mountpoint.clone(),
                })
                .collect(),
        };
        assert!(ceiling.configuration_valid());
        assert_eq!(
            ceiling.authorize_contract(104, &contract).unwrap_err().0,
            ErrorCode::Forbidden
        );
        ceiling.allow_prepare_targets = true;
        assert!(ceiling.authorize_contract(104, &contract).is_ok());
        assert!(ceiling.authorize_contract(0, &contract).is_err());
        assert!(ceiling.authorize_contract(105, &contract).is_err());
        ceiling.managed_targets[0].filesystem_uuid = "replacement".into();
        assert!(ceiling.authorize_contract(104, &contract).is_err());
        ceiling.managed_targets[0].filesystem_uuid = contract.devices[0].filesystem_uuid.clone();
        for path in [
            "/",
            "/mnt/Data",
            "/mnt/data/../other",
            "/mnt//data",
            "/mnt/data/",
            "/mnt/data\n",
        ] {
            ceiling.managed_targets[0].mountpoint = path.into();
            assert!(
                ceiling.authorize_contract(104, &contract).is_err(),
                "{path:?}"
            );
        }
        let read_only: Ceiling = serde_json::from_str(
            r#"{"version":1,"core_uid":104,"allow_health":true,"allow_storage_read":true}"#,
        )
        .unwrap();
        assert!(
            read_only
                .validate(
                    104,
                    &Request::InspectStorageTargets {
                        version: VERSION,
                        contract
                    }
                )
                .is_err()
        );
    }
    #[test]
    fn storage_reader_has_an_independent_read_only_ceiling_and_no_path_or_effect_input() {
        let old: Ceiling =
            serde_json::from_str(r#"{"version":1,"core_uid":1001,"allow_health":true}"#).unwrap();
        let request = Request::StorageInventory { version: VERSION };
        assert!(old.validate(1001, &request).is_err());
        let mut reader = Ceiling {
            allow_storage_read: true,
            ..old
        };
        assert!(reader.validate(1001, &request).is_ok());
        assert!(!reader.can_write());
        assert!(reader.validate(1002, &request).is_err());
        reader.allow_host_read = true;
        assert!(!reader.configuration_valid());
        for extra in ["path", "fstab", "command", "options", "principal"] {
            let raw =
                format!(r#"{{"operation":"storage_inventory","version":1,"{extra}":"attack"}}"#);
            assert!(serde_json::from_str::<Request>(&raw).is_err());
        }
        assert!(
            serde_json::from_str::<Request>(r#"{"operation":"storage_mount","version":1}"#)
                .is_err()
        );
    }
    #[test]
    fn log_ceiling_is_independent_managed_and_capped() {
        let id = "a".repeat(64);
        let mut ceiling = Ceiling {
            version: VERSION,
            core_uid: 1001,
            allow_health: true,
            allow_host_read: false,
            allow_storage_read: false,
            allow_stack_read: false,
            allow_container_health_read: false,
            allow_container_read: true,
            allow_restart: false,
            allow_start: false,
            allow_stop: false,
            allow_container_logs: false,
            allow_stack: false,
            managed_containers: vec![id.clone()],
            managed_stacks: Vec::new(),
        };
        let request = Request::ContainerLogs {
            version: VERSION,
            resource: format!("container:{id}"),
            options: limeos_domain::LogOptions { tail: 100 },
        };
        assert!(ceiling.validate(1001, &request).is_err());
        ceiling.allow_container_logs = true;
        assert!(ceiling.validate(1001, &request).is_ok());
        assert!(!ceiling.can_write());
        assert!(ceiling.validate(1002, &request).is_err());
        for tail in [0, 201, u16::MAX] {
            assert!(
                ceiling
                    .validate(
                        1001,
                        &Request::ContainerLogs {
                            version: VERSION,
                            resource: format!("container:{id}"),
                            options: limeos_domain::LogOptions { tail }
                        }
                    )
                    .is_err()
            );
        }
        ceiling.managed_containers.clear();
        assert!(ceiling.validate(1001, &request).is_err());
    }
    #[test]
    fn peer_and_ceiling_are_independent_and_no_content_or_command_is_accepted() {
        let ceiling = Ceiling {
            version: VERSION,
            core_uid: 1001,
            allow_health: true,
            allow_host_read: false,
            allow_storage_read: false,
            allow_stack_read: false,
            allow_container_health_read: false,
            allow_container_read: false,
            allow_restart: false,
            allow_start: false,
            allow_stop: false,
            allow_container_logs: false,
            allow_stack: false,
            managed_containers: Vec::new(),
            managed_stacks: Vec::new(),
        };
        assert!(
            ceiling
                .validate(1002, &Request::Health { version: VERSION })
                .is_err()
        );
        assert!(
            ceiling
                .validate(
                    1001,
                    &Request::Observe {
                        version: VERSION,
                        source: limeos_contracts::Source::Host,
                        active: true
                    }
                )
                .is_err()
        );
        let enabled = Ceiling {
            allow_host_read: true,
            ..ceiling.clone()
        };
        assert!(
            enabled
                .validate(
                    1001,
                    &Request::Observe {
                        version: VERSION,
                        source: limeos_contracts::Source::Host,
                        active: false
                    }
                )
                .is_ok()
        );
        assert!(
            enabled
                .validate(
                    1001,
                    &Request::Observe {
                        version: VERSION,
                        source: limeos_contracts::Source::Docker,
                        active: false
                    }
                )
                .is_err()
        );
        assert!(
            enabled
                .validate(
                    1002,
                    &Request::Observe {
                        version: VERSION,
                        source: limeos_contracts::Source::Host,
                        active: false
                    }
                )
                .is_err()
        );
        assert!(
            ceiling
                .validate(1001, &Request::Health { version: 99 })
                .is_err()
        );
        for extra in ["content", "command", "path", "principal"] {
            let raw = format!("{{\"operation\":\"health\",\"version\":1,\"{extra}\":\"attack\"}}");
            assert!(serde_json::from_str::<Request>(&raw).is_err());
        }
    }
    fn stack_member(n: char, running: bool) -> limeos_domain::ContainerSnapshot {
        limeos_domain::ContainerSnapshot {
            resource: format!("container:{}", n.to_string().repeat(64)),
            image: format!("sha256:{}", "b".repeat(64)),
            started_at: "2026-10-05T07:00:00.000000000Z".into(),
            running,
        }
    }
    fn stack_request(operation: limeos_domain::ContainerAction) -> Request {
        let expected = limeos_domain::StackSnapshot::new(
            "media",
            vec![
                stack_member('a', true),
                stack_member('b', false),
                stack_member('c', true),
            ],
        )
        .unwrap();
        Request::ExecuteStack {
            version: VERSION,
            request: limeos_domain::StackRequest {
                action: "d".repeat(64),
                plan_digest: "e".repeat(64),
                plan: limeos_domain::StackPlan {
                    id: "f".repeat(64),
                    version: limeos_domain::STACK_VERSION,
                    operation,
                    principal: "alice".into(),
                    grant_revision: 1,
                    catalog_stack: "registered".into(),
                    expected,
                    created_at: 100,
                    expires_at: 400,
                },
            },
        }
    }
    fn stack_ceiling() -> Ceiling {
        serde_json::from_value(serde_json::json!({
            "version": 1, "core_uid": 1001, "allow_health": true,
            "allow_container_read": true, "allow_restart": true, "allow_stop": true,
            "allow_stack": true,
            "managed_containers": ["a".repeat(64), "b".repeat(64), "c".repeat(64)],
            "managed_stacks": ["media"]
        }))
        .unwrap()
    }
    #[test]
    fn stack_authority_needs_flag_action_project_peer_and_every_target() {
        use limeos_domain::ContainerAction::*;
        let ceiling = stack_ceiling();
        assert!(ceiling.configuration_valid() && ceiling.can_write());
        assert!(ceiling.validate(1001, &stack_request(Restart)).is_ok());
        assert!(ceiling.validate(1001, &stack_request(Stop)).is_ok());
        assert!(ceiling.validate(1002, &stack_request(Restart)).is_err());
        // Start is not enabled even though the stack itself is.
        assert!(ceiling.validate(1001, &stack_request(Start)).is_err());
        let change = |f: &dyn Fn(&mut Ceiling)| {
            let mut c = stack_ceiling();
            f(&mut c);
            c
        };
        for denied in [
            change(&|c| c.allow_stack = false),
            change(&|c| c.managed_stacks = vec!["other".into()]),
            change(&|c| c.managed_stacks.clear()),
            change(&|c| c.allow_restart = false),
            // A running target missing from the container allowlist.
            change(&|c| c.managed_containers.retain(|id| !id.starts_with('c'))),
        ] {
            assert!(denied.validate(1001, &stack_request(Restart)).is_err());
        }
        // Start acts only on the stopped member, so only that ID is required.
        let mut start = stack_ceiling();
        start.allow_start = true;
        start.managed_containers = vec!["b".repeat(64)];
        assert!(start.validate(1001, &stack_request(Start)).is_ok());
        // An unmanaged non-target member does not block, an unmanaged target does.
        let mut other = start.clone();
        other.managed_containers = vec!["a".repeat(64)];
        assert!(other.validate(1001, &stack_request(Start)).is_err());
    }
    #[test]
    fn stack_configuration_is_closed_and_shadow_visible_as_a_write_capability() {
        let mut c = stack_ceiling();
        c.allow_restart = false;
        c.allow_stop = false;
        assert!(!c.configuration_valid());
        for bad in ["Media", "../x", "", "a b", &"a".repeat(65)] {
            let mut c = stack_ceiling();
            c.managed_stacks = vec![bad.into()];
            assert!(!c.configuration_valid(), "{bad:?}");
        }
        let mut many = stack_ceiling();
        many.managed_stacks = (0..17).map(|i| format!("s{i}")).collect();
        assert!(!many.configuration_valid());
        // The read-only shadow refuses a ceiling for which can_write() is true.
        let mut flag_only: Ceiling = serde_json::from_str(
            r#"{"version":1,"core_uid":1001,"allow_health":true,"allow_container_read":true}"#,
        )
        .unwrap();
        assert!(!flag_only.can_write());
        flag_only.allow_stack = true;
        assert!(flag_only.can_write());
        // Defaults keep older policy files read-only for stacks.
        assert!(
            !serde_json::from_str::<Ceiling>(r#"{"version":1,"core_uid":1,"allow_health":true}"#)
                .unwrap()
                .allow_stack
        );
    }
    #[test]
    fn resolve_action_needs_the_matching_write_authority_and_closed_fields() {
        let resolve = |kind| Request::ResolveAction {
            version: VERSION,
            kind,
            action: "d".repeat(64),
            digest: "e".repeat(64),
            evidence_digest: "f".repeat(64),
            declared: limeos_domain::DeclaredOutcome::NotDone,
            resolved_by: "a".repeat(64),
        };
        use limeos_domain::ResolveKind::{Container, Stack};
        let ceiling = stack_ceiling();
        assert!(ceiling.validate(1001, &resolve(Stack)).is_ok());
        assert!(ceiling.validate(1001, &resolve(Container)).is_ok());
        assert!(ceiling.validate(1002, &resolve(Stack)).is_err());
        let mut no_stack = stack_ceiling();
        no_stack.allow_stack = false;
        assert!(no_stack.validate(1001, &resolve(Stack)).is_err());
        assert!(no_stack.validate(1001, &resolve(Container)).is_ok());
        let mut read_only = stack_ceiling();
        read_only.allow_restart = false;
        read_only.allow_stop = false;
        read_only.allow_start = false;
        read_only.allow_stack = false;
        assert!(read_only.validate(1001, &resolve(Container)).is_err());
        let mut wire = serde_json::to_value(resolve(Stack)).unwrap();
        assert_eq!(wire["operation"], "resolve_action");
        wire["force"] = serde_json::json!(true);
        assert!(serde_json::from_value::<Request>(wire).is_err());
        let Request::ResolveAction {
            version,
            kind,
            declared,
            ..
        } = resolve(Stack)
        else {
            unreachable!()
        };
        let bad = Request::ResolveAction {
            version,
            kind,
            action: "../x".into(),
            digest: "e".repeat(64),
            evidence_digest: "f".repeat(64),
            declared,
            resolved_by: "a".repeat(64),
        };
        assert!(ceiling.validate(1001, &bad).is_err());
    }
    #[test]
    fn stack_reads_receipts_and_verification_are_independent_bounded_and_closed() {
        let ceiling = stack_ceiling();
        let receipt = |version| Request::StackReceipt {
            version,
            action: "d".repeat(64),
            digest: "e".repeat(64),
        };
        assert!(ceiling.validate(1001, &receipt(VERSION)).is_ok());
        assert!(ceiling.validate(1001, &receipt(2)).is_err());
        assert!(ceiling.validate(1002, &receipt(VERSION)).is_err());
        let mut off = stack_ceiling();
        off.allow_stack = false;
        assert!(off.validate(1001, &receipt(VERSION)).is_err());
        let verify = |n: usize| Request::VerifyStack {
            version: VERSION,
            action: "d".repeat(64),
            digest: "e".repeat(64),
            after: vec![stack_member('a', true); n],
        };
        assert!(ceiling.validate(1001, &verify(2)).is_ok());
        assert!(ceiling.validate(1001, &verify(17)).is_err());
        let inspect = |project: &str| Request::InspectStack {
            version: VERSION,
            project: project.into(),
        };
        assert!(ceiling.validate(1001, &inspect("media")).is_ok());
        assert!(ceiling.validate(1001, &inspect("../media")).is_err());
        let mut no_read = stack_ceiling();
        no_read.allow_container_read = false;
        no_read.allow_restart = false;
        no_read.allow_stop = false;
        no_read.allow_stack = false;
        assert!(no_read.validate(1001, &inspect("media")).is_err());
        let mut wire =
            serde_json::to_value(stack_request(limeos_domain::ContainerAction::Stop)).unwrap();
        assert_eq!(wire["operation"], "execute_stack");
        wire["request"]["compose_file"] = serde_json::json!("/x");
        assert!(serde_json::from_value::<Request>(wire).is_err());
    }
}
