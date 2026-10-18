//! Bounded owned-section proposals; no fstab semantic, file or execution authority.
use super::{PoolsConfig, StoragePlanningFindingCode, StoragePlanningResult, beneath, finding};
use serde::Serialize;
use std::{collections::BTreeSet, fmt, ops::Range};

pub const POOL_FSTAB_MAX_BYTES: usize = 65536;
pub const POOL_FSTAB_MAX_LINE_BYTES: usize = 4096;
pub const POOL_FSTAB_MAX_ENTRIES: usize = 128;
pub const POOL_FSTAB_MAX_TARGET_BYTES: usize = 512;
const BEGIN: &str = "# BEGIN LIMEOS MERGERFS V1";
const END: &str = "# END LIMEOS MERGERFS V1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct PoolFstabEditSummary {
    pub changed: bool,
    pub replacing_managed_block: bool,
    pub before_bytes: usize,
    pub candidate_bytes: usize,
    pub external_entries: usize,
    pub enabled_pools: usize,
}

/// Borrowed content binding only. Markers and previous JSON do not prove ownership.
///
/// ```compile_fail
/// use limeos_domain::pools::document::PoolFstabEdit;
/// let imported: PoolFstabEdit<'static> = serde_json::from_str("{}").unwrap();
/// ```
/// ```compile_fail
/// use limeos_domain::pools::{PoolsConfig, document::{PoolFstabEdit, plan_document_edit}};
/// fn leak(config: &'static PoolsConfig) -> PoolFstabEdit<'static> {
///     let source = Vec::new();
///     plan_document_edit(&source, None, config).unwrap()
/// }
/// ```
pub struct PoolFstabEdit<'a> {
    source: &'a [u8],
    desired: &'a PoolsConfig,
    candidate: Vec<u8>,
    summary: PoolFstabEditSummary,
}
impl fmt::Debug for PoolFstabEdit<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PoolFstabEdit")
            .field("summary", &self.summary)
            .finish_non_exhaustive()
    }
}
impl PoolFstabEdit<'_> {
    pub fn candidate(&self) -> &[u8] {
        &self.candidate
    }
    pub fn desired(&self) -> &PoolsConfig {
        self.desired
    }
    pub fn summary(&self) -> PoolFstabEditSummary {
        self.summary
    }
    /// Retained file/name/metadata/physical identities remain the caller's obligation.
    pub fn check_current(&self, current: &[u8]) -> StoragePlanningResult<()> {
        if current != self.source {
            return Err(refuse(
                "pools.fstab.current",
                StoragePlanningFindingCode::UnverifiedSource,
            ));
        }
        Ok(())
    }
}
fn refuse(field: &str, code: StoragePlanningFindingCode) -> Vec<crate::StoragePlanningFinding> {
    vec![finding(field, code)]
}
fn bounded_config(config: &PoolsConfig) -> StoragePlanningResult<()> {
    // Bound work before the existing validator builds findings/relationship sets.
    // These are its existing valid-input limits, not new configuration semantics.
    if config.pools.len() > 32
        || config.pools.iter().any(|pool| {
            pool.branches.len() > 32
                || pool.id.len() > 64
                || pool.name.len() > 64
                || pool.mount_point.len() > 512
                || pool.min_free_space.len() > 32
                || pool
                    .options
                    .min_free_space
                    .as_ref()
                    .is_some_and(|v| v.len() > 32)
                || pool.branches.iter().any(|branch| branch.len() > 512)
        })
    {
        return Err(refuse(
            "pools.fstab.configuration",
            StoragePlanningFindingCode::ThresholdExceeded,
        ));
    }
    config.validate()?;
    for pool in &config.pools {
        // Apply the same closed spelling profile to owned and external targets.
        // The public configuration renderer retains its existing broader rules.
        target(&pool.mount_point, "fuse.mergerfs")?;
    }
    Ok(())
}
fn body(line: &str) -> &str {
    match line.strip_suffix('\n') {
        Some(line) => line.strip_suffix('\r').unwrap_or(line),
        None => line,
    }
}
fn prefix(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len())
        .is_some_and(|s| s.eq_ignore_ascii_case(prefix))
}
fn locate(text: &str) -> StoragePlanningResult<Option<Range<usize>>> {
    use StoragePlanningFindingCode::*;
    let (mut begin, mut span, mut offset) = (None, None, 0);
    for line in text.split_inclusive('\n') {
        let content = body(line);
        if content.len() > POOL_FSTAB_MAX_LINE_BYTES {
            return Err(refuse("pools.fstab.line", ThresholdExceeded));
        }
        if content.contains('\r') {
            return Err(refuse("pools.fstab.line", Malformed));
        }
        let trimmed = content.trim_matches([' ', '\t']);
        if trimmed.ends_with('\\') {
            return Err(refuse("pools.fstab.continuation", Unsupported));
        }
        if prefix(trimmed, "# pi-health mergerfs") {
            return Err(refuse("pools.fstab.legacy_block", Unsupported));
        }
        if prefix(trimmed, "# BEGIN LIMEOS MERGERFS") || prefix(trimmed, "# END LIMEOS MERGERFS") {
            if content == BEGIN {
                if begin.is_some() || span.is_some() {
                    return Err(refuse("pools.fstab.markers", Duplicate));
                }
                begin = Some(offset);
            } else if content == END {
                let start = begin
                    .take()
                    .ok_or_else(|| refuse("pools.fstab.markers", Malformed))?;
                span = Some(start..offset + line.len());
            } else {
                return Err(refuse("pools.fstab.markers", Unsupported));
            }
        }
        offset += line.len();
    }
    if begin.is_some() {
        return Err(refuse("pools.fstab.markers", Malformed));
    }
    Ok(span)
}

