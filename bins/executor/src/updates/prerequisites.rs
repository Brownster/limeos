//! HOST-001 pure planning. Snapshot fingerprints prove equality, never authority.
//!
//! Callers must retain trusted read owners, use central claims/locks, recollect
//! before effects and apply proposals through the shared atomic recovery path.

use sha2::{Digest, Sha256};
use std::fmt;

pub const MAX_BOOT_BYTES: usize = 4096;
/// ARM64 COMMAND_LINE_SIZE is 2048 including the terminating NUL. Count the
/// file's newline too, conservatively, rather than depend on firmware trimming.
pub const MAX_PI_ARM64_CMDLINE_BYTES: usize = 2047;
pub const MAX_CONTROLLER_BYTES: usize = 4096;
pub const MAX_JOURNAL_SOURCE_BYTES: usize = 65_536;
pub const MAX_JOURNAL_BYTES: usize = 262_144;
pub const MAX_DROP_INS: usize = 64;
pub const MANAGED_DROP_IN: &str = "limeos.conf";
pub const JOURNAL_TEMPLATE: &[u8] =
    b"# Managed by LimeOS: bound journal growth on flash storage.\n[Journal]\nSystemMaxUse=200M\n";
const FLAGS: [&str; 2] = ["cgroup_enable=memory", "cgroup_memory=1"];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FileFact<'a> {
    Absent,
    Unavailable,
    Bytes(&'a [u8]),
    /// Only valid for a resolved drop-in symlink to /dev/null.
    Masked,
}

impl fmt::Debug for FileFact<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bytes(bytes) => f.debug_tuple("Bytes").field(&bytes.len()).finish(),
            Self::Absent => f.write_str("Absent"),
            Self::Unavailable => f.write_str("Unavailable"),
            Self::Masked => f.write_str("Masked"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootPlatform {
    /// Collector established Pi hardware, ARM64 and the supported boot layout.
    RaspberryPiArm64,
    OtherOrUnknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryFacts<'a> {
    /// Root /sys/fs/cgroup/cgroup.controllers, not a delegated subtree.
    pub controllers: FileFact<'a>,
    pub boot_platform: BootPlatform,
    pub firmware_cmdline: FileFact<'a>,
    pub legacy_cmdline: FileFact<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropInOrigin {
    Vendor,
    LocalVendor,
    Runtime,
    Administrator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectedDropIn<'a> {
    pub basename: &'a str,
    pub origin: DropInOrigin,
    pub content: FileFact<'a>,
}

/// Trusted collection must resolve same-name directory precedence and masks.
/// Drop-ins must contain all selected .conf sources, sorted by basename.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JournalFacts<'a> {
    pub complete: bool,
    pub main: FileFact<'a>,
    pub drop_ins: &'a [SelectedDropIn<'a>],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Unavailable,
    Incomplete,
    Limit,
    InvalidText,
    InvalidSourceSet,
    UnsupportedSyntax,
    ConflictingBootFlags,
    UnsupportedBootLayout,
    ForeignManagedDestination,
    LaterJournalOverride,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    FirmwareCmdline,
    LegacyCmdline,
    JournalDropIn,
}

impl Target {
    pub const fn path(self) -> &'static str {
        match self {
            Self::FirmwareCmdline => "/boot/firmware/cmdline.txt",
            Self::LegacyCmdline => "/boot/cmdline.txt",
            Self::JournalDropIn => "/etc/systemd/journald.conf.d/limeos.conf",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerificationRequired {
    RebootAndMemoryController,
    JournalReloadAndEffectiveCap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BeforeImage {
    Absent,
    Sha256([u8; 32]),
}

#[derive(Clone, PartialEq, Eq)]
pub struct Proposal {
    target: Target,
    before: BeforeImage,
    snapshot: [u8; 32],
    replacement: Vec<u8>,
    verification: VerificationRequired,
}

impl fmt::Debug for Proposal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Proposal")
            .field("target", &self.target)
            .field("before", &self.before)
            .field("replacement_bytes", &self.replacement.len())
            .field("verification", &self.verification)
            .finish_non_exhaustive()
    }
}

impl Proposal {
    pub fn target(&self) -> Target {
        self.target
    }

    pub fn before_image(&self) -> BeforeImage {
        self.before
    }

