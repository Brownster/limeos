//! Pure frozen health-settings migration. No file, credential, Store or authority access.
use super::{AlertKind, AlertPolicy, Silence, valid_time};
use serde::Serialize;
use serde::de::IgnoredAny;
use serde_json::{Map, Value};

pub const MAX_POLICY_BYTES: usize = 64 * 1024;
pub const MAX_ENVIRONMENT_BYTES: usize = 4096;
const MAX_JSON_DEPTH: usize = 64;
const KINDS: [(AlertKind, &str); 4] = [
    (AlertKind::Container, "container"),
    (AlertKind::Smart, "smart"),
    (AlertKind::Mount, "mount"),
    (AlertKind::Snapraid, "snapraid"),
];

/// Selected non-secret fields only. Never pass the complete process environment.
#[derive(Clone, Copy, Default)]
pub struct LegacyHealthEnvironment<'a> {
    pub poll_seconds: Option<&'a str>,
    pub fail_threshold: Option<&'a str>,
    pub required_mounts: Option<&'a str>,
}
/// A protected adapter must supply the actual read outcome; this planner does no I/O.
#[derive(Clone, Copy)]
pub enum LegacyPolicyRead<'a> {
    NotConfigured,
    Missing,
    Unreadable,
    Contents(&'a str),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyFallback {
    NotConfigured,
    Missing,
    Unreadable,
    Malformed,
    Null,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "source", content = "reason", rename_all = "snake_case")]
