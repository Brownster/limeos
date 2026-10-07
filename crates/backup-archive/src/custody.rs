//! Private custody of one admitted compressed archive across restarts.
//!
//! [`retain`] consumes a selected regular-file descriptor, copies its bytes once
//! into an internally named private file, seals and syncs that copy, admits it
//! through [`staging::admit_for_staging`] and publishes an *admitted retained
//! archive*: the sealed bytes plus closed, bounded metadata, renamed into the
//! caller's private [`CustodyRoot`] without replacement. The returned
//! [`CustodyBinding`] belongs in the caller's durable job record.
//!
//! [`recover`] opens only the record a trusted binding names, checks every
//! object through held descriptors, compares the saved full policy with the
//! current trusted policy and re-admits the actual sealed bytes. Saved metadata
//! never authorizes anything by itself. [`RetainedArchive::stage`] then
//! produces a fresh [`VerifiedCatalog`] through [`staging::replay`].
//!
//! Nothing here restores, extracts or opens a live destination. The caller owns
//! protected ancestors, ACLs, mount identity, space reservation and exclusion
//! of other writers, including processes with the same UID. See
//! `docs/p04-backup-archive-custody.md`.

use crate::staging::{self, AdmittedArchive, StagingError, StagingRoot, VerifiedCatalog};
use crate::{CHUNK, buffer};
use limeos_domain::backups::{AdmissionPolicy, FindingCode, Rejection, RestoreManifest};
use rustix::fs::{AtFlags, FileType, Mode, OFlags, RenameFlags, ResolveFlags, Stat};
use rustix::io::Errno;
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

mod identity;
#[cfg(test)]
mod tests;

/// Version of the binding and of the record format it names.
pub const FORMAT_VERSION: u32 = 1;
/// Hard ceiling for [`CustodyLimits::max_record_bytes`]. Records are read into
/// memory once, so this bounds that allocation independently of callers.
pub const MAX_RECORD_BYTES: u64 = 64 * 1024 * 1024;

const ARCHIVE: &str = "archive";
const RECORD: &str = "record.json";
const PENDING: &str = "pending-";
const PUBLISHED: &str = "retained-";
const RECORD_ID_HEX: usize = 32;

/// Finite trusted custody limits. Admission limits stay in the policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustodyLimits {
    /// Compressed bytes copied from the selected source. A larger source is
    /// refused after reading at most one byte past this limit.
    pub max_archive_bytes: u64,
    /// Bytes in one serialized record and in each identity encoding.
    pub max_record_bytes: u64,
}

impl CustodyLimits {
    fn validate(&self) -> Result<(), CustodyError> {
        if self.max_archive_bytes == 0
            || self.max_archive_bytes > i64::MAX as u64
            || self.max_record_bytes == 0
            || self.max_record_bytes > MAX_RECORD_BYTES
        {
            return Err(CustodyError::InvalidLimits);
        }
        Ok(())
    }
}

/// The identity a trusted caller stores in its durable job record and later
/// supplies, unchanged, to [`recover`]. It is a claim to verify, never proof:
/// recovery recomputes every field from the actual sealed bytes and policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustodyBinding {
    /// Record format version; currently [`FORMAT_VERSION`].
    pub format_version: u32,
    /// Internally generated 128-bit identifier, lowercase hex.
    pub record_id: String,
    /// SHA-256 of every retained compressed byte, lowercase hex.
    pub archive_sha256: String,
    pub archive_bytes: u64,
    /// SHA-256 of the complete policy's versioned identity encoding.
    pub policy_identity: String,
    /// SHA-256 of the complete manifest's versioned identity encoding.
    pub manifest_identity: String,
}

impl CustodyBinding {
    fn well_formed(&self) -> bool {
        self.format_version == FORMAT_VERSION
            && lower_hex(&self.record_id, RECORD_ID_HEX)
            && lower_hex(&self.archive_sha256, 64)
            && self.archive_bytes > 0
            && lower_hex(&self.policy_identity, 64)
            && lower_hex(&self.manifest_identity, 64)
    }
}

fn lower_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Closed record contents. The binding, full typed policy and full manifest
/// must agree with each other and be byte-identical to their canonical form.
#[derive(Serialize)]
struct RecordRef<'a> {
    binding: &'a CustodyBinding,
    policy: &'a AdmissionPolicy,
    manifest: &'a RestoreManifest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    binding: CustodyBinding,
    policy: AdmissionPolicy,
    manifest: RestoreManifest,
}

