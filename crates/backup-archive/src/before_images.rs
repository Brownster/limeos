//! Independent private copies of existing caller-authorized managed objects.
//! The caller owns authorization, protected ancestors and exclusion of writers.
//! Directory entries capture metadata only, never implicitly select children.

use limeos_domain::backups::EntryKind;
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
use private::{Attempt, Barrier, CaptureIo, Object, Point, RealIo};
#[cfg(test)]
mod tests;

pub const MAX_OBJECTS: u32 = 256;
pub const MAX_RECORD_BYTES: u64 = 16 * 1024 * 1024;
const CHUNK: usize = 64 * 1024;
const MAX_PATH_BYTES: usize = 4096;

/// Borrowed current target facts. Only `name` is used for descriptor-relative
/// stat; resource/path labels never become filesystem traversal instructions.
pub struct Source<'a> {
    pub resource: &'a str,
    pub relative_path: &'a str,
    pub parent: &'a File,
    pub name: &'a str,
    pub object: &'a File,
}

/// Per-capture logical ceilings, with no production defaults or reservations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureLimits {
    pub max_objects: u32,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_record_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    Objects,
    FileBytes,
    TotalBytes,
    RecordBytes,
}
#[derive(Debug)]
pub enum Failure {
    InvalidSelection,
    InvalidLimits,
    UnsafeRoot,
    UnsupportedSource,
    UnsupportedMetadata,
    WritableDescriptor,
    ChangedSource,
    ChangedObject,
    Collision,
    DiscardFailed,
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
#[derive(Debug)]
pub struct CaptureError {
    pub failure: Failure,
    pub cleanup: Option<CleanupReport>,
}
impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "before-image capture refused: {:?}", self.failure)?;
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
impl Error for CaptureError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.failure {
            Failure::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
impl From<Failure> for CaptureError {
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
fn reserve<T>(values: &mut Vec<T>, count: usize) -> Result<(), Failure> {
    values
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
fn stable(a: &Stat, b: &Stat) -> bool {
    same_inode(a, b)
        && a.st_mode == b.st_mode
        && a.st_uid == b.st_uid
        && a.st_gid == b.st_gid
        && a.st_size == b.st_size
        && a.st_nlink == b.st_nlink
        && a.st_mtime == b.st_mtime
        && a.st_mtime_nsec == b.st_mtime_nsec
        && a.st_ctime == b.st_ctime
        && a.st_ctime_nsec == b.st_ctime_nsec
}
fn own_directory(s: &Stat, mode: u32) -> bool {
    FileType::from_raw_mode(s.st_mode) == FileType::Directory
        && s.st_uid == rustix::process::geteuid().as_raw()
        && s.st_mode & 0o7777 == mode
}
fn no_attributes(file: &File) -> Result<(), Failure> {
    attributes(file).map(|_| ())
}
fn attributes(file: &File) -> Result<Option<Vec<u8>>, Failure> {
    // Admit exactly one known attribute. Unknown/oversized lists never grow
    // memory, and ACL/capability/user attributes are not silently discarded.
    let mut names = [0u8; 256];
    let count = rustix::fs::flistxattr(file, names.as_mut_slice()).map_err(|e| {
        if e == rustix::io::Errno::RANGE {
            Failure::UnsupportedMetadata
        } else {
            io_failure("inspect extended attributes", e)
        }
    })?;
    if count == 0 {
        return Ok(None);
    }
    if &names[..count] != b"security.selinux\0" {
        return Err(Failure::UnsupportedMetadata);
    }
    let mut label = [0u8; 4096];
    let count =
        rustix::fs::fgetxattr(file, "security.selinux", label.as_mut_slice()).map_err(|e| {
            if e == rustix::io::Errno::RANGE {
                Failure::UnsupportedMetadata
            } else {
                io_failure("read SELinux label", e)
            }
        })?;
    let mut value = Vec::new();
    reserve(&mut value, count)?;
    value.extend_from_slice(&label[..count]);
    Ok(Some(value))
}
fn readonly(file: &File) -> Result<(), Failure> {
    let flags =
        rustix::fs::fcntl_getfl(file).map_err(|e| io_failure("inspect source descriptor", e))?;
    if flags.intersects(OFlags::WRONLY | OFlags::RDWR | OFlags::PATH) {
        return Err(Failure::WritableDescriptor);
    }
    Ok(())
}
fn relative(path: &str, empty: bool) -> bool {
    if path.is_empty() {
        return empty;
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
    parent.is_empty() && !path.is_empty()
        || path
            .strip_prefix(parent)
            .is_some_and(|rest| rest.starts_with('/'))
}

#[derive(Debug)]
pub struct BeforeImageRoot {
    directory: File,
}
impl BeforeImageRoot {
    pub fn open(path: &Path) -> Result<Self, CaptureError> {
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
        let directory = File::from(fd);
        let stat =
            rustix::fs::fstat(&directory).map_err(|e| io_failure("stat before-image root", e))?;
        if !own_directory(&stat, 0o700) {
            return Err(Failure::UnsafeRoot.into());
        }
        no_attributes(&directory)?;
        Ok(Self { directory })
    }
}

/// Original metadata, separate from collector-owned private modes and owners.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct SourceMetadata {
    pub device: u64,
    pub inode: u64,
    pub parent_device: u64,
    pub parent_inode: u64,
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
    pub modified_seconds: i64,
    pub modified_nanoseconds: u64,
    pub changed_seconds: i64,
    pub changed_nanoseconds: u64,
    /// Exact bytes, including any terminator. Application needs current policy.
    pub selinux_label: Option<Vec<u8>>,
}
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct BeforeImageMetadata {
    pub resource: String,
    pub relative_path: String,
    pub kind: EntryKind,
    pub size: u64,
    pub sha256: Option<String>,
    pub source: SourceMetadata,
    pub private_uid: u32,
    pub private_gid: u32,
    pub private_mode: u32,
}
struct BoundSource<'a> {
    source: &'a Source<'a>,
    stat: Stat,
    parent: Stat,
    kind: EntryKind,
    selinux_label: Option<Vec<u8>>,
}
impl BoundSource<'_> {
    fn check(&self) -> Result<(), Failure> {
        readonly(self.source.object)?;
        readonly(self.source.parent)?;
        let parent = rustix::fs::fstat(self.source.parent)
            .map_err(|e| io_failure("recheck source parent", e))?;
        let held =
            rustix::fs::fstat(self.source.object).map_err(|e| io_failure("recheck source", e))?;
        let named = rustix::fs::statat(
            self.source.parent,
            self.source.name,
            AtFlags::SYMLINK_NOFOLLOW,
        )
        .map_err(|e| io_failure("recheck source name", e))?;
        if !stable(&parent, &self.parent)
            || !stable(&held, &self.stat)
            || !stable(&named, &self.stat)
        {
            return Err(Failure::ChangedSource);
        }
        if attributes(self.source.object)? != self.selinux_label {
            return Err(Failure::ChangedSource);
        }
        let after = rustix::fs::fstat(self.source.object)
            .map_err(|e| io_failure("recheck source metadata", e))?;
        if !stable(&after, &self.stat) {
            return Err(Failure::ChangedSource);
        }
        Ok(())
    }
    fn metadata(&self) -> Result<BeforeImageMetadata, Failure> {
        let s = &self.stat;
        Ok(BeforeImageMetadata {
            resource: string(self.source.resource)?,
            relative_path: string(self.source.relative_path)?,
            kind: self.kind,
            size: if self.kind == EntryKind::File {
                s.st_size as u64
            } else {
                0
            },
            sha256: None,
            source: SourceMetadata {
                device: s.st_dev,
                inode: s.st_ino,
                parent_device: self.parent.st_dev,
                parent_inode: self.parent.st_ino,
                uid: s.st_uid,
                gid: s.st_gid,
                mode: s.st_mode & 0o777,
                modified_seconds: s.st_mtime,
                modified_nanoseconds: s.st_mtime_nsec,
                changed_seconds: s.st_ctime,
                changed_nanoseconds: s.st_ctime_nsec,
                selinux_label: match &self.selinux_label {
                    Some(label) => {
                        let mut copied = Vec::new();
                        reserve(&mut copied, label.len())?;
                        copied.extend_from_slice(label);
                        Some(copied)
                    }
                    None => None,
                },
            },
            private_uid: 0,
            private_gid: 0,
            private_mode: if self.kind == EntryKind::File {
                0o400
            } else {
                0o500
            },
        })
    }
}
fn bind<'a>(
    sources: &'a [Source<'a>],
    root: &BeforeImageRoot,
    limits: CaptureLimits,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<BoundSource<'a>>, Failure> {
    if limits.max_objects == 0
        || limits.max_objects > MAX_OBJECTS
        || limits.max_file_bytes == 0
        || limits.max_file_bytes > i64::MAX as u64
        || limits.max_total_bytes == 0
        || limits.max_total_bytes > i64::MAX as u64
        || limits.max_record_bytes == 0
        || limits.max_record_bytes > MAX_RECORD_BYTES
    {
        return Err(Failure::InvalidLimits);
    }
    if sources.is_empty() {
        return Err(Failure::InvalidSelection);
    }
    if sources.len() > limits.max_objects as usize {
        return Err(Failure::Limit(Limit::Objects));
    }
    // Validate all labels before any source I/O or owned creation.
    for (i, source) in sources.iter().enumerate() {
        poll(cancelled)?;
        if source.resource.is_empty()
            || source.resource.len() > 128
            || !source
                .resource
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
            || !relative(source.relative_path, true)
            || !relative(source.name, false)
            || source.name.contains('/')
            || (!source.relative_path.is_empty()
                && source.relative_path.rsplit('/').next() != Some(source.name))
            || sources[..i].iter().any(|other| {
                other.resource == source.resource && other.relative_path == source.relative_path
            })
        {
            return Err(Failure::InvalidSelection);
        }
    }
    let root_stat =
        rustix::fs::fstat(&root.directory).map_err(|e| io_failure("check capture root", e))?;
    if !own_directory(&root_stat, 0o700) {
        return Err(Failure::UnsafeRoot);
    }
    no_attributes(&root.directory)?;
    let mut bound: Vec<BoundSource<'a>> = Vec::new();
    reserve(&mut bound, sources.len())?;
    let mut total = 0u64;
    for source in sources {
        poll(cancelled)?;
        readonly(source.object)?;
        readonly(source.parent)?;
        let stat = rustix::fs::fstat(source.object).map_err(|e| io_failure("stat source", e))?;
        let parent =
            rustix::fs::fstat(source.parent).map_err(|e| io_failure("stat source parent", e))?;
        let kind = match FileType::from_raw_mode(stat.st_mode) {
            FileType::RegularFile if stat.st_nlink == 1 && stat.st_size >= 0 => EntryKind::File,
            FileType::Directory => EntryKind::Directory,
            _ => return Err(Failure::UnsupportedSource),
        };
        if stat.st_mode & 0o7000 != 0 || stat.st_uid == u32::MAX || stat.st_gid == u32::MAX {
            return Err(Failure::UnsupportedMetadata);
        }
        if FileType::from_raw_mode(parent.st_mode) != FileType::Directory
            || same_inode(&stat, &root_stat)
            || same_inode(&parent, &root_stat)
            || bound.iter().any(|other| same_inode(&stat, &other.stat))
            || (kind == EntryKind::File && source.relative_path.is_empty())
        {
            return Err(Failure::InvalidSelection);
        }
        if kind == EntryKind::File {
            let size = stat.st_size as u64;
            if size > limits.max_file_bytes {
                return Err(Failure::Limit(Limit::FileBytes));
            }
            total = total
                .checked_add(size)
                .filter(|n| *n <= limits.max_total_bytes)
                .ok_or(Failure::Limit(Limit::TotalBytes))?;
        }
        let item = BoundSource {
            source,
            stat,
            parent,
            kind,
            selinux_label: attributes(source.object)?,
        };
        item.check()?;
        bound.push(item);
    }
    for source in &bound {
        if source.kind == EntryKind::File
            && bound.iter().any(|other| {
                other.source.resource == source.source.resource
                    && below(other.source.relative_path, source.source.relative_path)
            })
        {
            return Err(Failure::InvalidSelection);
        }
    }
    Ok(bound)
}