pub enum PolicyOrigin {
    Environment(PolicyFallback),
    Policy,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingField {
    PollSeconds,
    FailThreshold,
    RequiredMounts,
    Policy,
    Categories,
    Silences,
    SilenceKind,
    SilenceKey,
    SilenceCreatedAt,
    SilenceExpiresAt,
    SilenceReason,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewReason {
    InputLimit,
    InvalidShape,
    InvalidValue,
    UnsupportedInteger,
    OutsideRuntimeBounds,
    TooManyEntries,
    InvalidPath,
    UnsupportedTimestamp,
    SubsecondExpiry,
}
/// Bounded diagnostics intentionally omit raw configuration and parser error text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct SettingsReview {
    pub field: SettingField,
    pub reason: ReviewReason,
    pub index: Option<usize>,
}
type Result<T> = std::result::Result<T, SettingsReview>;
fn issue(field: SettingField, reason: ReviewReason) -> SettingsReview {
    SettingsReview {
        field,
        reason,
        index: None,
    }
}

/// Original text is retained; the parsed instant cannot be supplied independently.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlannedTimestamp {
    text: String,
    unix_seconds: i64,
    micros: u32,
}
impl PlannedTimestamp {
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn unix_seconds(&self) -> i64 {
        self.unix_seconds
    }
    pub fn micros(&self) -> u32 {
        self.micros
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlannedSilence {
    pub kind: AlertKind,
    pub key: String,
    pub created_at: PlannedTimestamp,
    pub expires_at: Option<PlannedTimestamp>,
    pub reason: String,
}
/// A complete representable candidate, not persisted configuration or execution authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlannedHealthSettings {
    poll_seconds: u32,
    fail_threshold: u8,
    disabled: Vec<AlertKind>,
    required_mounts: Vec<String>,
    silences: Vec<PlannedSilence>,
    origin: PolicyOrigin,
}
impl PlannedHealthSettings {
    pub fn poll_seconds(&self) -> u32 {
        self.poll_seconds
    }
    pub fn fail_threshold(&self) -> u8 {
        self.fail_threshold
    }
    pub fn required_mounts(&self) -> &[String] {
        &self.required_mounts
    }
    pub fn silences(&self) -> &[PlannedSilence] {
        &self.silences
    }
    pub fn origin(&self) -> PolicyOrigin {
        self.origin
    }
    pub fn alert_policy(&self) -> AlertPolicy {
        AlertPolicy {
            fail_threshold: self.fail_threshold,
            disabled: self.disabled.clone(),
            silences: self
                .silences
                .iter()
                .map(|silence| Silence {
                    kind: silence.kind,
                    key: silence.key.clone(),
                    expires_at: silence.expires_at.as_ref().map(|at| at.unix_seconds),
                })
                .collect(),
        }
    }
}

// Python str.strip also includes these four ASCII information separators.
fn trim(value: &str) -> &str {
    value.trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}
fn integer(value: Option<&str>, default: u32, field: SettingField) -> Result<u32> {
    let value = value.unwrap_or("").trim();
    if value.chars().any(|c| !c.is_ascii() && c.is_numeric()) {
        return Err(issue(field, ReviewReason::UnsupportedInteger));
    }
    let (negative, digits) = match value.as_bytes().first() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };
    let bytes = digits.as_bytes();
    if bytes.is_empty()
        || bytes.iter().enumerate().any(|(i, byte)| {
            !(byte.is_ascii_digit()
                || *byte == b'_'
                    && i > 0
                    && bytes[i - 1].is_ascii_digit()
                    && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
        })
    {
        return Ok(default);
    }
    if negative {
        return Ok(1);
    }
    let mut number = 0u32;
    for byte in bytes.iter().filter(|byte| **byte != b'_') {
        number = number
            .checked_mul(10)
            .and_then(|n| n.checked_add(u32::from(*byte - b'0')))
            .ok_or(issue(field, ReviewReason::OutsideRuntimeBounds))?;
    }
    Ok(number.max(1))
}
fn json_depth(body: &str) -> Result<()> {
    let (mut depth, mut quoted, mut escape) = (0usize, false, false);
    for byte in body.bytes() {
        if escape {
            escape = false;
        } else if quoted && byte == b'\\' {
            escape = true;
        } else if byte == b'"' {
            quoted = !quoted;
        } else if !quoted {
            match byte {
                b'{' | b'[' => depth += 1,
                b'}' | b']' => depth = depth.saturating_sub(1),
                _ => {}
            }
            if depth > MAX_JSON_DEPTH {
                return Err(issue(SettingField::Policy, ReviewReason::InputLimit));
            }
        }
    }
    Ok(())
}
fn read_policy(input: LegacyPolicyRead<'_>) -> Result<(Option<Value>, PolicyOrigin)> {
    let fallback = |reason| (None, PolicyOrigin::Environment(reason));
    let body = match input {
        LegacyPolicyRead::NotConfigured => return Ok(fallback(PolicyFallback::NotConfigured)),
        LegacyPolicyRead::Missing => return Ok(fallback(PolicyFallback::Missing)),
        LegacyPolicyRead::Unreadable => return Ok(fallback(PolicyFallback::Unreadable)),
        LegacyPolicyRead::Contents(body) => body,
    };
    if body.len() > MAX_POLICY_BYTES {
        return Err(issue(SettingField::Policy, ReviewReason::InputLimit));
    }
    json_depth(body)?;
    // Syntax-only validation separates malformed legacy JSON (fallback) from a
    // valid JSON value that this bounded Rust representation cannot decode.
    if serde_json::from_str::<IgnoredAny>(body).is_err() {
        return Ok(fallback(PolicyFallback::Malformed));
    }
    let value: Value = serde_json::from_str(body)
        .map_err(|_| issue(SettingField::Policy, ReviewReason::InvalidValue))?;
    if value.is_null() {
        return Ok(fallback(PolicyFallback::Null));
    }
    let value = match value {
        Value::Object(mut object) if object.get("policy").is_some_and(Value::is_object) => {
            object.remove("policy").expect("checked policy object")
        }
        value => value,
    };
    Ok((Some(value), PolicyOrigin::Policy))
}
fn falsey(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(value) => !value,
        Value::Number(value) => value.as_f64() == Some(0.0),
        Value::String(value) => value.is_empty(),
        Value::Array(value) => value.is_empty(),
        Value::Object(value) => value.is_empty(),
    }
}
fn mount(value: &str, mounts: &mut Vec<String>) -> Result<()> {
    if !value.starts_with('/') || value.contains('\0') {
        return Err(issue(
            SettingField::RequiredMounts,
            ReviewReason::InvalidPath,
        ));
    }
    if value.len() > 250 || value.chars().any(char::is_control) {
        return Err(issue(
            SettingField::RequiredMounts,
            ReviewReason::OutsideRuntimeBounds,
        ));
    }
    if !mounts.iter().any(|old| old == value) {
        if mounts.len() >= super::MAX_ALERTS {
            return Err(issue(
                SettingField::RequiredMounts,
                ReviewReason::TooManyEntries,
            ));
        }
        mounts.push(value.into());
    }
    Ok(())
}
fn policy_mounts(object: &Map<String, Value>) -> Result<Vec<String>> {
    let mut mounts = Vec::new();
    if let Some(value) = object.get("required_mounts") {
        let values = value.as_array().ok_or(issue(
            SettingField::RequiredMounts,
            ReviewReason::InvalidShape,
        ))?;
        for (index, value) in values.iter().enumerate() {
            let result = value
                .as_str()
                .ok_or(issue(
                    SettingField::RequiredMounts,
                    ReviewReason::InvalidShape,
                ))
                .and_then(|value| mount(value, &mut mounts));
            result.map_err(|mut error| {
                error.index = Some(index);
                error
            })?;
        }
    }
    Ok(mounts)
}
fn categories(object: &Map<String, Value>) -> Result<Vec<AlertKind>> {
    let mut disabled = Vec::new();
    if let Some(value) = object.get("categories") {
        let values = value
            .as_object()
            .ok_or(issue(SettingField::Categories, ReviewReason::InvalidShape))?;
        for (index, (kind, name)) in KINDS.iter().enumerate() {
            if let Some(category) = values.get(*name) {
                let result = category
                    .as_object()
                    .ok_or(issue(SettingField::Categories, ReviewReason::InvalidShape))
                    .and_then(|category| {
                        category.get("enabled").map_or(Ok(true), |value| {
                            value
                                .as_bool()
                                .ok_or(issue(SettingField::Categories, ReviewReason::InvalidShape))
                        })
                    });
                if !result.map_err(|mut error| {
                    error.index = Some(index);
                    error
                })? {
                    disabled.push(*kind);
                }
            }
        }
    }
    Ok(disabled)
}

