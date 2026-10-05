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
        if !opaque_id(id) || !opaque_id(image) || engine_timestamp(&self.started_at).is_none() {
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

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct RestartRequest {
    pub action: String,
    pub plan_digest: String,
    pub plan: RestartPlan,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    Prepared,
    EffectAccepted,
    Verified,
    PreconditionChanged,
    OutcomeUnknown,
}
impl ExecutionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::EffectAccepted => "effect_accepted",
            Self::Verified => "verified",
            Self::PreconditionChanged => "precondition_changed",
            Self::OutcomeUnknown => "outcome_unknown",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct RestartReceipt {
    pub action: String,
    pub plan_digest: String,
    pub before: ContainerSnapshot,
    pub state: ExecutionState,
    pub error: Option<ErrorCode>,
}

/// A successful Engine response alone is insufficient. Inspect the same image
/// and container again and require a new running incarnation.
pub fn verify_restart(before: &ContainerSnapshot, after: &ContainerSnapshot) -> Result<()> {
    before.validate()?;
    after.validate()?;
    if before.resource != after.resource
        || before.image != after.image
        || !after.running
        || engine_timestamp(&after.started_at) <= engine_timestamp(&before.started_at)
    {
        return Err(Error(ErrorCode::Conflict));
    }
    Ok(())
}

/// Docker emits UTC RFC3339 nanosecond timestamps. Validate calendar fields and
/// normalize fractional precision before comparing incarnations.
fn engine_timestamp(value: &str) -> Option<(u32, u32, u32, u32, u32, u32, u32)> {
    let bytes = value.as_bytes();
    if !(20..=30).contains(&bytes.len())
        || !value.is_ascii()
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || *bytes.last()? != b'Z'
    {
        return None;
    }
    let number = |start, end| value.get(start..end)?.parse::<u32>().ok();
    let (year, month, day, hour, minute, second) = (
        number(0, 4)?,
        number(5, 7)?,
        number(8, 10)?,
        number(11, 13)?,
        number(14, 16)?,
        number(17, 19)?,
    );
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        _ => return None,
    };
    if year == 0 || day == 0 || day > days || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let nano = if bytes.len() == 20 {
        0
    } else {
        if bytes[19] != b'.' || bytes.len() < 22 {
            return None;
        }
        let fraction = &value[20..bytes.len() - 1];
        if !fraction.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        fraction
            .parse::<u32>()
            .ok()?
            .checked_mul(10u32.pow((9 - fraction.len()) as u32))?
    };
    Some((year, month, day, hour, minute, second, nano))
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
    #[test]
    fn verification_normalizes_precision_and_rejects_invalid_or_earlier_timestamps() {
        let before = snapshot();
        for value in [
            "2026-02-30T07:00:00Z",
            "2026-10-05T25:00:00Z",
            "2026-10-05T07:00:00.000Z",
            "2026-10-05T06:59:00Z",
        ] {
            assert!(
                verify_restart(
                    &before,
                    &ContainerSnapshot {
                        started_at: value.into(),
                        ..before.clone()
                    }
                )
                .is_err()
            );
        }
        verify_restart(
            &before,
            &ContainerSnapshot {
                started_at: "2026-10-05T07:00:00.000000001Z".into(),
                ..before.clone()
            },
        )
        .unwrap();
    }
}
