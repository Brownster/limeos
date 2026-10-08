//! Verified, ephemeral private replacements. No live destination is opened.
//! The caller owns trusted selection, protected ancestors, space reservation
//! and exclusion of concurrent same-UID/privileged writers.

use crate::staging::{StagingError, VerifiedCatalog};
use limeos_domain::backups::{EntryKind, RestoreManifest};
use rustix::fs::{AtFlags, FileType, Mode, OFlags, ResolveFlags, Stat};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fmt,
    fs::File,
    io::{self, Read, Write},
    os::unix::fs::FileExt,
    path::Path,
};

mod private;
use private::{Attempt, Barrier, Object, PreparationIo, RealIo};
#[cfg(test)]
mod tests;

pub const MAX_OBJECTS: u32 = 256;
pub const MAX_RESOURCES: u32 = 64;
pub const MAX_RECORD_BYTES: u64 = 16 * 1024 * 1024;
const CHUNK: usize = 64 * 1024;
const MAX_PATH_BYTES: usize = 4096;

/// Trusted installation facts; private copies retain collector ownership.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstalledMetadata {
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedTarget {
    pub relative_path: String,
    pub kind: EntryKind,
    pub installed: InstalledMetadata,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceSelection {
    pub id: String,
    /// Trusted plan fact only. Preparation never opens this path.
    pub destination_root: String,
    pub targets: Vec<ManagedTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub resources: Vec<ResourceSelection>,
}

/// Explicit per-attempt logical limits, not disk or global quota reservations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparationLimits {
    pub max_resources: u32,
    pub max_objects: u32,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_record_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    Resources,
    Objects,
    FileBytes,
    TotalBytes,
    RecordBytes,
}

#[derive(Debug)]
pub enum Failure {
    InvalidPolicy,
    SelectionMismatch,
    ManifestMismatch,
    PolicyChanged,
    DiscardFailed,
    Catalog(StagingError),
    UnsafeRoot,
    ChangedObject,
    Collision,
    Limit(Limit),
    Allocation,
    Cancelled,
    Io {
        operation: &'static str,
        source: io::Error,
    },
}

#[derive(Debug)]
pub enum CleanupFailure {
    UnverifiedObject,
    ReplacedObject,
    ForeignObjects,
    Io(io::Error),
}

#[derive(Debug)]
pub struct CleanupReport {
    pub attempt_name: String,
    pub failure: CleanupFailure,
}

/// Both the primary failure and cleanup ambiguity survive refusal.
#[derive(Debug)]
pub struct PreparationError {
    pub failure: Failure,
    pub cleanup: Option<CleanupReport>,
}

impl fmt::Display for PreparationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "private preparation refused: {:?}", self.failure)?;
        if let Some(cleanup) = &self.cleanup {
            write!(
                f,
                "; cleanup unresolved for {}: {:?}",
                cleanup.attempt_name, cleanup.failure
            )?;
        }
        Ok(())
    }
}
impl Error for PreparationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.failure {
            Failure::Io { source, .. } => Some(source),
            Failure::Catalog(source) => Some(source),
            _ => None,
        }
    }
}
impl From<Failure> for PreparationError {
    fn from(failure: Failure) -> Self {
        Self {
            failure,
            cleanup: None,
        }
    }
}
fn io_failure(operation: &'static str, source: impl Into<io::Error>) -> Failure {
    Failure::Io {
        operation,
        source: source.into(),
    }
}
fn poll(cancelled: &dyn Fn() -> bool) -> Result<(), Failure> {
    if cancelled() {
        Err(Failure::Cancelled)
    } else {
        Ok(())
    }
}
fn catalog_failure(error: StagingError) -> Failure {
    match error {
        StagingError::Cancelled => Failure::Cancelled,
        StagingError::Allocation => Failure::Allocation,
        error => Failure::Catalog(error),
    }
}
fn reserve<T>(vector: &mut Vec<T>, count: usize) -> Result<(), Failure> {
    vector
        .try_reserve_exact(count)
        .map_err(|_| Failure::Allocation)
}
fn string(value: &str) -> Result<String, Failure> {
    let mut result = String::new();
    result
        .try_reserve_exact(value.len())
        .map_err(|_| Failure::Allocation)?;
    result.push_str(value);
    Ok(result)
}
fn bytes(count: usize) -> Result<Vec<u8>, Failure> {
    let mut result = Vec::new();
    reserve(&mut result, count)?;
    result.resize(count, 0);
    Ok(result)
}
fn same_inode(a: &Stat, b: &Stat) -> bool {
    a.st_dev == b.st_dev && a.st_ino == b.st_ino
}
fn own_directory(s: &Stat, mode: u32) -> bool {
    FileType::from_raw_mode(s.st_mode) == FileType::Directory
        && s.st_uid == rustix::process::geteuid().as_raw()
        && s.st_mode & 0o7777 == mode
}

