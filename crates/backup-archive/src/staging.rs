//! Verified replay into owned private quarantine, without restore authority.
//!
//! Capture [`PolicySnapshot`] at initial admission and retain it with the v2
//! manifest. [`replay`] checks the complete current policy, streams tentative
//! files using the inspector, checks the entire recomputed manifest and then
//! independently verifies and syncs the staged files. Only its successful
//! return yields a [`VerifiedCatalog`]. No archive string is a filesystem name.
//!
//! The caller owns the trusted private root, its ACLs and mount, and must keep
//! it inaccessible to other writers (including processes with the same UID).
//! Future executors must keep both the selected archive and completed staging
//! immutable through apply; this catalog supplies evidence, never authority.

use crate::{CHUNK, FileSink, buffer, inspect_and_admit, inspect_stream};
use limeos_domain::backups::{
    AdmissionPolicy, EntryKind, FindingCode, InspectedEntry, InspectionReport, MANIFEST_VERSION,
    ManifestEntry, Rejection, RestoreManifest, admit,
};
use rustix::fs::{AtFlags, FileType, Mode, OFlags, ResolveFlags, Stat};
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fmt,
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    os::unix::fs::FileExt,
    path::Path,
};

/// Full immutable typed policy content, including registry roots, mappings,
/// formats and limits, plus the inspected archive identity. Only initial
/// admission can create it. Revision equality is deliberately insufficient.
#[derive(Debug)]
pub struct PolicySnapshot {
    policy: AdmissionPolicy,
    archive_sha256: String,
}

impl PolicySnapshot {
    pub fn policy(&self) -> &AdmissionPolicy {
        &self.policy
    }
}

/// Initial admission and its trusted policy identity travel together.
#[derive(Debug)]
pub struct AdmittedArchive {
    manifest: RestoreManifest,
    policy: PolicySnapshot,
}

impl AdmittedArchive {
    pub fn manifest(&self) -> &RestoreManifest {
        &self.manifest
    }
    pub fn policy_snapshot(&self) -> &PolicySnapshot {
        &self.policy
    }
}

pub fn admit_for_staging<R: Read>(
    source: R,
    policy: AdmissionPolicy,
    cancelled: &dyn Fn() -> bool,
) -> Result<AdmittedArchive, Rejection> {
    let manifest = inspect_and_admit(source, &policy, cancelled)?;
    let snapshot = PolicySnapshot {
        policy,
        archive_sha256: manifest.archive_sha256.clone(),
    };
    Ok(AdmittedArchive {
        manifest,
        policy: snapshot,
    })
}

/// No failure contains a completed or partial catalog.
#[derive(Debug)]
pub enum StagingError {
    Admission(Rejection),
    PolicyChanged,
    ManifestMismatch,
    UnsafeRoot,
    Quota,
    Allocation,
    Cancelled,
    Io {
        operation: &'static str,
        source: io::Error,
    },
    Cleanup {
        failure: Box<StagingError>,
        source: io::Error,
    },
}

impl From<Rejection> for StagingError {
    fn from(value: Rejection) -> Self {
        if value
            .findings
            .iter()
            .any(|f| f.code == FindingCode::Cancelled)
        {
            Self::Cancelled
        } else if value
            .findings
            .iter()
            .any(|f| f.code == FindingCode::Allocation)
        {
            Self::Allocation
        } else {
            Self::Admission(value)
        }
    }
}

impl fmt::Display for StagingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Admission(r) => write!(f, "archive refused: {:?}", r.codes()),
            Self::PolicyChanged => f.write_str("trusted admission policy changed"),
            Self::ManifestMismatch => {
                f.write_str("replayed archive differs from admitted manifest")
            }
            Self::UnsafeRoot => f.write_str("staging root is not a protected private directory"),
            Self::Quota => f.write_str("staging exceeds admitted count or byte budget"),
            Self::Allocation => f.write_str("bounded staging allocation failed"),
            Self::Cancelled => f.write_str("staging cancelled"),
            Self::Io { operation, source } => write!(f, "{operation}: {source}"),
            Self::Cleanup { failure, source } => {
                write!(f, "{failure}; private cleanup failed: {source}")
            }
        }
    }
}

