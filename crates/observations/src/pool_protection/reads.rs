use limeos_domain::*;
use std::collections::BTreeMap;
use std::result::Result;

pub const STORAGE_READ_MAX_BYTES: usize = 200_000;
pub const STORAGE_READ_MAX_LINE_BYTES: usize = 4096;
pub const STORAGE_LOG_MAX_EVENTS: usize = 1000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolMountObservation {
    pub mountpoint: String,
    pub filesystem: String,
    pub mount_id: u64,
    pub total_bytes: Option<u64>,
    pub free_bytes: Option<u64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolReadHealth {
    Disabled,
    Mounted,
    Missing,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolReadStatus {
    pub pool_id: String,
    pub health: PoolReadHealth,
    pub branches: usize,
    pub total_bytes: Option<u64>,
    pub free_bytes: Option<u64>,
}
/// A mount-table/statvfs projection for display, never protection-source proof.
pub fn pool_status(
    config: &PoolsConfig,
    mounts: &[PoolMountObservation],
) -> StoragePlanningResult<Vec<PoolReadStatus>> {
    config.validate()?;
    Ok(config
        .pools
        .iter()
        .map(|pool| {
            let matches: Vec<_> = mounts
                .iter()
                .filter(|m| m.mountpoint == pool.mount_point)
                .collect();
            let verified = matches.len() == 1
                && matches[0].filesystem == "fuse.mergerfs"
                && matches[0].mount_id > 0;
            let health = if !pool.enabled {
                PoolReadHealth::Disabled
            } else if matches.is_empty() {
                PoolReadHealth::Missing
            } else if verified {
                PoolReadHealth::Mounted
            } else {
                PoolReadHealth::Unknown
            };
            let capacity = if verified {
                matches[0]
                    .total_bytes
                    .zip(matches[0].free_bytes)
                    .filter(|(t, f)| f <= t)
            } else {
                None
            };
            PoolReadStatus {
                pool_id: pool.id.clone(),
                health,
                branches: pool.branches.len(),
                total_bytes: capacity.map(|(t, _)| t),
                free_bytes: capacity.map(|(_, f)| f),
            }
        })
        .collect())
}

/// Captured by a future bounded runner. Truncation and termination are mandatory
/// evidence, never inferred from whether stdout looks successful.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedStorageRead {
    pub termination: StorageReadTermination,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}
fn failure(result: &CapturedStorageRead, detail: &str) -> StorageReadFailure {
    let stderr = sanitize_storage_text(
        &String::from_utf8_lossy(
            &result.stderr[..result.stderr.len().min(STORAGE_READ_MAX_LINE_BYTES)],
        ),
        1024,
    );
    StorageReadFailure {
        termination: Some(result.termination.clone()),
        reason: if result.termination == (StorageReadTermination::Exited { code: 0 }) {
            StorageReadFailureReason::MalformedOutput
        } else {
            StorageReadFailureReason::ProcessFailure
        },
        detail: if stderr.is_empty() {
            detail.into()
        } else {
            stderr
        },
    }
}
fn output(result: &CapturedStorageRead) -> Result<&str, StorageReadFailure> {
    if result.termination != (StorageReadTermination::Exited { code: 0 }) {
        return Err(failure(result, "Storage read failed."));
    }
    if result.stdout_truncated
        || result.stderr_truncated
        || result.stdout.len().saturating_add(result.stderr.len()) > STORAGE_READ_MAX_BYTES
    {
        return Err(StorageReadFailure {
            termination: Some(result.termination.clone()),
            reason: StorageReadFailureReason::OutputLimit,
            detail: "Storage read exceeded its output limit.".into(),
        });
    }
    let text = std::str::from_utf8(&result.stdout)
        .map_err(|_| failure(result, "Storage read is not UTF-8."))?;
    if text.lines().any(|line| {
        line.len() > STORAGE_READ_MAX_LINE_BYTES
            || line
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\t' | '\r'))
    }) {
        return Err(failure(result, "Storage read contains invalid text."));
    }
    Ok(text)
}

/// Strip terminal control sequences and redact credential-bearing lines. Treat
/// resulting text as literal UI text; never HTML or authority.
pub fn sanitize_storage_text(text: &str, max_bytes: usize) -> String {
    let mut clean = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.next() {
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' || (c == '\u{1b}' && chars.next() == Some('\\')) {
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if !c.is_control() || matches!(c, '\n' | '\t') {
            clean.push(c);
        }
    }
    let mut safe = String::new();
    for line in clean.lines() {
        let lower = line.to_ascii_lowercase();
        let sensitive = [
            "password",
            "passwd",
            "secret",
            "token",
            "authorization",
            "bearer ",
            "api_key",
            "apikey",
            "private key",
            "://",
        ]
        .iter()
        .any(|key| lower.contains(key));
        let line = if sensitive {
            "[credential-bearing line redacted]"
        } else {
            line
        };
        for c in line.chars().chain(std::iter::once('\n')) {
            if safe.len() + c.len_utf8() > max_bytes {
                return safe.trim_end().into();
            }
            safe.push(c);
        }
    }
    safe.trim_end().into()
}

const DIFF_KEYS: [&str; 6] = ["added", "removed", "updated", "moved", "copied", "restored"];
pub fn parse_snapraid_diff(
    result: &CapturedStorageRead,
) -> Result<SnapraidDiff, StorageReadFailure> {
    let text = output(result)?;
    let invalid = || failure(result, "SnapRAID diff output is incomplete or unparseable.");
    let mut counts = BTreeMap::new();
    let mut exit = None;
    let mut tagged = false;
    for line in text.lines().map(str::trim) {
        let fields: Vec<_> = line.split_whitespace().collect();
        let pair = if let Some(rest) = line.strip_prefix("summary:") {
            tagged = true;
            let (key, value) = rest.split_once(':').ok_or_else(invalid)?;
            if key == "exit" {
                if exit.replace(value).is_some() || !matches!(value, "equal" | "diff") {
                    return Err(invalid());
                }
                None
            } else if DIFF_KEYS.contains(&key) {
                Some((key, value))
            } else if key == "equal" && value.parse::<u64>().is_ok() {
                None
            } else {
                return Err(invalid());
            }
        } else if fields.iter().any(|s| DIFF_KEYS.contains(s)) {
            if fields.len() != 2 || !DIFF_KEYS.contains(&fields[1]) {
                return Err(invalid());
            }
            Some((fields[1], fields[0]))
        } else {
            None
        };
        if let Some((key, value)) = pair {
            if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(invalid());
            }
            let value = value.parse::<u64>().map_err(|_| invalid())?;
            if counts.insert(key, value).is_some() {
                return Err(invalid());
            }
        }
    }
    if counts.len() != 6
        || (tagged && exit.is_none())
        || (exit == Some("equal") && counts.values().any(|n| *n > 0))
    {
        return Err(invalid());
    }
    Ok(SnapraidDiff {
        added: counts["added"],
        removed: counts["removed"],
        updated: counts["updated"],
        moved: counts["moved"],
        copied: counts["copied"],
        restored: counts["restored"],
    })
}
/// Bind a parsed read to the exact configuration and fresh source snapshot used
/// by the runner. This function does not manufacture fresh storage observations.
pub fn snapraid_diff_evidence(
    config: &SnapraidConfig,
    sources: &ProtectionSourceEvidence,
    observed_at: i64,
    result: &CapturedStorageRead,
) -> SnapraidDiffEvidence {
    let outcome = match parse_snapraid_diff(result) {
        Ok(diff) => SnapraidDiffOutcome::Parsed(diff),
        Err(error) => SnapraidDiffOutcome::Unavailable(error),
    };
    SnapraidDiffEvidence {
        observed_at,
        configuration: config.clone(),
        topology_digest: sources.inventory.topology_digest.clone(),
        mounts_digest: sources.inventory.mounts_digest.clone(),
        outcome,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapraidHealth {
    Healthy,
    SyncRequired,
    Errors,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapraidReadStatus {
    pub health: SnapraidHealth,
    pub missing_files: Option<u64>,
    pub damaged_files: Option<u64>,
}
pub fn parse_snapraid_status(
    result: &CapturedStorageRead,
) -> Result<SnapraidReadStatus, StorageReadFailure> {
    let text = output(result)?;
    let mut healthy = false;
    let mut sync = false;
    let (mut missing, mut damaged) = (None, None);
    for line in text.lines().map(str::trim) {
        let lower = line.to_ascii_lowercase();
        healthy |= line == "No error detected." || line == "No error detected";
        sync |= lower == "sync required"
            || lower == "sync required."
            || lower == "the array is not fully synced.";
        let fields: Vec<_> = lower.split_whitespace().collect();
        for (label, count) in [("missing", &mut missing), ("damaged", &mut damaged)] {
            let value = if fields.len() == 2 && fields[1] == label {
                Some(fields[0])
            } else if fields.len() == 3 && fields[0] == label && fields[1] == "file" {
                Some(fields[2])
            } else {
                None
            };
            if let Some(value) = value {
                let number = value
                    .parse::<u64>()
                    .map_err(|_| failure(result, "Invalid SnapRAID status count."))?;
                if count.replace(number).is_some() {
                    return Err(failure(result, "Duplicate SnapRAID status count."));
                }
            }
        }
    }
    let errors = missing.is_some_and(|n| n > 0) || damaged.is_some_and(|n| n > 0);
    let health = if errors {
        SnapraidHealth::Errors
    } else if sync {
        SnapraidHealth::SyncRequired
    } else if healthy {
        SnapraidHealth::Healthy
    } else {
        return Err(failure(result, "SnapRAID status is unknown."));
    };
    Ok(SnapraidReadStatus {
        health,
        missing_files: missing,
        damaged_files: damaged,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct SnapraidRunProgress {
    pub blockpos: u64,
    pub countpos: u64,
    pub countsize: u64,
    pub percent: u8,
    pub eta: u64,
    pub size_speed: f64,
    pub cpu: f64,
    pub elapsed: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapraidLogMessage {
    pub level: String,
    pub message: String,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnapraidLogView {
    pub summary: BTreeMap<String, u64>,
    pub exit: Option<String>,
    pub scan_counts: BTreeMap<String, u64>,
    pub progress: Option<SnapraidRunProgress>,
    pub messages: Vec<SnapraidLogMessage>,
    pub ignored_lines: usize,
}
fn unescape(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            result.push(c);
            continue;
        }
        match chars.next() {
            Some('d') => result.push(':'),
            Some('n') => result.push('\n'),
            Some('r') => result.push('\r'),
            Some('\\') => result.push('\\'),
            Some(other) => {
                result.push('\\');
                result.push(other);
            }
            None => result.push('\\'),
        }
    }
    result
}
/// Aggregate only the supported tag subset. Paths, argv and configuration tags
/// are omitted. A summary exit tag never substitutes for process termination.
pub fn parse_snapraid_log(bytes: &[u8]) -> Result<SnapraidLogView, StorageReadFailure> {
    let invalid = || StorageReadFailure {
        termination: None,
        reason: StorageReadFailureReason::MalformedOutput,
        detail: "SnapRAID log contains unsupported or malformed evidence.".into(),
    };
    if bytes.len() > STORAGE_READ_MAX_BYTES {
        return Err(StorageReadFailure {
            termination: None,
            reason: StorageReadFailureReason::OutputLimit,
            detail: "SnapRAID log exceeded its output limit.".into(),
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| invalid())?;
    let mut view = SnapraidLogView::default();
    for (events, line) in text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .enumerate()
    {
        if line.len() > STORAGE_READ_MAX_LINE_BYTES || events >= STORAGE_LOG_MAX_EVENTS {
            return Err(StorageReadFailure {
                termination: None,
                reason: StorageReadFailureReason::OutputLimit,
                detail: "SnapRAID log exceeded its event limit.".into(),
            });
        }
        let fields: Vec<_> = line.split(':').collect();
        match fields.as_slice() {
            ["summary", "exit", value]
                if [
                    "equal",
                    "diff",
                    "ok",
                    "error",
                    "recovered",
                    "recoverable",
                    "unrecoverable",
                ]
                .contains(value) =>
            {
                if view.exit.replace((*value).into()).is_some() {
                    return Err(invalid());
                }
            }
            ["summary", key, value]
                if DIFF_KEYS.contains(key)
                    || [
                        "equal",
                        "error_file",
                        "error_io",
                        "error_data",
                        "recovered",
                        "unrecoverable",
                    ]
                    .contains(key) =>
            {
                let value = value.parse::<u64>().map_err(|_| invalid())?;
                if view.summary.insert((*key).into(), value).is_some() {
                    return Err(invalid());
                }
            }
            ["scan", key, ..]
                if [
                    "equal", "add", "remove", "update", "move", "copy", "restore",
                ]
                .contains(key)
                    && fields.len() >= 4 =>
            {
                *view.scan_counts.entry((*key).into()).or_default() += 1;
            }
            [
                "run",
                "pos",
                block,
                count,
                size,
                percent,
                eta,
                speed,
                cpu,
                elapsed,
            ] => {
                let p = SnapraidRunProgress {
                    blockpos: block.parse().map_err(|_| invalid())?,
                    countpos: count.parse().map_err(|_| invalid())?,
                    countsize: size.parse().map_err(|_| invalid())?,
                    percent: percent.parse().map_err(|_| invalid())?,
                    eta: eta.parse().map_err(|_| invalid())?,
                    size_speed: speed.parse().map_err(|_| invalid())?,
                    cpu: cpu.parse().map_err(|_| invalid())?,
                    elapsed: elapsed.parse().map_err(|_| invalid())?,
                };
                if p.percent > 100
                    || !p.size_speed.is_finite()
                    || p.size_speed < 0.0
                    || !p.cpu.is_finite()
                    || p.cpu < 0.0
                {
                    return Err(invalid());
                }
                view.progress = Some(p);
            }
            ["msg", level, message]
                if ["status", "info", "warning", "error", "fatal"].contains(level) =>
            {
                view.messages.push(SnapraidLogMessage {
                    level: (*level).into(),
                    message: sanitize_storage_text(&unescape(message), 1024),
                });
            }
            ["run", "begin", _, _, _] | ["run", "end"] => {}
            ["summary" | "scan" | "run" | "msg", ..] => return Err(invalid()),
            _ => view.ignored_lines += 1,
        }
    }
    Ok(view)
}
