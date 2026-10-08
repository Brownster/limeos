use super::*;
use rustix::io::Errno;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Barrier {
    File,
    Directory,
    Attempt,
    Root,
    Cleanup,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Point {
    AttemptCreated,
    ObjectCreated,
    Copied,
    Sealed,
    AttemptSynced,
    RootSynced,
}

pub(super) trait PreparationIo {
    fn cleanup_count(&mut self, directory: &File) -> Result<usize, CleanupFailure> {
        entries(directory).map_err(|failure| match failure {
            Failure::Io { source, .. } => CleanupFailure::Io(source),
            _ => CleanupFailure::ForeignObjects,
        })
    }
    fn write(&mut self, file: &mut File, input: &[u8]) -> io::Result<usize> {
        file.write(input)
    }
    fn read_at(&mut self, file: &File, out: &mut [u8], offset: u64) -> io::Result<usize> {
        file.read_at(out, offset)
    }
    fn sync(&mut self, file: &File, _: Barrier) -> io::Result<()> {
        file.sync_all()
    }
    fn stat_created(&mut self, file: &File) -> io::Result<Stat> {
        rustix::fs::fstat(file).map_err(Into::into)
    }
    fn reserve<T>(&mut self, values: &mut Vec<T>, count: usize) -> Result<(), Failure> {
        reserve(values, count)
    }
    fn checkpoint(&mut self, _: Point, _: &File) -> Result<(), Failure> {
        Ok(())
    }
}
pub(super) struct RealIo;
impl PreparationIo for RealIo {}

fn bound(parent: &File, name: &str, identity: &Stat) -> Result<(), Failure> {
    let named = rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|e| io_failure("stat private object name", e))?;
    if !same_inode(&named, identity) {
        return Err(Failure::ChangedObject);
    }
    Ok(())
}