impl Error for StagingError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } | Self::Cleanup { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn io_error(operation: &'static str, source: impl Into<io::Error>) -> StagingError {
    StagingError::Io {
        operation,
        source: source.into(),
    }
}

// Used only immediately after successful exclusive creation, before any
// payload exists. The caller's private-root contract excludes other writers.
fn created_failure(
    parent: &File,
    name: &str,
    flags: AtFlags,
    failure: StagingError,
) -> StagingError {
    match rustix::fs::unlinkat(parent, name, flags) {
        Ok(()) => failure,
        Err(e) if e == rustix::io::Errno::NOENT => failure,
        Err(e) => StagingError::Cleanup {
            failure: Box::new(failure),
            source: e.into(),
        },
    }
}

fn poll(cancelled: &dyn Fn() -> bool) -> Result<(), StagingError> {
    if cancelled() {
        Err(StagingError::Cancelled)
    } else {
        Ok(())
    }
}

/// A held no-symlink descriptor to an existing, caller-owned private root.
/// No directory is created on behalf of a path supplied by an archive.
#[derive(Debug)]
pub struct StagingRoot {
    directory: File,
}

impl StagingRoot {
    pub fn open(path: &Path) -> Result<Self, StagingError> {
        if !path.is_absolute() {
            return Err(StagingError::UnsafeRoot);
        }
        let fd = rustix::fs::openat2(
            rustix::fs::CWD,
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(|_| StagingError::UnsafeRoot)?;
        let stat = rustix::fs::fstat(&fd).map_err(|e| io_error("stat staging root", e))?;
        if !private_directory(&stat) {
            return Err(StagingError::UnsafeRoot);
        }
        Ok(Self {
            directory: File::from(fd),
        })
    }
}

fn private_directory(stat: &Stat) -> bool {
    FileType::from_raw_mode(stat.st_mode) == FileType::Directory
        && stat.st_uid == rustix::process::geteuid().as_raw()
        && stat.st_mode & 0o7077 == 0
        && stat.st_mode & 0o700 == 0o700
}

fn same_inode(a: &Stat, b: &Stat) -> bool {
    a.st_dev == b.st_dev && a.st_ino == b.st_ino
}

// Private fault-injection boundary. Production callers cannot supply a writer
// or publish chunks before verification. Tests exercise real files plus faults.
trait FileIo {
    fn write(&mut self, file: &mut File, bytes: &[u8]) -> io::Result<usize> {
        file.write(bytes)
    }
    fn sync(&mut self, file: &File) -> io::Result<()> {
        file.sync_all()
    }
}
struct RealIo;
impl FileIo for RealIo {}

#[derive(Debug)]
struct PrivateFile {
    name: String,
    archive_path: String,
    declared: u64,
    written: u64,
    file: File,
    identity: Stat,
}

#[derive(Debug)]
struct Quarantine {
    root: File,
    directory: File,
    name: String,
    identity: Stat,
    files: Vec<PrivateFile>,
    removed: bool,
}

impl Quarantine {
    fn create(root: &StagingRoot, count: u64) -> Result<Self, StagingError> {
        let count = usize::try_from(count).map_err(|_| StagingError::Allocation)?;
        let mut files = Vec::new();
        files
            .try_reserve_exact(count)
            .map_err(|_| StagingError::Allocation)?;
        let mut random = [0u8; 16];
        getrandom::getrandom(&mut random)
            .map_err(|e| io_error("quarantine randomness", io::Error::other(e.to_string())))?;
        let name = format!("incomplete-{}", hex::encode(random));
        Self::create_named(root, name, files)
    }

