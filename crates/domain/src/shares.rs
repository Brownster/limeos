//! Checked Samba section previews and declared storage relationships.
//! No directory, mount, service, user or effect authority is established here.

use crate::{
    PoolsConfig, StorageContract, StorageDevice, StorageMediaIdentity, StoragePlanningFindingCode,
    StoragePlanningResult,
    pools::{finding, overlap},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub mod document;

pub const SAMBA_MAX_SHARES: usize = 64;
pub const SAMBA_MAX_DEVICE_REQUIREMENTS: usize = 512;

/// Closed share options; arbitrary Samba directives and global settings are absent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SambaShareConfig {
    pub name: String,
    pub path: String,
    pub comment: String,
    pub read_only: bool,
    pub guest_ok: bool,
    pub browseable: bool,
    /// Simple local usernames separated by commas, in their configured order.
    pub valid_users: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SambaSharesConfig {
    pub shares: Vec<SambaShareConfig>,
}

fn share_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !matches!(name, "." | "..")
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}

fn literal_path(path: &str) -> bool {
    path.starts_with('/')
        && (2..=512).contains(&path.len())
        && !path.contains(['\\', '%', '[', ']', '#', ';', '=', '"', '\''])
        && !path
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() && c != ' ')
        && path[1..]
            .split('/')
            .all(|p| !matches!(p, "" | "." | "..") && p.trim() == p)
}

fn literal_comment(comment: &str) -> bool {
    comment.len() <= 240
        && comment.trim() == comment
        && !comment.chars().any(char::is_control)
        && !comment.contains(['\\', '%', '#', ';'])
}

fn local_users(users: &str) -> bool {
    if users.is_empty() {
        return true;
    }
    if users.len() > 2048
        || users.trim() != users
        || users
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() && c != ' ')
    {
        return false;
    }
    let mut names = BTreeSet::new();
    for user in users.split(',') {
        let user = user.trim();
        if user.is_empty()
            || user.len() > 32
            || !(user.as_bytes()[0].is_ascii_alphabetic() || user.starts_with('_'))
            || !user
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            || !names.insert(user.to_ascii_lowercase())
            || names.len() > 32
        {
            return false;
        }
    }
    true
}

impl SambaSharesConfig {
    pub fn validate(&self) -> StoragePlanningResult<()> {
        use StoragePlanningFindingCode::*;
        if self.shares.len() > SAMBA_MAX_SHARES {
            return Err(vec![finding("samba.shares", ThresholdExceeded)]);
        }
        let mut names = BTreeSet::new();
        for (index, share) in self.shares.iter().enumerate() {
            let field = format!("samba.shares[{index}]");
            if !share_name(&share.name) {
                return Err(vec![finding(
                    &format!("{field}.name"),
                    InvalidConfiguration,
                )]);
            }
            let name = share.name.to_ascii_lowercase();
            if matches!(name.as_str(), "global" | "homes" | "printers") {
                return Err(vec![finding(&format!("{field}.name"), Unsupported)]);
            }
            if !names.insert(name) {
                return Err(vec![finding(&format!("{field}.name"), Duplicate)]);
            }
            if !literal_path(&share.path) {
                return Err(vec![finding(
                    &format!("{field}.path"),
                    InvalidConfiguration,
                )]);
            }
            if !literal_comment(&share.comment) {
                return Err(vec![finding(
                    &format!("{field}.comment"),
                    InvalidConfiguration,
                )]);
            }
            if !local_users(&share.valid_users) {
                return Err(vec![finding(&format!("{field}.valid_users"), Unsupported)]);
            }
        }
        Ok(())
    }

    /// Owned section preview only. The integrator must qualify global inheritance
    /// and independently render from checked types before any approved write.
    pub fn render_sections(&self) -> StoragePlanningResult<String> {
        self.validate()?;
        let mut text = String::from("# BEGIN LIMEOS SAMBA V1\n");
        for share in self.shares.iter().filter(|s| s.enabled) {
            text.push_str(&format!("[{}]\n", share.name));
            if !share.comment.is_empty() {
                text.push_str(&format!("   comment = {}\n", share.comment));
            }
            let yes_no = |value| if value { "yes" } else { "no" };
            text.push_str(&format!(
                "   path = {}\n   read only = {}\n   guest ok = {}\n   browseable = {}\n",
                share.path,
                yes_no(share.read_only),
                yes_no(share.guest_ok),
                yes_no(share.browseable),
            ));
            if !share.valid_users.is_empty() {
                text.push_str(&format!("   valid users = {}\n", share.valid_users));
            }
            text.push('\n');
        }
        text.push_str("# END LIMEOS SAMBA V1\n");
        Ok(text)
    }

