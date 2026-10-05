//! Deterministic pool previews. Configuration relationships are not live authority.
use crate::{StorageContract, StorageDevice, StorageMediaIdentity, StorageRole};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum StoragePlanningFindingCode {
    Malformed,
    Unsupported,
    DefaultApplied,
    InvalidConfiguration,
    Duplicate,
    PoolPath,
    UnknownIdentity,
    UnverifiedSource,
    StaleEvidence,
    DiffUnavailable,
    ThresholdExceeded,
}

/// Fixed field paths and codes; never include imported values or raw errors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StoragePlanningFinding {
    pub field: String,
    pub code: StoragePlanningFindingCode,
}
pub type StoragePlanningResult<T> = Result<T, Vec<StoragePlanningFinding>>;
pub(crate) fn finding(field: &str, code: StoragePlanningFindingCode) -> StoragePlanningFinding {
    StoragePlanningFinding {
        field: field.into(),
        code,
    }
}
pub(crate) fn require(
    ok: bool,
    field: &str,
    code: StoragePlanningFindingCode,
    findings: &mut Vec<StoragePlanningFinding>,
) {
    if !ok {
        findings.push(finding(field, code));
    }
}
pub(crate) fn finish<T>(
    value: T,
    findings: Vec<StoragePlanningFinding>,
) -> StoragePlanningResult<T> {
    if findings.is_empty() {
        Ok(value)
    } else {
        Err(findings)
    }
}
pub(crate) fn name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}
pub(crate) fn mount_path(path: &str) -> bool {
    path.starts_with("/mnt/")
        && path.len() <= 512
        && !path.chars().any(|c| c.is_control() || c.is_whitespace())
        && !path.contains(['\\', ':', ',', '#', '*', '?', '[', ']', '='])
        && path[1..].split('/').all(|p| !matches!(p, "" | "." | ".."))
}
pub(crate) fn beneath(path: &str, root: &str) -> bool {
    // Compare components; /mnt/pool-other is not beneath /mnt/pool.
    let mut path = path.split('/');
    root.split('/')
        .all(|component| path.next() == Some(component))
}
pub(crate) fn overlap(a: &str, b: &str) -> bool {
    beneath(a, b) || beneath(b, a)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
pub enum PoolCreatePolicy {
    Epmfs,
    Eplfs,
    Eplus,
    Mfs,
    Lfs,
    Lus,
    Rand,
    Pfrd,
    Ff,
}
impl PoolCreatePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Epmfs => "epmfs",
            Self::Eplfs => "eplfs",
            Self::Eplus => "eplus",
            Self::Mfs => "mfs",
            Self::Lfs => "lfs",
            Self::Lus => "lus",
            Self::Rand => "rand",
            Self::Pfrd => "pfrd",
            Self::Ff => "ff",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum PoolPreset {
    #[serde(rename = "linux_6_6_plus")]
    Linux66Plus,
    #[serde(rename = "linux_6_5_mmap")]
    Linux65Mmap,
    #[serde(rename = "linux_6_5_no_mmap")]
    Linux65NoMmap,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum PoolFileCache {
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "auto-full")]
    AutoFull,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
pub enum PoolGetattr {
    Newest,
}

/// Closed subset of legacy option overrides; no free-form mount option string.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct PoolOptions {
    pub create_policy: Option<PoolCreatePolicy>,
    pub cache_files: Option<PoolFileCache>,
    pub getattr: Option<PoolGetattr>,
    pub drop_cache_on_close: Option<bool>,
    pub min_free_space: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct PoolConfig {
    pub id: String,
    pub name: String,
    pub branches: Vec<String>,
    pub mount_point: String,
    pub create_policy: PoolCreatePolicy,
    pub preset: PoolPreset,
    pub min_free_space: String,
    pub options: PoolOptions,
    pub enabled: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct PoolsConfig {
    pub pools: Vec<PoolConfig>,
}

pub(crate) fn size(value: &str) -> bool {
    // Frozen validator grammar, with an explicit bounded numeric representation.
    let digit_count = value.bytes().take_while(u8::is_ascii_digit).count();
    if digit_count == 0 || value.len() > 32 || value[..digit_count].parse::<u64>().is_err() {
        return false;
    }
    let suffix = value[digit_count..].to_ascii_uppercase();
    suffix.is_empty()
        || suffix == "B"
        || ["K", "M", "G", "T", "P"].iter().any(|unit| {
            [
                unit.to_string(),
                format!("{unit}B"),
                format!("{unit}I"),
                format!("{unit}IB"),
            ]
            .contains(&suffix)
        })
}

impl PoolsConfig {
    pub fn validate(&self) -> StoragePlanningResult<()> {
        use StoragePlanningFindingCode::*;
        let mut findings = vec![];
        require(
            self.pools.len() <= 32,
            "pools",
            InvalidConfiguration,
            &mut findings,
        );
        let (mut ids, mut names) = (BTreeSet::new(), BTreeSet::new());
        let mut mounts: Vec<&str> = vec![];
        let mut branches: Vec<&str> = vec![];
        for (i, pool) in self.pools.iter().enumerate() {
            let field = format!("pools[{i}]");
            require(
                name(&pool.id) && name(&pool.name),
                &field,
                InvalidConfiguration,
                &mut findings,
            );
            require(
                ids.insert(&pool.id) && names.insert(&pool.name),
                &field,
                Duplicate,
                &mut findings,
            );
            require(
                mount_path(&pool.mount_point)
                    && (2..=32).contains(&pool.branches.len())
                    && size(&pool.min_free_space)
                    && pool.options.min_free_space.as_ref().is_none_or(|v| size(v)),
                &field,
                InvalidConfiguration,
                &mut findings,
            );
            require(
                mounts.iter().all(|m| !overlap(m, &pool.mount_point)),
                &field,
                Duplicate,
                &mut findings,
            );
            mounts.push(&pool.mount_point);
            for (j, branch) in pool.branches.iter().enumerate() {
                let field = format!("pools[{i}].branches[{j}]");
                require(
                    mount_path(branch),
                    &field,
                    InvalidConfiguration,
                    &mut findings,
                );
                require(
                    branches.iter().all(|b| !overlap(b, branch)),
                    &field,
                    Duplicate,
                    &mut findings,
                );
                branches.push(branch);
            }
        }
        for branch in branches {
            require(
                mounts.iter().all(|m| !overlap(m, branch)),
                "pools.branches",
                PoolPath,
                &mut findings,
            );
        }
        finish((), findings)
    }

    /// Render an owned preview section. The executor must render again from types.
    pub fn render_fstab(&self) -> StoragePlanningResult<String> {
        self.validate()?;
        let mut lines = vec!["# BEGIN LIMEOS MERGERFS V1".into()];
        for pool in self.pools.iter().filter(|p| p.enabled) {
            let (cache, drop) = match pool.preset {
                PoolPreset::Linux65Mmap => (PoolFileCache::AutoFull, true),
                _ => (PoolFileCache::Off, false),
            };
            let cache = match pool.options.cache_files.unwrap_or(cache) {
                PoolFileCache::Off => "off",
                PoolFileCache::AutoFull => "auto-full",
            };
            let policy = pool
                .options
                .create_policy
                .unwrap_or(pool.create_policy)
                .as_str();
            let drop = pool.options.drop_cache_on_close.unwrap_or(drop);
            let min = pool
                .options
                .min_free_space
                .as_ref()
                .unwrap_or(&pool.min_free_space);
            lines.push(format!("# mergerfs pool: {}", pool.name));
            lines.push(format!("{} {} fuse.mergerfs defaults,allow_other,use_ino,fsname=mergerfs,cache.files={cache},category.create={policy},func.getattr=newest,dropcacheonclose={drop},minfreespace={min} 0 0", pool.branches.join(":"), pool.mount_point));
        }
        lines.push("# END LIMEOS MERGERFS V1".into());
        Ok(lines.join("\n") + "\n")
    }

    pub fn plan(&self, contract: &StorageContract) -> StoragePlanningResult<PoolPreview> {
        self.validate()?;
        contract.validate().map_err(|_| {
            vec![finding(
                "storage_contract",
                StoragePlanningFindingCode::InvalidConfiguration,
            )]
        })?;
        let mut dependencies = vec![];
        for (i, pool) in self.pools.iter().enumerate() {
            let mut devices = vec![];
            for (j, branch) in pool.branches.iter().enumerate() {
                let matches: Vec<_> = contract
                    .devices
                    .iter()
                    .filter(|d| beneath(branch, &d.mountpoint))
                    .collect();
                if matches.len() != 1 || matches[0].role != StorageRole::Data {
                    return Err(vec![finding(
                        &format!("pools[{i}].branches[{j}]"),
                        StoragePlanningFindingCode::UnknownIdentity,
                    )]);
                }
                if !devices.contains(matches[0]) {
                    devices.push(matches[0].clone());
                }
            }
            dependencies.push(PoolDependency {
                pool_id: pool.id.clone(),
                devices,
            });
        }
        Ok(PoolPreview {
            desired: self.clone(),
            media_identity: contract.media_identity.clone(),
            dependencies,
            managed_fstab: self.render_fstab()?,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct PoolDependency {
    pub pool_id: String,
    pub devices: Vec<StorageDevice>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct PoolPreview {
    pub desired: PoolsConfig,
    pub media_identity: StorageMediaIdentity,
    pub dependencies: Vec<PoolDependency>,
    pub managed_fstab: String,
}

#[cfg(test)]
pub(crate) mod tests;