mod timestamp;
pub(super) fn policy_timestamp(value: &str) -> Option<PlannedTimestamp> {
    timestamp::parse(value)
}
fn silence(value: &Value, default_created_at: &str) -> Result<PlannedSilence> {
    let object = value
        .as_object()
        .ok_or(issue(SettingField::Silences, ReviewReason::InvalidShape))?;
    let name = object.get("kind").and_then(Value::as_str);
    let (kind, prefix) = KINDS
        .iter()
        .find(|(_, kind)| Some(*kind) == name)
        .ok_or(issue(SettingField::SilenceKind, ReviewReason::InvalidValue))?;
    let key = trim(
        object
            .get("key")
            .and_then(Value::as_str)
            .ok_or(issue(SettingField::SilenceKey, ReviewReason::InvalidShape))?,
    );
    if !key.starts_with(prefix) || key.as_bytes().get(prefix.len()) != Some(&b':') {
        return Err(issue(SettingField::SilenceKey, ReviewReason::InvalidValue));
    }
    if key.len() <= prefix.len() + 1 || key.len() > 256 || key.chars().any(char::is_control) {
        return Err(issue(
            SettingField::SilenceKey,
            ReviewReason::OutsideRuntimeBounds,
        ));
    }
    let metadata_text = |field: &'static str, fallback, target| {
        object
            .get(field)
            .filter(|value| !falsey(value))
            .map_or(Ok(fallback), |value| {
                value
                    .as_str()
                    .ok_or(issue(target, ReviewReason::InvalidShape))
            })
    };
    let created_at = timestamp::parse(metadata_text(
        "created_at",
        default_created_at,
        SettingField::SilenceCreatedAt,
    )?)
    .ok_or(issue(
        SettingField::SilenceCreatedAt,
        ReviewReason::UnsupportedTimestamp,
    ))?;
    let expires_at = object
        .get("expires_at")
        .filter(|value| !falsey(value))
        .map(|value| {
            let at = value.as_str().and_then(timestamp::parse).ok_or(issue(
                SettingField::SilenceExpiresAt,
                ReviewReason::UnsupportedTimestamp,
            ))?;
            if at.micros != 0 {
                return Err(issue(
                    SettingField::SilenceExpiresAt,
                    ReviewReason::SubsecondExpiry,
                ));
            }
            Ok(at)
        })
        .transpose()?;
    let reason = trim(metadata_text("reason", "", SettingField::SilenceReason)?);
    if reason.chars().count() > 200 {
        return Err(issue(SettingField::SilenceReason, ReviewReason::InputLimit));
    }
    Ok(PlannedSilence {
        kind: *kind,
        key: key.into(),
        created_at,
        expires_at,
        reason: reason.into(),
    })
}