fn encode_record(
    binding: &CustodyBinding,
    policy: &AdmissionPolicy,
    manifest: &RestoreManifest,
) -> Result<Vec<u8>, CustodyError> {
    serde_json::to_vec(&RecordRef {
        binding,
        policy,
        manifest,
    })
    .map_err(|e| io_error("encode custody record", io::Error::other(e)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustodyLimit {
    Archive,
    Record,
}

/// A cleanup step that could not complete. Foreign or replaced objects are
/// left in place and reported; nothing is followed or recursed into.
#[derive(Debug)]
pub enum CleanupFailure {
    /// An owned name now refers to a different object, which was retained.
    Replaced(&'static str),
    /// Entries this library did not create remain in an owned directory.
    Foreign(u64),
    Io {
        operation: &'static str,
        source: io::Error,
    },
    Staging(StagingError),
}

impl fmt::Display for CleanupFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Replaced(what) => write!(f, "{what} was replaced; foreign object retained"),
            Self::Foreign(count) => write!(f, "{count} foreign entries retained"),
            Self::Io { operation, source } => write!(f, "{operation}: {source}"),
            Self::Staging(error) => write!(f, "staging cleanup: {error}"),
        }
    }
}

/// No failure carries a retained archive or catalog.
#[derive(Debug)]
pub enum CustodyError {
    /// The root is not an existing, effective-UID owned 0700 directory reached
    /// without symlinks, or the kernel lacks `openat2` protection.
    UnsafeRoot,
    InvalidLimits,
    /// The selected descriptor is not a regular file.
    UnsafeSource,
    MalformedBinding,
    /// Saved metadata is not one canonical, closed, consistent record.
    MalformedRecord,
    Limit(CustodyLimit),
    Admission(Rejection),
    /// Current trusted policy differs from the retained admission policy.
    PolicyChanged,
    /// Actual bytes, saved facts and the trusted binding disagree.
    BindingMismatch,
    /// A custody object has the wrong type, owner, mode, link count, size or
    /// identity, or is reached through a symlink.
    UnsafeObject(&'static str),
    /// A record directory contains an entry this library did not create.
    ForeignEntry,
    /// No published record has the binding's identifier.
    NotFound,
    /// A generated name already exists; nothing was replaced.
    Collision,
    Cancelled,
    Allocation,
    Io {
        operation: &'static str,
        source: io::Error,
    },
    /// The record was renamed into place but the root fsync failed. It may or
    /// may not survive a crash; it was not rolled back. A later [`recover`]
    /// with this binding still performs full validation.
    AmbiguousDurability {
        binding: Box<CustodyBinding>,
        source: io::Error,
    },
    Staging(StagingError),
    /// Cleanup failed; `failure` is the primary error, absent for `discard`.
    Cleanup {
        failure: Option<Box<CustodyError>>,
        cleanup: CleanupFailure,
    },
}

impl From<Rejection> for CustodyError {
    fn from(value: Rejection) -> Self {
        let has = |code| value.findings.iter().any(|f| f.code == code);
        if has(FindingCode::Cancelled) {
            Self::Cancelled
        } else if has(FindingCode::Allocation) {
            Self::Allocation
        } else {
            Self::Admission(value)
        }
    }
}

impl fmt::Display for CustodyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsafeRoot => f.write_str("custody root is not a protected private directory"),
            Self::InvalidLimits => f.write_str("custody limits are not finite and positive"),
            Self::UnsafeSource => f.write_str("selected archive is not a regular file"),
            Self::MalformedBinding => f.write_str("custody binding is malformed"),
            Self::MalformedRecord => f.write_str("custody record is malformed"),
            Self::Limit(CustodyLimit::Archive) => f.write_str("archive exceeds custody limit"),
            Self::Limit(CustodyLimit::Record) => f.write_str("record exceeds custody limit"),
            Self::Admission(r) => write!(f, "archive refused: {:?}", r.codes()),
            Self::PolicyChanged => f.write_str("trusted admission policy changed"),
            Self::BindingMismatch => f.write_str("retained archive differs from its binding"),
            Self::UnsafeObject(what) => write!(f, "unsafe custody object: {what}"),
            Self::ForeignEntry => f.write_str("record directory contains a foreign entry"),
            Self::NotFound => f.write_str("no published record for this binding"),
            Self::Collision => f.write_str("generated custody name already exists"),
            Self::Cancelled => f.write_str("custody cancelled"),
            Self::Allocation => f.write_str("bounded custody allocation failed"),
            Self::Io { operation, source } => write!(f, "{operation}: {source}"),
            Self::AmbiguousDurability { source, .. } => {
                write!(f, "record published but durability unknown: {source}")
            }
            Self::Staging(error) => write!(f, "staging: {error}"),
            Self::Cleanup {
                failure: Some(failure),
                cleanup,
            } => write!(f, "{failure}; cleanup failed: {cleanup}"),
            Self::Cleanup {
                failure: None,
                cleanup,
            } => write!(f, "cleanup failed: {cleanup}"),
        }
    }
}

impl Error for CustodyError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } | Self::AmbiguousDurability { source, .. } => Some(source),
            Self::Staging(error) => Some(error),
            _ => None,
        }
    }
}

fn io_error(operation: &'static str, source: impl Into<io::Error>) -> CustodyError {
    CustodyError::Io {
        operation,
        source: source.into(),
    }
}

fn poll(cancelled: &dyn Fn() -> bool) -> Result<(), CustodyError> {
    if cancelled() {
        Err(CustodyError::Cancelled)
    } else {
        Ok(())
    }
}

fn euid() -> u32 {
    rustix::process::geteuid().as_raw()
}

fn same_inode(a: &Stat, b: &Stat) -> bool {
    a.st_dev == b.st_dev && a.st_ino == b.st_ino
}

fn private_directory(stat: &Stat) -> bool {
    FileType::from_raw_mode(stat.st_mode) == FileType::Directory
        && stat.st_uid == euid()
        && stat.st_mode & 0o7777 == 0o700
}