/// Component costs only; these values neither reserve resources nor grant work.
/// Caller source/parent/root handles, other owners and service baseline are extra.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureRequirements {
    pub objects: u32,
    pub files: u32,
    pub directories: u32,
    pub logical_file_bytes: u64,
    pub largest_file_bytes: u64,
    pub retained_descriptors: u32,
    pub transient_descriptors: u32,
    pub streaming_buffer_bytes: u64,
    /// Existing bound-vector and attribute capacities, not allocator/RSS bounds.
    pub bound_metadata_capacity_bytes: u64,
    /// Configured output ceiling, not a promise that a record will fit.
    pub record_byte_limit: u64,
}

/// Read-only metadata bindings to original borrowed handles and private root.
/// No content digest, approval, quota reservation or restart reconstruction.
///
/// ```compile_fail
/// use limeos_backup_archive::before_images::CapturePlan;
/// let forged: CapturePlan<'static> = serde_json::from_str("{}").unwrap();
/// ```
pub struct CapturePlan<'a> {
    root: &'a BeforeImageRoot,
    root_identity: Stat,
    sources: Vec<BoundSource<'a>>,
    limits: CaptureLimits,
    requirements: CaptureRequirements,
}
impl fmt::Debug for CapturePlan<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CapturePlan")
            .field("requirements", &self.requirements)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}
