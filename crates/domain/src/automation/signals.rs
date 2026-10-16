//! Frozen signal rules applied only to trusted, normalized collector facts.
use super::*;
use crate::{Error, ErrorCode, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartPolicy {
    Always,
    UnlessStopped,
    No,
    OnFailure,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerHealth {
    Healthy,
    Unhealthy,
    Starting,
    Unreported,
    Unavailable,
}
fn key(kind: &str, resource: &str) -> Result<String> {
    // Check before formatting/copying; keep the entire namespaced key <=256 bytes.
    if !super::text(resource, 256 - kind.len() - 1) {
        return Err(Error(ErrorCode::InvalidInput));
    }
    Ok(format!("{kind}:{resource}"))
}
pub fn container_signal(
    resource: &str,
    running: Option<bool>,
    health: ContainerHealth,
    policy: RestartPolicy,
) -> Result<Option<AlertSignal>> {
    let key = key("container", resource)?;
    if !matches!(policy, RestartPolicy::Always | RestartPolicy::UnlessStopped) {
        return Ok(None);
    }
    let (status, summary) = match (running, health) {
        (Some(false), _) => (SignalStatus::Failing, format!("{resource} is not running")),
        (Some(true), ContainerHealth::Unhealthy) => (
            SignalStatus::Failing,
            format!("{resource} healthcheck is failing"),
        ),
        (None, _) | (_, ContainerHealth::Unavailable) => (
            SignalStatus::Unavailable,
            format!("{resource} observation unavailable"),
        ),
        _ => (SignalStatus::Healthy, format!("{resource} is healthy")),
    };
    Ok(Some(AlertSignal {
        key,
        kind: AlertKind::Container,
        status,
        severity: Severity::Warning,
        summary,
    }))
}
pub fn smart_signal(
    resource: &str,
    passed: Option<bool>,
    summary: Option<&str>,
) -> Result<AlertSignal> {
    let key = key("smart", resource)?;
    if summary.is_some_and(|value| !super::text(value, 512)) {
        return Err(Error(ErrorCode::InvalidInput));
    }
    let (status, message) = match passed {
        Some(true) => (SignalStatus::Healthy, format!("{resource} SMART OK")),
        Some(false) => (
            SignalStatus::Failing,
            summary.map_or_else(
                || format!("{resource} SMART assessment FAILED"),
                str::to_owned,
            ),
        ),
        None => (
            SignalStatus::Unavailable,
            format!("{resource} SMART assessment unavailable"),
        ),
    };
    Ok(AlertSignal {
        key,
        kind: AlertKind::Smart,
        status,
        severity: Severity::Critical,
        summary: message,
    })
}
/// `present=None` means collection is partial/unavailable, never verified absence.
pub fn required_mount_signal(path: &str, present: Option<bool>) -> Result<AlertSignal> {
    if !path.starts_with('/') {
        return Err(Error(ErrorCode::InvalidInput));
    }
    let key = key("mount", path)?;
    let (status, suffix) = match present {
        Some(true) => (SignalStatus::Healthy, "is mounted"),
        Some(false) => (SignalStatus::Failing, "is not mounted"),
        None => (SignalStatus::Unavailable, "observation unavailable"),
    };
    Ok(AlertSignal {
        key,
        kind: AlertKind::Mount,
        status,
        severity: Severity::Critical,
        summary: format!("{path} {suffix}"),
    })
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtectionHealth {
    Unconfigured,
    Healthy,
    Error,
    NeedsSync,
    Unavailable,
}
pub fn protection_signal(
    state: ProtectionHealth,
    message: Option<&str>,
) -> Result<Option<AlertSignal>> {
    if message.is_some_and(|value| !super::text(value, 512)) {
        return Err(Error(ErrorCode::InvalidInput));
    }
    let (status, severity, fallback) = match state {
        ProtectionHealth::Unconfigured => return Ok(None),
        ProtectionHealth::Healthy => (
            SignalStatus::Healthy,
            Severity::Critical,
            "SnapRAID healthy",
        ),
        ProtectionHealth::Error => (
            SignalStatus::Failing,
            Severity::Critical,
            "SnapRAID reported an error",
        ),
        ProtectionHealth::NeedsSync => (
            SignalStatus::Failing,
            Severity::Warning,
            "SnapRAID needs a sync",
        ),
        ProtectionHealth::Unavailable => (
            SignalStatus::Unavailable,
            Severity::Warning,
            "SnapRAID observation unavailable",
        ),
    };
    Ok(Some(AlertSignal {
        key: "snapraid:status".into(),
        kind: AlertKind::Snapraid,
        status,
        severity,
        summary: message.unwrap_or(fallback).into(),
    }))
}
