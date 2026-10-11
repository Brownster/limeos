//! Durable claims to revalidate, never imported filesystem authority.
use crate::{Error, ErrorCode, JobState, PLAN_TTL_SECONDS, Result, identifier, opaque_id};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const CUSTODY_BINDING_VERSION: u32 = 1;
pub const BACKUP_PREPARATION_VERSION: u16 = 1;
pub const BACKUP_PREPARATION_RESOURCE: &str = "backup:preparation";
pub const MAX_BACKUP_SELECTED_RESOURCES: usize = 64;

/// Existing custody encoding: field order and validation are unchanged.
/// A genuine owner must recompute all fields from actual bytes and full policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct CustodyBinding {
    pub format_version: u32,
    pub record_id: String,
    pub archive_sha256: String,
    #[ts(type = "number")]
    pub archive_bytes: u64,
    pub policy_identity: String,
    pub manifest_identity: String,
}
impl CustodyBinding {
    pub fn well_formed(&self) -> bool {
        self.format_version == CUSTODY_BINDING_VERSION
            && self.record_id.len() == 32
            && self
                .record_id
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            && opaque_id(&self.archive_sha256)
            && self.archive_bytes > 0
            && opaque_id(&self.policy_identity)
            && opaque_id(&self.manifest_identity)
    }
}

/// The trusted coordinator derives these facts from genuine private owners.
/// No path, caller claims or reconstruction of an opaque owner is accepted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct BackupPreparationBinding {
    pub custody: CustodyBinding,
    pub preparation_identity: String,
    pub selected_resources: Vec<String>,
    pub workspace_policy_identity: String,
}
impl BackupPreparationBinding {
    pub fn validate(&self) -> Result<()> {
        if !self.custody.well_formed()
            || !opaque_id(&self.preparation_identity)
            || !opaque_id(&self.workspace_policy_identity)
            || self.selected_resources.is_empty()
            || self.selected_resources.len() > MAX_BACKUP_SELECTED_RESOURCES
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        for (index, id) in self.selected_resources.iter().enumerate() {
            if id.is_empty()
                || id.len() > 128
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                || self.selected_resources[..index].contains(id)
            {
                return Err(Error(ErrorCode::InvalidInput));
            }
        }
        Ok(())
    }
    /// Complete coarse logical requirements; not locks or physical write proof.
    pub fn resources(&self) -> Vec<String> {
        vec![
            format!("backup:custody:{}", self.custody.record_id),
            BACKUP_PREPARATION_RESOURCE.into(),
            "storage:configuration".into(),
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct BackupPreparationJobPlan {
    pub id: String,
    pub version: u16,
    pub action: String,
    pub principal: String,
    #[ts(type = "number")]
    pub grant_revision: i64,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub expires_at: i64,
    pub binding: BackupPreparationBinding,
}
impl BackupPreparationJobPlan {
    pub fn validate(&self, at: i64) -> Result<()> {
        self.binding.validate()?;
        if self.version != BACKUP_PREPARATION_VERSION
            || !opaque_id(&self.id)
            || !opaque_id(&self.action)
            || !identifier(&self.principal)
            || self.grant_revision < 1
            || self.created_at < 0
            || self.created_at.checked_add(PLAN_TTL_SECONDS) != Some(self.expires_at)
            || at < self.created_at
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        if at >= self.expires_at {
            return Err(Error(ErrorCode::Expired));
        }
        Ok(())
    }
    pub fn check_current(&self, binding: &BackupPreparationBinding, at: i64) -> Result<()> {
        self.validate(at)?;
        binding.validate()?;
        if &self.binding != binding {
            return Err(Error(ErrorCode::Conflict));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct PlannedBackupPreparation {
    pub plan: BackupPreparationJobPlan,
    pub digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct BackupPreparationJob {
    pub id: String,
    pub state: JobState,
    pub plan: BackupPreparationJobPlan,
}

#[cfg(test)]
mod tests;
