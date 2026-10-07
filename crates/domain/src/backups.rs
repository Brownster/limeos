//! Backup archive admission: closed policy, inspected entry facts and the
//! deterministic restore manifest.
//!
//! This module is pure. An adapter streams and decodes an archive into an
//! [`InspectionReport`]; [`admit`] then decides, from trusted policy alone,
//! whether every entry maps to a managed destination. Archive contents never
//! choose a host path: a member is admitted only when a trusted legacy mapping
//! places it inside a registered managed resource.
//!
//! A manifest is evidence that one exact archive passed inspection. It is not
//! permission to restore; see `docs/p04-backup-archive-admission.md`.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MANIFEST_VERSION: u32 = 2;

/// Archive compression formats the legacy helper produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveFormat {
    TarGzip,
    TarZstd,
}

/// Finite limits owned by trusted policy. Archive metadata cannot relax them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveLimits {
    /// Compressed bytes read from the archive, including any trailing data.
    pub max_compressed_bytes: u64,
    /// Every decompressed byte: headers, extension records, data, padding.
    pub max_decompressed_bytes: u64,
    /// Declared size of one regular file.
    pub max_file_bytes: u64,
    /// Tar headers of every kind, including extension headers.
    pub max_entries: u64,
    /// Bytes in one effective member path.
    pub max_path_bytes: u32,
    /// Components in one effective member path.
    pub max_path_depth: u32,
    /// Bytes in one path component.
    pub max_component_bytes: u32,
    /// Declared size of one GNU long-name or pax extension record.
    pub max_metadata_bytes: u64,
    /// Sum of all extension record sizes in the archive.
    pub max_total_metadata_bytes: u64,
    /// Largest zstd window, as log2 bytes, the decoder may allocate.
    pub max_zstd_window_log: u32,
}

/// A trusted managed resource: one destination directory tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedResource {
    pub id: String,
    /// Absolute, normalized root the future executor resolves with protected
    /// descriptors. Informational in a manifest; never chosen by an archive.
    pub destination_root: String,
}

/// Trusted mapping from a legacy archive member prefix into a managed resource.
///
/// The frozen helper archived absolute source paths; GNU tar stored them
/// without the leading `/`, so `/etc/limeos/core.json` became the member
/// `etc/limeos/core.json`. A mapping names exactly which such prefixes are
/// supported, so a broad `/etc`, `/var` or `/home` allowlist cannot return.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyMapping {
    /// Relative archive prefix, such as `etc/limeos`. Matches the member equal
    /// to it or members beneath it on a component boundary.
    pub archive_prefix: String,
    pub resource: String,
    /// Relative path inside the resource; empty for the resource root.
    pub resource_prefix: String,
}

/// Closed admission policy. Every field is required; there are no defaults
/// an archive could rely on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionPolicy {
    pub revision: u64,
    pub formats: Vec<ArchiveFormat>,
    pub limits: ArchiveLimits,
    pub resources: Vec<ManagedResource>,
    pub legacy_mappings: Vec<LegacyMapping>,
}

/// Entry kinds the inspector reports. Redundant legacy self-links produce no
/// entry; every other tar type is rejected while streaming.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Directory,
}

/// Facts about one admitted-kind member, from its effective header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectedEntry {
    /// Zero-based index of the member's own header among all tar headers.
    pub header_index: u64,
    /// Effective path after GNU long-name or pax `path` records.
    pub archive_path: String,
    pub kind: EntryKind,
    /// Data bytes; zero for directories.
    pub size: u64,
    /// SHA-256 of the data, lowercase hex; files only.
    pub sha256: Option<String>,
    /// Mode bits as archived. Informational: never authorizes installation.
    pub archived_mode: u32,
}

/// The result of a complete inspection. Adapters return one only after the
/// whole archive has been read to a verified end; there is no partial report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectionReport {
    pub format: ArchiveFormat,
    /// SHA-256 of every compressed byte, lowercase hex.
    pub archive_sha256: String,
    pub compressed_bytes: u64,
    pub decompressed_bytes: u64,
    /// Every tar header, including extension headers.
    pub header_count: u64,
    pub metadata_bytes: u64,
    /// Zero-length self-links following an inspected regular file at the
    /// exact same archive path. These produce no entry or destination.
    pub coalesced_self_hardlinks: u64,
    pub entries: Vec<InspectedEntry>,
}