/// A sealed object: our regular file, one link, owner read-only, exact size.
fn sealed(stat: &Stat, size: u64) -> bool {
    FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile
        && stat.st_uid == euid()
        && stat.st_nlink == 1
        && stat.st_mode & 0o7777 == 0o400
        && u64::try_from(stat.st_size).is_ok_and(|s| s == size)
}

/// A held no-symlink descriptor to an existing, caller-owned private root.
#[derive(Debug)]
pub struct CustodyRoot {
    directory: File,
}

impl CustodyRoot {
    /// Open with `openat2`, refusing symlink and magic-link components. The
    /// root must be owned by the effective UID with mode exactly 0700.
    pub fn open(path: &Path) -> Result<Self, CustodyError> {
        if !path.is_absolute() {
            return Err(CustodyError::UnsafeRoot);
        }
        let fd = rustix::fs::openat2(
            rustix::fs::CWD,
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(|_| CustodyError::UnsafeRoot)?;
        let stat = rustix::fs::fstat(&fd).map_err(|e| io_error("stat custody root", e))?;
        if !private_directory(&stat) {
            return Err(CustodyError::UnsafeRoot);
        }
        Ok(Self {
            directory: File::from(fd),
        })
    }
}

/// Durability barriers, named for private fault injection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Barrier {
    Archive,
    Record,
    Pending,
    Root,
    Discard,
}

/// Points at which a test child can be stopped deterministically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Point {
    Copying,
    ArchiveSynced,
    Admitted,
    RecordSynced,
    Renamed,
    Published,
    Readmitting,
    Replaying,
}

// Private fault-injection boundary. Production callers cannot supply it.
trait CustodyIo {
    fn random(&mut self, bytes: &mut [u8; 16]) -> io::Result<()> {
        getrandom::getrandom(bytes).map_err(|e| io::Error::other(e.to_string()))
    }
    fn read_at(&mut self, file: &File, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
        file.read_at(buffer, offset)
    }
    fn write(&mut self, file: &mut File, bytes: &[u8]) -> io::Result<usize> {
        file.write(bytes)
    }
    fn sync(&mut self, file: &File, _barrier: Barrier) -> io::Result<()> {
        file.sync_all()
    }
    fn rename(&mut self, root: &File, from: &str, to: &str) -> io::Result<()> {
        rename_no_replace(root, from, to)
    }
    fn reached(&mut self, _point: Point) {}
}

/// Publication never replaces an existing name, and needs `renameat2`.
fn rename_no_replace(root: &File, from: &str, to: &str) -> io::Result<()> {
    rustix::fs::renameat_with(root, from, root, to, RenameFlags::NOREPLACE).map_err(Into::into)
}

struct RealIo;
impl CustodyIo for RealIo {}

/// Positional reads of one held sealed file; the descriptor offset is unused.
struct Sealed<'a, I> {
    file: &'a File,
    offset: u64,
    io: &'a mut I,
    point: Option<Point>,
}

impl<I: CustodyIo> Read for Sealed<'_, I> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.offset > 0
            && let Some(point) = self.point.take()
        {
            self.io.reached(point);
        }
        let n = self.io.read_at(self.file, buffer, self.offset)?;
        if n > buffer.len() {
            return Err(io::Error::other(
                "reader reported more bytes than requested",
            ));
        }
        self.offset += n as u64;
        Ok(n)
    }
}

fn write_all<I: CustodyIo>(
    io: &mut I,
    file: &mut File,
    bytes: &[u8],
    cancelled: &dyn Fn() -> bool,
    operation: &'static str,
) -> Result<(), CustodyError> {
    let mut remaining = bytes;
    while !remaining.is_empty() {
        poll(cancelled)?;
        match io.write(file, remaining) {
            Ok(0) => return Err(io_error(operation, io::ErrorKind::WriteZero)),
            Ok(n) if n <= remaining.len() => remaining = &remaining[n..],
            Ok(_) => {
                return Err(io_error(
                    operation,
                    io::Error::other("writer reported more bytes than supplied"),
                ));
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(io_error(operation, e)),
        }
    }
    Ok(())
}

/// Copy at most `limit` bytes, reading one sentinel byte past it to detect
/// overflow. Returns the copied size and SHA-256.
fn copy<I: CustodyIo>(
    source: &File,
    target: &mut File,
    limit: u64,
    io: &mut I,
    cancelled: &dyn Fn() -> bool,
) -> Result<(u64, String), CustodyError> {
    let mut chunk = buffer(CHUNK, None)?;
    let mut hasher = Sha256::new();
    let mut count = 0u64;
    loop {
        poll(cancelled)?;
        let allowed = limit.saturating_sub(count).saturating_add(1);
        let want = chunk
            .len()
            .min(usize::try_from(allowed).unwrap_or(usize::MAX));
        let n = match io.read_at(source, &mut chunk[..want], count) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(io_error("read selected archive", e)),
            Ok(n) if n > want => {
                return Err(io_error(
                    "read selected archive",
                    io::Error::other("reader reported more bytes than requested"),
                ));
            }
            Ok(n) => n,
        };
        if n == 0 {
            break;
        }
        count = count
            .checked_add(n as u64)
            .filter(|c| *c <= limit)
            .ok_or(CustodyError::Limit(CustodyLimit::Archive))?;
        hasher.update(&chunk[..n]);
        write_all(io, target, &chunk[..n], cancelled, "write retained archive")?;
        if count == n as u64 {
            io.reached(Point::Copying);
        }
    }
    Ok((count, hex::encode(hasher.finalize())))
}

