//! Pure supplied-byte drafts. Artifacts may contain secrets; never log or serialize them.
use serde::Serialize;
use std::{fmt, ops::Range};

mod ini;
mod json;

pub const MAX_INPUT_BYTES: usize = 64 * 1024;
pub const MAX_OUTPUT_BYTES: usize = MAX_INPUT_BYTES;
pub const MAX_PATH_BYTES: usize = 512;
pub const MAX_FIELDS: usize = 128;
pub const MAX_LINES: usize = 2048;
pub const MAX_DEPTH: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Application {
    TransmissionLegacy,
    Sabnzbd,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DownloadDirectories {
    pub complete: String,
    pub incomplete: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    CompleteDirectory,
    IncompleteDirectory,
    IncompleteEnabled,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigError {
    InvalidInput,
    Unavailable,
    Unsupported,
    Limit,
}
type Result<T> = std::result::Result<T, ConfigError>;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigGate {
    ProtectedFileAndCurrentInstance,
    VerifiedStoragePathsAndNumericIdentity,
    VerifiedServiceQuiescence,
    CurrentAuthorityClaimsAndLocks,
    AtomicBeforeImageAndRecovery,
    VerifiedLoadedServiceResult,
}
const GATES: [ConfigGate; 6] = [
    ConfigGate::ProtectedFileAndCurrentInstance,
    ConfigGate::VerifiedStoragePathsAndNumericIdentity,
    ConfigGate::VerifiedServiceQuiescence,
    ConfigGate::CurrentAuthorityClaimsAndLocks,
    ConfigGate::AtomicBeforeImageAndRecovery,
    ConfigGate::VerifiedLoadedServiceResult,
];
/// No Deserialize/Serialize: source-bearing artifacts are private executor material.
pub struct ConfigDraft {
    application: Application,
    desired: DownloadDirectories,
    changed: Vec<Field>,
    artifact: Option<Vec<u8>>,
}
impl fmt::Debug for ConfigDraft {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfigDraft")
            .field("application", &self.application)
            .field("changed", &self.changed)
            .field("artifact_bytes", &self.artifact.as_ref().map(Vec::len))
            .finish_non_exhaustive()
    }
}
#[derive(Serialize)]
pub struct ConfigSummary<'a> {
    pub version: u16,
    pub application: Application,
    pub desired: &'a DownloadDirectories,
    pub changed: &'a [Field],
    pub artifact_bytes: Option<usize>,
    pub unresolved: &'static [ConfigGate],
}
impl ConfigDraft {
    /// May contain existing passwords/API keys. Never return through diagnostic APIs.
    pub fn artifact(&self) -> Option<&[u8]> {
        self.artifact.as_deref()
    }
    pub fn summary(&self) -> ConfigSummary<'_> {
        ConfigSummary {
            version: 1,
            application: self.application,
            desired: &self.desired,
            changed: &self.changed,
            artifact_bytes: self.artifact.as_ref().map(Vec::len),
            unresolved: &GATES,
        }
    }
}
fn path(value: &str) -> bool {
    value.len() <= MAX_PATH_BYTES
        && value.starts_with('/')
        && value != "/"
        && !value.contains(['$', '{', '}', '\\', '"'])
        && !value
            .chars()
            .any(|c| c.is_control() || (c.is_whitespace() && c != ' '))
        && value[1..]
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != ".." && s.trim() == s)
}
fn beneath(a: &str, b: &str) -> bool {
    a == b || a.strip_prefix(b).is_some_and(|s| s.starts_with('/'))
}
struct Patch {
    range: Range<usize>,
    bytes: Vec<u8>,
}
fn apply(source: &[u8], mut patches: Vec<Patch>) -> Result<Option<Vec<u8>>> {
    if patches.is_empty() {
        return Ok(None);
    }
    patches.sort_by_key(|p| (p.range.start, p.range.end));
    let mut result = Vec::new();
    let mut cursor = 0;
    let append = |out: &mut Vec<u8>, bytes: &[u8]| -> Result<()> {
        if bytes.len() > MAX_OUTPUT_BYTES - out.len() {
            return Err(ConfigError::Limit);
        }
        out.extend_from_slice(bytes);
        Ok(())
    };
    for patch in patches {
        if patch.range.start < cursor
            || patch.range.end < patch.range.start
            || patch.range.end > source.len()
        {
            return Err(ConfigError::Unsupported);
        }
        append(&mut result, &source[cursor..patch.range.start])?;
        append(&mut result, &patch.bytes)?;
        cursor = patch.range.end;
    }
    append(&mut result, &source[cursor..])?;
    Ok(Some(result))
}
/// Bytes are supplied facts, not a protected owner, current read or effect authority.
pub fn plan(
    application: Application,
    desired: &DownloadDirectories,
    source: Option<&[u8]>,
) -> Result<ConfigDraft> {
    if !path(&desired.complete)
        || desired.incomplete.as_ref().is_some_and(|s| {
            !path(s) || beneath(s, &desired.complete) || beneath(&desired.complete, s)
        })
    {
        return Err(ConfigError::InvalidInput);
    }
    let source = source.ok_or(ConfigError::Unavailable)?;
    if source.len() > MAX_INPUT_BYTES {
        return Err(ConfigError::Limit);
    }
    let text = std::str::from_utf8(source).map_err(|_| ConfigError::Unsupported)?;
    if text
        .chars()
        .any(|c| c.is_control() && !['\n', '\r', '\t'].contains(&c))
    {
        return Err(ConfigError::Unsupported);
    }
    let (changed, patches) = match application {
        Application::TransmissionLegacy => json::patches(source)?,
        Application::Sabnzbd => ini::patches(text)?,
    }
    .prepare(desired)?;
    let artifact = apply(source, patches)?;
    if application == Application::Sabnzbd
        && artifact.as_ref().is_some_and(|bytes| {
            bytes.iter().filter(|b| **b == b'\n').count() + usize::from(!bytes.ends_with(b"\n"))
                > MAX_LINES
        })
    {
        return Err(ConfigError::Limit);
    }
    Ok(ConfigDraft {
        application,
        desired: desired.clone(),
        changed,
        artifact,
    })
}
struct Setting {
    key: &'static str,
    field: Field,
    value: Option<(String, Range<usize>)>,
}
struct Layout {
    settings: Vec<Setting>,
    insertion: usize,
    prefix: String,
    suffix: String,
    newline: &'static str,
    fields: usize,
    json: bool,
}
impl Layout {
    fn prepare(self, desired: &DownloadDirectories) -> Result<(Vec<Field>, Vec<Patch>)> {
        let mut changed = Vec::new();
        let mut patches = Vec::new();
        let mut missing = Vec::new();
        for setting in self.settings {
            let wanted = match setting.field {
                Field::CompleteDirectory => &desired.complete,
                Field::IncompleteDirectory => match &desired.incomplete {
                    Some(s) => s,
                    None => continue,
                },
                Field::IncompleteEnabled => {
                    if desired.incomplete.is_some() {
                        "true"
                    } else {
                        continue;
                    }
                }
            };
            if setting.value.as_ref().is_some_and(|(old, _)| old == wanted) {
                continue;
            }
            changed.push(setting.field);
            let bytes = if setting.field == Field::IncompleteEnabled {
                b"true".to_vec()
            } else if self.json {
                serde_json::to_vec(wanted).map_err(|_| ConfigError::Unsupported)?
            } else {
                format!("\"{wanted}\"").into_bytes()
            };
            if let Some((_, range)) = setting.value {
                patches.push(Patch { range, bytes });
            } else {
                missing.push((setting.key, bytes));
            }
        }
        if self.fields + missing.len() > MAX_FIELDS {
            return Err(ConfigError::Limit);
        }
        if !missing.is_empty() {
            let mut bytes = self.prefix.into_bytes();
            for (i, (key, value)) in missing.iter().enumerate() {
                if self.json {
                    if i > 0 {
                        bytes.extend_from_slice(format!(",{}", self.newline).as_bytes());
                    }
                    bytes.extend_from_slice(format!("  \"{key}\": ").as_bytes());
                    bytes.extend_from_slice(value);
                } else {
                    bytes.extend_from_slice(format!("{key} = ").as_bytes());
                    bytes.extend_from_slice(value);
                    bytes.extend_from_slice(self.suffix.as_bytes());
                }
            }
            patches.push(Patch {
                range: self.insertion..self.insertion,
                bytes,
            });
        }
        Ok((changed, patches))
    }
}
#[cfg(test)]
mod tests;