    /// All configured relationships, or refusal. Ancestors expand conservatively.
    /// Disabled and read-only shares remain consumers. An unmapped path remains
    /// explicit and requires further evidence; this never proves safe unmounting.
    pub fn plan(
        &self,
        contract: &StorageContract,
        pools: &PoolsConfig,
    ) -> StoragePlanningResult<SambaSharesPreview> {
        self.validate()?;
        let pool_plan = pools.plan(contract)?;
        if contract.devices.iter().enumerate().any(|(i, device)| {
            contract
                .devices
                .iter()
                .skip(i + 1)
                .any(|other| overlap(&device.mountpoint, &other.mountpoint))
        }) {
            return Err(vec![finding(
                "storage_contract",
                StoragePlanningFindingCode::InvalidConfiguration,
            )]);
        }
        if pools.pools.iter().any(|pool| {
            contract
                .devices
                .iter()
                .any(|device| overlap(&pool.mount_point, &device.mountpoint))
        }) {
            return Err(vec![finding(
                "pools",
                StoragePlanningFindingCode::UnknownIdentity,
            )]);
        }
        let mut count = 0usize;
        let mut dependencies = Vec::with_capacity(self.shares.len());
        let mut unmapped_shares = Vec::new();
        for share in &self.shares {
            let mut requirements = Vec::new();
            let mut add = |device: &StorageDevice, via| {
                if count == SAMBA_MAX_DEVICE_REQUIREMENTS {
                    return Err(vec![finding(
                        "samba.dependencies",
                        StoragePlanningFindingCode::ThresholdExceeded,
                    )]);
                }
                count += 1;
                requirements.push(SambaDeviceRequirement {
                    expected: device.clone(),
                    via,
                });
                Ok(())
            };
            for device in &contract.devices {
                if overlap(&share.path, &device.mountpoint) {
                    add(device, SambaDependencyRoute::Direct)?;
                }
            }
            for (pool, dependency) in pools.pools.iter().zip(&pool_plan.dependencies) {
                if !overlap(&share.path, &pool.mount_point) {
                    continue;
                }
                for device in &dependency.devices {
                    add(
                        device,
                        SambaDependencyRoute::Pool {
                            pool_id: pool.id.clone(),
                            enabled: pool.enabled,
                            branches: pool
                                .branches
                                .iter()
                                .filter(|branch| crate::pools::beneath(branch, &device.mountpoint))
                                .cloned()
                                .collect(),
                        },
                    )?;
                }
            }
            requirements.sort_by(|a, b| (&a.expected.id, &a.via).cmp(&(&b.expected.id, &b.via)));
            if requirements.is_empty() {
                unmapped_shares.push(share.name.clone());
            }
            dependencies.push(SambaShareDependency {
                share_name: share.name.clone(),
                requirements,
            });
        }
        Ok(SambaSharesPreview {
            version: 1,
            desired: self.clone(),
            media_identity: contract.media_identity.clone(),
            dependencies,
            unmapped_shares,
            managed_sections: self.render_sections()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SambaDependencyRoute {
    Direct,
    Pool {
        pool_id: String,
        enabled: bool,
        branches: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SambaDeviceRequirement {
    pub expected: StorageDevice,
    pub via: SambaDependencyRoute,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SambaShareDependency {
    pub share_name: String,
    pub requirements: Vec<SambaDeviceRequirement>,
}

/// Serialize-only preview, absent from operation registration and wire contracts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SambaSharesPreview {
    pub version: u16,
    pub desired: SambaSharesConfig,
    pub media_identity: StorageMediaIdentity,
    pub dependencies: Vec<SambaShareDependency>,
    pub unmapped_shares: Vec<String>,
    pub managed_sections: String,
}

#[cfg(test)]
mod tests;
