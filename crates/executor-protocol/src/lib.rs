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
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