#[derive(Debug)]
pub(super) struct Object {
    name: String,
    pub(super) file: File,
    identity: Stat,
    pub(super) metadata: PreparedMetadata,
}
impl Object {
    pub(super) fn check(&self, parent: &File) -> Result<(), Failure> {
        bound(parent, &self.name, &self.identity)?;
        let stat =
            rustix::fs::fstat(&self.file).map_err(|e| io_failure("stat prepared object", e))?;
        let kind = match self.metadata.kind {
            EntryKind::File => FileType::RegularFile,
            EntryKind::Directory => FileType::Directory,
        };
        if !same_inode(&stat, &self.identity)
            || FileType::from_raw_mode(stat.st_mode) != kind
            || stat.st_uid != self.metadata.private_uid
            || stat.st_gid != self.metadata.private_gid
            || stat.st_mode & 0o7777 != self.metadata.private_mode
            || stat.st_mtime != self.identity.st_mtime
            || stat.st_mtime_nsec != self.identity.st_mtime_nsec
            || stat.st_ctime != self.identity.st_ctime
            || stat.st_ctime_nsec != self.identity.st_ctime_nsec
            || (self.metadata.kind == EntryKind::File
                && (stat.st_nlink != 1
                    || stat.st_size < 0
                    || stat.st_size as u64 != self.metadata.size))
        {
            return Err(Failure::ChangedObject);
        }
        if self.metadata.kind == EntryKind::Directory && entries(&self.file)? != 0 {
            return Err(Failure::ChangedObject);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(super) struct Attempt {
    pub(super) root: File,
    pub(super) directory: File,
    pub(super) name: String,
    identity: Stat,
    pub(super) objects: Vec<Object>,
    armed: bool,
    unverified: bool,
}
impl Attempt {
    pub(super) fn create<I: PreparationIo>(
        root: &PreparationRoot,
        count: usize,
        io: &mut I,
    ) -> Result<Self, PreparationError> {
        let stat = rustix::fs::fstat(&root.directory)
            .map_err(|e| io_failure("recheck preparation root", e))?;
        if !own_directory(&stat, 0o700) {
            return Err(Failure::UnsafeRoot.into());
        }
        let mut objects = Vec::new();
        io.reserve(&mut objects, count)?;
        let mut random = [0u8; 16];
        getrandom::getrandom(&mut random).map_err(|e| {
            io_failure("generate preparation name", io::Error::other(e.to_string()))
        })?;
        let mut encoded = [0u8; 32];
        hex::encode_to_slice(random, &mut encoded).map_err(|_| Failure::Allocation)?;
        let mut name = string("preparing-")?;
        name.try_reserve_exact(32)
            .map_err(|_| Failure::Allocation)?;
        name.push_str(std::str::from_utf8(&encoded).map_err(|_| Failure::Allocation)?);
        let root = root
            .directory
            .try_clone()
            .map_err(|e| io_failure("retain preparation root", e))?;
        rustix::fs::mkdirat(&root, name.as_str(), Mode::from_raw_mode(0o700)).map_err(|e| {
            if e == Errno::EXIST {
                Failure::Collision
            } else {
                io_failure("create preparation attempt", e)
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
                return Err(PreparationError {
                    failure: io_failure("bind created preparation attempt", source),
                    cleanup: Some(CleanupReport {
                        attempt_name: name,
                        failure: CleanupFailure::UnverifiedObject,
                    }),
                });
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
            bound(&attempt.root, &attempt.name, &attempt.identity)?;
            if FileType::from_raw_mode(attempt.identity.st_mode) != FileType::Directory
                || attempt.identity.st_uid != rustix::process::geteuid().as_raw()
            {
                return Err(Failure::ChangedObject);
            }
            rustix::fs::fchmod(&attempt.directory, Mode::from_raw_mode(0o700))
                .map_err(|e| io_failure("protect preparation attempt", e))?;
            io.checkpoint(Point::AttemptCreated, &attempt.directory)
        })();
        if let Err(failure) = result {
            return Err(attempt.fail_with_io(failure, io));
        }
        Ok(attempt)
    }

    pub(super) fn create_object<I: PreparationIo>(
        &mut self,
        mut metadata: PreparedMetadata,
        io: &mut I,
    ) -> Result<usize, Failure> {
        let index = self.objects.len();
        let mut name = String::new();
        name.try_reserve_exact(16)
            .map_err(|_| Failure::Allocation)?;
        fmt::write(&mut name, format_args!("object-{index:08x}"))
            .map_err(|_| Failure::Allocation)?;
        let file = if metadata.kind == EntryKind::Directory {
            rustix::fs::mkdirat(&self.directory, name.as_str(), Mode::from_raw_mode(0o700))
                .map_err(|e| io_failure("create required private directory", e))?;
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
        .map_err(|e| io_failure("open created private object", e))?;
        self.unverified = true;
        let identity = io
            .stat_created(&file)
            .map_err(|e| io_failure("bind created private object", e))?;
        metadata.private_uid = identity.st_uid;
        metadata.private_gid = identity.st_gid;
        self.objects.push(Object {
            name,
            file,
            identity,
            metadata,
        });
        self.unverified = false;
        let object = &self.objects[index];
        bound(&self.directory, &object.name, &object.identity)?;
        if object.identity.st_uid != rustix::process::geteuid().as_raw()
            || FileType::from_raw_mode(object.identity.st_mode)
                != match object.metadata.kind {
                    EntryKind::File => FileType::RegularFile,
                    EntryKind::Directory => FileType::Directory,
                }
        {
            return Err(Failure::ChangedObject);
        }
        io.checkpoint(Point::ObjectCreated, &self.directory)?;
        Ok(index)
    }

    pub(super) fn seal<I: PreparationIo>(
        &mut self,
        index: usize,
        io: &mut I,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), Failure> {
        let object = &mut self.objects[index];
        bound(&self.directory, &object.name, &object.identity)?;
        rustix::fs::fchmod(
            &object.file,
            Mode::from_raw_mode(object.metadata.private_mode),
        )
        .map_err(|e| io_failure("seal prepared object", e))?;
        let barrier = if object.metadata.kind == EntryKind::File {
            Barrier::File
        } else {
            Barrier::Directory
        };
        io.sync(&object.file, barrier)
            .map_err(|e| io_failure("fsync prepared object", e))?;
        let flags = OFlags::RDONLY
            | OFlags::NOFOLLOW
            | OFlags::CLOEXEC
            | if object.metadata.kind == EntryKind::Directory {
                OFlags::DIRECTORY
            } else {
                OFlags::empty()
            };
        let readonly =
            rustix::fs::openat(&self.directory, object.name.as_str(), flags, Mode::empty())
                .map(File::from)
                .map_err(|e| io_failure("reopen sealed object", e))?;
        let stat = rustix::fs::fstat(&readonly).map_err(|e| io_failure("stat sealed object", e))?;
        if !same_inode(&stat, &object.identity) {
            return Err(Failure::ChangedObject);
        }
        object.identity = stat;
        object.file = readonly;
        object.check(&self.directory)?;
        verify_file(object, &self.directory, io, cancelled)?;
        io.checkpoint(Point::Sealed, &self.directory)
    }

    pub(super) fn verify<I: PreparationIo>(
        &self,
        io: &mut I,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), Failure> {
        poll(cancelled)?;
        self.check()?;
        for object in &self.objects {
            object.check(&self.directory)?;
            verify_file(object, &self.directory, io, cancelled)?;
        }
        poll(cancelled)
    }
    pub(super) fn check(&self) -> Result<(), Failure> {
        bound(&self.root, &self.name, &self.identity)?;
        let stat = rustix::fs::fstat(&self.directory)
            .map_err(|e| io_failure("stat private attempt", e))?;
        if !own_directory(&stat, 0o700) || entries(&self.directory)? != self.objects.len() {
            return Err(Failure::ChangedObject);
        }
        let root_stat = rustix::fs::fstat(&self.root)
            .map_err(|e| io_failure("stat retained private root", e))?;
        if !own_directory(&root_stat, 0o700) {
            return Err(Failure::ChangedObject);
        }
        Ok(())
    }

    pub(super) fn cleanup(&mut self) -> Result<(), CleanupFailure> {
        self.cleanup_with_io(&mut RealIo)
    }
    fn cleanup_with_io<I: PreparationIo>(&mut self, io: &mut I) -> Result<(), CleanupFailure> {
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
                    if object.metadata.kind == EntryKind::Directory
                        && io.cleanup_count(&object.file)? != 0
                    {
                        return Err(CleanupFailure::ForeignObjects);
                    }
                    rustix::fs::unlinkat(
                        &self.directory,
                        object.name.as_str(),
                        if object.metadata.kind == EntryKind::Directory {
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
        bound(&self.root, &self.name, &self.identity).map_err(|failure| match failure {
            Failure::Io { source, .. } => CleanupFailure::Io(source),
            _ => CleanupFailure::ReplacedObject,
        })?;
        rustix::fs::unlinkat(&self.root, self.name.as_str(), AtFlags::REMOVEDIR)
            .map_err(|e| CleanupFailure::Io(e.into()))?;
        io.sync(&self.root, Barrier::Cleanup)
            .map_err(CleanupFailure::Io)
    }
    pub(super) fn fail_with_io<I: PreparationIo>(
        mut self,
        failure: Failure,
        io: &mut I,
    ) -> PreparationError {
        let cleanup = self.cleanup_with_io(io).err().map(|failure| CleanupReport {
            attempt_name: std::mem::take(&mut self.name),
            failure,
        });
        PreparationError { failure, cleanup }
    }
}
impl Drop for Attempt {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn entries(directory: &File) -> Result<usize, Failure> {
    let mut dir = rustix::fs::Dir::read_from(directory)
        .map_err(|e| io_failure("list private directory", e))?;
    let mut count = 0usize;
    while let Some(entry) = dir.read() {
        let entry = entry.map_err(|e| io_failure("read private entry", e))?;
        if !matches!(entry.file_name().to_bytes(), b"." | b"..") {
            count += 1;
            if count > MAX_OBJECTS as usize {
                return Err(Failure::ChangedObject);
            }
        }
    }
    Ok(count)
}

fn verify_file<I: PreparationIo>(
    object: &Object,
    parent: &File,
    io: &mut I,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), Failure> {
    if object.metadata.kind != EntryKind::File {
        return Ok(());
    }
    let mut chunk = bytes(CHUNK)?;
    let mut hash = Sha256::new();
    let mut offset = 0u64;
    loop {
        poll(cancelled)?;
        let n = match io.read_at(&object.file, &mut chunk, offset) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            v => v.map_err(|e| io_failure("verify private copy", e))?,
        };
        if n == 0 {
            break;
        }
        offset = offset
            .checked_add(n as u64)
            .filter(|n| *n <= object.metadata.size)
            .ok_or(Failure::ChangedObject)?;
        hash.update(&chunk[..n]);
    }
    if offset != object.metadata.size
        || object.metadata.sha256.as_deref() != Some(hex::encode(hash.finalize()).as_str())
    {
        return Err(Failure::ChangedObject);
    }
    object.check(parent)
}
pub(super) fn write_all<I: PreparationIo>(
    io: &mut I,
    file: &mut File,
    mut input: &[u8],
    cancelled: &dyn Fn() -> bool,
) -> Result<(), Failure> {
    while !input.is_empty() {
        poll(cancelled)?;
        match io.write(file, input) {
            Ok(0) => return Err(io_failure("copy private payload", io::ErrorKind::WriteZero)),
            Ok(n) if n <= input.len() => input = &input[n..],
            Ok(_) => return Err(Failure::ChangedObject),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(io_failure("copy private payload", e)),
        }
    }
    Ok(())
}