/// Produce all settings or one review issue. No partial candidate is returned.
/// `default_created_at` is an explicit trusted migration observation time, used
/// only where the legacy normalizer would supply its wall clock.
pub fn plan_health_settings(
    environment: LegacyHealthEnvironment<'_>,
    policy: LegacyPolicyRead<'_>,
    default_created_at: &str,
) -> Result<PlannedHealthSettings> {
    for (field, value) in [
        (SettingField::PollSeconds, environment.poll_seconds),
        (SettingField::FailThreshold, environment.fail_threshold),
        (SettingField::RequiredMounts, environment.required_mounts),
    ] {
        if value.is_some_and(|value| value.len() > MAX_ENVIRONMENT_BYTES) {
            return Err(issue(field, ReviewReason::InputLimit));
        }
    }
    let poll_seconds = integer(environment.poll_seconds, 60, SettingField::PollSeconds)?;
    let fail_threshold = integer(environment.fail_threshold, 2, SettingField::FailThreshold)?;
    for (field, supported) in [
        (
            SettingField::PollSeconds,
            (30..=2_678_400).contains(&poll_seconds),
        ),
        (
            SettingField::FailThreshold,
            (1..=16).contains(&fail_threshold),
        ),
    ] {
        if !supported {
            return Err(issue(field, ReviewReason::OutsideRuntimeBounds));
        }
    }
    let (value, origin) = read_policy(policy)?;
    let empty = Map::new();
    let object = value
        .as_ref()
        .filter(|value| !falsey(value))
        .map(|value| {
            value
                .as_object()
                .ok_or(issue(SettingField::Policy, ReviewReason::InvalidShape))
        })
        .transpose()?
        .unwrap_or(&empty);
    let required_mounts = if value.is_none() {
        let mut mounts = Vec::new();
        for (index, path) in environment
            .required_mounts
            .unwrap_or("")
            .split(',')
            .map(trim)
            .filter(|path| !path.is_empty())
            .enumerate()
        {
            mount(path, &mut mounts).map_err(|mut error| {
                error.index = Some(index);
                error
            })?;
        }
        mounts
    } else {
        policy_mounts(object)?
    };
    let mut silences = Vec::new();
    if let Some(value) = object.get("silences") {
        let values = value
            .as_array()
            .ok_or(issue(SettingField::Silences, ReviewReason::InvalidShape))?;
        if values.len() > 64 {
            return Err(issue(SettingField::Silences, ReviewReason::TooManyEntries));
        }
        for (index, value) in values.iter().enumerate() {
            silences.push(silence(value, default_created_at).map_err(|mut error| {
                error.index = Some(index);
                error
            })?);
        }
    }
    let candidate = PlannedHealthSettings {
        poll_seconds,
        fail_threshold: fail_threshold as u8,
        disabled: categories(object)?,
        required_mounts,
        silences,
        origin,
    };
    candidate
        .alert_policy()
        .validate()
        .map_err(|_| issue(SettingField::Policy, ReviewReason::OutsideRuntimeBounds))?;
    Ok(candidate)
}

#[cfg(test)]
mod tests;
