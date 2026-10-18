//! Byte-preserving owned-block proposals, without Samba semantic or write authority.
use super::{SambaSharesConfig, StoragePlanningFindingCode, StoragePlanningResult, finding};
use serde::Serialize;
use std::{collections::BTreeSet, fmt, ops::Range};

pub const SAMBA_DOCUMENT_MAX_BYTES: usize = 256 * 1024;
pub const SAMBA_DOCUMENT_MAX_LOGICAL_LINE_BYTES: usize = 4096;
pub const SAMBA_DOCUMENT_MAX_EXTERNAL_SECTIONS: usize = 1024;
const BEGIN: &str = "# BEGIN LIMEOS SAMBA V1";
const END: &str = "# END LIMEOS SAMBA V1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct SambaDocumentEditSummary {
    pub changed: bool,
    pub replacing_managed_block: bool,
    pub before_bytes: usize,
    pub candidate_bytes: usize,
    pub enabled_shares: usize,
}

/// Original bytes stay borrowed. This is not protected file or effect authority.
///
/// ```compile_fail
/// use limeos_domain::shares::document::SambaDocumentEdit;
/// let imported: SambaDocumentEdit<'static> = serde_json::from_str("{}").unwrap();
/// ```
/// ```compile_fail
/// use limeos_domain::shares::{SambaSharesConfig, document::{SambaDocumentEdit, plan_document_edit}};
/// fn leak(config: &'static SambaSharesConfig) -> SambaDocumentEdit<'static> {
///     let source = Vec::new();
///     plan_document_edit(&source, None, config).unwrap()
/// }
/// ```
pub struct SambaDocumentEdit<'a> {
    source: &'a [u8],
    desired: &'a SambaSharesConfig,
    candidate: Vec<u8>,
    summary: SambaDocumentEditSummary,
}
impl fmt::Debug for SambaDocumentEdit<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SambaDocumentEdit")
            .field("summary", &self.summary)
            .finish_non_exhaustive()
    }
}
impl SambaDocumentEdit<'_> {
    pub fn candidate(&self) -> &[u8] {
        &self.candidate
    }
    pub fn desired(&self) -> &SambaSharesConfig {
        self.desired
    }
    pub fn summary(&self) -> SambaDocumentEditSummary {
        self.summary
    }
    /// Exact content only: caller still checks retained file/name/physical identity.
    pub fn check_current(&self, current: &[u8]) -> StoragePlanningResult<()> {
        if current != self.source {
            return Err(refuse(
                "samba.document.current",
                StoragePlanningFindingCode::UnverifiedSource,
            ));
        }
        Ok(())
    }
}

fn refuse(field: &str, code: StoragePlanningFindingCode) -> Vec<crate::StoragePlanningFinding> {
    vec![finding(field, code)]
}
fn horizontal(text: &str) -> &str {
    text.trim_matches([' ', '\t', '\r'])
}
fn body(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}
fn comment(text: &str) -> bool {
    text.starts_with(['#', ';'])
}

fn locate(text: &str) -> StoragePlanningResult<Option<Range<usize>>> {
    use StoragePlanningFindingCode::*;
    let mut begin = None;
    let mut span = None;
    let mut offset = 0;
    let mut continued = false;
    let mut logical_bytes = 0usize;
    for line in text.split_inclusive('\n') {
        let content = body(line);
        let trimmed = horizontal(content);
        let reserved = ["# BEGIN LIMEOS SAMBA", "# END LIMEOS SAMBA"]
            .iter()
            .any(|prefix| {
                trimmed
                    .get(..prefix.len())
                    .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
            });
        if reserved {
            if continued {
                return Err(refuse("samba.document.markers", Malformed));
            }
            if content == BEGIN {
                if begin.is_some() || span.is_some() {
                    return Err(refuse("samba.document.markers", Duplicate));
                }
                begin = Some(offset);
            } else if content == END {
                let start = begin
                    .take()
                    .ok_or_else(|| refuse("samba.document.markers", Malformed))?;
                span = Some(start..offset + line.len());
            } else {
                return Err(refuse("samba.document.markers", Unsupported));
            }
        }
        logical_bytes = logical_bytes
            .checked_add(content.len())
            .filter(|n| *n <= SAMBA_DOCUMENT_MAX_LOGICAL_LINE_BYTES)
            .ok_or_else(|| refuse("samba.document.line", ThresholdExceeded))?;
        let next = trimmed.ends_with('\\');
        // Historical and current Samba parsers differ for comment continuation.
        // Refuse this ambiguity rather than miss a following real section.
        if !continued && comment(trimmed) && next {
            return Err(refuse("samba.document.continuation", Unsupported));
        }
        continued = next;
        if !continued {
            logical_bytes = 0;
        }
        offset += line.len();
    }
    if begin.is_some() {
        return Err(refuse("samba.document.markers", Malformed));
    }
    if continued {
        return Err(refuse("samba.document.continuation", Malformed));
    }
    Ok(span)
}

