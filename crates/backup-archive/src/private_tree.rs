//! The private object tree behind before-image capture (`before_images`) and
//! restore preparation (`preparation`): one exclusive randomly named attempt
//! directory (mode 0700) in a caller-owned private root, holding flat
//! generated `object-<index>` names created exclusively without following
//! symlinks, each bound to the inode it was created as.
//!
//! - Objects are sealed (files 0400, directories 0500, or the mode the
//!   metadata names), synced, reopened read-only, nonblocking and
//!   no-follow, and checked again: name binding, inode, type, private owner
//!   and mode, timestamps, link count and size, emptiness of a directory and
//!   the absence of any extended attribute but a bounded SELinux label.
//!   Files are read back whole and hashed against their recorded SHA-256.
//! - The attempt and the root are rechecked (owner, exact mode, object count,
//!   attributes) on every verification and reader access.
//! - Cleanup runs once (never undone by Drop): it removes only the objects
//!   still bound to their recorded inodes, in reverse order, then the
//!   attempt, then syncs the root; anything else is reported as
//!   unverified, replaced or foreign, never removed.
//!
//! Each user names its own prefix, error-operation labels, failure and
//! metadata types (`Kind`) and keeps its own IO seam trait for tests, bridged
//! into [`Io`]. The attempt and root syncs, and the order of everything
//! between objects, stay with the user, which knows its record.
use crate::CHUNK;
use limeos_domain::backups::EntryKind;
use rustix::{
    fs::{AtFlags, FileType, Mode, OFlags, Stat},
    io::Errno,
};
use sha2::{Digest, Sha256};
use std::{fmt, fs::File, io};

