use limeos_domain::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::result::Result;

pub const STORAGE_IMPORT_MAX_BYTES: usize = 64 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageImportReport<T> {
    pub configuration: Option<T>,
    pub findings: Vec<StoragePlanningFinding>,
}
fn issue(field: &str, code: StoragePlanningFindingCode) -> StoragePlanningFinding {
    StoragePlanningFinding {
        field: field.into(),
        code,
    }
}
fn defaults(
    original: &Value,
    applied: &Value,
    field: &str,
    findings: &mut Vec<StoragePlanningFinding>,
) {
    match (original, applied) {
        (Value::Object(before), Value::Object(after)) => {
            for (key, value) in after {
                let field = format!("{field}.{key}");
                if let Some(before) = before.get(key) {
                    defaults(before, value, &field, findings);
                } else {
                    findings.push(issue(&field, StoragePlanningFindingCode::DefaultApplied));
                }
            }
        }
        (Value::Array(before), Value::Array(after)) => {
            for (index, (before, after)) in before.iter().zip(after).enumerate() {
                defaults(before, after, &format!("{field}[{index}]"), findings);
            }
        }
        _ => {}
    }
}
fn same_shape(original: &Value, applied: &Value) -> bool {
    match (original, applied) {
        (Value::Object(before), Value::Object(after)) => after
            .iter()
            .all(|(key, value)| before.get(key).is_none_or(|old| same_shape(old, value))),
        (Value::Array(before), Value::Array(after)) => {
            before.len() == after.len()
                && before
                    .iter()
                    .zip(after)
                    .all(|(old, new)| same_shape(old, new))
        }
        (Value::String(_), Value::String(_))
        | (Value::Number(_), Value::Number(_))
        | (Value::Bool(_), Value::Bool(_))
        | (Value::Null, Value::Null) => true,
        _ => false,
    }
}
fn decode<T: serde::de::DeserializeOwned + Serialize>(
    bytes: &[u8],
    field: &str,
) -> Result<(T, Vec<StoragePlanningFinding>), Vec<StoragePlanningFinding>> {
    if bytes.len() > STORAGE_IMPORT_MAX_BYTES {
        return Err(vec![issue(field, StoragePlanningFindingCode::Malformed)]);
    }
    let original: Value = serde_json::from_slice(bytes)
        .map_err(|_| vec![issue(field, StoragePlanningFindingCode::Malformed)])?;
    // Serde structs can also deserialize sequences; legacy JSON must be an object.
    if !original.is_object() {
        return Err(vec![issue(field, StoragePlanningFindingCode::Malformed)]);
    }
    // Deserialize directly before building a Value: serde rejects duplicate keys.
    let config: T = serde_json::from_slice(bytes).map_err(|error| {
        let text = error.to_string();
        let code = if text.contains("unknown field") || text.contains("unknown variant") {
            StoragePlanningFindingCode::Unsupported
        } else {
            StoragePlanningFindingCode::Malformed
        };
        vec![issue(field, code)] // Never return raw serde messages/values.
    })?;
    let applied = serde_json::to_value(&config)
        .map_err(|_| vec![issue(field, StoragePlanningFindingCode::Malformed)])?;
    if !same_shape(&original, &applied) {
        return Err(vec![issue(field, StoragePlanningFindingCode::Malformed)]);
    }
    let mut findings = vec![];
    defaults(&original, &applied, field, &mut findings);
    Ok((config, findings))
}
fn epmfs() -> PoolCreatePolicy {
    PoolCreatePolicy::Epmfs
}
fn preset() -> PoolPreset {
    PoolPreset::Linux65NoMmap
}
fn min_free() -> String {
    "4G".into()
}
fn yes() -> bool {
    true
}
fn one() -> u8 {
    1
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyPools {
    #[serde(default)]
    pools: Vec<LegacyPool>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyPool {
    id: String,
    name: String,
    branches: Vec<String>,
    mount_point: String,
    #[serde(default = "epmfs")]
    create_policy: PoolCreatePolicy,
    #[serde(default = "preset")]
    preset: PoolPreset,
    #[serde(default = "min_free")]
    min_free_space: String,
    #[serde(default)]
    options: String,
    #[serde(default = "yes")]
    enabled: bool,
}
fn options(text: &str, field: &str) -> Result<PoolOptions, Vec<StoragePlanningFinding>> {
    let mut result = PoolOptions::default();
    let mut seen = std::collections::BTreeSet::new();
    let fail = |code| vec![issue(field, code)];
    for item in text
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let (key, value) = item
            .split_once('=')
            .ok_or_else(|| fail(StoragePlanningFindingCode::Unsupported))?;
        let (key, value) = (key.trim(), value.trim());
        if !seen.insert(key) {
            return Err(fail(StoragePlanningFindingCode::Duplicate));
        }
        let parse = |value: &str| Value::String(value.into());
        match key {
            "category.create" => {
                result.create_policy = Some(
                    serde_json::from_value(parse(value))
                        .map_err(|_| fail(StoragePlanningFindingCode::Unsupported))?,
                )
            }
            "cache.files" => {
                result.cache_files = Some(
                    serde_json::from_value(parse(value))
                        .map_err(|_| fail(StoragePlanningFindingCode::Unsupported))?,
                )
            }
            "func.getattr" => {
                result.getattr = Some(
                    serde_json::from_value(parse(value))
                        .map_err(|_| fail(StoragePlanningFindingCode::Unsupported))?,
                )
            }
            "dropcacheonclose" => {
                result.drop_cache_on_close = Some(match value {
                    "true" => true,
                    "false" => false,
                    _ => return Err(fail(StoragePlanningFindingCode::Malformed)),
                })
            }
            "minfreespace" => result.min_free_space = Some(value.into()),
            _ => return Err(fail(StoragePlanningFindingCode::Unsupported)),
        }
    }
    Ok(result)
}
pub fn import_legacy_pools(bytes: &[u8]) -> StorageImportReport<PoolsConfig> {
    let (legacy, mut findings) = match decode::<LegacyPools>(bytes, "mergerfs") {
        Ok(value) => value,
        Err(findings) => {
            return StorageImportReport {
                configuration: None,
                findings,
            };
        }
    };
    let mut pools = vec![];
    for (index, pool) in legacy.pools.into_iter().enumerate() {
        let options = match options(&pool.options, &format!("mergerfs.pools[{index}].options")) {
            Ok(value) => value,
            Err(errors) => {
                findings.extend(errors);
                return StorageImportReport {
                    configuration: None,
                    findings,
                };
            }
        };
        pools.push(PoolConfig {
            id: pool.id,
            name: pool.name,
            branches: pool.branches,
            mount_point: pool.mount_point,
            create_policy: pool.create_policy,
            preset: pool.preset,
            min_free_space: pool.min_free_space,
            options,
            enabled: pool.enabled,
        });
    }
    let config = PoolsConfig { pools };
    match config.validate() {
        Ok(()) => StorageImportReport {
            configuration: Some(config),
            findings,
        },
        Err(errors) => {
            findings.extend(errors);
            StorageImportReport {
                configuration: None,
                findings,
            }
        }
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyDrive {
    id: String,
    name: String,
    path: String,
    uuid: String,
    role: SnapraidRole,
    // Frozen renderer/validator use false when absent, despite schema's true.
    #[serde(default)]
    content: bool,
    #[serde(default = "one")]
    parity_level: u8,
}
#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct LegacySnapraid {
    enabled: bool,
    drives: Vec<LegacyDrive>,
    excludes: Vec<String>,
    settings: LegacySettings,
    thresholds: LegacyThresholds,
    scrub: LegacyScrub,
    schedule: LegacySchedule,
}
impl Default for LegacySnapraid {
    fn default() -> Self {
        Self {
            enabled: false,
            drives: vec![],
            excludes: SNAPRAID_DEFAULT_EXCLUDES
                .iter()
                .map(|s| (*s).into())
                .collect(),
            settings: LegacySettings::default(),
            thresholds: LegacyThresholds::default(),
            scrub: LegacyScrub::default(),
            schedule: LegacySchedule::default(),
        }
    }
}
// Nested defaults reproduce get_config, rather than replacing supplied objects.
macro_rules! legacy_defaults {
    ($legacy:ident, $domain:ident, { $($field:ident: $ty:ty),+ $(,)? }) => {
        #[derive(Deserialize, Serialize)]
        #[serde(default, deny_unknown_fields)]
        struct $legacy { $($field: $ty),+ }
        impl Default for $legacy { fn default() -> Self { let d = $domain::default(); Self { $($field: d.$field),+ } } }
        impl From<$legacy> for $domain { fn from(d: $legacy) -> Self { Self { $($field: d.$field),+ } } }
    };
}
legacy_defaults!(LegacySettings, SnapraidSettings, { blocksize: u16, hashsize: u8, autosave: u32, nohidden: bool, prehash: bool });
legacy_defaults!(LegacyThresholds, SnapraidThresholds, { delete_threshold: u32, update_threshold: u32 });
legacy_defaults!(LegacyScrub, SnapraidScrub, { enabled: bool, percent: u8, age_days: u16 });
legacy_defaults!(LegacySchedule, SnapraidSchedule, { sync_enabled: bool, sync_cron: String, scrub_enabled: bool, scrub_cron: String });

pub fn import_legacy_snapraid(
    bytes: &[u8],
    pools: &PoolsConfig,
) -> StorageImportReport<SnapraidConfig> {
    // A missing excludes field uses the plugin defaults. An explicit [] stays [].
    let (legacy, mut findings) = match decode::<LegacySnapraid>(bytes, "snapraid") {
        Ok(value) => value,
        Err(findings) => {
            return StorageImportReport {
                configuration: None,
                findings,
            };
        }
    };
    let config = SnapraidConfig {
        enabled: legacy.enabled,
        drives: legacy
            .drives
            .into_iter()
            .map(|d| SnapraidDrive {
                id: d.id,
                name: d.name,
                path: d.path,
                uuid: d.uuid,
                role: d.role,
                content: d.content,
                parity_level: d.parity_level,
            })
            .collect(),
        excludes: legacy.excludes,
        settings: legacy.settings.into(),
        thresholds: legacy.thresholds.into(),
        scrub: legacy.scrub.into(),
        schedule: legacy.schedule.into(),
    };
    match config.validate(pools) {
        Ok(()) => StorageImportReport {
            configuration: Some(config),
            findings,
        },
        Err(errors) => {
            findings.extend(errors);
            StorageImportReport {
                configuration: None,
                findings,
            }
        }
    }
}