    pub fn replacement(&self) -> &[u8] {
        &self.replacement
    }

    pub fn verification_required(&self) -> VerificationRequired {
        self.verification
    }

    /// Equality check only: a caller-supplied snapshot is not trusted evidence.
    pub fn matches_memory(&self, facts: MemoryFacts<'_>) -> bool {
        matches!(self.target, Target::FirmwareCmdline | Target::LegacyCmdline)
            && memory_fingerprint(facts).is_ok_and(|digest| digest == self.snapshot)
    }

    pub fn matches_journal(&self, facts: JournalFacts<'_>) -> bool {
        self.target == Target::JournalDropIn
            && journal_fingerprint(facts).is_ok_and(|digest| digest == self.snapshot)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Repair {
    Proposed(Proposal),
    /// Required flags are already present; runtime availability is still missing.
    RebootOrRuntimeInvestigation,
    Blocked(Reason),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Assessment {
    Satisfied,
    Unknown(Reason),
    Missing(Repair),
}

pub fn assess_memory(facts: MemoryFacts<'_>) -> Assessment {
    match memory_plan(facts) {
        Ok(value) => value,
        Err(reason) => Assessment::Unknown(reason),
    }
}

fn memory_plan(facts: MemoryFacts<'_>) -> Result<Assessment, Reason> {
    let snapshot = memory_fingerprint(facts)?;
    let FileFact::Bytes(bytes) = facts.controllers else {
        return Err(Reason::Unavailable);
    };
    let controllers = text(bytes)?;
    let mut seen = std::collections::BTreeSet::new();
    for token in controllers.split_ascii_whitespace() {
        if !token.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') || !seen.insert(token) {
            return Err(Reason::UnsupportedSyntax);
        }
    }
    if seen.is_empty() {
        return Err(Reason::Unavailable);
    }
    if seen.contains("memory") {
        return Ok(Assessment::Satisfied);
    }
    if facts.boot_platform != BootPlatform::RaspberryPiArm64 {
        return Ok(Assessment::Missing(Repair::Blocked(
            Reason::UnsupportedBootLayout,
        )));
    }
    let (target, current) = match facts.firmware_cmdline {
        FileFact::Bytes(bytes) => (Target::FirmwareCmdline, bytes),
        FileFact::Absent => match facts.legacy_cmdline {
            FileFact::Bytes(bytes) => (Target::LegacyCmdline, bytes),
            FileFact::Absent => {
                return Ok(Assessment::Missing(Repair::Blocked(
                    Reason::UnsupportedBootLayout,
                )));
            }
            _ => return Err(Reason::Unavailable),
        },
        _ => return Err(Reason::Unavailable),
    };
    match boot_replacement(current) {
        Ok(Some(replacement)) => Ok(Assessment::Missing(Repair::Proposed(Proposal {
            target,
            before: BeforeImage::Sha256(Sha256::digest(current).into()),
            snapshot,
            replacement,
            verification: VerificationRequired::RebootAndMemoryController,
        }))),
        Ok(None) => Ok(Assessment::Missing(Repair::RebootOrRuntimeInvestigation)),
        Err(reason) => Ok(Assessment::Missing(Repair::Blocked(reason))),
    }
}

fn boot_replacement(bytes: &[u8]) -> Result<Option<Vec<u8>>, Reason> {
    if bytes.len() > MAX_PI_ARM64_CMDLINE_BYTES {
        return Err(Reason::Limit);
    }
    let original = text(bytes)?;
    if !original.is_ascii() {
        return Err(Reason::UnsupportedSyntax);
    }
    let body = original
        .strip_suffix("\r\n")
        .or_else(|| original.strip_suffix('\n'))
        .unwrap_or(original);
    if body.is_empty()
        || body.contains(['\r', '\n', '\'', '"', '\\'])
        || body.trim_ascii().is_empty()
    {
        return Err(Reason::UnsupportedSyntax);
    }
    let mut present = [false; 2];
    for token in body.split_ascii_whitespace() {
        if token == "--" {
            return Err(Reason::UnsupportedSyntax);
        }
        let (key, value) = token.split_once('=').unwrap_or((token, ""));
        // Kernel parameter names treat '-' and '_' as equivalent. Preserve
        // original bytes, but reject aliases rather than append conflicting flags.
        let key = key.replace('-', "_");
        let index = match key.as_str() {
            "cgroup_enable" => Some(0),
            "cgroup_memory" => Some(1),
            "cgroup_disable" if value.split(',').any(|v| matches!(v, "memory" | "all")) => {
                return Err(Reason::ConflictingBootFlags);
            }
            _ => None,
        };
        if let Some(index) = index {
            if present[index] || token != FLAGS[index] {
                return Err(Reason::ConflictingBootFlags);
            }
            present[index] = true;
        }
    }
    if present.iter().all(|v| *v) {
        return Ok(None);
    }
    let mut updated = body.as_bytes().to_vec();
    for (index, flag) in FLAGS.iter().enumerate() {
        if !present[index] {
            if !updated.last().is_some_and(u8::is_ascii_whitespace) {
                updated.push(b' ');
            }
            updated.extend_from_slice(flag.as_bytes());
        }
    }
    updated.extend_from_slice(&bytes[body.len()..]);
    if updated.len() > MAX_PI_ARM64_CMDLINE_BYTES {
        return Err(Reason::Limit);
    }
    Ok(Some(updated))
}

pub fn assess_journal(facts: JournalFacts<'_>) -> Assessment {
    match journal_plan(facts) {
        Ok(value) => value,
        Err(reason) => Assessment::Unknown(reason),
    }
}

fn journal_plan(facts: JournalFacts<'_>) -> Result<Assessment, Reason> {
    let snapshot = journal_fingerprint(facts)?;
    let mut cap = None;
    apply_source(facts.main, &mut cap)?;
    for source in facts.drop_ins {
        apply_source(source.content, &mut cap)?;
    }
    if cap.is_some() {
        return Ok(Assessment::Satisfied);
    }
    if let Some(destination) = facts
        .drop_ins
        .iter()
        .find(|s| s.basename == MANAGED_DROP_IN)
    {
        let owned = destination.origin == DropInOrigin::Administrator
            && destination.content == FileFact::Bytes(JOURNAL_TEMPLATE);
        return Ok(Assessment::Missing(Repair::Blocked(if owned {
            Reason::LaterJournalOverride
        } else {
            Reason::ForeignManagedDestination
        })));
    }
    // Simulate insertion: a later empty assignment can defeat the fixed file.
    cap = Some(200 * 1024 * 1024);
    for source in facts
        .drop_ins
        .iter()
        .filter(|s| s.basename > MANAGED_DROP_IN)
    {
        apply_source(source.content, &mut cap)?;
    }
    if cap != Some(200 * 1024 * 1024) {
        return Ok(Assessment::Missing(Repair::Blocked(
            Reason::LaterJournalOverride,
        )));
    }
    Ok(Assessment::Missing(Repair::Proposed(Proposal {
        target: Target::JournalDropIn,
        before: BeforeImage::Absent,
        snapshot,
        replacement: JOURNAL_TEMPLATE.to_vec(),
        verification: VerificationRequired::JournalReloadAndEffectiveCap,
    })))
}

fn apply_source(source: FileFact<'_>, cap: &mut Option<u64>) -> Result<(), Reason> {
    match source {
        FileFact::Absent | FileFact::Masked => return Ok(()),
        FileFact::Unavailable => return Err(Reason::Unavailable),
        FileFact::Bytes(bytes) => {
            // systemd also splits on bare CR; Rust's lines() only splits LF or
            // CRLF. Refuse unsupported separators before comments can hide a
            // subsequent assignment. NUL separators are rejected by text().
            if bytes
                .iter()
                .enumerate()
                .any(|(index, byte)| *byte == b'\r' && bytes.get(index + 1) != Some(&b'\n'))
            {
                return Err(Reason::UnsupportedSyntax);
            }
            let mut journal = false;
            for line in text(bytes)?.lines() {
                let line = line.trim_ascii();
                if line.is_empty() || line.starts_with(['#', ';']) {
                    continue;
                }
                if line.ends_with('\\') {
                    return Err(Reason::UnsupportedSyntax);
                }
                if line.starts_with('[') {
                    let section = line
                        .strip_prefix('[')
                        .and_then(|s| s.strip_suffix(']'))
                        .filter(|s| !s.is_empty() && !s.contains(['[', ']']))
                        .ok_or(Reason::UnsupportedSyntax)?;
                    journal = section == "Journal";
                    continue;
                }
                let (key, value) = line.split_once('=').ok_or(Reason::UnsupportedSyntax)?;
                if journal && key.trim_ascii() == "SystemMaxUse" {
                    *cap = parse_cap(value.trim_ascii())?;
                }
            }
        }
    }
    Ok(())
}

fn parse_cap(value: &str) -> Result<Option<u64>, Reason> {
    if value.is_empty() {
        return Ok(None);
    }
    let (number, power) = match value.as_bytes().last() {
        Some(unit @ (b'K' | b'M' | b'G' | b'T' | b'P' | b'E')) => (
            &value[..value.len() - 1],
            b"KMGTPE".iter().position(|v| v == unit).unwrap() + 1,
        ),
        _ => (value, 0),
    };
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Reason::UnsupportedSyntax);
    }
    let bytes = number
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(1024_u64.pow(power as u32)))
        .filter(|n| *n > 0)
        .ok_or(Reason::UnsupportedSyntax)?;
    Ok(Some(bytes))
}

