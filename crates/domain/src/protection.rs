//! Pure SnapRAID configuration and sync eligibility. These previews cannot execute.
use crate::pools::{beneath, finding, finish, mount_path, name, overlap, require};
use crate::{
    PoolsConfig, StorageBlockDevice, StorageContract, StorageDevice, StorageInventory,
    StorageMediaIdentity, StoragePlanningFindingCode, StoragePlanningResult, StorageRole,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use ts_rs::TS;

pub const PROTECTION_EVIDENCE_MAX_AGE_SECONDS: i64 = 5;
pub const SNAPRAID_DEFAULT_EXCLUDES: &[&str] = &[
    "*.tmp",
    "*.temp",
    "*.bak",
    "/lost+found/",
    "*.unrecoverable",
    ".Thumbs.db",
    ".DS_Store",
    "._*",
    ".fseventsd/",
    ".Spotlight-V100/",
    ".Trashes/",
    "aquota.group",
    "aquota.user",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum SnapraidRole {
    Data,
    Parity,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SnapraidDrive {
    pub id: String,
    pub name: String,
    pub path: String,
    pub uuid: String,
    pub role: SnapraidRole,
    pub content: bool,
    pub parity_level: u8,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SnapraidSettings {
    pub blocksize: u16,
    pub hashsize: u8,
    pub autosave: u32,
    pub nohidden: bool,
    pub prehash: bool,
}
impl Default for SnapraidSettings {
    fn default() -> Self {
        Self {
            blocksize: 256,
            hashsize: 16,
            autosave: 500,
            nohidden: false,
            prehash: true,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SnapraidThresholds {
    pub delete_threshold: u32,
    pub update_threshold: u32,
}
impl Default for SnapraidThresholds {
    fn default() -> Self {
        Self {
            delete_threshold: 50,
            update_threshold: 500,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SnapraidScrub {
    pub enabled: bool,
    pub percent: u8,
    pub age_days: u16,
}
impl Default for SnapraidScrub {
    fn default() -> Self {
        Self {
            enabled: true,
            percent: 12,
            age_days: 10,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SnapraidSchedule {
    pub sync_enabled: bool,
    pub sync_cron: String,
    pub scrub_enabled: bool,
    pub scrub_cron: String,
}
impl Default for SnapraidSchedule {
    fn default() -> Self {
        Self {
            sync_enabled: false,
            sync_cron: "0 3 * * *".into(),
            scrub_enabled: false,
            scrub_cron: "0 4 * * 0".into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SnapraidConfig {
    pub enabled: bool,
    pub drives: Vec<SnapraidDrive>,
    pub excludes: Vec<String>,
    pub settings: SnapraidSettings,
    pub thresholds: SnapraidThresholds,
    pub scrub: SnapraidScrub,
    pub schedule: SnapraidSchedule,
}

fn cron(value: &str) -> bool {
    // Deliberately bounded subset: five fields, '*' or one decimal number.
    let fields: Vec<_> = value.split(' ').collect();
    fields.len() == 5
        && fields
            .iter()
            .zip([(0, 59), (0, 23), (1, 31), (1, 12), (0, 7)])
            .all(|(s, (min, max))| {
                *s == "*"
                    || (!s.is_empty()
                        && s.bytes().all(|b| b.is_ascii_digit())
                        && s.parse::<u8>().is_ok_and(|n| n >= min && n <= max))
            })
}
impl SnapraidConfig {
    pub fn validate(&self, pools: &PoolsConfig) -> StoragePlanningResult<()> {
        pools.validate()?; // Missing/malformed pool configuration must not imply no pools.
        use StoragePlanningFindingCode::*;
        let mut findings = vec![];
        require(
            self.drives.len() <= 32 && (!self.drives.is_empty() || !self.enabled),
            "drives",
            InvalidConfiguration,
            &mut findings,
        );
        if !self.drives.is_empty() {
            require(
                self.drives.iter().any(|d| d.role == SnapraidRole::Data)
                    && self.drives.iter().any(|d| d.role == SnapraidRole::Parity)
                    && self.drives.iter().any(|d| d.content),
                "drives.roles",
                InvalidConfiguration,
                &mut findings,
            );
        }
        let (mut ids, mut names, mut uuids, mut levels) = (
            BTreeSet::new(),
            BTreeSet::new(),
            BTreeSet::new(),
            BTreeSet::new(),
        );
        let mut paths: Vec<&str> = vec![];
        for (i, drive) in self.drives.iter().enumerate() {
            let field = format!("drives[{i}]");
            require(
                name(&drive.id) && name(&drive.name) && mount_path(&drive.path),
                &field,
                InvalidConfiguration,
                &mut findings,
            );
            require(
                !drive.uuid.is_empty()
                    && drive.uuid.len() <= 128
                    && drive.uuid.as_bytes()[0].is_ascii_alphanumeric()
                    && drive
                        .uuid
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)),
                &format!("{field}.uuid"),
                UnknownIdentity,
                &mut findings,
            );
            require(
                ids.insert(&drive.id)
                    && names.insert(&drive.name)
                    && uuids.insert(&drive.uuid)
                    && paths.iter().all(|p| !overlap(p, &drive.path)),
                &field,
                Duplicate,
                &mut findings,
            );
            paths.push(&drive.path);
            require(
                (1..=6).contains(&drive.parity_level)
                    && (drive.role != SnapraidRole::Data || drive.parity_level == 1),
                &format!("{field}.parity_level"),
                InvalidConfiguration,
                &mut findings,
            );
            if drive.role == SnapraidRole::Parity {
                require(
                    levels.insert(drive.parity_level),
                    &format!("{field}.parity_level"),
                    Duplicate,
                    &mut findings,
                );
            }
            // All configured pool roots apply, including disabled pools.
            for (_, path) in drive.paths() {
                require(
                    pools.pools.iter().all(|p| !overlap(&path, &p.mount_point)),
                    &format!("{field}.path"),
                    PoolPath,
                    &mut findings,
                );
            }
        }
        require(
            levels.iter().copied().eq(1..=levels.len() as u8),
            "drives.parity_level",
            InvalidConfiguration,
            &mut findings,
        );
        require(
            [256, 512].contains(&self.settings.blocksize)
                && [8, 16].contains(&self.settings.hashsize),
            "settings",
            Unsupported,
            &mut findings,
        );
        require(
            (1..=100).contains(&self.scrub.percent),
            "scrub.percent",
            InvalidConfiguration,
            &mut findings,
        );
        require(
            cron(&self.schedule.sync_cron) && cron(&self.schedule.scrub_cron),
            "schedule",
            Unsupported,
            &mut findings,
        );
        require(
            self.excludes.len() <= 128
                && self.excludes.iter().all(|p| {
                    !p.is_empty()
                        && p.len() <= 512
                        && p.trim() == p
                        && !p.chars().any(char::is_control)
                }),
            "excludes",
            InvalidConfiguration,
            &mut findings,
        );
        finish((), findings)
    }

    pub fn render(&self, pools: &PoolsConfig) -> StoragePlanningResult<String> {
        self.validate(pools)?;
        let mut lines = vec!["# Generated by LimeOS SnapRAID V1".into(), String::new()];
        if self.settings.prehash {
            lines.push("prehash".into());
        }
        if self.settings.nohidden {
            lines.push("nohidden".into());
        }
        if self.settings.blocksize != 256 {
            lines.push(format!("blocksize {}", self.settings.blocksize));
        }
        if self.settings.hashsize != 16 {
            lines.push(format!("hashsize {}", self.settings.hashsize));
        }
        if self.settings.autosave > 0 {
            lines.push(format!("autosave {}", self.settings.autosave));
        }
        lines.push(String::new());
        let mut parity: Vec<_> = self
            .drives
            .iter()
            .filter(|d| d.role == SnapraidRole::Parity)
            .collect();
        parity.sort_by_key(|d| d.parity_level);
        for drive in parity {
            let prefix = if drive.parity_level == 1 {
                "parity".into()
            } else {
                format!("{}-parity", drive.parity_level)
            };
            lines.push(format!("{prefix} {}/snapraid.{prefix}", drive.path));
        }
        lines.push(String::new());
        for d in self.drives.iter().filter(|d| d.content) {
            lines.push(format!("content {}/snapraid.content", d.path));
        }
        lines.push(String::new());
        for d in self.drives.iter().filter(|d| d.role == SnapraidRole::Data) {
            lines.push(format!("data {} {}", d.name, d.path));
        }
        lines.push(String::new());
        for pattern in &self.excludes {
            lines.push(format!("exclude {pattern}"));
        }
        Ok(lines.join("\n") + "\n")
    }

    pub fn plan(
        &self,
        pools: &PoolsConfig,
        contract: &StorageContract,
    ) -> StoragePlanningResult<SnapraidPreview> {
        self.validate(pools)?;
        contract.validate().map_err(|_| {
            vec![finding(
                "storage_contract",
                StoragePlanningFindingCode::InvalidConfiguration,
            )]
        })?;
        let mut requirements = vec![];
        for (i, drive) in self.drives.iter().enumerate() {
            let matches: Vec<_> = contract
                .devices
                .iter()
                .filter(|d| d.filesystem_uuid == drive.uuid && d.mountpoint == drive.path)
                .collect();
            let role = match drive.role {
                SnapraidRole::Data => StorageRole::Data,
                SnapraidRole::Parity => StorageRole::Parity,
            };
            if matches.len() != 1 || matches[0].role != role {
                return Err(vec![finding(
                    &format!("drives[{i}]"),
                    StoragePlanningFindingCode::UnknownIdentity,
                )]);
            }
            for (kind, path) in drive.paths() {
                requirements.push(ProtectionPathRequirement {
                    path,
                    kind,
                    expected: matches[0].clone(),
                });
            }
        }
        Ok(SnapraidPreview {
            desired: self.clone(),
            media_identity: contract.media_identity.clone(),
            requirements,
            configuration: self.render(pools)?,
        })
    }
}
impl SnapraidDrive {
    fn paths(&self) -> Vec<(ProtectionPathKind, String)> {
        // A parity-only drive still has a source requirement for its mount root.
        let mut paths = vec![(ProtectionPathKind::Source, self.path.clone())];
        if self.role == SnapraidRole::Data {
            paths.push((ProtectionPathKind::Data, self.path.clone()));
        }
        if self.content {
            paths.push((
                ProtectionPathKind::Content,
                format!("{}/snapraid.content", self.path),
            ));
        }
        if self.role == SnapraidRole::Parity {
            let suffix = if self.parity_level == 1 {
                "parity".into()
            } else {
                format!("{}-parity", self.parity_level)
            };
            paths.push((
                ProtectionPathKind::Parity,
                format!("{}/snapraid.{suffix}", self.path),
            ));
        }
        paths
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProtectionPathKind {
    Source,
    Data,
    Content,
    Parity,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ProtectionPathRequirement {
    pub path: String,
    pub kind: ProtectionPathKind,
    pub expected: StorageDevice,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SnapraidPreview {
    pub desired: SnapraidConfig,
    pub media_identity: StorageMediaIdentity,
    pub requirements: Vec<ProtectionPathRequirement>,
    pub configuration: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProtectionPathState {
    Directory,
    RegularFile,
    AbsentFile,
    Unavailable,
    Unsafe,
}
/// Adapter attestation from retained, no-symlink descriptors in the host namespace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ProtectionPathObservation {
    pub path: String,
    pub state: ProtectionPathState,
    pub device: StorageBlockDevice,
    #[ts(type = "number")]
    pub mount_id: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ProtectionSourceEvidence {
    #[ts(type = "number")]
    pub observed_at: i64,
    pub inventory: StorageInventory,
    pub paths: Vec<ProtectionPathObservation>,
}
pub(crate) fn recent(observed_at: i64, now: i64) -> bool {
    observed_at > 0
        && now
            .checked_sub(observed_at)
            .is_some_and(|age| (0..=PROTECTION_EVIDENCE_MAX_AGE_SECONDS).contains(&age))
}
impl SnapraidPreview {
    fn check_sources(
        &self,
        evidence: &ProtectionSourceEvidence,
        now: i64,
    ) -> StoragePlanningResult<()> {
        use StoragePlanningFindingCode::*;
        if !recent(evidence.observed_at, now) {
            return Err(vec![finding("sources", StaleEvidence)]);
        }
        evidence
            .inventory
            .validate()
            .map_err(|_| vec![finding("sources.inventory", UnverifiedSource)])?;
        if evidence.paths.len() != self.requirements.len() {
            return Err(vec![finding("sources.paths", UnverifiedSource)]);
        }
        for (index, required) in self.requirements.iter().enumerate() {
            let field = format!("sources.paths[{index}]");
            let devices: Vec<_> = evidence
                .inventory
                .devices
                .iter()
                .filter(|d| d.filesystem_uuid == required.expected.filesystem_uuid)
                .collect();
            if devices.len() != 1 {
                return Err(vec![finding(&field, UnknownIdentity)]);
            }
            let d = devices[0];
            let filesystem = match required.expected.filesystem {
                crate::StorageFilesystem::Ext2 => "ext2",
                crate::StorageFilesystem::Ext3 => "ext3",
                crate::StorageFilesystem::Ext4 => "ext4",
                crate::StorageFilesystem::Xfs => "xfs",
                _ => return Err(vec![finding(&field, Unsupported)]),
            };
            if d.boot_backing
                || d.in_use_as_swap
                || d.filesystem != filesystem
                || required
                    .expected
                    .serial
                    .as_ref()
                    .is_some_and(|serial| d.serial.as_ref() != Some(serial))
            {
                return Err(vec![finding(&field, UnverifiedSource)]);
            }
            let mounts: Vec<_> = evidence
                .inventory
                .devices
                .iter()
                .flat_map(|d| d.mounts.iter().map(move |m| (d, m)))
                .filter(|(_, m)| m.mountpoint == required.expected.mountpoint)
                .collect();
            if mounts.len() != 1 {
                return Err(vec![finding(&field, UnverifiedSource)]);
            }
            let (mounted_device, m) = mounts[0];
            if mounted_device.device != d.device
                || m.filesystem_root != "/"
                || m.filesystem != filesystem
                || !m.writable
                || m.mount_id == evidence.inventory.host_root_mount_id
            {
                return Err(vec![finding(&field, UnverifiedSource)]);
            }
            // Reject additional mounts hiding a content/parity leaf or data subtree.
            if evidence
                .inventory
                .devices
                .iter()
                .flat_map(|d| &d.mounts)
                .any(|other| {
                    other.mountpoint != required.expected.mountpoint
                        && beneath(&other.mountpoint, &required.expected.mountpoint)
                })
            {
                return Err(vec![finding(&field, UnverifiedSource)]);
            }
            let path = &evidence.paths[index];
            let state_ok = match required.kind {
                ProtectionPathKind::Source | ProtectionPathKind::Data => {
                    path.state == ProtectionPathState::Directory
                }
                _ => matches!(
                    path.state,
                    ProtectionPathState::RegularFile | ProtectionPathState::AbsentFile
                ),
            };
            if path.path != required.path
                || path.device != d.device
                || path.mount_id != m.mount_id
                || !state_ok
            {
                return Err(vec![finding(&field, UnverifiedSource)]);
            }
        }
        Ok(())
    }
}

/// Real process termination is preserved independently of parsing or log tags.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StorageReadTermination {
    Exited { code: i32 },
    Signaled { signal: i32 },
    MissingBinary,
    TimedOut,
    OutputLimit,
    IoError,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageReadFailure {
    // None for imported logs or failed reads that never started a process.
    pub termination: Option<StorageReadTermination>,
    pub reason: StorageReadFailureReason,
    pub detail: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum StorageReadFailureReason {
    ProcessFailure,
    OutputLimit,
    MalformedOutput,
    SourceUnavailable,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SnapraidDiff {
    #[ts(type = "number")]
    pub added: u64,
    #[ts(type = "number")]
    pub removed: u64,
    #[ts(type = "number")]
    pub updated: u64,
    #[ts(type = "number")]
    pub moved: u64,
    #[ts(type = "number")]
    pub copied: u64,
    #[ts(type = "number")]
    pub restored: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum SnapraidDiffOutcome {
    Parsed(SnapraidDiff),
    Unavailable(StorageReadFailure),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SnapraidDiffEvidence {
    #[ts(type = "number")]
    pub observed_at: i64,
    pub configuration: SnapraidConfig,
    pub topology_digest: String,
    pub mounts_digest: String,
    pub outcome: SnapraidDiffOutcome,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SnapraidSyncPreview {
    pub configuration: SnapraidPreview,
    pub expected_sources: ProtectionSourceEvidence,
    pub diff: SnapraidDiffEvidence,
}

/// No force/override parameter exists. Future overrides need their own audited operation.
pub fn plan_snapraid_sync(
    config: &SnapraidConfig,
    pools: &PoolsConfig,
    contract: &StorageContract,
    sources: &ProtectionSourceEvidence,
    diff: &SnapraidDiffEvidence,
    now: i64,
) -> StoragePlanningResult<SnapraidSyncPreview> {
    use StoragePlanningFindingCode::*;
    let preview = config.plan(pools, contract)?;
    if !config.enabled {
        return Err(vec![finding("enabled", InvalidConfiguration)]);
    }
    preview.check_sources(sources, now)?;
    if !recent(diff.observed_at, now)
        || diff.configuration != *config
        || diff.topology_digest != sources.inventory.topology_digest
        || diff.mounts_digest != sources.inventory.mounts_digest
    {
        return Err(vec![finding("diff", StaleEvidence)]);
    }
    let SnapraidDiffOutcome::Parsed(counts) = &diff.outcome else {
        return Err(vec![finding("diff", DiffUnavailable)]);
    };
    let mut findings = vec![];
    require(
        counts.removed <= u64::from(config.thresholds.delete_threshold),
        "thresholds.delete_threshold",
        ThresholdExceeded,
        &mut findings,
    );
    require(
        counts.updated <= u64::from(config.thresholds.update_threshold),
        "thresholds.update_threshold",
        ThresholdExceeded,
        &mut findings,
    );
    finish(
        SnapraidSyncPreview {
            configuration: preview,
            expected_sources: sources.clone(),
            diff: diff.clone(),
        },
        findings,
    )
}

#[cfg(test)]
mod tests;
