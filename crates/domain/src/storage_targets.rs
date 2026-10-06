//! Versioned, human-approved preparation of empty mount targets.
use crate::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const STORAGE_TARGET_VERSION: u16 = 2;
pub const STORAGE_TARGET_OPERATION: &str = "storage.prepare_targets";
pub const STORAGE_PREPARE_TARGETS: OperationDefinition = OperationDefinition {
    name: STORAGE_TARGET_OPERATION,
    version: STORAGE_TARGET_VERSION,
    permission: Operation::StorageManage,
    risk: Risk::Disruptive,
    timeout_seconds: 30,
    output_limit: 64 * 1024,
    recovery: Recovery::ReconcileBeforeRetry,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct DirectoryIdentity {
    pub major: u32,
    pub minor: u32,
    #[ts(type = "number")]
    pub mount_id: u64,
    #[ts(type = "number")]
    pub inode: u64,
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct TargetEvidence {
    pub mountpoint: String,
    pub parent: DirectoryIdentity,
    pub existing: Option<DirectoryIdentity>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageTargetSnapshot {
    pub inventory: StorageInventory,
    pub targets: Vec<TargetEvidence>,
}

/// This root-executor contract preserves the original operator receipt bytes.
/// It does not itself carry human authority; API authority is the v2 wrapper.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct TargetPreparationPlan {
    pub version: u16,
    pub operation: String,
    pub action: String,
    pub expected: StorageInventory,
    pub contract: StorageContract,
    pub targets: Vec<TargetEvidence>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub expires_at: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PreparationState {
    Prepared,
    Verified,
    PreconditionChanged,
    OutcomeUnknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct TargetPreparationReceipt {
    pub plan: TargetPreparationPlan,
    pub digest: String,
    pub state: PreparationState,
    pub after: Vec<TargetEvidence>,
}
impl TargetPreparationPlan {
    pub fn validate(&self, at: i64) -> Result<()> {
        self.contract.validate()?;
        self.expected.validate()?;
        storage_managed_fstab(&self.contract, 10, &self.expected)?;
        if self.contract.devices.iter().any(|d| {
            self.expected
                .devices
                .iter()
                .filter(|a| a.filesystem_uuid == d.filesystem_uuid)
                .any(|a| !a.mounts.is_empty())
        }) {
            return Err(Error(ErrorCode::Conflict));
        }
        if self.version != 1
            || self.operation != STORAGE_TARGET_OPERATION
            || !opaque_id(&self.action)
            || self.created_at < 0
            || self.expires_at <= self.created_at
            || self.expires_at > self.created_at.saturating_add(PLAN_TTL_SECONDS)
            || self.targets.len() != self.contract.devices.len()
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        if at < self.created_at || at >= self.expires_at {
            return Err(Error(ErrorCode::Expired));
        }
        // A snapshot is ordered by literal mountpoint, as the protected reader
        // does. Paths, identity and ownership never come from the queue caller.
        let mut mountpoints: Vec<_> = self
            .contract
            .devices
            .iter()
            .map(|d| &d.mountpoint)
            .collect();
        mountpoints.sort();
        for (target, mountpoint) in self.targets.iter().zip(mountpoints) {
            if &target.mountpoint != mountpoint
                || !protected_directory(&target.parent)
                || target.parent.mount_id != self.expected.host_root_mount_id
                || target.existing.as_ref().is_some_and(|e| {
                    !protected_directory(e)
                        || e.mount_id != target.parent.mount_id
                        || (e.major, e.minor) != (target.parent.major, target.parent.minor)
                })
            {
                return Err(Error(ErrorCode::InvalidInput));
            }
        }
        Ok(())
    }
    pub fn resources(&self) -> Vec<String> {
        let mut resources = vec!["storage:configuration".into()];
        for d in &self.contract.devices {
            resources.push(format!("storage:uuid:{}", d.filesystem_uuid));
            resources.push(format!("storage:mount:{}", d.mountpoint));
        }
        resources.sort();
        resources.dedup();
        resources
    }
    pub fn check_current(&self, current: &StorageTargetSnapshot, at: i64) -> Result<()> {
        self.validate(at)?;
        if self.expected != current.inventory || self.targets != current.targets {
            return Err(Error(ErrorCode::Conflict));
        }
        Ok(())
    }
}
fn protected_directory(e: &DirectoryIdentity) -> bool {
    e.uid == 0
        && e.mode & 0o170000 == 0o040000
        && e.mode & 0o022 == 0
        && e.mount_id > 0
        && e.mount_id < (1u64 << 53)
        && e.inode > 0
        && e.inode < (1u64 << 53)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageTargetPlan {
    pub id: String,
    pub version: u16,
    pub principal: String,
    #[ts(type = "number")]
    pub grant_revision: i64,
    pub preparation: TargetPreparationPlan,
}
impl StorageTargetPlan {
    pub fn validate(&self, at: i64) -> Result<()> {
        if self.version != STORAGE_TARGET_VERSION
            || !opaque_id(&self.id)
            || !identifier(&self.principal)
            || self.grant_revision < 1
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        self.preparation.validate(at)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct PlannedStorageTargets {
    pub plan: StorageTargetPlan,
    pub digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageTargetJob {
    pub id: String,
    pub state: JobState,
    pub plan: StorageTargetPlan,
}
/// Independent fresh inspection must agree with the persisted root receipt.
pub fn verify_target_preparation(
    plan: &TargetPreparationPlan,
    receipt: &TargetPreparationReceipt,
    current: &StorageTargetSnapshot,
) -> Result<()> {
    if receipt.plan != *plan
        || receipt.state != PreparationState::Verified
        || current.inventory != plan.expected
        || current.targets != receipt.after
        || current.targets.len() != plan.targets.len()
    {
        return Err(Error(ErrorCode::Conflict));
    }
    for (after, before) in current.targets.iter().zip(&plan.targets) {
        if after.mountpoint != before.mountpoint
            || after.parent != before.parent
            || after.existing.as_ref().is_none_or(|e| {
                !protected_directory(e)
                    || e.mount_id != before.parent.mount_id
                    || before.existing.as_ref().is_some_and(|old| old != e)
                    || (before.existing.is_none() && (e.gid != 0 || e.mode & 0o7777 != 0o755))
            })
        {
            return Err(Error(ErrorCode::Conflict));
        }
    }
    Ok(())
}
