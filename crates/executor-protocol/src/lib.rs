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
    pub allow_container_read: bool,
    #[serde(default)]
    pub allow_restart: bool,
    #[serde(default)]
    pub allow_start: bool,
    #[serde(default)]
    pub allow_stop: bool,
    #[serde(default)]
    pub allow_container_logs: bool,
    #[serde(default)]
    pub managed_containers: Vec<String>,
}
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
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
}
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub version: u16,
    pub ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observations: Option<limeos_contracts::ObservationBatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspection: Option<limeos_domain::ContainerSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart: Option<limeos_domain::RestartReceipt>,
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
            && (!(self.can_write() || self.allow_container_logs) || self.allow_container_read)
            && (!self.allow_storage_read
                || (!self.can_write()
                    && !self.allow_container_read
                    && !self.allow_host_read
                    && !self.allow_container_logs))
    }
    pub fn can_write(&self) -> bool {
        self.allow_restart || self.allow_start || self.allow_stop
    }
    pub fn validate(&self, peer_uid: u32, request: &Request) -> Result<Receipt> {
        if !self.configuration_valid() || peer_uid != self.core_uid {
            return Err(Error(ErrorCode::Forbidden));
        }
        match request {
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
                Ok(Receipt {
                    version: VERSION,
                    ready: true,
                    observations: None,
                    inspection: None,
                    restart: None,
                    logs: None,
                    error: None,
                    storage: None,
                    storage_targets: None,
                    target_preparation: None,
                    container_storage: None,
                })
            }
            Request::Observe {
                version, source, ..
            } if *version == VERSION
                && ((*source == limeos_contracts::Source::Host && self.allow_host_read)
                    || (*source == limeos_contracts::Source::Docker
                        && self.allow_container_read)) =>
            {
                Ok(Receipt {
                    version: VERSION,
                    ready: true,
                    observations: None,
                    inspection: None,
                    restart: None,
                    logs: None,
                    error: None,
                    storage: None,
                    storage_targets: None,
                    target_preparation: None,
                    container_storage: None,
                })
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
            version: VERSION,
            ready: true,
            observations: None,
            inspection: None,
            restart: None,
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
            allow_container_read: true,
            allow_restart: false,
            allow_start: false,
            allow_stop: false,
            allow_container_logs: false,
            managed_containers: vec![id.clone()],
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
            allow_container_read: false,
            allow_restart: false,
            allow_start: false,
            allow_stop: false,
            allow_container_logs: false,
            managed_containers: Vec::new(),
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
}