// Only compare targets; never rewrite an operator's escaped spelling. Unknown
// escapes and ambiguous path components refuse rather than hiding a collision.
fn target(raw: &str, filesystem: &str) -> StoragePlanningResult<String> {
    use StoragePlanningFindingCode::*;
    let mut decoded = Vec::new();
    decoded
        .try_reserve_exact(POOL_FSTAB_MAX_TARGET_BYTES)
        .map_err(|_| refuse("pools.fstab.target", ThresholdExceeded))?;
    let bytes = raw.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = if bytes[index] == b'\\' {
            let digits = bytes
                .get(index + 1..index + 4)
                .ok_or_else(|| refuse("pools.fstab.target", Unsupported))?;
            if !digits.iter().all(|b| (b'0'..=b'7').contains(b)) {
                return Err(refuse("pools.fstab.target", Unsupported));
            }
            let value = u16::from(digits[0] - b'0') * 64
                + u16::from(digits[1] - b'0') * 8
                + u16::from(digits[2] - b'0');
            index += 4;
            u8::try_from(value).map_err(|_| refuse("pools.fstab.target", Unsupported))?
        } else {
            let byte = bytes[index];
            index += 1;
            byte
        };
        if decoded.len() == POOL_FSTAB_MAX_TARGET_BYTES {
            return Err(refuse("pools.fstab.target", ThresholdExceeded));
        }
        decoded.push(byte);
    }
    let decoded =
        String::from_utf8(decoded).map_err(|_| refuse("pools.fstab.target", Unsupported))?;
    if decoded == "none" && filesystem == "swap" {
        return Ok(decoded);
    }
    if !decoded.starts_with('/')
        || decoded.contains(['"', '\''])
        || decoded.chars().any(|c| c.is_control() && c != '\t')
        || (decoded != "/"
            && decoded[1..]
                .split('/')
                .any(|part| matches!(part, "" | "." | "..")))
    {
        return Err(refuse("pools.fstab.target", Unsupported));
    }
    Ok(decoded)
}
fn scan_external(
    text: &str,
    names: &BTreeSet<&str>,
    entries: &mut usize,
) -> StoragePlanningResult<()> {
    use StoragePlanningFindingCode::*;
    for line in text.split_inclusive('\n') {
        let line = body(line).trim_matches([' ', '\t']);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut tokens = line.split([' ', '\t']).filter(|s| !s.is_empty());
        let mut fields = [""; 6];
        for field in &mut fields {
            *field = tokens
                .next()
                .filter(|s| !s.starts_with('#'))
                .ok_or_else(|| refuse("pools.fstab.entry", Unsupported))?;
        }
        if tokens.next().is_some_and(|token| !token.starts_with('#'))
            || fields[4].parse::<u32>().is_err()
            || fields[5].parse::<u32>().is_err()
            || !fields[4].bytes().all(|c| c.is_ascii_digit())
            || !fields[5].bytes().all(|c| c.is_ascii_digit())
        {
            return Err(refuse("pools.fstab.entry", Unsupported));
        }
        *entries += 1;
        if *entries > POOL_FSTAB_MAX_ENTRIES {
            return Err(refuse("pools.fstab.entries", ThresholdExceeded));
        }
        let target = target(fields[1], fields[2])?;
        if names.iter().any(|name| beneath(&target, name)) {
            return Err(refuse("pools.fstab.collision", Duplicate));
        }
    }
    Ok(())
}

struct BoundedText {
    bytes: Vec<u8>,
    line_bytes: usize,
    crlf: bool,
}
impl fmt::Write for BoundedText {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for line in text.split_inclusive('\n') {
            let content = line.strip_suffix('\n').unwrap_or(line);
            self.line_bytes = self
                .line_bytes
                .checked_add(content.len())
                .ok_or(fmt::Error)?;
            if self.line_bytes > POOL_FSTAB_MAX_LINE_BYTES {
                return Err(fmt::Error);
            }
            let newline = line.ends_with('\n');
            let extra = if newline {
                if self.crlf { 2 } else { 1 }
            } else {
                0
            };
            if self.bytes.len() + content.len() + extra > POOL_FSTAB_MAX_BYTES + 2 {
                return Err(fmt::Error);
            }
            self.bytes.extend_from_slice(content.as_bytes());
            if newline {
                self.bytes
                    .extend_from_slice(if self.crlf { b"\r\n" } else { b"\n" });
                self.line_bytes = 0;
            }
        }
        Ok(())
    }
}
fn rendered(
    config: &PoolsConfig,
    crlf: bool,
    final_newline: bool,
) -> StoragePlanningResult<Vec<u8>> {
    use StoragePlanningFindingCode::*;
    let mut output = BoundedText {
        bytes: Vec::new(),
        line_bytes: 0,
        crlf,
    };
    output
        .bytes
        .try_reserve_exact(POOL_FSTAB_MAX_BYTES + 2)
        .map_err(|_| refuse("pools.fstab.render", ThresholdExceeded))?;
    config
        .write_fstab(&mut output)
        .map_err(|_| refuse("pools.fstab.render", ThresholdExceeded))?;
    if !final_newline {
        output
            .bytes
            .truncate(output.bytes.len() - if crlf { 2 } else { 1 });
    }
    if output.bytes.len() > POOL_FSTAB_MAX_BYTES {
        return Err(refuse("pools.fstab.render", ThresholdExceeded));
    }
    Ok(output.bytes)
}