fn scan_external(
    text: &str,
    names: &BTreeSet<String>,
    suffix: bool,
    sections: &mut usize,
) -> StoragePlanningResult<()> {
    use StoragePlanningFindingCode::*;
    let mut logical = String::new();
    logical
        .try_reserve_exact(SAMBA_DOCUMENT_MAX_LOGICAL_LINE_BYTES)
        .map_err(|_| refuse("samba.document.line", ThresholdExceeded))?;
    let mut first = true;
    for line in text.split_inclusive('\n') {
        let content = body(line);
        let trimmed_end = content.trim_end_matches([' ', '\t', '\r']);
        if let Some(piece) = trimmed_end.strip_suffix('\\') {
            logical.push_str(piece);
            continue;
        }
        logical.push_str(content);
        let text = horizontal(&logical);
        if !text.is_empty() && !comment(text) {
            if suffix && first && !text.starts_with('[') {
                return Err(refuse("samba.document.suffix", UnverifiedSource));
            }
            first = false;
            if let Some(header) = text.strip_prefix('[') {
                let end = header
                    .find(']')
                    .ok_or_else(|| refuse("samba.document.section", Malformed))?;
                let name = &header[..end];
                // Managed names are ASCII. A separate qualified Samba case-fold
                // adapter is needed before admitting non-ASCII external names.
                if !name.is_ascii() {
                    return Err(refuse("samba.document.section", Unsupported));
                }
                let name: String = name
                    .bytes()
                    .filter(|b| !b.is_ascii_whitespace())
                    .map(|b| b.to_ascii_lowercase() as char)
                    .collect();
                if name.is_empty() {
                    return Err(refuse("samba.document.section", Malformed));
                }
                *sections += 1;
                if *sections > SAMBA_DOCUMENT_MAX_EXTERNAL_SECTIONS {
                    return Err(refuse("samba.document.sections", ThresholdExceeded));
                }
                if names.contains(&name) {
                    return Err(refuse("samba.document.collision", Duplicate));
                }
            }
        }
        logical.clear();
    }
    Ok(())
}

fn rendered(
    config: &SambaSharesConfig,
    crlf: bool,
    final_newline: bool,
) -> StoragePlanningResult<String> {
    let text = config.render_sections()?;
    let mut text = if crlf {
        text.replace('\n', "\r\n")
    } else {
        text
    };
    if !final_newline {
        text.truncate(text.len() - if crlf { 2 } else { 1 });
    }
    Ok(text)
}

/// Preserve operator bytes and require exact previous typed rendering to replace.
/// Markers/previous data are caller-supplied claims, never ownership authorization.
pub fn plan_document_edit<'a>(
    source: &'a [u8],
    previous: Option<&SambaSharesConfig>,
    desired: &'a SambaSharesConfig,
) -> StoragePlanningResult<SambaDocumentEdit<'a>> {
    use StoragePlanningFindingCode::*;
    if source.len() > SAMBA_DOCUMENT_MAX_BYTES {
        return Err(refuse("samba.document.bytes", ThresholdExceeded));
    }
    let text = std::str::from_utf8(source).map_err(|_| refuse("samba.document", Malformed))?;
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(refuse("samba.document", Malformed));
    }
    desired.validate()?;
    if let Some(previous) = previous {
        previous.validate()?;
    }
    let span = locate(text)?;
    if span.is_some() != previous.is_some() {
        return Err(refuse("samba.document.managed_block", UnverifiedSource));
    }
    let mut names: BTreeSet<String> = desired
        .shares
        .iter()
        .map(|share| share.name.to_ascii_lowercase())
        .collect();
    if let Some(previous) = previous {
        names.extend(
            previous
                .shares
                .iter()
                .map(|share| share.name.to_ascii_lowercase()),
        );
    }
    let (prefix, suffix, replacement, separator) = if let Some(range) = &span {
        let actual = &text[range.clone()];
        let crlf = actual.starts_with(&format!("{BEGIN}\r\n"));
        let final_newline = actual.ends_with('\n');
        let previous =
            previous.ok_or_else(|| refuse("samba.document.managed_block", UnverifiedSource))?;
        let expected = rendered(previous, crlf, final_newline)?;
        if actual != expected {
            return Err(refuse("samba.document.managed_block", UnverifiedSource));
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
            .is_some_and(|index| index > 0 && source[index - 1] == b'\r');
        let separator = if source.is_empty() || source.ends_with(b"\n") {
            ""
        } else if crlf {
            "\r\n"
        } else {
            "\n"
        };
        (source, &[][..], rendered(desired, crlf, true)?, separator)
    };
    let mut sections = 0;
    scan_external(
        std::str::from_utf8(prefix).map_err(|_| refuse("samba.document", Malformed))?,
        &names,
        false,
        &mut sections,
    )?;
    scan_external(
        std::str::from_utf8(suffix).map_err(|_| refuse("samba.document", Malformed))?,
        &names,
        true,
        &mut sections,
    )?;
    let size = prefix
        .len()
        .checked_add(separator.len())
        .and_then(|n| n.checked_add(replacement.len()))
        .and_then(|n| n.checked_add(suffix.len()))
        .filter(|n| *n <= SAMBA_DOCUMENT_MAX_BYTES)
        .ok_or_else(|| refuse("samba.document.candidate", ThresholdExceeded))?;
    let mut candidate = Vec::new();
    candidate
        .try_reserve_exact(size)
        .map_err(|_| refuse("samba.document.candidate", ThresholdExceeded))?;
    candidate.extend_from_slice(prefix);
    candidate.extend_from_slice(separator.as_bytes());
    candidate.extend_from_slice(replacement.as_bytes());
    candidate.extend_from_slice(suffix);
    let summary = SambaDocumentEditSummary {
        changed: candidate != source,
        replacing_managed_block: span.is_some(),
        before_bytes: source.len(),
        candidate_bytes: candidate.len(),
        enabled_shares: desired.shares.iter().filter(|share| share.enabled).count(),
    };
    Ok(SambaDocumentEdit {
        source,
        desired,
        candidate,
        summary,
    })
}

#[cfg(test)]
mod tests;