fn text(bytes: &[u8]) -> Result<&str, Reason> {
    if bytes
        .iter()
        .any(|b| b.is_ascii_control() && !matches!(b, b'\n' | b'\r' | b'\t'))
    {
        return Err(Reason::InvalidText);
    }
    std::str::from_utf8(bytes).map_err(|_| Reason::InvalidText)
}

fn hash_fact(hash: &mut Sha256, fact: FileFact<'_>, limit: usize) -> Result<usize, Reason> {
    match fact {
        FileFact::Absent => hash.update([0]),
        FileFact::Unavailable => hash.update([1]),
        FileFact::Masked => hash.update([2]),
        FileFact::Bytes(bytes) => {
            if bytes.len() > limit {
                return Err(Reason::Limit);
            }
            hash.update([3]);
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
            return Ok(bytes.len());
        }
    }
    Ok(0)
}

fn memory_fingerprint(facts: MemoryFacts<'_>) -> Result<[u8; 32], Reason> {
    let mut hash = Sha256::new();
    hash.update(b"limeos/host-prerequisites/memory/v1");
    hash.update([match facts.boot_platform {
        BootPlatform::RaspberryPiArm64 => 0,
        BootPlatform::OtherOrUnknown => 1,
    }]);
    hash_fact(&mut hash, facts.controllers, MAX_CONTROLLER_BYTES)?;
    hash_fact(&mut hash, facts.firmware_cmdline, MAX_BOOT_BYTES)?;
    hash_fact(&mut hash, facts.legacy_cmdline, MAX_BOOT_BYTES)?;
    Ok(hash.finalize().into())
}