/// Preserve external bytes, bind the exact previous rendering and reserve both
/// old/new targets. Caller still proves genuine ownership and complete semantics.
pub fn plan_document_edit<'a>(
    source: &'a [u8],
    previous: Option<&PoolsConfig>,
    desired: &'a PoolsConfig,
) -> StoragePlanningResult<PoolFstabEdit<'a>> {
    use StoragePlanningFindingCode::*;
    if source.len() > POOL_FSTAB_MAX_BYTES {
        return Err(refuse("pools.fstab.bytes", ThresholdExceeded));
    }
    let text = std::str::from_utf8(source).map_err(|_| refuse("pools.fstab", Malformed))?;
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(refuse("pools.fstab", Malformed));
    }
    bounded_config(desired)?;
    if let Some(previous) = previous {
        bounded_config(previous)?;
    }
    let span = locate(text)?;
    if span.is_some() != previous.is_some() {
        return Err(refuse("pools.fstab.managed_block", UnverifiedSource));
    }
    let mut names: BTreeSet<&str> = desired
        .pools
        .iter()
        .map(|pool| pool.mount_point.as_str())
        .collect();
    if let Some(previous) = previous {
        names.extend(previous.pools.iter().map(|pool| pool.mount_point.as_str()));
    }
    let (prefix, suffix, replacement, separator) = if let Some(range) = &span {
        let actual = &source[range.clone()];
        let crlf = actual.starts_with(format!("{BEGIN}\r\n").as_bytes());
        let final_newline = actual.ends_with(b"\n");
        let previous =
            previous.ok_or_else(|| refuse("pools.fstab.managed_block", UnverifiedSource))?;
        if actual != rendered(previous, crlf, final_newline)? {
            return Err(refuse("pools.fstab.managed_block", UnverifiedSource));
        }
        (
            &source[..range.start],
            &source[range.end..],
            rendered(desired, crlf, final_newline)?,
            "",
        )
    } else {
        let crlf = text
            .rfind('\n')
            .is_some_and(|i| i > 0 && source[i - 1] == b'\r');
        let separator = if source.is_empty() || source.ends_with(b"\n") {
            ""
        } else if crlf {
            "\r\n"
        } else {
            "\n"
        };
        (source, &[][..], rendered(desired, crlf, true)?, separator)
    };
    let mut entries = 0;
    scan_external(
        std::str::from_utf8(prefix).map_err(|_| refuse("pools.fstab", Malformed))?,
        &names,
        &mut entries,
    )?;
    scan_external(
        std::str::from_utf8(suffix).map_err(|_| refuse("pools.fstab", Malformed))?,
        &names,
        &mut entries,
    )?;
    let enabled = |config: &PoolsConfig| config.pools.iter().filter(|pool| pool.enabled).count();
    if entries + enabled(desired) > POOL_FSTAB_MAX_ENTRIES
        || entries + previous.map_or(0, enabled) > POOL_FSTAB_MAX_ENTRIES
    {
        return Err(refuse("pools.fstab.entries", ThresholdExceeded));
    }
    let size = prefix
        .len()
        .checked_add(separator.len())
        .and_then(|n| n.checked_add(replacement.len()))
        .and_then(|n| n.checked_add(suffix.len()))
        .filter(|n| *n <= POOL_FSTAB_MAX_BYTES)
        .ok_or_else(|| refuse("pools.fstab.candidate", ThresholdExceeded))?;
    let mut candidate = Vec::new();
    candidate
        .try_reserve_exact(size)
        .map_err(|_| refuse("pools.fstab.candidate", ThresholdExceeded))?;
    candidate.extend_from_slice(prefix);
    candidate.extend_from_slice(separator.as_bytes());
    candidate.extend_from_slice(&replacement);
    candidate.extend_from_slice(suffix);
    let summary = PoolFstabEditSummary {
        changed: candidate != source,
        replacing_managed_block: span.is_some(),
        before_bytes: source.len(),
        candidate_bytes: candidate.len(),
        external_entries: entries,
        enabled_pools: enabled(desired),
    };
    Ok(PoolFstabEdit {
        source,
        desired,
        candidate,
        summary,
    })
}

#[cfg(test)]
mod tests;