/// Hash exactly `size` bytes of a sealed file and require EOF after them.
fn hash_sealed<I: CustodyIo>(
    file: &File,
    size: u64,
    io: &mut I,
    cancelled: &dyn Fn() -> bool,
) -> Result<String, CustodyError> {
    let mut chunk = buffer(CHUNK, None)?;
    let mut hasher = Sha256::new();
    let mut offset = 0u64;
    loop {
        poll(cancelled)?;
        let n = match io.read_at(file, &mut chunk, offset) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(io_error("verify retained archive", e)),
            Ok(n) => n.min(chunk.len()),
        };
        if n == 0 {
            break;
        }
        offset = offset
            .checked_add(n as u64)
            .filter(|o| *o <= size)
            .ok_or(CustodyError::BindingMismatch)?;
        hasher.update(&chunk[..n]);
    }
    if offset != size {
        return Err(CustodyError::BindingMismatch);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn open_private_file(directory: &File, name: &str) -> Result<File, CustodyError> {
    rustix::fs::openat(
        directory,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|e| match e {
        Errno::LOOP => CustodyError::UnsafeObject(name_of(name)),
        Errno::NOENT => CustodyError::UnsafeObject(name_of(name)),
        e => io_error("open custody object", e),
    })
}

fn name_of(name: &str) -> &'static str {
    if name == ARCHIVE { "archive" } else { "record" }
}

/// Require `name` in `directory` to still be the held inode `identity`.
fn bound(directory: &File, name: &str, identity: &Stat) -> bool {
    rustix::fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW)
        .is_ok_and(|s| same_inode(&s, identity))
}

struct Owned {
    name: &'static str,
    identity: Stat,
}

/// A generated pending directory and the files created in it. Until
/// publication, failure removes only these owned names after inode checks.
struct Pending {
    root: File,
    directory: File,
    name: String,
    identity: Stat,
    files: Vec<Owned>,
    armed: bool,
}

