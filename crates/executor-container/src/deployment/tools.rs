//! Decode supplied legacy config and fixed-service properties. No IO or effects.
use super::media::Object;
use limeos_domain::deployment::tools::{
    Config, ConfigIssue, ConfigState, DEFAULT_PORT, DEFAULT_SHARE_PATH, ServiceIssue, ServiceState,
};
use serde::Deserialize;
use std::fmt;

pub const MAX_CONFIG_BYTES: usize = 16 * 1024;
pub const MAX_SERVICE_BYTES: usize = 512;

/// Absence requires a successful protected lookup of the fixed config target.
#[derive(Clone, Copy)]
pub enum ConfigRead<'a> {
    Absent,
    Unavailable,
    Incomplete,
    /// Complete bounded bytes from a regular file; never an accepted prefix.
    Bytes(&'a [u8]),
}
impl fmt::Debug for ConfigRead<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent => f.write_str("Absent"),
            Self::Unavailable => f.write_str("Unavailable"),
            Self::Incomplete => f.write_str("Incomplete"),
            Self::Bytes(bytes) => f.debug_tuple("Bytes").field(&bytes.len()).finish(),
        }
    }
}
fn default_path() -> String {
    DEFAULT_SHARE_PATH.into()
}
fn default_port() -> u32 {
    u32::from(DEFAULT_PORT)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyConfig {
    #[serde(default = "default_path")]
    share_path: String,
    #[serde(default = "default_port")]
    port: u32,
    #[serde(default)]
    extra_args: String,
}
pub fn config(read: ConfigRead<'_>) -> ConfigState {
    let bytes = match read {
        ConfigRead::Absent => {
            return ConfigState::Defaults {
                config: Config::default(),
            };
        }
        ConfigRead::Unavailable => {
            return ConfigState::Unknown {
                reason: ConfigIssue::Unavailable,
            };
        }
        ConfigRead::Incomplete => {
            return ConfigState::Unknown {
                reason: ConfigIssue::Incomplete,
            };
        }
        ConfigRead::Bytes(bytes) => bytes,
    };
    let decode = || {
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(ConfigIssue::Limit);
        }
        let Object(wire): Object<LegacyConfig> =
            serde_json::from_slice(bytes).map_err(|_| ConfigIssue::InvalidDocument)?;
        if !wire.extra_args.trim().is_empty() {
            return Err(ConfigIssue::UnsupportedOptions);
        }
        Config::new(&wire.share_path, wire.port)
    };
    match decode() {
        Ok(config) => ConfigState::Stored { config },
        Err(reason) => ConfigState::Unknown { reason },
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureOutcome {
    Exited(i32),
    TimedOut,
    Failed,
    Pending,
}
/// Expected fixed query: systemctl show copyparty.service, LoadState/ActiveState.
/// Collector owns target selection, original time, budgets and stdout completeness.
pub struct ServiceCapture<'a> {
    pub outcome: CaptureOutcome,
    pub complete: bool,
    pub stdout: &'a [u8],
}
impl fmt::Debug for ServiceCapture<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceCapture")
            .field("outcome", &self.outcome)
            .field("complete", &self.complete)
            .field("stdout_bytes", &self.stdout.len())
            .finish()
    }
}
pub fn service(capture: ServiceCapture<'_>) -> ServiceState {
    use ServiceIssue as I;
    use ServiceState as S;
    if capture.outcome != CaptureOutcome::Exited(0) {
        return S::Unknown(I::Unavailable);
    }
    if !capture.complete {
        return S::Unknown(I::Incomplete);
    }
    if capture.stdout.len() > MAX_SERVICE_BYTES {
        return S::Unknown(I::Limit);
    }
    let parse = || {
        let text = std::str::from_utf8(capture.stdout).ok()?;
        let mut load = None;
        let mut active = None;
        for line in text.lines() {
            let (key, value) = line.split_once('=')?;
            let slot = match key {
                "LoadState" => &mut load,
                "ActiveState" => &mut active,
                _ => return None,
            };
            if slot.replace(value).is_some() {
                return None;
            }
        }
        match (load?, active?) {
            ("not-found", "inactive") => Some(S::NotFound),
            ("masked", "inactive" | "failed") => Some(S::Masked),
            ("error" | "bad-setting", "inactive" | "failed") => Some(S::LoadError),
            ("loaded", state) => match state {
                "active" => Some(S::Active),
                "inactive" => Some(S::Inactive),
                "failed" => Some(S::Failed),
                "activating" => Some(S::Activating),
                "deactivating" => Some(S::Deactivating),
                "reloading" => Some(S::Reloading),
                _ => None,
            },
            _ => None,
        }
    };
    parse().unwrap_or(S::Unknown(I::InvalidProperties))
}

#[cfg(test)]
mod tests;