impl CapturePlan<'_> {
    pub fn requirements(&self) -> CaptureRequirements {
        self.requirements
    }

    pub fn limits(&self) -> CaptureLimits {
        self.limits
    }

    /// Recheck metadata/name/attribute bindings; this does not hash source bytes.
    pub fn revalidate(&self, cancelled: &dyn Fn() -> bool) -> Result<(), CaptureError> {
        poll(cancelled)?;
        let root = rustix::fs::fstat(&self.root.directory)
            .map_err(|e| io_failure("recheck planned capture root", e))?;
        if !same_inode(&root, &self.root_identity) || !own_directory(&root, 0o700) {
            return Err(Failure::UnsafeRoot.into());
        }
        no_attributes(&self.root.directory)?;
        for source in &self.sources {
            poll(cancelled)?;
            source.check()?;
        }
        poll(cancelled)?;
        Ok(())
    }
}

/// Bind/size the complete explicit selection before creating any private name.
/// The coordinator must reserve its whole request before calling capture_planned.
/// No source offset changes, data copy, descriptor clone or durable record occurs.
pub fn plan<'a>(
    sources: &'a [Source<'a>],
    root: &'a BeforeImageRoot,
    limits: CaptureLimits,
    cancelled: &dyn Fn() -> bool,
) -> Result<CapturePlan<'a>, CaptureError> {
    poll(cancelled)?;
    let sources = bind(sources, root, limits, cancelled)?;
    let root_identity = rustix::fs::fstat(&root.directory)
        .map_err(|e| io_failure("bind planned capture root", e))?;
    let objects = u32::try_from(sources.len()).map_err(|_| Failure::InvalidSelection)?;
    let mut files = 0u32;
    let mut logical_file_bytes = 0u64;
    let mut largest_file_bytes = 0u64;
    let metadata_bytes = sources
        .capacity()
        .checked_mul(std::mem::size_of::<BoundSource<'_>>())
        .ok_or(Failure::Allocation)?;
    let mut bound_metadata_capacity_bytes =
        u64::try_from(metadata_bytes).map_err(|_| Failure::Allocation)?;
    for source in &sources {
        poll(cancelled)?;
        if source.kind == EntryKind::File {
            files = files.checked_add(1).ok_or(Failure::InvalidSelection)?;
            let size =
                u64::try_from(source.stat.st_size).map_err(|_| Failure::UnsupportedSource)?;
            logical_file_bytes = logical_file_bytes
                .checked_add(size)
                .ok_or(Failure::Limit(Limit::TotalBytes))?;
            largest_file_bytes = largest_file_bytes.max(size);
        }
        if let Some(label) = &source.selinux_label {
            bound_metadata_capacity_bytes = bound_metadata_capacity_bytes
                .checked_add(u64::try_from(label.capacity()).map_err(|_| Failure::Allocation)?)
                .ok_or(Failure::Allocation)?;
        }
    }
    let requirements = CaptureRequirements {
        objects,
        files,
        directories: objects
            .checked_sub(files)
            .ok_or(Failure::InvalidSelection)?,
        logical_file_bytes,
        largest_file_bytes,
        retained_descriptors: objects.checked_add(2).ok_or(Failure::InvalidSelection)?,
        transient_descriptors: 1,
        streaming_buffer_bytes: CHUNK as u64,
        bound_metadata_capacity_bytes,
        record_byte_limit: limits.max_record_bytes,
    };
    let plan = CapturePlan {
        root,
        root_identity,
        sources,
        limits,
        requirements,
    };
    plan.revalidate(cancelled)?;
    Ok(plan)
}