    fn create_named(
        root: &StagingRoot,
        name: String,
        files: Vec<PrivateFile>,
    ) -> Result<Self, StagingError> {
        let root = root
            .directory
            .try_clone()
            .map_err(|e| io_error("duplicate staging root", e))?;
        rustix::fs::mkdirat(&root, name.as_str(), Mode::from_raw_mode(0o700))
            .map_err(|e| io_error("create private quarantine", e))?;
        let fd = match rustix::fs::openat(
            &root,
            name.as_str(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(error) => {
                return Err(created_failure(
                    &root,
                    &name,
                    AtFlags::REMOVEDIR,
                    io_error("open private quarantine", error),
                ));
            }
        };
        let directory = File::from(fd);
        let identity = rustix::fs::fstat(&directory).map_err(|e| {
            created_failure(
                &root,
                &name,
                AtFlags::REMOVEDIR,
                io_error("stat private quarantine", e),
            )
        })?;
        let quarantine = Self {
            root,
            directory,
            name,
            identity,
            files,
            removed: false,
        };
        if !private_directory(&quarantine.identity) {
            return Err(StagingError::UnsafeRoot);
        }
        Ok(quarantine)
    }

    fn cleanup(&mut self) -> io::Result<()> {
        if self.removed {
            return Ok(());
        }
        for file in &self.files {
            match rustix::fs::statat(
                &self.directory,
                file.name.as_str(),
                AtFlags::SYMLINK_NOFOLLOW,
            ) {
                Ok(stat) if same_inode(&stat, &file.identity) => {
                    rustix::fs::unlinkat(&self.directory, file.name.as_str(), AtFlags::empty())?;
                }
                Ok(_) => {
                    return Err(io::Error::other(
                        "private file name was replaced; foreign path retained",
                    ));
                }
                Err(e) if e == rustix::io::Errno::NOENT => {}
                Err(e) => return Err(e.into()),
            }
        }
        match rustix::fs::statat(&self.root, self.name.as_str(), AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) if same_inode(&stat, &self.identity) => {
                rustix::fs::unlinkat(&self.root, self.name.as_str(), AtFlags::REMOVEDIR)?;
            }
            Ok(_) => {
                return Err(io::Error::other(
                    "quarantine name was replaced; foreign path retained",
                ));
            }
            Err(e) if e == rustix::io::Errno::NOENT => {}
            Err(e) => return Err(e.into()),
        }
        self.removed = true;
        self.root.sync_all()
    }
}

impl Drop for Quarantine {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

struct QuarantineSink<'a, I> {
    quarantine: &'a mut Quarantine,
    io: &'a mut I,
    cancelled: &'a dyn Fn() -> bool,
    max_files: u64,
    max_bytes: u64,
    declared: u64,
    current: Option<usize>,
}

impl<I: FileIo> FileSink for QuarantineSink<'_, I> {
    type Error = StagingError;
    fn begin_file(&mut self, _index: u64, path: &str, size: u64) -> Result<(), StagingError> {
        poll(self.cancelled)?;
        if self.current.is_some() || self.quarantine.files.len() as u64 >= self.max_files {
            return Err(StagingError::Quota);
        }
        self.declared = self
            .declared
            .checked_add(size)
            .filter(|n| *n <= self.max_bytes)
            .ok_or(StagingError::Quota)?;
        let name = format!("file-{:016x}", self.quarantine.files.len());
        let mut archive_path = String::new();
        archive_path
            .try_reserve_exact(path.len())
            .map_err(|_| StagingError::Allocation)?;
        archive_path.push_str(path);
        let file = File::from(
            rustix::fs::openat(
                &self.quarantine.directory,
                name.as_str(),
                OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )
            .map_err(|e| io_error("create private file", e))?,
        );
        let identity = rustix::fs::fstat(&file).map_err(|e| {
            created_failure(
                &self.quarantine.directory,
                &name,
                AtFlags::empty(),
                io_error("stat private file", e),
            )
        })?;
        self.current = Some(self.quarantine.files.len());
        self.quarantine.files.push(PrivateFile {
            name,
            archive_path,
            declared: size,
            written: 0,
            file,
            identity,
        });
        Ok(())
    }
    fn chunk(&mut self, bytes: &[u8]) -> Result<(), StagingError> {
        poll(self.cancelled)?;
        let index = self.current.ok_or(StagingError::ManifestMismatch)?;
        let file = &mut self.quarantine.files[index];
        file.written = file
            .written
            .checked_add(bytes.len() as u64)
            .filter(|n| *n <= file.declared)
            .ok_or(StagingError::Quota)?;
        let mut remaining = bytes;
        while !remaining.is_empty() {
            poll(self.cancelled)?;
            match self.io.write(&mut file.file, remaining) {
                Ok(0) => return Err(io_error("write private file", io::ErrorKind::WriteZero)),
                Ok(n) if n <= remaining.len() => remaining = &remaining[n..],
                Ok(_) => return Err(StagingError::ManifestMismatch),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(io_error("write private file", e)),
            }
        }
        Ok(())
    }
    fn end_file(&mut self) -> Result<(), StagingError> {
        let index = self.current.take().ok_or(StagingError::ManifestMismatch)?;
        let file = &self.quarantine.files[index];
        if file.written != file.declared {
            return Err(StagingError::ManifestMismatch);
        }
        Ok(())
    }
}

// Re-admit the expected entry mapping before creating any quarantine. Synthetic
// header indexes are used only for pure consistency checking; the full replay
// recomputes the real header order/count and compressed identity independently.
fn validate_expected(
    policy: &AdmissionPolicy,
    expected: &RestoreManifest,
) -> Result<(), StagingError> {
    if expected.manifest_version != MANIFEST_VERSION
        || expected.policy_revision != policy.revision
        || expected.entries.len() as u64 > policy.limits.max_entries
    {
        return Err(StagingError::ManifestMismatch);
    }
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(expected.entries.len())
        .map_err(|_| StagingError::Allocation)?;
    for (index, entry) in expected.entries.iter().enumerate() {
        if entry.archive_path.len() > policy.limits.max_path_bytes as usize
            || entry.sha256.as_ref().is_some_and(|s| s.len() != 64)
        {
            return Err(StagingError::ManifestMismatch);
        }
        entries.push(InspectedEntry {
            header_index: index as u64,
            archive_path: entry.archive_path.clone(),
            kind: entry.kind,
            size: entry.size,
            sha256: entry.sha256.clone(),
            archived_mode: entry.archived_permissions,
        });
    }
    let report = InspectionReport {
        format: expected.format,
        archive_sha256: expected.archive_sha256.clone(),
        compressed_bytes: expected.compressed_bytes,
        decompressed_bytes: expected.decompressed_bytes,
        header_count: expected.header_count,
        metadata_bytes: 0,
        coalesced_self_hardlinks: expected.coalesced_self_hardlinks,
        entries,
    };
    if admit(policy, &report)? != *expected {
        return Err(StagingError::ManifestMismatch);
    }
    Ok(())
}

fn verify_staged<I: FileIo>(
    quarantine: &mut Quarantine,
    manifest: &RestoreManifest,
    io: &mut I,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), StagingError> {
    if quarantine.files.len() as u64 != manifest.file_count {
        return Err(StagingError::ManifestMismatch);
    }
    let mut chunk = buffer(CHUNK, None)?;
    for private in &mut quarantine.files {
        poll(cancelled)?;
        let entry = manifest
            .entries
            .iter()
            .find(|e| e.kind == EntryKind::File && e.archive_path == private.archive_path)
            .ok_or(StagingError::ManifestMismatch)?;
        let stat = rustix::fs::fstat(&private.file).map_err(|e| io_error("stat staged file", e))?;
        if !same_inode(&stat, &private.identity)
            || stat.st_nlink != 1
            || stat.st_size < 0
            || stat.st_size as u64 != entry.size
            || private.written != entry.size
        {
            return Err(StagingError::ManifestMismatch);
        }
        private
            .file
            .seek(SeekFrom::Start(0))
            .map_err(|e| io_error("rewind staged file", e))?;
        let mut hasher = Sha256::new();
        let mut read = 0u64;
        loop {
            poll(cancelled)?;
            let n = match private.file.read(&mut chunk) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                result => result.map_err(|e| io_error("verify staged file", e))?,
            };
            if n == 0 {
                break;
            }
            read = read
                .checked_add(n as u64)
                .filter(|n| *n <= entry.size)
                .ok_or(StagingError::ManifestMismatch)?;
            hasher.update(&chunk[..n]);
        }
        if read != entry.size
            || entry.sha256.as_deref() != Some(hex::encode(hasher.finalize()).as_str())
        {
            return Err(StagingError::ManifestMismatch);
        }
        rustix::fs::fchmod(&private.file, Mode::from_raw_mode(0o400))
            .map_err(|e| io_error("seal staged file permissions", e))?;
        io.sync(&private.file)
            .map_err(|e| io_error("fsync staged file", e))?;
        let readonly = File::from(
            rustix::fs::openat(
                &quarantine.directory,
                private.name.as_str(),
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|e| io_error("open verified file read-only", e))?,
        );
        let observed =
            rustix::fs::fstat(&readonly).map_err(|e| io_error("stat read-only staged file", e))?;
        if !same_inode(&observed, &private.identity)
            || observed.st_nlink != 1
            || observed.st_size != stat.st_size
        {
            return Err(StagingError::ManifestMismatch);
        }
        private.file = readonly;
    }
    io.sync(&quarantine.directory)
        .map_err(|e| io_error("fsync quarantine directory", e))?;
    io.sync(&quarantine.root)
        .map_err(|e| io_error("fsync staging root", e))?;
    poll(cancelled)
}