impl Pending {
    fn create(root: &CustodyRoot, record_id: &str) -> Result<Self, CustodyError> {
        let root = root
            .directory
            .try_clone()
            .map_err(|e| io_error("duplicate custody root", e))?;
        match rustix::fs::statat(
            &root,
            format!("{PUBLISHED}{record_id}"),
            AtFlags::SYMLINK_NOFOLLOW,
        ) {
            Err(Errno::NOENT) => {}
            Ok(_) => return Err(CustodyError::Collision),
            Err(e) => return Err(io_error("check published name", e)),
        }
        let name = format!("{PENDING}{record_id}");
        match rustix::fs::mkdirat(&root, name.as_str(), Mode::from_raw_mode(0o700)) {
            Ok(()) => {}
            Err(Errno::EXIST) => return Err(CustodyError::Collision),
            Err(e) => return Err(io_error("create pending directory", e)),
        }
        let opened = rustix::fs::openat(
            &root,
            name.as_str(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(File::from)
        .and_then(|directory| {
            rustix::fs::fchmod(&directory, Mode::from_raw_mode(0o700))?;
            Ok((rustix::fs::fstat(&directory)?, directory))
        });
        let (identity, directory) = match opened {
            Ok(opened) => opened,
            // Just created and still empty; the caller's root excludes writers.
            Err(e) => {
                let failure = io_error("open pending directory", e);
                return Err(
                    match rustix::fs::unlinkat(&root, name.as_str(), AtFlags::REMOVEDIR) {
                        Ok(()) | Err(Errno::NOENT) => failure,
                        Err(e) => CustodyError::Cleanup {
                            failure: Some(Box::new(failure)),
                            cleanup: CleanupFailure::Io {
                                operation: "remove pending directory",
                                source: e.into(),
                            },
                        },
                    },
                );
            }
        };
        if !private_directory(&identity) {
            // Not provably ours: leave it for the caller to inspect.
            return Err(CustodyError::UnsafeObject("pending directory"));
        }
        let mut files = Vec::new();
        files
            .try_reserve_exact(2)
            .map_err(|_| CustodyError::Allocation)?;
        Ok(Self {
            root,
            directory,
            name,
            identity,
            files,
            armed: true,
        })
    }

    fn create_file(&mut self, name: &'static str) -> Result<File, CustodyError> {
        let file = rustix::fs::openat(
            &self.directory,
            name,
            OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map(File::from)
        .map_err(|e| match e {
            Errno::EXIST => CustodyError::Collision,
            e => io_error("create custody object", e),
        })?;
        let identity = rustix::fs::fstat(&file).map_err(|e| io_error("stat custody object", e));
        // Record ownership first so any later failure removes this name.
        let identity = match identity {
            Ok(identity) => identity,
            Err(failure) => {
                return Err(
                    match rustix::fs::unlinkat(&self.directory, name, AtFlags::empty()) {
                        Ok(()) | Err(Errno::NOENT) => failure,
                        Err(e) => CustodyError::Cleanup {
                            failure: Some(Box::new(failure)),
                            cleanup: CleanupFailure::Io {
                                operation: "remove custody object",
                                source: e.into(),
                            },
                        },
                    },
                );
            }
        };
        self.files.push(Owned { name, identity });
        Ok(file)
    }

    fn identity(&self, name: &str) -> Option<&Stat> {
        self.files
            .iter()
            .find(|f| f.name == name)
            .map(|f| &f.identity)
    }

    fn cleanup(&mut self) -> Result<(), CleanupFailure> {
        if !self.armed {
            return Ok(());
        }
        for owned in self.files.iter().rev() {
            remove_owned(&self.directory, owned.name, &owned.identity, owned.name)?;
        }
        remove_directory(&self.root, &self.name, &self.directory, &self.identity)?;
        self.armed = false;
        self.root.sync_all().map_err(|source| CleanupFailure::Io {
            operation: "fsync custody root after cleanup",
            source,
        })
    }

    fn fail(&mut self, failure: CustodyError) -> CustodyError {
        match self.cleanup() {
            Ok(()) => failure,
            Err(cleanup) => CustodyError::Cleanup {
                failure: Some(Box::new(failure)),
                cleanup,
            },
        }
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn remove_owned(
    directory: &File,
    name: &str,
    identity: &Stat,
    what: &'static str,
) -> Result<(), CleanupFailure> {
    match rustix::fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) if same_inode(&stat, identity) => {
            rustix::fs::unlinkat(directory, name, AtFlags::empty()).map_err(|e| {
                CleanupFailure::Io {
                    operation: "remove custody object",
                    source: e.into(),
                }
            })
        }
        Ok(_) => Err(CleanupFailure::Replaced(what)),
        Err(Errno::NOENT) => Ok(()),
        Err(e) => Err(CleanupFailure::Io {
            operation: "stat custody object",
            source: e.into(),
        }),
    }
}

/// Remove an owned directory only if its name is still the held inode and it
/// holds nothing else; foreign entries are counted and left in place.
fn remove_directory(
    root: &File,
    name: &str,
    directory: &File,
    identity: &Stat,
) -> Result<(), CleanupFailure> {
    match rustix::fs::statat(root, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) if same_inode(&stat, identity) => {
            let foreign = count_entries(directory)?;
            if foreign > 0 {
                return Err(CleanupFailure::Foreign(foreign));
            }
            rustix::fs::unlinkat(root, name, AtFlags::REMOVEDIR).map_err(|e| CleanupFailure::Io {
                operation: "remove custody directory",
                source: e.into(),
            })
        }
        Ok(_) => Err(CleanupFailure::Replaced("directory")),
        Err(Errno::NOENT) => Ok(()),
        Err(e) => Err(CleanupFailure::Io {
            operation: "stat custody directory",
            source: e.into(),
        }),
    }
}

/// Entries other than `.` and `..`, counted through the held descriptor.
fn count_entries(directory: &File) -> Result<u64, CleanupFailure> {
    let io = |source: Errno| CleanupFailure::Io {
        operation: "list custody directory",
        source: source.into(),
    };
    let mut listing = rustix::fs::Dir::read_from(directory).map_err(io)?;
    let mut count = 0u64;
    while let Some(entry) = listing.read() {
        let entry = entry.map_err(io)?;
        if !matches!(entry.file_name().to_bytes(), b"." | b"..") {
            count += 1;
        }
    }
    Ok(count)
}

/// An admitted archive held in private custody. Fields are private; it is
/// constructed only by [`retain`] or [`recover`] from verified actual bytes.
/// Dropping it closes descriptors and keeps the published record for restart.
#[derive(Debug)]
pub struct RetainedArchive {
    root: File,
    directory: File,
    name: String,
    directory_identity: Stat,
    archive: File,
    archive_identity: Stat,
    record_identity: Stat,
    admitted: AdmittedArchive,
    binding: CustodyBinding,
    limits: CustodyLimits,
}

/// Copy, seal, admit and publish one selected archive. The descriptor is
/// consumed; its path is never used. The copied bytes are what was admitted.
pub fn retain(
    source: File,
    policy: AdmissionPolicy,
    root: &CustodyRoot,
    limits: &CustodyLimits,
    cancelled: &dyn Fn() -> bool,
) -> Result<RetainedArchive, CustodyError> {
    retain_with_io(source, policy, root, limits, cancelled, &mut RealIo)
}

fn retain_with_io<I: CustodyIo>(
    source: File,
    policy: AdmissionPolicy,
    root: &CustodyRoot,
    limits: &CustodyLimits,
    cancelled: &dyn Fn() -> bool,
    io: &mut I,
) -> Result<RetainedArchive, CustodyError> {
    poll(cancelled)?;
    limits.validate()?;
    policy.validate()?;
    let stat = rustix::fs::fstat(&source).map_err(|e| io_error("stat selected archive", e))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(CustodyError::UnsafeSource);
    }
    if u64::try_from(stat.st_size).map_or(true, |s| s > limits.max_archive_bytes) {
        return Err(CustodyError::Limit(CustodyLimit::Archive));
    }
    let mut id = [0u8; 16];
    io.random(&mut id)
        .map_err(|e| io_error("generate record identifier", e))?;
    let record_id = hex::encode(id);
    let mut pending = Pending::create(root, &record_id)?;
    match seal_and_admit(source, policy, &mut pending, limits, cancelled, io) {
        Ok((archive, admitted, digest, size)) => publish(
            pending, archive, admitted, record_id, digest, size, limits, cancelled, io,
        ),
        Err(failure) => Err(pending.fail(failure)),
    }
}

fn seal_and_admit<I: CustodyIo>(
    source: File,
    policy: AdmissionPolicy,
    pending: &mut Pending,
    limits: &CustodyLimits,
    cancelled: &dyn Fn() -> bool,
    io: &mut I,
) -> Result<(File, AdmittedArchive, String, u64), CustodyError> {
    let mut writable = pending.create_file(ARCHIVE)?;
    let (size, digest) = copy(
        &source,
        &mut writable,
        limits.max_archive_bytes,
        io,
        cancelled,
    )?;
    drop(source);
    let created = *pending.identity(ARCHIVE).ok_or(CustodyError::Allocation)?;
    rustix::fs::fchmod(&writable, Mode::from_raw_mode(0o400))
        .map_err(|e| io_error("seal retained archive", e))?;
    let stat = rustix::fs::fstat(&writable).map_err(|e| io_error("stat retained archive", e))?;
    if !same_inode(&stat, &created) || !sealed(&stat, size) {
        return Err(CustodyError::UnsafeObject("archive"));
    }
    io.sync(&writable, Barrier::Archive)
        .map_err(|e| io_error("fsync retained archive", e))?;
    drop(writable);
    io.reached(Point::ArchiveSynced);
    let archive = open_private_file(&pending.directory, ARCHIVE)?;
    let stat = rustix::fs::fstat(&archive).map_err(|e| io_error("stat retained archive", e))?;
    if !same_inode(&stat, &created) || !sealed(&stat, size) {
        return Err(CustodyError::UnsafeObject("archive"));
    }
    if hash_sealed(&archive, size, io, cancelled)? != digest {
        return Err(CustodyError::BindingMismatch);
    }
    let admitted = staging::admit_for_staging(
        Sealed {
            file: &archive,
            offset: 0,
            io,
            point: None,
        },
        policy,
        cancelled,
    )?;
    let manifest = admitted.manifest();
    let after = rustix::fs::fstat(&archive).map_err(|e| io_error("stat retained archive", e))?;
    if manifest.archive_sha256 != digest
        || manifest.compressed_bytes != size
        || !same_inode(&after, &created)
        || !sealed(&after, size)
    {
        return Err(CustodyError::BindingMismatch);
    }
    Ok((archive, admitted, digest, size))
}

#[allow(clippy::too_many_arguments)]
fn publish<I: CustodyIo>(
    mut pending: Pending,
    archive: File,
    admitted: AdmittedArchive,
    record_id: String,
    archive_sha256: String,
    archive_bytes: u64,
    limits: &CustodyLimits,
    cancelled: &dyn Fn() -> bool,
    io: &mut I,
) -> Result<RetainedArchive, CustodyError> {
    let prepared = (|| {
        let policy = admitted.policy_snapshot().policy();
        let binding = CustodyBinding {
            format_version: FORMAT_VERSION,
            record_id: record_id.clone(),
            archive_sha256,
            archive_bytes,
            policy_identity: identity::policy(policy, limits.max_record_bytes)?,
            manifest_identity: identity::manifest(admitted.manifest(), limits.max_record_bytes)?,
        };
        io.reached(Point::Admitted);
        let bytes = encode_record(&binding, policy, admitted.manifest())?;
        if bytes.len() as u64 > limits.max_record_bytes {
            return Err(CustodyError::Limit(CustodyLimit::Record));
        }
        let mut record = pending.create_file(RECORD)?;
        write_all(io, &mut record, &bytes, cancelled, "write custody record")?;
        let created = *pending.identity(RECORD).ok_or(CustodyError::Allocation)?;
        rustix::fs::fchmod(&record, Mode::from_raw_mode(0o400))
            .map_err(|e| io_error("seal custody record", e))?;
        let stat = rustix::fs::fstat(&record).map_err(|e| io_error("stat custody record", e))?;
        if !same_inode(&stat, &created) || !sealed(&stat, bytes.len() as u64) {
            return Err(CustodyError::UnsafeObject("record"));
        }
        io.sync(&record, Barrier::Record)
            .map_err(|e| io_error("fsync custody record", e))?;
        drop(record);
        io.sync(&pending.directory, Barrier::Pending)
            .map_err(|e| io_error("fsync pending directory", e))?;
        io.reached(Point::RecordSynced);
        // The last point at which cancellation or failure can still roll back.
        poll(cancelled)?;
        Ok((binding, stat))
    })();
    let (binding, record_identity) = match prepared {
        Ok(prepared) => prepared,
        Err(failure) => return Err(pending.fail(failure)),
    };
    let published = format!("{PUBLISHED}{record_id}");
    if let Err(e) = io.rename(&pending.root, &pending.name, &published) {
        let failure = if e.kind() == io::ErrorKind::AlreadyExists {
            CustodyError::Collision
        } else {
            io_error("publish custody record", e)
        };
        return Err(pending.fail(failure));
    }
    // From here the record is published: never roll it back.
    pending.armed = false;
    io.reached(Point::Renamed);
    if let Err(source) = io.sync(&pending.root, Barrier::Root) {
        return Err(CustodyError::AmbiguousDurability {
            binding: Box::new(binding),
            source,
        });
    }
    io.reached(Point::Published);
    let archive_identity = *pending.identity(ARCHIVE).ok_or(CustodyError::Allocation)?;
    Ok(RetainedArchive {
        root: pending
            .root
            .try_clone()
            .map_err(|e| io_error("duplicate custody root", e))?,
        directory: pending
            .directory
            .try_clone()
            .map_err(|e| io_error("duplicate record directory", e))?,
        name: published,
        directory_identity: pending.identity,
        archive,
        archive_identity,
        record_identity,
        admitted,
        binding,
        limits: limits.clone(),
    })
}

/// Re-open the exact record a trusted binding names, verify every object and
/// re-admit the actual sealed bytes under the current trusted policy.
pub fn recover(
    root: &CustodyRoot,
    expected: &CustodyBinding,
    policy: AdmissionPolicy,
    limits: &CustodyLimits,
    cancelled: &dyn Fn() -> bool,
) -> Result<RetainedArchive, CustodyError> {
    recover_with_io(root, expected, policy, limits, cancelled, &mut RealIo)
}

fn recover_with_io<I: CustodyIo>(
    root: &CustodyRoot,
    expected: &CustodyBinding,
    policy: AdmissionPolicy,
    limits: &CustodyLimits,
    cancelled: &dyn Fn() -> bool,
    io: &mut I,
) -> Result<RetainedArchive, CustodyError> {
    poll(cancelled)?;
    limits.validate()?;
    if !expected.well_formed() {
        return Err(CustodyError::MalformedBinding);
    }
    policy.validate()?;
    if expected.archive_bytes > limits.max_archive_bytes {
        return Err(CustodyError::Limit(CustodyLimit::Archive));
    }
    let root = root
        .directory
        .try_clone()
        .map_err(|e| io_error("duplicate custody root", e))?;
    let name = format!("{PUBLISHED}{}", expected.record_id);
    let directory = rustix::fs::openat(
        &root,
        name.as_str(),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|e| match e {
        Errno::NOENT => CustodyError::NotFound,
        Errno::LOOP | Errno::NOTDIR => CustodyError::UnsafeObject("record directory"),
        e => io_error("open record directory", e),
    })?;
    let directory_identity =
        rustix::fs::fstat(&directory).map_err(|e| io_error("stat record directory", e))?;
    if !private_directory(&directory_identity) || !bound(&root, &name, &directory_identity) {
        return Err(CustodyError::UnsafeObject("record directory"));
    }
    exact_entries(&directory)?;
    let (record, record_identity) = read_record(&directory, limits)?;
    let bound_record = |record: &Record| -> Result<bool, CustodyError> {
        Ok(record.binding == *expected
            && identity::policy(&record.policy, limits.max_record_bytes)?
                == expected.policy_identity
            && identity::manifest(&record.manifest, limits.max_record_bytes)?
                == expected.manifest_identity
            && record.manifest.archive_sha256 == expected.archive_sha256
            && record.manifest.compressed_bytes == expected.archive_bytes
            && record.manifest.policy_revision == record.policy.revision)
    };
    if !bound_record(&record)? {
        return Err(CustodyError::BindingMismatch);
    }
    if record.policy != policy
        || identity::policy(&policy, limits.max_record_bytes)? != expected.policy_identity
    {
        return Err(CustodyError::PolicyChanged);
    }
    poll(cancelled)?;
    let archive = open_private_file(&directory, ARCHIVE)?;
    let archive_identity =
        rustix::fs::fstat(&archive).map_err(|e| io_error("stat retained archive", e))?;
    if !sealed(&archive_identity, expected.archive_bytes)
        || !bound(&directory, ARCHIVE, &archive_identity)
    {
        return Err(CustodyError::UnsafeObject("archive"));
    }
    let admitted = staging::admit_for_staging(
        Sealed {
            file: &archive,
            offset: 0,
            io,
            point: Some(Point::Readmitting),
        },
        policy,
        cancelled,
    )?;
    let manifest = admitted.manifest();
    if *manifest != record.manifest
        || manifest.archive_sha256 != expected.archive_sha256
        || manifest.compressed_bytes != expected.archive_bytes
        || identity::manifest(manifest, limits.max_record_bytes)? != expected.manifest_identity
    {
        return Err(CustodyError::BindingMismatch);
    }
    let after = rustix::fs::fstat(&archive).map_err(|e| io_error("stat retained archive", e))?;
    if !same_inode(&after, &archive_identity)
        || !sealed(&after, expected.archive_bytes)
        || !bound(&directory, ARCHIVE, &archive_identity)
        || !bound(&directory, RECORD, &record_identity)
        || !bound(&root, &name, &directory_identity)
    {
        return Err(CustodyError::UnsafeObject("archive"));
    }
    Ok(RetainedArchive {
        root,
        directory,
        name,
        directory_identity,
        archive,
        archive_identity,
        record_identity,
        admitted,
        binding: expected.clone(),
        limits: limits.clone(),
    })
}

/// The record directory must hold exactly the archive and record names.
fn exact_entries(directory: &File) -> Result<(), CustodyError> {
    let io = |e: Errno| io_error("list record directory", e);
    let mut listing = rustix::fs::Dir::read_from(directory).map_err(io)?;
    let (mut archive, mut record) = (false, false);
    while let Some(entry) = listing.read() {
        let entry = entry.map_err(io)?;
        match entry.file_name().to_bytes() {
            b"." | b".." => {}
            b"archive" if !archive => archive = true,
            b"record.json" if !record => record = true,
            _ => return Err(CustodyError::ForeignEntry),
        }
    }
    if !archive {
        return Err(CustodyError::UnsafeObject("archive"));
    }
    if !record {
        return Err(CustodyError::UnsafeObject("record"));
    }
    Ok(())
}

fn read_record(directory: &File, limits: &CustodyLimits) -> Result<(Record, Stat), CustodyError> {
    let file = open_private_file(directory, RECORD)?;
    let stat = rustix::fs::fstat(&file).map_err(|e| io_error("stat custody record", e))?;
    let size = u64::try_from(stat.st_size).map_err(|_| CustodyError::MalformedRecord)?;
    if size > limits.max_record_bytes {
        return Err(CustodyError::Limit(CustodyLimit::Record));
    }
    if size == 0 || !sealed(&stat, size) || !bound(directory, RECORD, &stat) {
        return Err(CustodyError::UnsafeObject("record"));
    }
    let len = usize::try_from(size).map_err(|_| CustodyError::Allocation)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(len + 1)
        .map_err(|_| CustodyError::Allocation)?;
    (&file)
        .take(size + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| io_error("read custody record", e))?;
    let after = rustix::fs::fstat(&file).map_err(|e| io_error("stat custody record", e))?;
    if bytes.len() != len || !same_inode(&after, &stat) || !sealed(&after, size) {
        return Err(CustodyError::UnsafeObject("record"));
    }
    let record: Record =
        serde_json::from_slice(&bytes).map_err(|_| CustodyError::MalformedRecord)?;
    // Closed form: the stored bytes must be exactly the canonical encoding.
    if !record.binding.well_formed()
        || encode_record(&record.binding, &record.policy, &record.manifest)? != bytes
    {
        return Err(CustodyError::MalformedRecord);
    }
    Ok((record, stat))
}

impl RetainedArchive {
    pub fn binding(&self) -> &CustodyBinding {
        &self.binding
    }