#[derive(Debug)]
pub struct PreparationRoot {
    directory: File,
    path: String,
}
impl PreparationRoot {
    pub fn open(path: &Path) -> Result<Self, PreparationError> {
        let label = path.to_str().ok_or(Failure::UnsafeRoot)?;
        if !label.starts_with('/') || !relative(&label[1..], false) {
            return Err(Failure::UnsafeRoot.into());
        }
        let fd = rustix::fs::openat2(
            rustix::fs::CWD,
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(|_| Failure::UnsafeRoot)?;
        let stat = rustix::fs::fstat(&fd).map_err(|e| io_failure("stat preparation root", e))?;
        if !own_directory(&stat, 0o700) {
            return Err(Failure::UnsafeRoot.into());
        }
        Ok(Self {
            directory: File::from(fd),
            path: string(label)?,
        })
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct PreparedMetadata {
    pub resource: String,
    pub relative_path: String,
    pub kind: EntryKind,
    pub size: u64,
    pub sha256: Option<String>,
    pub installed: InstalledMetadata,
    pub private_uid: u32,
    pub private_gid: u32,
    pub private_mode: u32,
}

/// Only successful preparation constructs this ephemeral owner.
///
/// ```compile_fail
/// use limeos_backup_archive::preparation::PreparedRestore;
/// let imported: PreparedRestore = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug)]
pub struct PreparedRestore {
    attempt: Attempt,
    record: Vec<u8>,
    record_sha256: String,
}
impl PreparedRestore {
    pub fn record(&self) -> &[u8] {
        &self.record
    }
    pub fn record_sha256(&self) -> &str {
        &self.record_sha256
    }
    pub fn entries(&self) -> impl Iterator<Item = PreparedEntry<'_>> {
        self.attempt.objects.iter().map(|object| PreparedEntry {
            object,
            attempt: &self.attempt,
        })
    }
    /// Fresh verification before a later caller borrows payloads for use.
    pub fn revalidate(&self, cancelled: &dyn Fn() -> bool) -> Result<(), PreparationError> {
        self.attempt
            .verify(&mut RealIo, cancelled)
            .map_err(Into::into)
    }
    pub fn discard(mut self) -> Result<(), PreparationError> {
        match self.attempt.cleanup() {
            Ok(()) => Ok(()),
            Err(failure) => Err(PreparationError {
                failure: Failure::DiscardFailed,
                cleanup: Some(CleanupReport {
                    attempt_name: std::mem::take(&mut self.attempt.name),
                    failure,
                }),
            }),
        }
    }
}
pub struct PreparedEntry<'a> {
    object: &'a Object,
    attempt: &'a Attempt,
}
impl PreparedEntry<'_> {
    pub fn metadata(&self) -> &PreparedMetadata {
        &self.object.metadata
    }
    /// Performs fresh checks at creation, and around every positional read.
    pub fn reader(&self) -> Result<Option<PreparedReader<'_>>, PreparationError> {
        self.attempt.check().map_err(PreparationError::from)?;
        self.object
            .check(&self.attempt.directory)
            .map_err(PreparationError::from)?;
        Ok(
            (self.object.metadata.kind == EntryKind::File).then_some(PreparedReader {
                object: self.object,
                attempt: self.attempt,
                offset: 0,
                hash: Sha256::new(),
                failed: false,
            }),
        )
    }
}
/// A reader cannot outlive its prepared owner.
///
/// ```compile_fail
/// use limeos_backup_archive::preparation::{PreparedRestore, PreparedReader};
/// fn leak(owner: &PreparedRestore) -> PreparedReader<'static> {
///     owner.entries().next().unwrap().reader().unwrap().unwrap()
/// }
/// ```
pub struct PreparedReader<'a> {
    object: &'a Object,
    attempt: &'a Attempt,
    offset: u64,
    hash: Sha256,
    failed: bool,
}
impl Read for PreparedReader<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.failed {
            return Err(io::Error::other("prepared reader refused changed evidence"));
        }
        if out.is_empty() {
            return Ok(0);
        }
        let result = (|| {
            self.attempt
                .check()
                .and_then(|_| self.object.check(&self.attempt.directory))
                .map_err(|_| io::Error::other("prepared object changed"))?;
            if self.offset == self.object.metadata.size {
                if self.object.metadata.sha256.as_deref()
                    != Some(hex::encode(self.hash.clone().finalize()).as_str())
                {
                    return Err(io::Error::other("prepared checksum changed"));
                }
                return Ok(0);
            }
            let want = out.len().min(
                usize::try_from(self.object.metadata.size - self.offset).unwrap_or(usize::MAX),
            );
            let n = self.object.file.read_at(&mut out[..want], self.offset)?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            self.attempt
                .check()
                .and_then(|_| self.object.check(&self.attempt.directory))
                .map_err(|_| io::Error::other("prepared object changed"))?;
            self.hash.update(&out[..n]);
            self.offset += n as u64;
            Ok(n)
        })();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
}