/// Why an archive was not admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingCode {
    // Policy and report integrity.
    InvalidPolicy,
    InconsistentReport,
    // Stream and format.
    UnrecognizedCompression,
    FormatNotAllowed,
    CompressedLimit,
    DecompressedLimit,
    DecoderMemoryLimit,
    Truncated,
    Malformed,
    TrailingData,
    Cancelled,
    /// A bounded input reader failed independently of archive syntax.
    Io,
    /// A checked buffer/catalog reservation failed.
    Allocation,
    // Counts and sizes.
    EntryLimit,
    FileSizeLimit,
    MetadataLimit,
    TotalMetadataLimit,
    // Unsupported constructs.
    UnsupportedHeaderFormat,
    UnsupportedEntryType,
    UnsupportedExtension,
    Symlink,
    Hardlink,
    LegacySelfHardlink,
    Device,
    Fifo,
    Sparse,
    AmbiguousName,
    ExtensionWithoutMember,
    DirectoryWithData,
    // Names and destinations.
    NonUtf8Name,
    UnsafeCharacter,
    AbsolutePath,
    ParentComponent,
    DotComponent,
    EmptyComponent,
    TrailingSlashOnFile,
    PathTooLong,
    PathTooDeep,
    ComponentTooLong,
    Unmapped,
    DuplicateDestination,
    FileDirectoryCollision,
    ParentIsFile,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub code: FindingCode,
    /// Header index of the offending member, when one is known.
    pub entry: Option<u64>,
    /// Bounded, control-character-free rendering of the member path.
    pub path: Option<String>,
    pub detail: Option<String>,
}

/// A typed refusal. It never carries a usable manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rejection {
    pub findings: Vec<Finding>,
}

impl Rejection {
    pub fn one(code: FindingCode, entry: Option<u64>, path: Option<&str>, detail: &str) -> Self {
        Rejection {
            findings: vec![Finding {
                code,
                entry,
                path: path.map(display_path),
                detail: (!detail.is_empty()).then(|| bounded(detail, 256)),
            }],
        }
    }