    pub fn manifest(&self) -> &RestoreManifest {
        self.admitted.manifest()
    }

    /// The held archive and every published name must still be the objects
    /// this owner verified.
    fn check_held(&self) -> Result<(), CustodyError> {
        let held =
            rustix::fs::fstat(&self.archive).map_err(|e| io_error("stat retained archive", e))?;
        if !same_inode(&held, &self.archive_identity)
            || !sealed(&held, self.binding.archive_bytes)
            || !bound(&self.directory, ARCHIVE, &self.archive_identity)
        {
            return Err(CustodyError::UnsafeObject("archive"));
        }
        if !bound(&self.directory, RECORD, &self.record_identity) {
            return Err(CustodyError::UnsafeObject("record"));
        }
        if !bound(&self.root, &self.name, &self.directory_identity) {
            return Err(CustodyError::UnsafeObject("record directory"));
        }
        Ok(())
    }

    /// Recheck the full policy and replay the held sealed bytes into a fresh
    /// private staging attempt. The catalog keeps its own ephemeral cleanup.
    pub fn stage(
        &mut self,
        policy: &AdmissionPolicy,
        root: &StagingRoot,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<VerifiedCatalog, CustodyError> {
        self.stage_with_io(policy, root, cancelled, &mut RealIo)
    }

    fn stage_with_io<I: CustodyIo>(
        &mut self,
        policy: &AdmissionPolicy,
        root: &StagingRoot,
        cancelled: &dyn Fn() -> bool,
        io: &mut I,
    ) -> Result<VerifiedCatalog, CustodyError> {
        poll(cancelled)?;
        if policy != self.admitted.policy_snapshot().policy()
            || identity::policy(policy, self.limits.max_record_bytes)?
                != self.binding.policy_identity
        {
            return Err(CustodyError::PolicyChanged);
        }
        self.check_held()?;
        let catalog = staging::replay(
            Sealed {
                file: &self.archive,
                offset: 0,
                io,
                point: Some(Point::Replaying),
            },
            policy,
            self.admitted.manifest(),
            self.admitted.policy_snapshot(),
            root,
            cancelled,
        )
        .map_err(CustodyError::Staging)?;
        if let Err(failure) = self.check_held() {
            return Err(match catalog.discard() {
                Ok(()) => failure,
                Err(e) => CustodyError::Cleanup {
                    failure: Some(Box::new(failure)),
                    cleanup: CleanupFailure::Staging(e),
                },
            });
        }
        Ok(catalog)
    }

    /// Remove this record after inode checks: the record first, so an
    /// interrupted discard can never leave a recoverable record. Foreign or
    /// replaced objects are retained and reported.
    pub fn discard(self) -> Result<(), CustodyError> {
        self.discard_with_io(&mut RealIo)
    }

    fn discard_with_io<I: CustodyIo>(self, io: &mut I) -> Result<(), CustodyError> {
        let result = (|| {
            remove_owned(&self.directory, RECORD, &self.record_identity, "record")?;
            remove_owned(&self.directory, ARCHIVE, &self.archive_identity, "archive")?;
            remove_directory(
                &self.root,
                &self.name,
                &self.directory,
                &self.directory_identity,
            )?;
            io.sync(&self.root, Barrier::Discard)
                .map_err(|source| CleanupFailure::Io {
                    operation: "fsync custody root after discard",
                    source,
                })
        })();
        result.map_err(|cleanup| CustodyError::Cleanup {
            failure: None,
            cleanup,
        })
    }
}