fn relative(path: &str, directory: bool) -> bool {
    if path.is_empty() {
        return directory;
    }
    path.len() <= MAX_PATH_BYTES
        && !path.starts_with('/')
        && !path.chars().any(|c| c.is_control() || c == '\\')
        && path.split('/').count() <= 32
        && path
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != ".." && p.len() <= 255)
}
fn below(path: &str, parent: &str) -> bool {
    path.strip_prefix(parent)
        .is_some_and(|rest| rest.starts_with('/'))
}
fn validate(
    selection: &Selection,
    limits: &PreparationLimits,
    manifest: &RestoreManifest,
) -> Result<(), Failure> {
    if limits.max_resources == 0
        || limits.max_resources > MAX_RESOURCES
        || limits.max_objects == 0
        || limits.max_objects > MAX_OBJECTS
        || limits.max_file_bytes == 0
        || limits.max_file_bytes > i64::MAX as u64
        || limits.max_total_bytes == 0
        || limits.max_total_bytes > i64::MAX as u64
        || limits.max_record_bytes == 0
        || limits.max_record_bytes > MAX_RECORD_BYTES
        || selection.resources.is_empty()
    {
        return Err(Failure::InvalidPolicy);
    }
    if selection.resources.len() > limits.max_resources as usize {
        return Err(Failure::Limit(Limit::Resources));
    }
    let mut count = 0usize;
    let mut total = 0u64;
    for (index, resource) in selection.resources.iter().enumerate() {
        if resource.id.is_empty()
            || resource.id.len() > 128
            || !resource
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            || !resource.destination_root.starts_with('/')
            || !relative(&resource.destination_root[1..], false)
            || resource.targets.is_empty()
            || selection.resources[..index].iter().any(|r| {
                r.id == resource.id
                    || r.destination_root == resource.destination_root
                    || below(&r.destination_root, &resource.destination_root)
                    || below(&resource.destination_root, &r.destination_root)
            })
        {
            return Err(Failure::InvalidPolicy);
        }
        if !manifest.entries.iter().any(|e| e.resource == resource.id) {
            return Err(Failure::SelectionMismatch);
        }
        count = count
            .checked_add(resource.targets.len())
            .filter(|n| *n <= limits.max_objects as usize)
            .ok_or(Failure::Limit(Limit::Objects))?;
        for (i, target) in resource.targets.iter().enumerate() {
            if !relative(&target.relative_path, target.kind == EntryKind::Directory)
                || target.installed.uid == u32::MAX
                || target.installed.gid == u32::MAX
                || target.installed.mode > 0o777
                || resource.targets[..i]
                    .iter()
                    .any(|t| t.relative_path == target.relative_path)
            {
                return Err(Failure::InvalidPolicy);
            }
            // Every ancestor is an explicit, trusted required directory.
            let mut rest = target.relative_path.as_str();
            while let Some((parent, _)) = rest.rsplit_once('/') {
                if !resource
                    .targets
                    .iter()
                    .any(|t| t.relative_path == parent && t.kind == EntryKind::Directory)
                {
                    return Err(Failure::InvalidPolicy);
                }
                rest = parent;
            }
            let entry = manifest
                .entries
                .iter()
                .find(|e| e.resource == resource.id && e.relative_path == target.relative_path);
            match entry {
                Some(entry) if entry.kind == target.kind => {
                    if entry.kind == EntryKind::File {
                        if entry.size > limits.max_file_bytes {
                            return Err(Failure::Limit(Limit::FileBytes));
                        }
                        total = total
                            .checked_add(entry.size)
                            .filter(|n| *n <= limits.max_total_bytes)
                            .ok_or(Failure::Limit(Limit::TotalBytes))?;
                    }
                }
                None if target.kind == EntryKind::Directory => {}
                _ => return Err(Failure::SelectionMismatch),
            }
        }
        if manifest
            .entries
            .iter()
            .filter(|e| e.resource == resource.id)
            .any(|e| {
                !resource
                    .targets
                    .iter()
                    .any(|t| t.relative_path == e.relative_path && t.kind == e.kind)
            })
        {
            return Err(Failure::SelectionMismatch);
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct Record<'a> {
    format_version: u32,
    selection: &'a Selection,
    limits: &'a PreparationLimits,
    manifest: &'a RestoreManifest,
    objects: Vec<&'a PreparedMetadata>,
}
struct RecordWriter<'a> {
    bytes: Vec<u8>,
    limit: u64,
    failure: Option<Failure>,
    cancelled: &'a dyn Fn() -> bool,
}
impl Write for RecordWriter<'_> {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let result = self
            .bytes
            .len()
            .checked_add(input.len())
            .filter(|n| *n as u64 <= self.limit);
        let failure = if (self.cancelled)() {
            Some(Failure::Cancelled)
        } else if result.is_none() {
            Some(Failure::Limit(Limit::RecordBytes))
        } else if result.is_some_and(|n| n > self.bytes.capacity()) {
            let capacity = self
                .bytes
                .capacity()
                .saturating_mul(2)
                .max(1024)
                .max(result.unwrap_or(0))
                .min(self.limit as usize);
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .err()
                .map(|_| Failure::Allocation)
        } else {
            None
        };
        if let Some(failure) = failure {
            self.failure = Some(failure);
            return Err(io::Error::other("bounded preparation record refused"));
        }
        self.bytes.extend_from_slice(input);
        Ok(input.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn record(
    selection: &Selection,
    limits: &PreparationLimits,
    manifest: &RestoreManifest,
    objects: &[Object],
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<u8>, Failure> {
    let mut ordered = Vec::new();
    reserve(&mut ordered, objects.len())?;
    for object in objects {
        ordered.push(&object.metadata);
    }
    let record = Record {
        format_version: 1,
        selection,
        limits,
        manifest,
        objects: ordered,
    };
    let mut writer = RecordWriter {
        bytes: Vec::new(),
        limit: limits.max_record_bytes,
        failure: None,
        cancelled,
    };
    if serde_json::to_writer(&mut writer, &record).is_err() {
        return Err(writer.failure.unwrap_or(Failure::InvalidPolicy));
    }
    Ok(writer.bytes)
}

#[derive(Debug)]
pub struct PreparationPlan {
    manifest: RestoreManifest,
    selection: Selection,
    limits: PreparationLimits,
    identity: String,
}
impl PreparationPlan {
    pub fn manifest(&self) -> &RestoreManifest {
        &self.manifest
    }
    pub fn selection(&self) -> &Selection {
        &self.selection
    }
    pub fn limits(&self) -> &PreparationLimits {
        &self.limits
    }
    pub fn identity(&self) -> &str {
        &self.identity
    }
}
/// Bind the exact trusted manifest, selection and limits without any writes.
pub fn plan(
    catalog: &VerifiedCatalog,
    expected: RestoreManifest,
    selection: Selection,
    limits: PreparationLimits,
) -> Result<PreparationPlan, PreparationError> {
    if catalog.manifest() != &expected {
        return Err(Failure::ManifestMismatch.into());
    }
    validate(&selection, &limits, &expected)?;
    #[derive(Serialize)]
    struct PlanRecord<'a> {
        version: u32,
        manifest: &'a RestoreManifest,
        selection: &'a Selection,
        limits: &'a PreparationLimits,
    }
    let record = PlanRecord {
        version: 1,
        manifest: &expected,
        selection: &selection,
        limits: &limits,
    };
    let mut writer = RecordWriter {
        bytes: Vec::new(),
        limit: limits.max_record_bytes,
        failure: None,
        cancelled: &|| false,
    };
    if serde_json::to_writer(&mut writer, &record).is_err() {
        return Err(writer.failure.unwrap_or(Failure::InvalidPolicy).into());
    }
    let mut hash = Sha256::new();
    hash.update(b"limeos.backup-preparation.plan.v1\0");
    hash.update(&writer.bytes);
    let identity = hex::encode(hash.finalize());
    Ok(PreparationPlan {
        manifest: expected,
        selection,
        limits,
        identity,
    })
}
/// Prepare the plan only while the caller's current complete policy matches.
pub fn prepare(
    catalog: &VerifiedCatalog,
    plan: &PreparationPlan,
    selection: &Selection,
    root: &PreparationRoot,
    limits: &PreparationLimits,
    cancelled: &dyn Fn() -> bool,
) -> Result<PreparedRestore, PreparationError> {
    if selection != &plan.selection || limits != &plan.limits {
        return Err(Failure::PolicyChanged.into());
    }
    prepare_with_io(
        catalog,
        &plan.manifest,
        selection,
        root,
        limits,
        cancelled,
        &mut RealIo,
    )
}

fn prepare_with_io<I: PreparationIo>(
    catalog: &VerifiedCatalog,
    expected: &RestoreManifest,
    selection: &Selection,
    root: &PreparationRoot,
    limits: &PreparationLimits,
    cancelled: &dyn Fn() -> bool,
    io: &mut I,
) -> Result<PreparedRestore, PreparationError> {
    poll(cancelled)?;
    if catalog.manifest() != expected {
        return Err(Failure::ManifestMismatch.into());
    }
    validate(selection, limits, expected)?;
    if selection.resources.iter().any(|r| {
        root.path == r.destination_root
            || below(&root.path, &r.destination_root)
            || below(&r.destination_root, &root.path)
    }) {
        return Err(Failure::InvalidPolicy.into());
    }
    catalog.revalidate(cancelled).map_err(catalog_failure)?;
    let mut targets = Vec::new();
    let count: usize = selection.resources.iter().map(|r| r.targets.len()).sum();
    io.reserve(&mut targets, count)?;
    for resource in &selection.resources {
        for target in &resource.targets {
            targets.push((resource, target));
        }
    }
    targets.sort_unstable_by(|(ra, a), (rb, b)| {
        (&ra.id, &a.relative_path).cmp(&(&rb.id, &b.relative_path))
    });
    let mut chunk = bytes(CHUNK)?;
    let mut attempt = Attempt::create(root, count, io)?;
    let result = (|| -> Result<(Vec<u8>, String), Failure> {
        for (resource, target) in targets {
            poll(cancelled)?;
            let entry = catalog.entries().find(|e| {
                e.metadata().resource == resource.id
                    && e.metadata().relative_path == target.relative_path
            });
            let size = entry.as_ref().map_or(0, |e| e.metadata().size);
            let checksum = entry
                .as_ref()
                .and_then(|e| e.metadata().sha256.as_deref())
                .map(string)
                .transpose()?;
            let metadata = PreparedMetadata {
                resource: string(&resource.id)?,
                relative_path: string(&target.relative_path)?,
                kind: target.kind,
                size,
                sha256: checksum,
                installed: target.installed,
                private_uid: 0,
                private_gid: 0,
                private_mode: if target.kind == EntryKind::File {
                    0o400
                } else {
                    0o500
                },
            };
            let index = attempt.create_object(metadata, io)?;
            if target.kind == EntryKind::File {
                let entry = entry.as_ref().ok_or(Failure::SelectionMismatch)?;
                let mut reader = entry.reader().ok_or(Failure::SelectionMismatch)?;
                let mut hash = Sha256::new();
                let mut copied = 0u64;
                loop {
                    poll(cancelled)?;
                    let n = match reader.read(&mut chunk) {
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        v => v.map_err(|e| io_failure("read catalog payload", e))?,
                    };
                    if n == 0 {
                        break;
                    }
                    copied = copied
                        .checked_add(n as u64)
                        .filter(|n| *n <= size)
                        .ok_or(Failure::ChangedObject)?;
                    hash.update(&chunk[..n]);
                    private::write_all(
                        io,
                        &mut attempt.objects[index].file,
                        &chunk[..n],
                        cancelled,
                    )?;
                }
                if copied != size
                    || attempt.objects[index].metadata.sha256.as_deref()
                        != Some(hex::encode(hash.finalize()).as_str())
                {
                    return Err(Failure::ChangedObject);
                }
            }
            io.checkpoint(private::Point::Copied, &attempt.directory)?;
            attempt.seal(index, io, cancelled)?;
        }
        attempt.verify(io, cancelled)?;
        let record = record(selection, limits, expected, &attempt.objects, cancelled)?;
        let mut hash = Sha256::new();
        hash.update(b"limeos.backup-preparation.record.v1\0");
        hash.update(&record);
        let digest = hex::encode(hash.finalize());
        io.sync(&attempt.directory, Barrier::Attempt)
            .map_err(|e| io_failure("fsync preparation attempt", e))?;
        io.checkpoint(private::Point::AttemptSynced, &attempt.directory)?;
        io.sync(&attempt.root, Barrier::Root)
            .map_err(|e| io_failure("fsync preparation root", e))?;
        io.checkpoint(private::Point::RootSynced, &attempt.directory)?;
        catalog.revalidate(cancelled).map_err(catalog_failure)?;
        attempt.verify(io, cancelled)?;
        poll(cancelled)?;
        Ok((record, digest))
    })();
    match result {
        Ok((record, record_sha256)) => Ok(PreparedRestore {
            attempt,
            record,
            record_sha256,
        }),
        Err(failure) => Err(attempt.fail_with_io(failure, io)),
    }
}