    pub fn codes(&self) -> Vec<FindingCode> {
        self.findings.iter().map(|f| f.code).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestEntry {
    pub resource: String,
    /// Relative path inside the resource; empty for the resource root.
    pub relative_path: String,
    pub kind: EntryKind,
    pub size: u64,
    pub sha256: Option<String>,
    /// Permission bits as archived (`mode & 0o777`). Special bits and archived
    /// ownership are dropped; installation modes come from trusted policy.
    pub archived_permissions: u32,
    pub archive_path: String,
}

/// Deterministic manifest bound to one exact inspected archive and policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreManifest {
    pub manifest_version: u32,
    pub archive_sha256: String,
    pub format: ArchiveFormat,
    pub policy_revision: u64,
    pub compressed_bytes: u64,
    pub decompressed_bytes: u64,
    pub header_count: u64,
    pub coalesced_self_hardlinks: u64,
    pub file_count: u64,
    pub directory_count: u64,
    pub file_bytes: u64,
    /// Sorted by resource, then relative path.
    pub entries: Vec<ManifestEntry>,
}

/// Prefixes too broad to map: mapping them would recreate the old authority.
const BROAD_PREFIXES: &[&str] = &[
    "var/lib",
    "var/log",
    "var/cache",
    "var/spool",
    "usr/lib",
    "usr/local",
    "usr/share",
    "etc/systemd",
    "etc/ssh",
    "etc/sudoers.d",
    "etc/apt",
];

fn bounded(value: &str, limit: usize) -> String {
    let mut out = String::new();
    for c in value.chars() {
        if out.len() + c.len_utf8() > limit {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}

/// Render an untrusted path for reports: escaped controls, bounded length.
pub fn display_path(path: &str) -> String {
    let escaped: String = path
        .chars()
        .flat_map(|c| {
            if c.is_control() {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect();
    bounded(&escaped, 512)
}

fn is_hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_'))
}

/// Validate one relative path made of literal components. Used for trusted
/// policy strings and, through [`check_member_path`], for archive members.
fn relative_components(path: &str) -> Result<Vec<&str>, FindingCode> {
    if path.starts_with('/') {
        return Err(FindingCode::AbsolutePath);
    }
    if path.is_empty() {
        return Ok(Vec::new());
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" => return Err(FindingCode::EmptyComponent),
            "." => return Err(FindingCode::DotComponent),
            ".." => return Err(FindingCode::ParentComponent),
            _ => {}
        }
        // Controls (C0, DEL, C1) and backslash have no place in managed paths
        // and render ambiguously in tools and logs.
        if part.chars().any(|c| c.is_control() || c == '\\') {
            return Err(FindingCode::UnsafeCharacter);
        }
        parts.push(part);
    }
    Ok(parts)
}

impl ArchiveLimits {
    fn validate(&self) -> Result<(), String> {
        let positive = [
            ("max_compressed_bytes", self.max_compressed_bytes),
            ("max_decompressed_bytes", self.max_decompressed_bytes),
            ("max_file_bytes", self.max_file_bytes),
            ("max_entries", self.max_entries),
            ("max_path_bytes", self.max_path_bytes.into()),
            ("max_path_depth", self.max_path_depth.into()),
            ("max_component_bytes", self.max_component_bytes.into()),
            ("max_metadata_bytes", self.max_metadata_bytes),
            ("max_total_metadata_bytes", self.max_total_metadata_bytes),
        ];
        if let Some((name, _)) = positive.iter().find(|(_, v)| *v == 0) {
            return Err(format!("{name} must be positive"));
        }
        if self.max_file_bytes > self.max_decompressed_bytes {
            return Err("max_file_bytes exceeds max_decompressed_bytes".into());
        }
        if self.max_metadata_bytes > self.max_total_metadata_bytes {
            return Err("max_metadata_bytes exceeds max_total_metadata_bytes".into());
        }
        if self.max_component_bytes > self.max_path_bytes {
            return Err("max_component_bytes exceeds max_path_bytes".into());
        }
        // zstd's own minimum window is 2^10; above 2^27 needs explicit opt-in
        // even in the reference decoder and would allow very large allocations.
        if !(10..=27).contains(&self.max_zstd_window_log) {
            return Err("max_zstd_window_log must be between 10 and 27".into());
        }
        Ok(())
    }
}

impl AdmissionPolicy {
    /// Reject an unusable policy before any archive is read.
    pub fn validate(&self) -> Result<(), Rejection> {
        let invalid =
            |detail: String| Rejection::one(FindingCode::InvalidPolicy, None, None, &detail);
        if self.formats.is_empty() {
            return Err(invalid("no archive format is allowed".into()));
        }
        self.limits.validate().map_err(invalid)?;
        let mut roots: Vec<&str> = Vec::new();
        let mut ids = BTreeSet::new();
        for resource in &self.resources {
            if !valid_id(&resource.id) || !ids.insert(resource.id.as_str()) {
                return Err(invalid(format!(
                    "invalid or duplicate resource id {:?}",
                    resource.id
                )));
            }
            let root = resource.destination_root.as_str();
            let rest = root
                .strip_prefix('/')
                .ok_or_else(|| invalid(format!("destination root {root:?} is not absolute")))?;
            match relative_components(rest) {
                Ok(parts) if !parts.is_empty() => {}
                _ => {
                    return Err(invalid(format!(
                        "destination root {root:?} is not normalized"
                    )));
                }
            }
            roots.push(root);
        }
        // Overlapping roots would let one file reach two resources.
        for (i, a) in roots.iter().enumerate() {
            for b in &roots[i + 1..] {
                let inside = |x: &str, y: &str| x == y || x.starts_with(&format!("{y}/"));
                if inside(a, b) || inside(b, a) {
                    return Err(invalid(format!(
                        "destination roots {a:?} and {b:?} overlap"
                    )));
                }
            }
        }
        let mut prefixes = BTreeSet::new();
        for mapping in &self.legacy_mappings {
            if !ids.contains(mapping.resource.as_str()) {
                return Err(invalid(format!(
                    "mapping names unknown resource {:?}",
                    mapping.resource
                )));
            }
            let parts = relative_components(&mapping.archive_prefix).map_err(|_| {
                invalid(format!(
                    "archive prefix {:?} is not normalized",
                    mapping.archive_prefix
                ))
            })?;
            let broad = parts.len() < 2
                || (parts[0] == "home" && parts.len() < 3)
                || BROAD_PREFIXES.contains(&mapping.archive_prefix.as_str());
            if broad {
                return Err(invalid(format!(
                    "archive prefix {:?} is too broad",
                    mapping.archive_prefix
                )));
            }
            relative_components(&mapping.resource_prefix).map_err(|_| {
                invalid(format!(
                    "resource prefix {:?} is not normalized",
                    mapping.resource_prefix
                ))
            })?;
            if !prefixes.insert(mapping.archive_prefix.as_str()) {
                return Err(invalid(format!(
                    "archive prefix {:?} is mapped twice",
                    mapping.archive_prefix
                )));
            }
        }
        Ok(())
    }

    /// Longest-prefix legacy mapping for a validated member path.
    fn map(&self, path: &str) -> Option<(&LegacyMapping, String)> {
        self.legacy_mappings
            .iter()
            .filter_map(|m| {
                let rest = if path == m.archive_prefix {
                    Some("")
                } else {
                    path.strip_prefix(&m.archive_prefix)
                        .and_then(|r| r.strip_prefix('/'))
                };
                rest.map(|r| (m, r))
            })
            .max_by_key(|(m, _)| m.archive_prefix.len())
            .map(|(m, rest)| {
                let relative = match (m.resource_prefix.is_empty(), rest.is_empty()) {
                    (true, _) => rest.to_string(),
                    (false, true) => m.resource_prefix.clone(),
                    (false, false) => format!("{}/{}", m.resource_prefix, rest),
                };
                (m, relative)
            })
    }
}

/// Check one effective member path against the policy's name limits. A single
/// trailing `/` is accepted for directories only.
pub fn check_member_path(
    path: &str,
    kind: EntryKind,
    limits: &ArchiveLimits,
) -> Result<String, FindingCode> {
    if path.len() > limits.max_path_bytes as usize {
        return Err(FindingCode::PathTooLong);
    }
    let trimmed = match (kind, path.strip_suffix('/')) {
        (EntryKind::Directory, Some(stripped)) => stripped,
        (EntryKind::File, Some(_)) => return Err(FindingCode::TrailingSlashOnFile),
        (_, None) => path,
    };
    if trimmed.is_empty() {
        return Err(FindingCode::EmptyComponent);
    }
    let parts = relative_components(trimmed)?;
    if parts.len() > limits.max_path_depth as usize {
        return Err(FindingCode::PathTooDeep);
    }
    if parts
        .iter()
        .any(|p| p.len() > limits.max_component_bytes as usize)
    {
        return Err(FindingCode::ComponentTooLong);
    }
    Ok(trimmed.to_string())
}

fn check_report(policy: &AdmissionPolicy, report: &InspectionReport) -> Result<u64, String> {
    let limits = &policy.limits;
    if !is_hex_digest(&report.archive_sha256) {
        return Err("archive digest is not lowercase SHA-256 hex".into());
    }
    if report.compressed_bytes == 0 || report.compressed_bytes > limits.max_compressed_bytes {
        return Err("compressed byte count outside policy".into());
    }
    if report.decompressed_bytes > limits.max_decompressed_bytes {
        return Err("decompressed byte count exceeds policy".into());
    }
    if report.header_count > limits.max_entries
        || (report.entries.len() as u64) > report.header_count
    {
        return Err("entry counts are inconsistent with policy".into());
    }
    if report.coalesced_self_hardlinks > report.header_count - report.entries.len() as u64
        || (report.coalesced_self_hardlinks != 0
            && !report.entries.iter().any(|e| e.kind == EntryKind::File))
    {
        return Err("legacy repeat count is inconsistent with inspected files".into());
    }
    if report.metadata_bytes > limits.max_total_metadata_bytes {
        return Err("metadata bytes exceed policy".into());
    }
    let mut file_bytes: u64 = 0;
    let mut last_index: Option<u64> = None;
    for entry in &report.entries {
        if last_index.is_some_and(|i| entry.header_index <= i)
            || entry.header_index >= report.header_count
        {
            return Err("entry header indexes are not strictly increasing".into());
        }
        last_index = Some(entry.header_index);
        match entry.kind {
            EntryKind::File => {
                if entry.size > limits.max_file_bytes
                    || !entry.sha256.as_deref().is_some_and(is_hex_digest)
                {
                    return Err("file entry size or digest invalid".into());
                }
                file_bytes = file_bytes
                    .checked_add(entry.size)
                    .ok_or("file byte total overflows")?;
            }
            EntryKind::Directory => {
                if entry.size != 0 || entry.sha256.is_some() {
                    return Err("directory entry carries data".into());
                }
            }
        }
    }
    if file_bytes > report.decompressed_bytes {
        return Err("file bytes exceed decompressed bytes".into());
    }
    Ok(file_bytes)
}

/// Decide admission from trusted policy and a complete inspection report.
/// All destination findings are collected so an operator sees every problem.
pub fn admit(
    policy: &AdmissionPolicy,
    report: &InspectionReport,
) -> Result<RestoreManifest, Rejection> {
    policy.validate()?;
    if !policy.formats.contains(&report.format) {
        return Err(Rejection::one(
            FindingCode::FormatNotAllowed,
            None,
            None,
            "",
        ));
    }
    let file_bytes = check_report(policy, report)
        .map_err(|detail| Rejection::one(FindingCode::InconsistentReport, None, None, &detail))?;

    let mut findings = Vec::new();
    let mut finding = |code, entry: &InspectedEntry, detail: &str| {
        findings.push(Finding {
            code,
            entry: Some(entry.header_index),
            path: Some(display_path(&entry.archive_path)),
            detail: (!detail.is_empty()).then(|| detail.to_string()),
        });
    };
    // (resource, relative path) -> kind
    let mut destinations: BTreeMap<(String, String), EntryKind> = BTreeMap::new();
    let mut entries = Vec::new();
    for entry in &report.entries {
        let path = match check_member_path(&entry.archive_path, entry.kind, &policy.limits) {
            Ok(path) => path,
            Err(code) => {
                finding(code, entry, "");
                continue;
            }
        };
        let Some((mapping, relative)) = policy.map(&path) else {
            finding(
                FindingCode::Unmapped,
                entry,
                "no trusted legacy mapping covers this member",
            );
            continue;
        };
        if entry.kind == EntryKind::File && relative.is_empty() {
            finding(
                FindingCode::FileDirectoryCollision,
                entry,
                "a file cannot replace a resource root",
            );
            continue;
        }
        let key = (mapping.resource.clone(), relative.clone());
        if let Some(existing) = destinations.get(&key) {
            let code = if *existing == entry.kind {
                FindingCode::DuplicateDestination
            } else {
                FindingCode::FileDirectoryCollision
            };
            finding(code, entry, "");
            continue;
        }
        destinations.insert(key, entry.kind);
        entries.push(ManifestEntry {
            resource: mapping.resource.clone(),
            relative_path: relative,
            kind: entry.kind,
            size: entry.size,
            sha256: entry.sha256.clone(),
            archived_permissions: entry.archived_mode & 0o777,
            archive_path: path,
        });
    }
    // Any ancestor that is a file blocks everything beneath it.
    for entry in &entries {
        let mut ancestor = entry.relative_path.as_str();
        while let Some((parent, _)) = ancestor.rsplit_once('/') {
            if destinations.get(&(entry.resource.clone(), parent.to_string()))
                == Some(&EntryKind::File)
            {
                findings.push(Finding {
                    code: FindingCode::ParentIsFile,
                    entry: None,
                    path: Some(display_path(&entry.archive_path)),
                    detail: Some(format!("ancestor {} is a file", display_path(parent))),
                });
                break;
            }
            ancestor = parent;
        }
    }
    if !findings.is_empty() {
        findings.sort_by_key(|f| (f.entry, f.code));
        return Err(Rejection { findings });
    }
    entries.sort_by(|a, b| (&a.resource, &a.relative_path).cmp(&(&b.resource, &b.relative_path)));
    let file_count = entries.iter().filter(|e| e.kind == EntryKind::File).count() as u64;
    Ok(RestoreManifest {
        manifest_version: MANIFEST_VERSION,
        archive_sha256: report.archive_sha256.clone(),
        format: report.format,
        policy_revision: policy.revision,
        compressed_bytes: report.compressed_bytes,
        decompressed_bytes: report.decompressed_bytes,
        header_count: report.header_count,
        coalesced_self_hardlinks: report.coalesced_self_hardlinks,
        file_count,
        directory_count: entries.len() as u64 - file_count,
        file_bytes,
        entries,
    })
}

#[cfg(test)]
mod tests;