/// Objects one attempt holds at most; a directory listing past it refuses.
pub(crate) const MAX_OBJECTS: u32 = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Barrier {
    File,
    Directory,
    Attempt,
    Root,
    Cleanup,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Point {
    AttemptCreated,
    ObjectCreated,
    Copied,
    Sealed,
    AttemptSynced,
    RootSynced,
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

/// The failures the tree reports, in its user's own failure type.
pub(crate) trait Failure: Sized {
    fn changed() -> Self;
    fn collision() -> Self;
    fn unsafe_root() -> Self;
    fn unsupported_metadata() -> Self;
    fn allocation() -> Self;
    fn cancelled() -> Self;
    fn io(operation: &'static str, source: io::Error) -> Self;
    /// The source of an I/O failure, or `None` for any other failure.
    fn into_io(self) -> Option<io::Error>;
}
/// What the tree reads and writes of an object's metadata.
pub(crate) trait Metadata {
    fn kind(&self) -> EntryKind;
    fn size(&self) -> u64;
    fn sha256(&self) -> Option<&str>;
    fn private_mode(&self) -> u32;
    fn private_owner(&self) -> (u32, u32);
    fn set_private_owner(&mut self, uid: u32, gid: u32);
}
/// The operation names a user's I/O failures carry.
pub(crate) struct Labels {
    pub recheck_root: &'static str,
    pub name: &'static str,
    pub retain_root: &'static str,
    pub create_attempt: &'static str,
    pub bind_attempt: &'static str,
    pub protect_attempt: &'static str,
    pub stat_object: &'static str,
    pub seal_object: &'static str,
    pub fsync_object: &'static str,
}
/// One user of the tree.
pub(crate) trait Kind: Sized {
    type Metadata: Metadata + fmt::Debug;
    type Failure: Failure;
    type Error: From<Self::Failure>;
    /// The attempt directory's name before its 128-bit hex suffix.
    const PREFIX: &'static str;
    const LABELS: Labels;
    fn error(failure: Self::Failure, cleanup: Option<CleanupReport>) -> Self::Error;
}
/// The tree's I/O, which tests replace to inject faults and crash points.
pub(crate) trait Io<K: Kind> {
    fn cleanup_count(&mut self, directory: &File) -> Result<usize, CleanupFailure>;
    fn write(&mut self, file: &mut File, input: &[u8]) -> io::Result<usize>;
    fn read_at(&mut self, file: &File, out: &mut [u8], offset: u64) -> io::Result<usize>;
    fn sync(&mut self, file: &File, barrier: Barrier) -> io::Result<()>;
    fn stat_created(&mut self, file: &File) -> io::Result<Stat>;
    fn reserve<T>(&mut self, values: &mut Vec<T>, count: usize) -> Result<(), K::Failure>;
    fn checkpoint(&mut self, point: Point, directory: &File) -> Result<(), K::Failure>;
}
/// The real I/O of every user, through the user's own IO seam's defaults.
pub(crate) struct RealIo;
/// The real I/O for cleanup on drop or discard, the same for every user (the
/// default `cleanup_count` and a plain `sync_all`); never a test seam.
struct Unseamed;
impl<K: Kind> Io<K> for Unseamed {
    fn cleanup_count(&mut self, directory: &File) -> Result<usize, CleanupFailure> {
        cleanup_count::<K::Failure>(directory)
    }
    fn write(&mut self, file: &mut File, input: &[u8]) -> io::Result<usize> {
        io::Write::write(file, input)
    }
    fn read_at(&mut self, file: &File, out: &mut [u8], offset: u64) -> io::Result<usize> {
        std::os::unix::fs::FileExt::read_at(file, out, offset)
    }
    fn sync(&mut self, file: &File, _: Barrier) -> io::Result<()> {
        file.sync_all()
    }
    fn stat_created(&mut self, file: &File) -> io::Result<Stat> {
        rustix::fs::fstat(file).map_err(Into::into)
    }
    fn reserve<T>(&mut self, values: &mut Vec<T>, count: usize) -> Result<(), K::Failure> {
        reserve(values, count)
    }
    fn checkpoint(&mut self, _: Point, _: &File) -> Result<(), K::Failure> {
        Ok(())
    }
}

pub(crate) fn same_inode(a: &Stat, b: &Stat) -> bool {
    a.st_dev == b.st_dev && a.st_ino == b.st_ino
}
pub(crate) fn own_directory(s: &Stat, mode: u32) -> bool {
    FileType::from_raw_mode(s.st_mode) == FileType::Directory
        && s.st_uid == rustix::process::geteuid().as_raw()
        && s.st_mode & 0o7777 == mode
}
fn poll<F: Failure>(cancelled: &dyn Fn() -> bool) -> Result<(), F> {
    if cancelled() {
        Err(F::cancelled())
    } else {
        Ok(())
    }
}
fn reserve<T, F: Failure>(values: &mut Vec<T>, count: usize) -> Result<(), F> {
    values.try_reserve_exact(count).map_err(|_| F::allocation())
}
pub(crate) fn no_attributes<F: Failure>(file: &File) -> Result<(), F> {
    attributes(file).map(|_| ())
}
pub(crate) fn attributes<F: Failure>(file: &File) -> Result<Option<Vec<u8>>, F> {
    // Admit exactly one known attribute. Unknown/oversized lists never grow
    // memory, and ACL/capability/user attributes are not silently discarded.
    let mut names = [0u8; 256];
    let count = rustix::fs::flistxattr(file, names.as_mut_slice()).map_err(|e| {
        if e == Errno::RANGE {
            F::unsupported_metadata()
        } else {
            F::io("inspect extended attributes", e.into())
        }
    })?;
    if count == 0 {
        return Ok(None);
    }
    if &names[..count] != b"security.selinux\0" {
        return Err(F::unsupported_metadata());
    }
    let mut label = [0u8; 4096];
    let count =
        rustix::fs::fgetxattr(file, "security.selinux", label.as_mut_slice()).map_err(|e| {
            if e == Errno::RANGE {
                F::unsupported_metadata()
            } else {
                F::io("read SELinux label", e.into())
            }
        })?;
    let mut value = Vec::new();
    reserve(&mut value, count)?;
    value.extend_from_slice(&label[..count]);
    Ok(Some(value))
}
/// The entries of a private directory, refused past `MAX_OBJECTS`.
pub(crate) fn entries<F: Failure>(directory: &File) -> Result<usize, F> {
    let mut dir = rustix::fs::Dir::read_from(directory)
        .map_err(|e| F::io("list private directory", e.into()))?;
    let mut count = 0usize;
    while let Some(entry) = dir.read() {
        let entry = entry.map_err(|e| F::io("read private entry", e.into()))?;
        if !matches!(entry.file_name().to_bytes(), b"." | b"..") {
            count += 1;
            if count > MAX_OBJECTS as usize {
                return Err(F::changed());
            }
        }
    }
    Ok(count)
}
/// The default `cleanup_count`: the entries, an I/O failure kept, any other
/// failure (too many entries) reported as foreign objects.
pub(crate) fn cleanup_count<F: Failure>(directory: &File) -> Result<usize, CleanupFailure> {
    entries::<F>(directory).map_err(|failure| match failure.into_io() {
        Some(source) => CleanupFailure::Io(source),
        None => CleanupFailure::ForeignObjects,
    })
}

fn bound<F: Failure>(parent: &File, name: &str, identity: &Stat) -> Result<(), F> {
    let named = rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|e| F::io("stat private object name", e.into()))?;
    if !same_inode(&named, identity) {
        return Err(F::changed());
    }
    Ok(())
}
fn file_type(kind: EntryKind) -> FileType {
    match kind {
        EntryKind::File => FileType::RegularFile,
        EntryKind::Directory => FileType::Directory,
    }
}

pub(crate) struct Object<K: Kind> {
    name: String,
    pub(crate) file: File,
    identity: Stat,
    pub(crate) metadata: K::Metadata,
}
impl<K: Kind> fmt::Debug for Object<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Object")
            .field("name", &self.name)
            .field("file", &self.file)
            .field("identity", &self.identity)
            .field("metadata", &self.metadata)
            .finish()
    }
}
impl<K: Kind> Object<K> {
    pub(crate) fn check(&self, parent: &File) -> Result<(), K::Failure> {
        bound::<K::Failure>(parent, &self.name, &self.identity)?;
        let stat = rustix::fs::fstat(&self.file)
            .map_err(|e| K::Failure::io(K::LABELS.stat_object, e.into()))?;
        let kind = self.metadata.kind();
        let (uid, gid) = self.metadata.private_owner();
        if !same_inode(&stat, &self.identity)
            || FileType::from_raw_mode(stat.st_mode) != file_type(kind)
            || stat.st_uid != uid
            || stat.st_gid != gid
            || stat.st_mode & 0o7777 != self.metadata.private_mode()
            || stat.st_mtime != self.identity.st_mtime
            || stat.st_mtime_nsec != self.identity.st_mtime_nsec
            || stat.st_ctime != self.identity.st_ctime
            || stat.st_ctime_nsec != self.identity.st_ctime_nsec
            || (kind == EntryKind::File
                && (stat.st_nlink != 1
                    || stat.st_size < 0
                    || stat.st_size as u64 != self.metadata.size()))
        {
            return Err(K::Failure::changed());
        }
        if kind == EntryKind::Directory && entries::<K::Failure>(&self.file)? != 0 {
            return Err(K::Failure::changed());
        }
        no_attributes(&self.file)
    }
}