fn journal_fingerprint(facts: JournalFacts<'_>) -> Result<[u8; 32], Reason> {
    if !facts.complete {
        return Err(Reason::Incomplete);
    }
    if facts.drop_ins.len() > MAX_DROP_INS {
        return Err(Reason::Limit);
    }
    if facts.main == FileFact::Masked {
        return Err(Reason::InvalidSourceSet);
    }
    let mut hash = Sha256::new();
    hash.update(b"limeos/host-prerequisites/journal/v1");
    let mut total = hash_fact(&mut hash, facts.main, MAX_JOURNAL_SOURCE_BYTES)?;
    let mut previous = None;
    for source in facts.drop_ins {
        let name = source.basename;
        if name.len() > 255
            || name.starts_with('.')
            || !name.ends_with(".conf")
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || previous.is_some_and(|p| p >= name)
            || source.content == FileFact::Absent
        {
            return Err(Reason::InvalidSourceSet);
        }
        previous = Some(name);
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        hash.update([match source.origin {
            DropInOrigin::Vendor => 0,
            DropInOrigin::LocalVendor => 1,
            DropInOrigin::Runtime => 2,
            DropInOrigin::Administrator => 3,
        }]);
        total += hash_fact(&mut hash, source.content, MAX_JOURNAL_SOURCE_BYTES)?;
        if total > MAX_JOURNAL_BYTES {
            return Err(Reason::Limit);
        }
    }
    Ok(hash.finalize().into())
}

#[cfg(test)]
#[path = "prerequisites_tests.rs"]
mod tests;
