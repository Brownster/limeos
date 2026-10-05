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
    pub allow_container_read: bool,
    #[serde(default)]
    pub allow_restart: bool,
    #[serde(default)]
    pub managed_containers: Vec<String>,
}
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
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
    Restart {
        version: u16,
        request: limeos_domain::RestartRequest,
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
    pub error: Option<ErrorCode>,
}
impl Ceiling {
    pub fn configuration_valid(&self) -> bool {
        self.version == VERSION
            && self.managed_containers.len() <= 64
            && self
                .managed_containers
                .iter()
                .all(|id| limeos_domain::opaque_id(id))
            && (!self.allow_restart || self.allow_container_read)
    }
    pub fn validate(&self, peer_uid: u32, request: &Request) -> Result<Receipt> {
        if !self.configuration_valid() || peer_uid != self.core_uid {
            return Err(Error(ErrorCode::Forbidden));
        }
        match request {
            Request::Health { version } if *version == VERSION && self.allow_health => {
                Ok(Receipt {
                    version: VERSION,
                    ready: true,
                    observations: None,
                    inspection: None,
                    restart: None,
                    error: None,
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
                    error: None,
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
                self.authorize_restart(peer_uid, &request.plan.expected.resource)?;
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
                && self.allow_restart
                && limeos_domain::opaque_id(action)
                && limeos_domain::opaque_id(digest) =>
            {
                Ok(Receipt::empty())
            }
            _ => Err(Error(ErrorCode::InvalidInput)),
        }
    }
    pub fn authorize_restart(&self, peer_uid: u32, resource: &str) -> Result<()> {
        let id = resource.strip_prefix("container:").unwrap_or_default();
        if self.version != VERSION
            || peer_uid != self.core_uid
            || !self.allow_restart
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
            error: None,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn peer_and_ceiling_are_independent_and_no_content_or_command_is_accepted() {
        let ceiling = Ceiling {
            version: VERSION,
            core_uid: 1001,
            allow_health: true,
            allow_host_read: false,
            allow_container_read: false,
            allow_restart: false,
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