pub(crate) struct Attempt<K: Kind> {
    pub(crate) root: File,
    pub(crate) directory: File,
    pub(crate) name: String,
    identity: Stat,
    pub(crate) objects: Vec<Object<K>>,
    armed: bool,
    unverified: bool,
}
impl<K: Kind> fmt::Debug for Attempt<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Attempt")
            .field("root", &self.root)
            .field("directory", &self.directory)
            .field("name", &self.name)
            .field("identity", &self.identity)
            .field("objects", &self.objects)
            .field("armed", &self.armed)
            .field("unverified", &self.unverified)
            .finish()
    }
}
impl<K: Kind> Attempt<K> {
    /// A new exclusive attempt directory in the private `root`.
    pub(crate) fn create<I: Io<K>>(
        root: &File,
        count: usize,
        io: &mut I,
    ) -> Result<Self, K::Error> {
        let stat = rustix::fs::fstat(root)
            .map_err(|e| K::Failure::io(K::LABELS.recheck_root, e.into()))?;
        if !own_directory(&stat, 0o700) {
            return Err(K::Failure::unsafe_root().into());
        }
        let mut objects = Vec::new();
        io.reserve(&mut objects, count)?;
        let mut random = [0u8; 16];
        getrandom::getrandom(&mut random)
            .map_err(|e| K::Failure::io(K::LABELS.name, io::Error::other(e.to_string())))?;
        let mut encoded = [0u8; 32];
        hex::encode_to_slice(random, &mut encoded).map_err(|_| K::Failure::allocation())?;
        let mut name = String::new();
        name.try_reserve_exact(K::PREFIX.len() + 32)
            .map_err(|_| K::Failure::allocation())?;
        name.push_str(K::PREFIX);
        name.push_str(std::str::from_utf8(&encoded).map_err(|_| K::Failure::allocation())?);
        let root = root
            .try_clone()
            .map_err(|e| K::Failure::io(K::LABELS.retain_root, e))?;
        rustix::fs::mkdirat(&root, name.as_str(), Mode::from_raw_mode(0o700)).map_err(|e| {
            if e == Errno::EXIST {
                K::Failure::collision()
            } else {
                K::Failure::io(K::LABELS.create_attempt, e.into())
            }
        })?;
        let opened = rustix::fs::openat(
            &root,
            name.as_str(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(io::Error::from)
        .and_then(|file| Ok((io.stat_created(&file)?, file)));
        let (identity, directory) = match opened {
            Ok(opened) => opened,
            Err(source) => {
                return Err(K::error(
                    K::Failure::io(K::LABELS.bind_attempt, source),
                    Some(CleanupReport {
                        attempt_name: name,
                        failure: CleanupFailure::UnverifiedObject,
                    }),
                ));
            }
        };
        let attempt = Self {
            root,
            directory,
            name,
            identity,
            objects,
            armed: true,
            unverified: false,
        };
        let result = (|| {
            bound::<K::Failure>(&attempt.root, &attempt.name, &attempt.identity)?;
            if FileType::from_raw_mode(attempt.identity.st_mode) != FileType::Directory
                || attempt.identity.st_uid != rustix::process::geteuid().as_raw()
            {
                return Err(K::Failure::changed());
            }
            rustix::fs::fchmod(&attempt.directory, Mode::from_raw_mode(0o700))
                .map_err(|e| K::Failure::io(K::LABELS.protect_attempt, e.into()))?;
            io.checkpoint(Point::AttemptCreated, &attempt.directory)
        })();
        if let Err(failure) = result {
            return Err(attempt.fail_with_io(failure, io));
        }
        Ok(attempt)
    }

    pub(crate) fn create_object<I: Io<K>>(
        &mut self,
        mut metadata: K::Metadata,
        io: &mut I,
    ) -> Result<usize, K::Failure> {
        let index = self.objects.len();
        let mut name = String::new();
        name.try_reserve_exact(16)
            .map_err(|_| K::Failure::allocation())?;
        fmt::write(&mut name, format_args!("object-{index:08x}"))
            .map_err(|_| K::Failure::allocation())?;
        let file = if metadata.kind() == EntryKind::Directory {
            rustix::fs::mkdirat(&self.directory, name.as_str(), Mode::from_raw_mode(0o700))
                .map_err(|e| K::Failure::io("create required private directory", e.into()))?;
            self.unverified = true;
            rustix::fs::openat(
                &self.directory,
                name.as_str(),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
        } else {
            rustix::fs::openat(
                &self.directory,
                name.as_str(),
                OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )
        }
        .map(File::from)
        .map_err(|e| K::Failure::io("open created private object", e.into()))?;
        self.unverified = true;
        let identity = io
            .stat_created(&file)
            .map_err(|e| K::Failure::io("bind created private object", e))?;
        metadata.set_private_owner(identity.st_uid, identity.st_gid);
        self.objects.push(Object {
            name,
            file,
            identity,
            metadata,
        });
        self.unverified = false;
        let object = &self.objects[index];
        bound::<K::Failure>(&self.directory, &object.name, &object.identity)?;
        if object.identity.st_uid != rustix::process::geteuid().as_raw()
            || FileType::from_raw_mode(object.identity.st_mode) != file_type(object.metadata.kind())
        {
            return Err(K::Failure::changed());
        }
        io.checkpoint(Point::ObjectCreated, &self.directory)?;
        Ok(index)
    }

    pub(crate) fn seal<I: Io<K>>(
        &mut self,
        index: usize,
        io: &mut I,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), K::Failure> {
        let object = &mut self.objects[index];
        bound::<K::Failure>(&self.directory, &object.name, &object.identity)?;
        rustix::fs::fchmod(
            &object.file,
            Mode::from_raw_mode(object.metadata.private_mode()),
        )
        .map_err(|e| K::Failure::io(K::LABELS.seal_object, e.into()))?;
        let barrier = if object.metadata.kind() == EntryKind::File {
            Barrier::File
        } else {
            Barrier::Directory
        };
        io.sync(&object.file, barrier)
            .map_err(|e| K::Failure::io(K::LABELS.fsync_object, e))?;
        let flags = OFlags::RDONLY
            | OFlags::NONBLOCK
            | OFlags::NOFOLLOW
            | OFlags::CLOEXEC
            | if object.metadata.kind() == EntryKind::Directory {
                OFlags::DIRECTORY
            } else {
                OFlags::empty()
            };
        let readonly =
            rustix::fs::openat(&self.directory, object.name.as_str(), flags, Mode::empty())
                .map(File::from)
                .map_err(|e| K::Failure::io("reopen sealed object", e.into()))?;
        let stat = rustix::fs::fstat(&readonly)
            .map_err(|e| K::Failure::io("stat sealed object", e.into()))?;
        if !same_inode(&stat, &object.identity) {
            return Err(K::Failure::changed());
        }
        object.identity = stat;
        object.file = readonly;
        object.check(&self.directory)?;
        verify_file(object, &self.directory, io, cancelled)?;
        io.checkpoint(Point::Sealed, &self.directory)
    }

    pub(crate) fn verify<I: Io<K>>(
        &self,
        io: &mut I,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), K::Failure> {
        poll::<K::Failure>(cancelled)?;
        self.check()?;
        for object in &self.objects {
            object.check(&self.directory)?;
            verify_file(object, &self.directory, io, cancelled)?;
        }
        poll(cancelled)
    }
    pub(crate) fn check(&self) -> Result<(), K::Failure> {
        bound::<K::Failure>(&self.root, &self.name, &self.identity)?;
        let stat = rustix::fs::fstat(&self.directory)
            .map_err(|e| K::Failure::io("stat private attempt", e.into()))?;
        if !own_directory(&stat, 0o700)
            || entries::<K::Failure>(&self.directory)? != self.objects.len()
        {
            return Err(K::Failure::changed());
        }
        let root_stat = rustix::fs::fstat(&self.root)
            .map_err(|e| K::Failure::io("stat retained private root", e.into()))?;
        if !own_directory(&root_stat, 0o700) {
            return Err(K::Failure::changed());
        }
        no_attributes::<K::Failure>(&self.root)?;
        no_attributes(&self.directory)
    }

    pub(crate) fn cleanup(&mut self) -> Result<(), CleanupFailure> {
        self.cleanup_with_io(&mut Unseamed)
    }
    fn cleanup_with_io<I: Io<K>>(&mut self, io: &mut I) -> Result<(), CleanupFailure> {
        if !self.armed {
            return Ok(());
        }
        // Exactly one attempt: failure reporting must not be undone by Drop.
        self.armed = false;
        let named = rustix::fs::statat(&self.root, self.name.as_str(), AtFlags::SYMLINK_NOFOLLOW);
        match named {
            Ok(stat) if same_inode(&stat, &self.identity) => {}
            Err(Errno::NOENT) => return Err(CleanupFailure::ReplacedObject),
            Ok(_) => return Err(CleanupFailure::ReplacedObject),
            Err(e) => return Err(CleanupFailure::Io(e.into())),
        }
        for object in self.objects.iter().rev() {
            let stat = rustix::fs::statat(
                &self.directory,
                object.name.as_str(),
                AtFlags::SYMLINK_NOFOLLOW,
            );
            match stat {
                Ok(stat) if same_inode(&stat, &object.identity) => {
                    if object.metadata.kind() == EntryKind::Directory
                        && io.cleanup_count(&object.file)? != 0
                    {
                        return Err(CleanupFailure::ForeignObjects);
                    }
                    rustix::fs::unlinkat(
                        &self.directory,
                        object.name.as_str(),
                        if object.metadata.kind() == EntryKind::Directory {
                            AtFlags::REMOVEDIR
                        } else {
                            AtFlags::empty()
                        },
                    )
                    .map_err(|e| CleanupFailure::Io(e.into()))?;
                }
                Ok(_) => return Err(CleanupFailure::ReplacedObject),
                Err(Errno::NOENT) => {}
                Err(e) => return Err(CleanupFailure::Io(e.into())),
            }
        }
        if io.cleanup_count(&self.directory)? != 0 {
            return Err(if self.unverified {
                CleanupFailure::UnverifiedObject
            } else {
                CleanupFailure::ForeignObjects
            });
        }
        bound::<K::Failure>(&self.root, &self.name, &self.identity).map_err(
            |failure| match failure.into_io() {
                Some(source) => CleanupFailure::Io(source),
                None => CleanupFailure::ReplacedObject,
            },
        )?;
        rustix::fs::unlinkat(&self.root, self.name.as_str(), AtFlags::REMOVEDIR)
            .map_err(|e| CleanupFailure::Io(e.into()))?;
        io.sync(&self.root, Barrier::Cleanup)
            .map_err(CleanupFailure::Io)
    }
    pub(crate) fn fail_with_io<I: Io<K>>(mut self, failure: K::Failure, io: &mut I) -> K::Error {
        let cleanup = self.cleanup_with_io(io).err().map(|failure| CleanupReport {
            attempt_name: std::mem::take(&mut self.name),
            failure,
        });
        K::error(failure, cleanup)
    }
}
impl<K: Kind> Drop for Attempt<K> {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn verify_file<K: Kind, I: Io<K>>(
    object: &Object<K>,
    parent: &File,
    io: &mut I,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), K::Failure> {
    if object.metadata.kind() != EntryKind::File {
        return Ok(());
    }
    let mut chunk = Vec::new();
    reserve::<u8, K::Failure>(&mut chunk, CHUNK)?;
    chunk.resize(CHUNK, 0);
    let mut hash = Sha256::new();
    let mut offset = 0u64;
    loop {
        poll::<K::Failure>(cancelled)?;
        let n = match io.read_at(&object.file, &mut chunk, offset) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            v => v.map_err(|e| K::Failure::io("verify private copy", e))?,
        };
        if n == 0 {
            break;
        }
        offset = offset
            .checked_add(n as u64)
            .filter(|n| *n <= object.metadata.size())
            .ok_or(K::Failure::changed())?;
        hash.update(&chunk[..n]);
    }
    if offset != object.metadata.size()
        || object.metadata.sha256() != Some(hex::encode(hash.finalize()).as_str())
    {
        return Err(K::Failure::changed());
    }
    object.check(parent)
}
pub(crate) fn write_all<K: Kind, I: Io<K>>(
    io: &mut I,
    file: &mut File,
    mut input: &[u8],
    cancelled: &dyn Fn() -> bool,
) -> Result<(), K::Failure> {
    while !input.is_empty() {
        poll::<K::Failure>(cancelled)?;
        match io.write(file, input) {
            Ok(0) => {
                return Err(K::Failure::io(
                    "copy private payload",
                    io::ErrorKind::WriteZero.into(),
                ));
            }
            Ok(n) if n <= input.len() => input = &input[n..],
            Ok(_) => return Err(K::Failure::changed()),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(K::Failure::io("copy private payload", e)),
        }
    }
    Ok(())
}
