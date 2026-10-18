//! CopyParty supplied observations and configuration previews; never authority.
use crate::{Error, ErrorCode, Result};
use serde::Serialize;
use std::{fmt, io::Write};

pub const DEFAULT_SHARE_PATH: &str = "/srv/copyparty";
pub const DEFAULT_PORT: u16 = 3923;
pub const LEGACY_PAYLOAD: &str = "/opt/copyparty/copyparty-sfx.py";
pub const SERVICE: &str = "copyparty.service";
pub const MAX_PATH_BYTES: usize = 512;
pub const MAX_REPORT_BYTES: usize = 8192;
pub const MAX_AGE_SECONDS: i64 = 90;
pub const MAX_TIME: i64 = 253_402_300_799;

/// Validated intent, not a resolved storage path or systemd argument.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct Config {
    share_path: String,
    port: u16,
}
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("share_path_bytes", &self.share_path.len())
            .field("port", &self.port)
            .finish()
    }
}
impl Config {
    pub fn new(share_path: &str, port: u32) -> std::result::Result<Self, ConfigIssue> {
        if share_path.len() > MAX_PATH_BYTES {
            return Err(ConfigIssue::Limit);
        }
        if !share_path.starts_with('/')
            || share_path.chars().any(char::is_control)
            || share_path[1..].split('/').any(|part| {
                matches!(part, "" | "." | "..") || part.trim() != part || part.len() > 255
            })
        {
            return Err(ConfigIssue::InvalidPath);
        }
        let port = u16::try_from(port)
            .ok()
            .filter(|port| *port != 0)
            .ok_or(ConfigIssue::InvalidPort)?;
        Ok(Self {
            share_path: share_path.into(),
            port,
        })
    }
    pub fn share_path(&self) -> &str {
        &self.share_path
    }
    pub fn port(&self) -> u16 {
        self.port
    }
}
impl Default for Config {
    fn default() -> Self {
        Self {
            share_path: DEFAULT_SHARE_PATH.into(),
            port: DEFAULT_PORT,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigIssue {
    Unavailable,
    Incomplete,
    InvalidDocument,
    UnsupportedOptions,
    InvalidPath,
    InvalidPort,
    Limit,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ConfigState {
    /// Only for verified absence of the fixed configuration file.
    Defaults {
        config: Config,
    },
    Stored {
        config: Config,
    },
    Unknown {
        reason: ConfigIssue,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadState {
    Absent,
    /// Legacy regular file presence, without package or executable provenance.
    Present,
    UnexpectedType,
    Unavailable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceIssue {
    Unavailable,
    Incomplete,
    Limit,
    InvalidProperties,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "reason", rename_all = "snake_case")]
pub enum ServiceState {
    Active,
    Inactive,
    Failed,
    Activating,
    Deactivating,
    Reloading,
    NotFound,
    Masked,
    LoadError,
    Unknown(ServiceIssue),
}
/// Caller must retain the protected owner and original host/boot/read lifetime.
#[derive(Clone, Debug)]
pub struct Observation<T> {
    pub sampled_at: i64,
    pub value: T,
}
#[derive(Clone, Copy, Default)]
pub struct Facts<'a> {
    pub config: Option<&'a Observation<ConfigState>>,
    pub payload: Option<&'a Observation<PayloadState>>,
    pub service: Option<&'a Observation<ServiceState>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    Fresh,
    Unavailable,
    Stale,
    Future,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Component<T> {
    pub sampled_at: Option<i64>,
    pub freshness: Freshness,
    pub value: Option<T>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppliedConfiguration {
    NotEstablished,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolsReport {
    pub version: u16,
    pub generated_at: i64,
    pub max_age_seconds: i64,
    pub config: Component<ConfigState>,
    pub payload: Component<PayloadState>,
    pub service: Component<ServiceState>,
    pub applied_configuration: AppliedConfiguration,
}
fn component<T: Clone>(now: i64, input: Option<&Observation<T>>) -> Result<Component<T>> {
    let Some(input) = input else {
        return Ok(Component {
            sampled_at: None,
            freshness: Freshness::Unavailable,
            value: None,
        });
    };
    if !(0..=MAX_TIME).contains(&input.sampled_at) {
        return Err(Error(ErrorCode::InvalidInput));
    }
    let freshness = if input.sampled_at > now {
        Freshness::Future
    } else if now - input.sampled_at > MAX_AGE_SECONDS {
        Freshness::Stale
    } else {
        Freshness::Fresh
    };
    Ok(Component {
        sampled_at: Some(input.sampled_at),
        freshness,
        value: (freshness == Freshness::Fresh).then(|| input.value.clone()),
    })
}
pub fn observe(now: i64, facts: Facts<'_>) -> Result<ToolsReport> {
    if !(0..=MAX_TIME).contains(&now) {
        return Err(Error(ErrorCode::InvalidInput));
    }
    if facts.config.is_some_and(|observation| {
        matches!(&observation.value, ConfigState::Defaults { config } if *config != Config::default())
    }) {
        return Err(Error(ErrorCode::InvalidInput));
    }
    let report = ToolsReport {
        version: 1,
        generated_at: now,
        max_age_seconds: MAX_AGE_SECONDS,
        config: component(now, facts.config)?,
        payload: component(now, facts.payload)?,
        service: component(now, facts.service)?,
        applied_configuration: AppliedConfiguration::NotEstablished,
    };
    report.check_size()?;
    Ok(report)
}
impl ToolsReport {
    pub fn check_size(&self) -> Result<()> {
        struct Budget(usize);
        impl Write for Budget {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0 = self
                    .0
                    .checked_sub(bytes.len())
                    .ok_or_else(|| std::io::Error::other("report limit"))?;
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        serde_json::to_writer(Budget(MAX_REPORT_BYTES), self)
            .map_err(|_| Error(ErrorCode::Unavailable))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    SharePath,
    Port,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    TrustedPackageAndServiceIdentity,
    ExplicitServiceAccountAndAccessPolicy,
    VerifiedStorageOwnershipAndPort,
    CurrentAuthorityClaimsAndLocks,
    AtomicSaveApplyAndRecovery,
    VerifiedLoadedServiceResult,
}
const GATES: [Gate; 6] = [
    Gate::TrustedPackageAndServiceIdentity,
    Gate::ExplicitServiceAccountAndAccessPolicy,
    Gate::VerifiedStorageOwnershipAndPort,
    Gate::CurrentAuthorityClaimsAndLocks,
    Gate::AtomicSaveApplyAndRecovery,
    Gate::VerifiedLoadedServiceResult,
];
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConfigPreview {
    pub desired: Config,
    pub changed: Vec<Field>,
    /// False means equal stored fields, never a verified applied result.
    pub save_required: bool,
    pub applied_configuration: AppliedConfiguration,
    pub unresolved: &'static [Gate],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Blocked {
    ConfigObservation,
    ConfigUnsupported,
    PayloadObservation,
    PayloadMissing,
    PayloadUnsupported,
    ServiceObservation,
    ServiceBusy,
    ServiceUnsupported,
}
/// No artifact, receipt, executable approval, persistence or service operation.
pub fn preview_config(
    now: i64,
    facts: Facts<'_>,
    desired: Config,
) -> Result<std::result::Result<ConfigPreview, Blocked>> {
    let report = observe(now, facts)?;
    let (current, absent) = match report.config.value {
        Some(ConfigState::Defaults { config }) => (config, true),
        Some(ConfigState::Stored { config }) => (config, false),
        Some(ConfigState::Unknown {
            reason: ConfigIssue::Unavailable | ConfigIssue::Incomplete,
        }) => return Ok(Err(Blocked::ConfigObservation)),
        Some(ConfigState::Unknown { .. }) => return Ok(Err(Blocked::ConfigUnsupported)),
        None => return Ok(Err(Blocked::ConfigObservation)),
    };
    match report.payload.value {
        Some(PayloadState::Present) => (),
        Some(PayloadState::Absent) => return Ok(Err(Blocked::PayloadMissing)),
        Some(PayloadState::UnexpectedType) => return Ok(Err(Blocked::PayloadUnsupported)),
        Some(PayloadState::Unavailable) | None => return Ok(Err(Blocked::PayloadObservation)),
    }
    match report.service.value {
        Some(
            ServiceState::Active
            | ServiceState::Inactive
            | ServiceState::Failed
            | ServiceState::NotFound,
        ) => (),
        Some(ServiceState::Activating | ServiceState::Deactivating | ServiceState::Reloading) => {
            return Ok(Err(Blocked::ServiceBusy));
        }
        Some(ServiceState::Masked | ServiceState::LoadError) => {
            return Ok(Err(Blocked::ServiceUnsupported));
        }
        Some(ServiceState::Unknown(_)) | None => return Ok(Err(Blocked::ServiceObservation)),
    }
    let mut changed = Vec::new();
    if current.share_path != desired.share_path {
        changed.push(Field::SharePath);
    }
    if current.port != desired.port {
        changed.push(Field::Port);
    }
    Ok(Ok(ConfigPreview {
        save_required: absent || !changed.is_empty(),
        desired,
        changed,
        applied_configuration: AppliedConfiguration::NotEstablished,
        unresolved: &GATES,
    }))
}

#[cfg(test)]
mod tests;