/// Recompute all v2 manifest facts and stage only internally named private files.
/// On normal refusal, owned files are removed. Killed processes can leave
/// `incomplete-*` artifacts; they have no success marker or resumable catalog.
pub fn replay<R: Read>(
    source: R,
    policy: &AdmissionPolicy,
    expected: &RestoreManifest,
    snapshot: &PolicySnapshot,
    root: &StagingRoot,
    cancelled: &dyn Fn() -> bool,
) -> Result<VerifiedCatalog, StagingError> {
    replay_with_io(
        source,
        policy,
        expected,
        snapshot,
        root,
        cancelled,
        &mut RealIo,
    )
}

fn replay_with_io<R: Read, I: FileIo>(
    source: R,
    policy: &AdmissionPolicy,
    expected: &RestoreManifest,
    snapshot: &PolicySnapshot,
    root: &StagingRoot,
    cancelled: &dyn Fn() -> bool,
    io: &mut I,
) -> Result<VerifiedCatalog, StagingError> {
    poll(cancelled)?;
    policy.validate()?;
    if policy != snapshot.policy() {
        return Err(StagingError::PolicyChanged);
    }
    if expected.archive_sha256 != snapshot.archive_sha256 {
        return Err(StagingError::ManifestMismatch);
    }
    validate_expected(policy, expected)?;
    let mut quarantine = Quarantine::create(root, expected.file_count)?;
    let result = (|| {
        let mut sink = QuarantineSink {
            quarantine: &mut quarantine,
            io,
            cancelled,
            max_files: expected.file_count,
            max_bytes: expected
                .file_bytes
                .min(policy.limits.max_decompressed_bytes),
            declared: 0,
            current: None,
        };
        let report = inspect_stream::<_, _, StagingError>(
            source,
            policy,
            cancelled,
            Some(expected),
            &mut sink,
        )?;
        let manifest = admit(policy, &report)?;
        if manifest != *expected {
            return Err(StagingError::ManifestMismatch);
        }
        verify_staged(&mut quarantine, &manifest, io, cancelled)?;
        Ok(manifest)
    })();
    match result {
        Ok(manifest) => Ok(VerifiedCatalog {
            manifest,
            quarantine,
        }),
        Err(failure) => match quarantine.cleanup() {
            Ok(()) => Err(failure),
            Err(source) => Err(StagingError::Cleanup {
                failure: Box::new(failure),
                source,
            }),
        },
    }
}