fn source_hash<I: CaptureIo>(
    source: &BoundSource<'_>,
    mut output: Option<&mut File>,
    io: &mut I,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<String>, Failure> {
    source.check()?;
    if source.kind == EntryKind::Directory {
        return Ok(None);
    }
    let mut chunk = bytes(CHUNK)?;
    let mut hash = Sha256::new();
    let mut offset = 0u64;
    loop {
        poll(cancelled)?;
        let n = match io.read_at(source.source.object, &mut chunk, offset) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            result => result.map_err(|e| io_failure("read source before-image", e))?,
        };
        if n == 0 {
            break;
        }
        offset = offset
            .checked_add(n as u64)
            .filter(|n| *n <= source.stat.st_size as u64)
            .ok_or(Failure::ChangedSource)?;
        hash.update(&chunk[..n]);
        if let Some(file) = output.as_deref_mut() {
            private::write_all(io, file, &chunk[..n], cancelled)?;
        }
    }
    if offset != source.stat.st_size as u64 {
        return Err(Failure::ChangedSource);
    }
    source.check()?;
    Ok(Some(hex::encode(hash.finalize())))
}

struct RecordWriter {
    bytes: Vec<u8>,
    max: usize,
    failure: Option<Failure>,
}
impl Write for RecordWriter {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        if self.failure.is_some() {
            return Err(io::Error::other("record refused"));
        }
        let result = self
            .bytes
            .len()
            .checked_add(input.len())
            .filter(|n| *n <= self.max)
            .ok_or(Failure::Limit(Limit::RecordBytes))
            .and_then(|_| reserve(&mut self.bytes, input.len()));
        if let Err(failure) = result {
            self.failure = Some(failure);
            return Err(io::Error::other("record refused"));
        }
        self.bytes.extend_from_slice(input);
        Ok(input.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn record(attempt: &Attempt, limits: CaptureLimits) -> Result<Vec<u8>, Failure> {
    #[derive(Serialize)]
    struct Record<'a> {
        format_version: u32,
        limits: CaptureLimits,
        objects: Vec<&'a BeforeImageMetadata>,
    }
    let mut objects = Vec::new();
    reserve(&mut objects, attempt.objects.len())?;
    objects.extend(attempt.objects.iter().map(|o| &o.metadata));
    let mut writer = RecordWriter {
        bytes: Vec::new(),
        max: limits.max_record_bytes as usize,
        failure: None,
    };
    let result = serde_json::to_writer(
        &mut writer,
        &Record {
            format_version: 1,
            limits,
            objects,
        },
    );
    if let Some(failure) = writer.failure {
        return Err(failure);
    }
    result.map_err(|e| io_failure("encode before-image record", io::Error::other(e)))?;
    Ok(writer.bytes)
}

/// Capture a complete explicitly selected set using positional reads. No source
/// descriptor escapes in the returned owner or stays borrowed after capture.
pub fn capture(
    sources: &[Source<'_>],
    root: &BeforeImageRoot,
    limits: CaptureLimits,
    cancelled: &dyn Fn() -> bool,
) -> Result<BeforeImages, CaptureError> {
    capture_with_io(sources, root, limits, cancelled, &mut RealIo)
}
fn capture_with_io<I: CaptureIo>(
    sources: &[Source<'_>],
    root: &BeforeImageRoot,
    limits: CaptureLimits,
    cancelled: &dyn Fn() -> bool,
    io: &mut I,
) -> Result<BeforeImages, CaptureError> {
    capture_planned_with_io(plan(sources, root, limits, cancelled)?, cancelled, io)
}

/// Consume the original preflight after caller-owned whole-request admission.
/// Recheck the entire set before any attempt creation; never silently replan.
pub fn capture_planned(
    plan: CapturePlan<'_>,
    cancelled: &dyn Fn() -> bool,
) -> Result<BeforeImages, CaptureError> {
    capture_planned_with_io(plan, cancelled, &mut RealIo)
}

fn capture_planned_with_io<I: CaptureIo>(
    plan: CapturePlan<'_>,
    cancelled: &dyn Fn() -> bool,
    io: &mut I,
) -> Result<BeforeImages, CaptureError> {
    plan.revalidate(cancelled)?;
    let CapturePlan {
        root,
        sources,
        limits,
        ..
    } = plan;
    let mut attempt = Attempt::create(root, sources.len(), io)?;
    let result = (|| {
        for source in &sources {
            poll(cancelled)?;
            source.check()?;
            let index = attempt.create_object(source.metadata()?, io)?;
            let hash = source_hash(
                source,
                Some(&mut attempt.objects[index].file),
                io,
                cancelled,
            )?;
            attempt.objects[index].metadata.sha256 = hash;
            io.checkpoint(Point::Copied, &attempt.directory)?;
            attempt.seal(index, io, cancelled)?;
        }
        let record = record(&attempt, limits)?;
        io.sync(&attempt.directory, Barrier::Attempt)
            .map_err(|e| io_failure("fsync before-image attempt", e))?;
        io.checkpoint(Point::AttemptSynced, &attempt.directory)?;
        io.sync(&attempt.root, Barrier::Root)
            .map_err(|e| io_failure("fsync before-image root", e))?;
        io.checkpoint(Point::RootSynced, &attempt.directory)?;
        // Recheck the complete source set rather than only the last copied file.
        for (source, object) in sources.iter().zip(&attempt.objects) {
            poll(cancelled)?;
            if source_hash(source, None, io, cancelled)? != object.metadata.sha256 {
                return Err(Failure::ChangedSource);
            }
        }
        attempt.verify(io, cancelled)?;
        poll(cancelled)?;
        let mut hash = Sha256::new();
        hash.update(b"limeos.backup-before-images.record.v1\0");
        hash.update(&record);
        Ok((record, hex::encode(hash.finalize())))
    })();
    match result {
        Ok((record, identity)) => Ok(BeforeImages {
            attempt,
            record,
            identity,
        }),
        Err(failure) => Err(attempt.fail_with_io(failure, io)),
    }
}

/// An ephemeral owner, never reconstructed from serialized records.
///
/// ```compile_fail
/// use limeos_backup_archive::before_images::BeforeImages;
/// let forged: BeforeImages = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug)]
pub struct BeforeImages {
    attempt: Attempt,
    record: Vec<u8>,
    identity: String,
}
impl BeforeImages {
    pub fn record(&self) -> &[u8] {
        &self.record
    }
    pub fn record_sha256(&self) -> &str {
        &self.identity
    }
    pub fn entries(&self) -> impl Iterator<Item = BeforeImageEntry<'_>> {
        self.attempt.objects.iter().map(|object| BeforeImageEntry {
            object,
            attempt: &self.attempt,
        })
    }
    pub fn revalidate(&self, cancelled: &dyn Fn() -> bool) -> Result<(), CaptureError> {
        self.attempt
            .verify(&mut RealIo, cancelled)
            .map_err(Into::into)
    }
    pub fn discard(mut self) -> Result<(), CaptureError> {
        self.attempt.cleanup().map_err(|failure| CaptureError {
            failure: Failure::DiscardFailed,
            cleanup: Some(CleanupReport {
                attempt_name: std::mem::take(&mut self.attempt.name),
                failure,
            }),
        })
    }
}
pub struct BeforeImageEntry<'a> {
    object: &'a Object,
    attempt: &'a Attempt,
}
impl<'a> BeforeImageEntry<'a> {
    pub fn metadata(&self) -> &'a BeforeImageMetadata {
        &self.object.metadata
    }
    pub fn reader(&self) -> Result<Option<BeforeImageReader<'a>>, CaptureError> {
        self.attempt.check()?;
        self.object.check(&self.attempt.directory)?;
        Ok(
            (self.object.metadata.kind == EntryKind::File).then_some(BeforeImageReader {
                object: self.object,
                attempt: self.attempt,
                offset: 0,
                hash: Sha256::new(),
                failed: false,
            }),
        )
    }
}
/// Readers borrow the private owner.
///
/// ```compile_fail
/// use limeos_backup_archive::before_images::{BeforeImages, BeforeImageReader};
/// fn leak(owner: &BeforeImages) -> BeforeImageReader<'static> {
///     owner.entries().next().unwrap().reader().unwrap().unwrap()
/// }
/// ```
pub struct BeforeImageReader<'a> {
    object: &'a Object,
    attempt: &'a Attempt,
    offset: u64,
    hash: Sha256,
    failed: bool,
}
impl Read for BeforeImageReader<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.failed {
            return Err(io::Error::other(
                "before-image reader refused changed evidence",
            ));
        }
        if out.is_empty() {
            return Ok(0);
        }
        let result = (|| {
            self.attempt
                .check()
                .and_then(|_| self.object.check(&self.attempt.directory))
                .map_err(|_| io::Error::other("before-image object changed"))?;
            if self.offset == self.object.metadata.size {
                if self.object.metadata.sha256.as_deref()
                    != Some(hex::encode(self.hash.clone().finalize()).as_str())
                {
                    return Err(io::Error::other("before-image checksum changed"));
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
                .map_err(|_| io::Error::other("before-image object changed"))?;
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
