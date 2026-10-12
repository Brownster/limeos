//! Read-only import of the frozen Samba plugin's saved JSON, never smb.conf.
use crate::pool_protection::{STORAGE_IMPORT_MAX_BYTES, StorageImportReport};
use limeos_domain::{
    StoragePlanningFinding, StoragePlanningFindingCode,
    shares::{SAMBA_MAX_SHARES, SambaShareConfig, SambaSharesConfig},
};
use serde::Deserialize;
use serde_json::Value;

fn issue(field: &str, code: StoragePlanningFindingCode) -> StoragePlanningFinding {
    StoragePlanningFinding {
        field: field.into(),
        code,
    }
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyShares {
    #[serde(default)]
    shares: Vec<LegacyShare>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyShare {
    name: String,
    path: String,
    #[serde(default)]
    comment: String,
    #[serde(default)]
    read_only: bool,
    #[serde(default)]
    guest_ok: bool,
    #[serde(default = "yes")]
    browseable: bool,
    #[serde(default)]
    valid_users: String,
    #[serde(default = "yes")]
    enabled: bool,
}

/// Whole import succeeds only for the checked subset; findings never echo values.
/// Missing fields use the frozen renderer defaults and are individually reported.
/// Input bytes and external files remain untouched. No command or I/O runs here.
pub fn import_legacy_samba(bytes: &[u8]) -> StorageImportReport<SambaSharesConfig> {
    use StoragePlanningFindingCode::*;
    let refuse = |code| StorageImportReport {
        configuration: None,
        findings: vec![issue("samba", code)],
    };
    if bytes.len() > STORAGE_IMPORT_MAX_BYTES {
        return refuse(ThresholdExceeded);
    }
    // Parse structs directly, so duplicate fields cannot disappear in a Value.
    let legacy = match serde_json::from_slice::<LegacyShares>(bytes) {
        Ok(value) => value,
        Err(error) => {
            return refuse(if error.to_string().contains("unknown field") {
                Unsupported
            } else {
                Malformed
            });
        }
    };
    let original: Value = match serde_json::from_slice(bytes) {
        Ok(value) => value,
        Err(_) => return refuse(Malformed),
    };
    if !original.is_object()
        || original.get("shares").is_some_and(|shares| {
            !shares.is_array()
                || shares
                    .as_array()
                    .is_some_and(|shares| shares.iter().any(|share| !share.is_object()))
        })
    {
        return refuse(Malformed);
    }
    if legacy.shares.len() > SAMBA_MAX_SHARES {
        return refuse(ThresholdExceeded);
    }
    let mut findings = Vec::new();
    if original.get("shares").is_none() {
        findings.push(issue("samba.shares", DefaultApplied));
    }
    for (index, share) in original
        .get("shares")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        for field in [
            "comment",
            "read_only",
            "guest_ok",
            "browseable",
            "valid_users",
            "enabled",
        ] {
            if share.get(field).is_none() {
                findings.push(issue(
                    &format!("samba.shares[{index}].{field}"),
                    DefaultApplied,
                ));
            }
        }
    }
    let config = SambaSharesConfig {
        shares: legacy
            .shares
            .into_iter()
            .map(|share| SambaShareConfig {
                name: share.name,
                path: share.path,
                comment: share.comment.trim().into(),
                read_only: share.read_only,
                guest_ok: share.guest_ok,
                browseable: share.browseable,
                valid_users: share.valid_users.trim().into(),
                enabled: share.enabled,
            })
            .collect(),
    };
    if let Err(errors) = config.validate() {
        findings.extend(errors);
        return StorageImportReport {
            configuration: None,
            findings,
        };
    }
    StorageImportReport {
        configuration: Some(config),
        findings,
    }
}

#[cfg(test)]
mod tests;