/// Immutable completed metadata and borrowed read-only access. Dropping it
/// removes only this attempt; use `discard` to observe cleanup I/O failures.
#[derive(Debug)]
pub struct VerifiedCatalog {
    manifest: RestoreManifest,
    quarantine: Quarantine,
}

impl VerifiedCatalog {
    pub fn manifest(&self) -> &RestoreManifest {
        &self.manifest
    }
    pub fn entries(&self) -> impl Iterator<Item = CatalogEntry<'_>> {
        self.manifest.entries.iter().map(|metadata| CatalogEntry {
            metadata,
            file: self
                .quarantine
                .files
                .iter()
                .find(|f| f.archive_path == metadata.archive_path)
                .map(|f| &f.file),
        })
    }
    pub fn discard(mut self) -> Result<(), StagingError> {
        self.quarantine
            .cleanup()
            .map_err(|e| io_error("discard private quarantine", e))
    }
}

pub struct CatalogEntry<'a> {
    metadata: &'a ManifestEntry,
    file: Option<&'a File>,
}
impl CatalogEntry<'_> {
    pub fn metadata(&self) -> &ManifestEntry {
        self.metadata
    }
    pub fn reader(&self) -> Option<StagedReader<'_>> {
        self.file.map(|file| StagedReader {
            file,
            offset: 0,
            size: self.metadata.size,
        })
    }
}

/// Bounded positional reads do not share offsets or expose a writable handle.
pub struct StagedReader<'a> {
    file: &'a File,
    offset: u64,
    size: u64,
}
impl Read for StagedReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let want = buffer
            .len()
            .min(usize::try_from(self.size - self.offset).unwrap_or(usize::MAX));
        if want == 0 {
            return Ok(0);
        }
        let n = self.file.read_at(&mut buffer[..want], self.offset)?;
        if n == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        self.offset += n as u64;
        Ok(n)
    }
}

#[cfg(test)]
mod tests;
