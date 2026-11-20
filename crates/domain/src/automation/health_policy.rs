use super::{AlertKind, AlertPolicy, MAX_ALERTS, MAX_REVISION, Silence, settings, valid_time};
use crate::{Error, ErrorCode, Result};
use serde::{Deserialize, Serialize};

pub const MAX_HEALTH_POLICY_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct HealthPolicySilence {
    pub kind: AlertKind,
    pub key: String,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub reason: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct HealthPolicyDefinition {
    pub fail_threshold: u8,
    pub disabled: Vec<AlertKind>,
    pub required_mounts: Vec<String>,
    pub silences: Vec<HealthPolicySilence>,
}
impl Default for HealthPolicyDefinition {
    fn default() -> Self {
        Self {
            fail_threshold: 2,
            disabled: vec![],
            required_mounts: vec![],
            silences: vec![],
        }
    }
}
impl HealthPolicyDefinition {
    #[cfg(feature = "deferred")]
    pub fn from_planned(value: &settings::PlannedHealthSettings) -> Result<Self> {
        let result = Self {
            fail_threshold: value.fail_threshold(),
            disabled: value.alert_policy().disabled,
            required_mounts: value.required_mounts().to_vec(),
            silences: value
                .silences()
                .iter()
                .map(|silence| HealthPolicySilence {
                    kind: silence.kind,
                    key: silence.key.clone(),
                    created_at: silence.created_at.text().into(),
                    expires_at: silence.expires_at.as_ref().map(|at| at.text().into()),
                    reason: silence.reason.clone(),
                })
                .collect(),
        };
        result.validate()?;
        Ok(result)
    }
    pub fn runtime_policy(&self) -> Result<AlertPolicy> {
        if self.disabled.len() > 5
            || self.required_mounts.len() > MAX_ALERTS
            || self.silences.len() > 64
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        for (index, path) in self.required_mounts.iter().enumerate() {
            if !path.starts_with('/')
                || path.len() > 250
                || path.chars().any(char::is_control)
                || self.required_mounts[..index].contains(path)
            {
                return Err(Error(ErrorCode::InvalidInput));
            }
        }
        let mut silences = Vec::new();
        for silence in &self.silences {
            if silence.key.trim() != silence.key
                || silence.reason.trim() != silence.reason
                || silence.reason.chars().count() > 200
                || settings::policy_timestamp(&silence.created_at).is_none()
            {
                return Err(Error(ErrorCode::InvalidInput));
            }
            let expires_at = silence
                .expires_at
                .as_ref()
                .map(|value| {
                    let at =
                        settings::policy_timestamp(value).ok_or(Error(ErrorCode::InvalidInput))?;
                    if at.micros() != 0 {
                        return Err(Error(ErrorCode::InvalidInput));
                    }
                    Ok(at.unix_seconds())
                })
                .transpose()?;
            silences.push(Silence {
                kind: silence.kind,
                key: silence.key.clone(),
                expires_at,
            });
        }
        let policy = AlertPolicy {
            fail_threshold: self.fail_threshold,
            disabled: self.disabled.clone(),
            silences,
        };
        policy.validate()?;
        Ok(policy)
    }
    pub fn validate(&self) -> Result<()> {
        self.runtime_policy()?;
        struct Count(usize);
        impl std::io::Write for Count {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if bytes.len() > MAX_HEALTH_POLICY_BYTES - self.0 {
                    return Err(std::io::Error::other("health policy size"));
                }
                self.0 += bytes.len();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        serde_json::to_writer(&mut Count(0), self).map_err(|_| Error(ErrorCode::InvalidInput))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct HealthPolicyView {
    #[ts(type = "number")]
    pub revision: i64,
    #[ts(type = "number | null")]
    pub updated: Option<i64>,
    pub definition: HealthPolicyDefinition,
}
impl HealthPolicyView {
    pub fn validate(&self) -> Result<()> {
        if !(0..=MAX_REVISION).contains(&self.revision)
            || (self.revision == 0) != self.updated.is_none()
            || self.updated.is_some_and(|at| !valid_time(at))
            || (self.revision == 0 && self.definition != HealthPolicyDefinition::default())
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        self.definition.validate()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct ReplaceHealthPolicy {
    #[ts(type = "number")]
    pub revision: i64,
    pub definition: HealthPolicyDefinition,
}
impl ReplaceHealthPolicy {
    pub fn validate(&self) -> Result<()> {
        if !(0..=MAX_REVISION).contains(&self.revision) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        self.definition.validate()
    }
}

#[cfg(test)]
mod tests;
