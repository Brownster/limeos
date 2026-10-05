//! Operation-owned risk, authority and preconditions; no caller policy overrides.
use crate::{Error, ErrorCode, Operation, Result, Scope, identifier};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const PLAN_TTL_SECONDS: i64 = 300;
pub const RESTART_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Risk {
    Disruptive,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    ReconcileBeforeRetry,
}
pub struct OperationDefinition {
    pub name: &'static str,
    pub version: u16,
    pub permission: Operation,
    pub risk: Risk,
    pub timeout_seconds: u32,
    pub output_limit: usize,
    pub recovery: Recovery,
}
pub const CONTAINER_RESTART: OperationDefinition = OperationDefinition {
    name: "container.restart",
    version: RESTART_VERSION,
    permission: Operation::ContainerManage,
    risk: Risk::Disruptive,
    timeout_seconds: 30,
    output_limit: 64 * 1024,
    recovery: Recovery::ReconcileBeforeRetry,
};

pub fn opaque_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Selected inspection evidence. Environment variables, labels and host paths
/// never enter a plan. The full ID, image and start timestamp identify an instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ContainerSnapshot {
    pub resource: String,
    pub image: String,
    pub started_at: String,
    pub running: bool,
}
impl ContainerSnapshot {
    pub fn validate(&self) -> Result<()> {
        let id = self.resource.strip_prefix("container:").unwrap_or_default();
        let image = self.image.strip_prefix("sha256:").unwrap_or_default();
        if !opaque_id(id)
            || !opaque_id(image)
            || self.started_at.len() < 20
            || self.started_at.len() > 64
            || !self
                .started_at
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-:.+".contains(&b))
            || self.started_at.as_bytes().get(10) != Some(&b'T')
            || !self.started_at.ends_with('Z')
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        Ok(())
    }
    pub fn scope(&self) -> Scope {
        Scope {
            operation: CONTAINER_RESTART.permission,
            resource: self.resource.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct RestartPlan {
    pub id: String,
    pub version: u16,
    pub principal: String,
    #[ts(type = "number")]
    pub grant_revision: i64,
    pub expected: ContainerSnapshot,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub expires_at: i64,
}
impl RestartPlan {
    pub fn validate(&self, now: i64) -> Result<()> {
        self.expected.validate()?;
        if self.version != RESTART_VERSION
            || !opaque_id(&self.id)
            || !identifier(&self.principal)
            || self.grant_revision < 1
            || self.created_at < 0
            || self.expires_at <= self.created_at
            || self.expires_at > self.created_at.saturating_add(PLAN_TTL_SECONDS)
            || !self.expected.running
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        if now < self.created_at || now >= self.expires_at {
            return Err(Error(ErrorCode::Expired));
        }
        Ok(())
    }
    pub fn check_current(&self, actual: &ContainerSnapshot, now: i64) -> Result<()> {
        self.validate(now)?;
        actual.validate()?;
        if self.expected != *actual {
            return Err(Error(ErrorCode::Conflict));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct PlannedRestart {
    pub plan: RestartPlan,
    pub digest: String,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct PlanApproval {
    pub token: String,
    #[ts(type = "number")]
    pub expires_at: i64,
}
impl std::fmt::Debug for PlanApproval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlanApproval")
            .field("token", &"[redacted]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct RestartJob {
    pub id: String,
    pub state: crate::JobState,
    pub plan: RestartPlan,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> ContainerSnapshot {
        ContainerSnapshot {
            resource: format!("container:{}", "a".repeat(64)),
            image: format!("sha256:{}", "b".repeat(64)),
            started_at: "2026-10-05T07:00:00.000000000Z".into(),
            running: true,
        }
    }
    #[test]
    fn selected_identity_requires_full_ids_and_forbids_paths_or_control_characters() {
        let s = snapshot();
        s.validate().unwrap();
        for resource in [
            "container:abcd",
            "container:../docker.sock",
            "container:--all",
        ] {
            assert!(
                ContainerSnapshot {
                    resource: resource.into(),
                    ..s.clone()
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            ContainerSnapshot {
                started_at: "2026-10-05T07:00:00Z\n".into(),
                ..s
            }
            .validate()
            .is_err()
        );
    }
    #[test]
    fn changed_incarnation_image_and_state_cannot_reuse_the_plan() {
        let s = snapshot();
        let plan = RestartPlan {
            id: "c".repeat(64),
            version: RESTART_VERSION,
            principal: "user".into(),
            grant_revision: 1,
            expected: s.clone(),
            created_at: 100,
            expires_at: 400,
        };
        plan.check_current(&s, 399).unwrap();
        assert_eq!(
            plan.check_current(&s, 400).unwrap_err().0,
            ErrorCode::Expired
        );
        for changed in [
            ContainerSnapshot {
                started_at: "2026-10-05T07:01:00Z".into(),
                ..s.clone()
            },
            ContainerSnapshot {
                image: format!("sha256:{}", "d".repeat(64)),
                ..s.clone()
            },
            ContainerSnapshot {
                running: false,
                ..s
            },
        ] {
            assert_eq!(
                plan.check_current(&changed, 101).unwrap_err().0,
                ErrorCode::Conflict
            );
        }
    }
}
