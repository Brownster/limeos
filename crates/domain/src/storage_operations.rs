//! Reviewed facts for one nonpersistent mount; these values are never effect authority.
use crate::{
    DirectoryIdentity, Error, ErrorCode, JobState, Operation, OperationDefinition,
    PLAN_TTL_SECONDS, Recovery, Result, Risk, StorageBlockDevice, StorageDevice, StorageFilesystem,
    StorageInventory, StorageMountWaitPlan, StorageRole, TargetEvidence, identifier, opaque_id,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const STORAGE_MOUNT_VERSION: u16 = 1;
pub const STORAGE_MOUNT_OPERATION: &str = "storage.mount";
pub const STORAGE_MOUNT_PLAN_BYTES: usize = 24 * 1024;
pub const STORAGE_MOUNT_RECEIPT_BYTES: usize = 64 * 1024;
pub const STORAGE_MOUNT_PHYSICAL_DEVICES: usize = 8;
pub const STORAGE_MOUNT: OperationDefinition = OperationDefinition {
    name: STORAGE_MOUNT_OPERATION,
    version: STORAGE_MOUNT_VERSION,
    permission: Operation::StorageManage,
    risk: Risk::Disruptive,
    timeout_seconds: 30,
    output_limit: STORAGE_MOUNT_RECEIPT_BYTES,
    recovery: Recovery::ReconcileBeforeRetry,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageMountSelection {
    pub target_id: String,
}
impl StorageMountSelection {
    pub fn validate(&self) -> Result<()> {
        valid(target_id(&self.target_id))
    }
}

/// Protected configuration choices, independent of live readiness or authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageMountCatalog {
    pub targets: Vec<StorageDevice>,
}
impl StorageMountCatalog {
    pub fn validate(&self) -> Result<()> {
        valid(self.targets.len() <= 32)?;
        bounded(self, 32 * 1024)?;
        valid(self.targets.iter().all(|device| {
            device.role == StorageRole::Data && device.filesystem == StorageFilesystem::Ext4
        }))?;
        if !self.targets.is_empty() {
            StorageMountWaitPlan {
                timeout_seconds: 3,
                devices: self.targets.clone(),
            }
            .validate()?;
        }
        Ok(())
    }
}

/// A protected reader's past facts. Empty-target/complete-consumer proofs are opaque
/// retained owners held by the executor, and cannot be reconstructed from this DTO.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageMountInspection {
    pub target_id: String,
    pub device: StorageDevice,
    pub expected: StorageInventory,
    pub target: TargetEvidence,
    pub physical_devices: Vec<StorageBlockDevice>,
    pub consumer_digest: String,
    #[ts(type = "number")]
    pub observed_at: i64,
}
impl StorageMountInspection {
    pub fn validate(&self, at: i64) -> Result<()> {
        valid(target_id(&self.target_id) && self.target_id == self.device.id)?;
        validate_facts(
            &self.device,
            &self.expected,
            &self.target,
            &self.physical_devices,
            &self.consumer_digest,
        )?;
        if self.observed_at < 0 || at < self.observed_at || at > self.observed_at.saturating_add(5)
        {
            return Err(Error(ErrorCode::Expired));
        }
        bounded(self, STORAGE_MOUNT_PLAN_BYTES)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageMountEffectPlan {
    pub version: u16,
    pub operation: String,
    pub action: String,
    pub device: StorageDevice,
    pub expected: StorageInventory,
    pub target: TargetEvidence,
    pub physical_devices: Vec<StorageBlockDevice>,
    pub consumer_digest: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub expires_at: i64,
}
impl StorageMountEffectPlan {
    pub fn validate(&self, at: i64) -> Result<()> {
        valid(
            self.version == STORAGE_MOUNT_VERSION
                && self.operation == STORAGE_MOUNT_OPERATION
                && opaque_id(&self.action)
                && self.created_at >= 0
                && self.expires_at > self.created_at
                && self.expires_at <= self.created_at.saturating_add(PLAN_TTL_SECONDS),
        )?;
        validate_facts(
            &self.device,
            &self.expected,
            &self.target,
            &self.physical_devices,
            &self.consumer_digest,
        )?;
        if at < self.created_at || at >= self.expires_at {
            return Err(Error(ErrorCode::Expired));
        }
        bounded(self, STORAGE_MOUNT_PLAN_BYTES)
    }
    pub fn check_current(&self, current: &StorageMountInspection, at: i64) -> Result<()> {
        self.validate(at)?;
        self.check_reconciliation(current, at)
    }
    /// Read-only reconciliation checks historical intent without extending its
    /// execution expiry. It supplies neither no-effect nor helper-exit proof.
    pub fn check_reconciliation(&self, current: &StorageMountInspection, at: i64) -> Result<()> {
        self.validate(self.created_at)?;
        current.validate(at)?;
        valid_or_conflict(
            self.device == current.device
                && self.expected == current.expected
                && self.target == current.target
                && self.physical_devices == current.physical_devices
                && self.consumer_digest == current.consumer_digest,
        )
    }
    pub fn resources(&self) -> Vec<String> {
        let mut resources = vec![
            "storage:configuration".into(),
            format!("storage:uuid:{}", self.device.filesystem_uuid),
            format!("storage:mount:{}", self.device.mountpoint),
        ];
        resources.extend(
            self.physical_devices
                .iter()
                .map(|d| format!("storage:device:{}:{}", d.major, d.minor)),
        );
        resources.sort();
        resources.dedup();
        resources
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageMountPlan {
    pub id: String,
    pub version: u16,
    pub principal: String,
    #[ts(type = "number")]
    pub grant_revision: i64,
    pub effect: StorageMountEffectPlan,
}
impl StorageMountPlan {
    pub fn validate(&self, at: i64) -> Result<()> {
        valid(
            self.version == STORAGE_MOUNT_VERSION
                && opaque_id(&self.id)
                && identifier(&self.principal)
                && self.grant_revision > 0,
        )?;
        self.effect.validate(at)?;
        bounded(self, STORAGE_MOUNT_PLAN_BYTES)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct PlannedStorageMount {
    pub plan: StorageMountPlan,
    pub digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageMountJob {
    pub id: String,
    pub state: JobState,
    pub plan: StorageMountPlan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum MountExecutionState {
    Prepared,
    EffectAccepted,
    Verified,
    PreconditionChanged,
    OutcomeUnknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageMountedSnapshot {
    pub filesystem_uuid: String,
    pub device: StorageBlockDevice,
    pub mountpoint: String,
    pub filesystem: String,
    pub filesystem_root: String,
    pub writable: bool,
    pub nosuid: bool,
    pub nodev: bool,
    #[ts(type = "number")]
    pub mount_id: u64,
    #[ts(type = "number")]
    pub host_root_mount_id: u64,
    pub target: DirectoryIdentity,
}

/// Authenticated receipt facts only. Construction of their genuine process owner
/// and confirmation that no descendant/helper can still run belong to supervision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct MountHelperIdentity {
    pub pid: u32,
    #[ts(type = "number")]
    pub start_time_ticks: u64,
    #[ts(type = "number")]
    pub mount_namespace_inode: u64,
    pub cgroup: String,
    pub image_digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum MountHelperOutcome {
    Exited { code: i32 },
    Signaled { signal: i32 },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum MountEffectQuiescence {
    NotLaunched {},
    Pending {
        helper: Option<MountHelperIdentity>,
    },
    Reaped {
        helper: MountHelperIdentity,
        outcome: MountHelperOutcome,
    },
}
impl MountEffectQuiescence {
    pub fn validate(&self) -> Result<()> {
        let helper = match self {
            Self::NotLaunched {} => return Ok(()),
            Self::Pending { helper } => helper.as_ref(),
            Self::Reaped { helper, outcome } => {
                valid(match outcome {
                    MountHelperOutcome::Exited { code } => (0..=255).contains(code),
                    MountHelperOutcome::Signaled { signal } => (1..=64).contains(signal),
                })?;
                Some(helper)
            }
        };
        if let Some(helper) = helper {
            valid(
                helper.pid > 1
                    && helper.start_time_ticks > 0
                    && helper.mount_namespace_inode > 0
                    && canonical(&helper.cgroup, true)
                    && opaque_id(&helper.image_digest),
            )?;
        }
        Ok(())
    }
    pub fn is_quiescent(&self) -> bool {
        matches!(self, Self::NotLaunched {} | Self::Reaped { .. })
    }
    pub fn succeeded(&self) -> bool {
        matches!(
            self,
            Self::Reaped {
                outcome: MountHelperOutcome::Exited { code: 0 },
                ..
            }
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageMountReceipt {
    pub plan: StorageMountEffectPlan,
    pub digest: String,
    pub state: MountExecutionState,
    pub quiescence: MountEffectQuiescence,
    pub after: Option<StorageMountedSnapshot>,
}
impl StorageMountReceipt {
    pub fn validate(&self) -> Result<()> {
        self.plan.validate(self.plan.created_at)?;
        valid(opaque_id(&self.digest))?;
        self.quiescence.validate()?;
        match self.state {
            MountExecutionState::EffectAccepted | MountExecutionState::Verified => {
                valid(self.quiescence.succeeded())?;
                validate_mounted(
                    &self.plan,
                    self.after.as_ref().ok_or(Error(ErrorCode::InvalidInput))?,
                )?;
            }
            MountExecutionState::PreconditionChanged => {
                valid(self.quiescence.is_quiescent() && self.after.is_none())?;
            }
            MountExecutionState::Prepared => {
                valid(self.after.is_none())?;
            }
            MountExecutionState::OutcomeUnknown => {}
        }
        bounded(self, STORAGE_MOUNT_RECEIPT_BYTES)
    }
}

/// Equality checks supplement authenticated receipt binding and an independent
/// root inspection; they never create helper termination or fresh mount authority.
pub fn verify_storage_mount(
    plan: &StorageMountEffectPlan,
    receipt: &StorageMountReceipt,
    current: &StorageMountedSnapshot,
) -> Result<()> {
    receipt.validate()?;
    valid_or_conflict(
        receipt.plan == *plan
            && matches!(
                receipt.state,
                MountExecutionState::EffectAccepted | MountExecutionState::Verified
            )
            && receipt.after.as_ref() == Some(current),
    )?;
    validate_mounted(plan, current)
}

fn validate_facts(
    device: &StorageDevice,
    expected: &StorageInventory,
    target: &TargetEvidence,
    physical: &[StorageBlockDevice],
    consumers: &str,
) -> Result<()> {
    // Count borrowed facts before cloning even the single device for the shared
    // assignment validator. No caller collection is cloned before admission.
    bounded(
        &(device, expected, target, physical, consumers),
        STORAGE_MOUNT_PLAN_BYTES,
    )?;
    StorageMountWaitPlan {
        timeout_seconds: 3,
        devices: vec![device.clone()],
    }
    .validate()?;
    expected.validate()?;
    valid(
        device.filesystem == StorageFilesystem::Ext4
            && device.role == StorageRole::Data
            && opaque_id(consumers),
    )?;
    let selected: Vec<_> = expected
        .devices
        .iter()
        .filter(|d| d.filesystem_uuid == device.filesystem_uuid)
        .collect();
    valid_or_conflict(selected.len() == 1)?;
    let selected = selected[0];
    valid_or_conflict(
        selected.filesystem == "ext4"
            && selected.mounts.is_empty()
            && !selected.boot_backing
            && !selected.in_use_as_swap
            && device
                .serial
                .as_ref()
                .is_none_or(|serial| selected.serial.as_ref() == Some(serial)),
    )?;
    valid(
        !physical.is_empty()
            && physical.len() <= STORAGE_MOUNT_PHYSICAL_DEVICES
            && physical.iter().all(|d| d.major > 0)
            && physical.windows(2).all(|w| w[0] < w[1])
            && physical.contains(&selected.device),
    )?;
    // A bound physical set must not contain another known filesystem. Its actual
    // topology completeness and absence of unreported users require opaque proof.
    valid_or_conflict(
        expected
            .devices
            .iter()
            .all(|d| d.device == selected.device || !physical.contains(&d.device)),
    )?;
    valid_or_conflict(expected.fstab_entries.iter().all(|f| {
        f.filesystem_uuid.as_ref() != Some(&device.filesystem_uuid)
            && f.device.is_none_or(|d| !physical.contains(&d))
            // A normal root-filesystem row is expected and does not own every
            // target below '/'. Its selected UUID/device would still conflict.
            && (f.mountpoint == "/" || !overlap(&f.mountpoint, &device.mountpoint))
    }))?;
    valid(target.mountpoint == device.mountpoint && protected(&target.parent))?;
    let existing = target.existing.as_ref().ok_or(Error(ErrorCode::Conflict))?;
    valid(
        protected(existing)
            && target.parent.mount_id == expected.host_root_mount_id
            && existing.mount_id == target.parent.mount_id
            && (existing.major, existing.minor) == (target.parent.major, target.parent.minor),
    )
}
fn validate_mounted(plan: &StorageMountEffectPlan, current: &StorageMountedSnapshot) -> Result<()> {
    let selected = plan
        .expected
        .devices
        .iter()
        .find(|d| d.filesystem_uuid == plan.device.filesystem_uuid)
        .ok_or(Error(ErrorCode::InvalidInput))?;
    valid_or_conflict(
        current.filesystem_uuid == plan.device.filesystem_uuid
            && current.device == selected.device
            && current.mountpoint == plan.device.mountpoint
            && current.filesystem == "ext4"
            && current.filesystem_root == "/"
            && current.writable
            && current.nosuid
            && current.nodev
            && current.host_root_mount_id == plan.expected.host_root_mount_id
            && current.mount_id > 0
            && current.mount_id < (1u64 << 53)
            && current.mount_id != plan.target.parent.mount_id
            && current.target.mount_id == current.mount_id
            && (current.target.major, current.target.minor)
                == (selected.device.major, selected.device.minor)
            && current.target.mode & 0o170000 == 0o040000
            && current.target.inode > 0
            && current.target.inode < (1u64 << 53),
    )
}
fn target_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.as_bytes()[0].is_ascii_alphanumeric()
        && id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-".contains(&c))
}
fn canonical(path: &str, root: bool) -> bool {
    (root && path == "/")
        || (path.starts_with('/')
            && path.len() > 1
            && path.len() <= 512
            && !path.chars().any(char::is_control)
            && path[1..].split('/').all(|s| !matches!(s, "" | "." | "..")))
}
fn overlap(a: &str, b: &str) -> bool {
    a == "/"
        || b == "/"
        || a == b
        || a.strip_prefix(b).is_some_and(|s| s.starts_with('/'))
        || b.strip_prefix(a).is_some_and(|s| s.starts_with('/'))
}
fn protected(identity: &DirectoryIdentity) -> bool {
    identity.uid == 0
        && identity.mode & 0o170000 == 0o040000
        && identity.mode & 0o022 == 0
        && identity.mount_id > 0
        && identity.mount_id < (1u64 << 53)
        && identity.inode > 0
        && identity.inode < (1u64 << 53)
}
fn valid(condition: bool) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(Error(ErrorCode::InvalidInput))
    }
}
fn valid_or_conflict(condition: bool) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(Error(ErrorCode::Conflict))
    }
}
fn bounded(value: &impl Serialize, limit: usize) -> Result<()> {
    struct Counter {
        bytes: usize,
        limit: usize,
    }
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.bytes = self
                .bytes
                .checked_add(bytes.len())
                .ok_or_else(|| std::io::Error::other("mount document limit"))?;
            if self.bytes > self.limit {
                return Err(std::io::Error::other("mount document limit"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Counter { bytes: 0, limit }, value)
        .map_err(|_| Error(ErrorCode::InvalidInput))
}

#[cfg(test)]
mod tests;
