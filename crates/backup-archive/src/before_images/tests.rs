use super::*;
use std::{
    cell::Cell,
    fs::{self, OpenOptions},
    io::{Seek, SeekFrom},
    os::unix::fs::{PermissionsExt, symlink},
    process::{Child, Command},
    time::{Duration, Instant},
};
use tempfile::TempDir;

#[path = "tests/preflight.rs"]
mod preflight;

struct Fixture {
    tree: TempDir,
    parent: File,
    file: File,
    directory: File,
    root: BeforeImageRoot,
    expected: Vec<u8>,
}
impl Fixture {
    fn new(size: usize) -> Self {
        let tree = tempfile::tempdir().unwrap();
        fs::create_dir(tree.path().join("managed")).unwrap();
        fs::create_dir(tree.path().join("private")).unwrap();
        fs::set_permissions(
            tree.path().join("private"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let expected: Vec<_> = (0..size).map(|i| ((i * 17) % 251) as u8).collect();
        fs::write(tree.path().join("managed/config"), &expected).unwrap();
        fs::set_permissions(
            tree.path().join("managed/config"),
            fs::Permissions::from_mode(0o640),
        )
        .unwrap();
        fs::create_dir(tree.path().join("managed/subdir")).unwrap();
        fs::write(
            tree.path().join("managed/subdir/unselected"),
            b"never recursively copied",
        )
        .unwrap();
        let parent = File::open(tree.path().join("managed")).unwrap();
        let file = File::open(tree.path().join("managed/config")).unwrap();
        let directory = File::open(tree.path().join("managed/subdir")).unwrap();
        let root = BeforeImageRoot::open(&tree.path().join("private")).unwrap();
        Self {
            tree,
            parent,
            file,
            directory,
            root,
            expected,
        }
    }
    fn sources(&self) -> [Source<'_>; 2] {
        [
            Source {
                resource: "application",
                relative_path: "config",
                name: "config",
                parent: &self.parent,
                object: &self.file,
            },
            Source {
                resource: "application",
                relative_path: "subdir",
                name: "subdir",
                parent: &self.parent,
                object: &self.directory,
            },
        ]
    }
    fn private(&self) -> std::path::PathBuf {
        self.tree.path().join("private")
    }
    fn attempt(&self) -> std::path::PathBuf {
        fs::read_dir(self.private())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path()
    }
    fn assert_clean(&self) {
        assert_eq!(fs::read_dir(self.private()).unwrap().count(), 0);
        assert_eq!(
            fs::read(self.tree.path().join("managed/config")).unwrap(),
            self.expected
        );
        assert_eq!(
            fs::read(self.tree.path().join("managed/subdir/unselected")).unwrap(),
            b"never recursively copied"
        );
    }
}
fn limits() -> CaptureLimits {
    CaptureLimits {
        max_objects: 8,
        max_file_bytes: 1024 * 1024,
        max_total_bytes: 2 * 1024 * 1024,
        max_record_bytes: 65536,
    }
}

#[test]
fn exact_bytes_metadata_offsets_and_independent_owner() {
    for size in [0, 1, CHUNK - 1, CHUNK, CHUNK + 1] {
        let f = Fixture::new(size);
        let mut offset = &f.file;
        offset.seek(SeekFrom::Start(1)).unwrap();
        let source_stat = rustix::fs::fstat(&f.file).unwrap();
        let owner = capture(&f.sources(), &f.root, limits(), &|| false).unwrap();
        assert_eq!(offset.stream_position().unwrap(), 1);
        assert_eq!(owner.entries().count(), 2);
        let entry = owner.entries().next().unwrap();
        let metadata = entry.metadata();
        assert_eq!(metadata.source.mode, 0o640);
        assert_eq!(metadata.source.uid, source_stat.st_uid);
        assert_eq!(metadata.source.gid, source_stat.st_gid);
        assert_eq!(metadata.source.inode, source_stat.st_ino);
        assert_eq!(metadata.size, size as u64);
        assert_eq!(
            metadata.sha256,
            Some(hex::encode(Sha256::digest(&f.expected)))
        );
        assert_eq!(metadata.private_mode, 0o400);
        assert_eq!(metadata.private_uid, rustix::process::geteuid().as_raw());
        let mut copied = Vec::new();
        entry
            .reader()
            .unwrap()
            .unwrap()
            .read_to_end(&mut copied)
            .unwrap();
        assert_eq!(copied, f.expected);
        assert!(owner.entries().nth(1).unwrap().reader().unwrap().is_none());
        let canonical: serde_json::Value = serde_json::from_slice(owner.record()).unwrap();
        assert_eq!(canonical["objects"].as_array().unwrap().len(), 2);
        assert!(!String::from_utf8_lossy(owner.record()).contains("before-images-"));
        let again = capture(&f.sources(), &f.root, limits(), &|| false).unwrap();
        assert_eq!(owner.record(), again.record());
        assert_eq!(owner.record_sha256(), again.record_sha256());
        again.discard().unwrap();
        // Sources can change after capture without redirecting private copies.
        fs::write(
            f.tree.path().join("managed/config"),
            b"different current target",
        )
        .unwrap();
        owner.revalidate(&|| false).unwrap();
        owner.discard().unwrap();
        assert_eq!(fs::read_dir(f.private()).unwrap().count(), 0);
    }
}

#[test]
fn selection_and_limits_refuse_before_owned_creation() {
    let f = Fixture::new(13);
    for invalid in ["", "../escape", "a/b", "a\nsecret"] {
        let mut sources = f.sources();
        sources[0].resource = invalid;
        assert!(matches!(
            capture(&sources, &f.root, limits(), &|| false)
                .unwrap_err()
                .failure,
            Failure::InvalidSelection
        ));
    }
    for path in ["../config", "/config", "config/", "a//config", "wrong", ""] {
        let mut sources = f.sources();
        sources[0].relative_path = path;
        assert!(matches!(
            capture(&sources, &f.root, limits(), &|| false)
                .unwrap_err()
                .failure,
            Failure::InvalidSelection
        ));
    }
    let duplicate = [
        f.sources().into_iter().next().unwrap(),
        f.sources().into_iter().next().unwrap(),
    ];
    assert!(capture(&duplicate, &f.root, limits(), &|| false).is_err());
    let mut sources = f.sources();
    sources[1].relative_path = "config/subdir";
    assert!(matches!(
        capture(&sources, &f.root, limits(), &|| false)
            .unwrap_err()
            .failure,
        Failure::InvalidSelection
    ));
    assert!(capture(&[], &f.root, limits(), &|| false).is_err());
    for changed in [
        CaptureLimits {
            max_objects: 0,
            ..limits()
        },
        CaptureLimits {
            max_objects: MAX_OBJECTS + 1,
            ..limits()
        },
        CaptureLimits {
            max_file_bytes: 0,
            ..limits()
        },
        CaptureLimits {
            max_total_bytes: u64::MAX,
            ..limits()
        },
        CaptureLimits {
            max_record_bytes: MAX_RECORD_BYTES + 1,
            ..limits()
        },
    ] {
        assert!(matches!(
            capture(&f.sources(), &f.root, changed, &|| false)
                .unwrap_err()
                .failure,
            Failure::InvalidLimits
        ));
    }
    for (changed, expected) in [
        (
            CaptureLimits {
                max_objects: 1,
                ..limits()
            },
            Limit::Objects,
        ),
        (
            CaptureLimits {
                max_file_bytes: 12,
                ..limits()
            },
            Limit::FileBytes,
        ),
        (
            CaptureLimits {
                max_total_bytes: 12,
                ..limits()
            },
            Limit::TotalBytes,
        ),
    ] {
        assert!(
            matches!(capture(&f.sources(), &f.root, changed, &|| false).unwrap_err().failure, Failure::Limit(l) if l == expected)
        );
    }
    f.assert_clean();
}

#[test]
fn writable_opath_special_hardlinked_and_extended_metadata_refuse() {
    let f = Fixture::new(13);
    let writable = OpenOptions::new()
        .read(true)
        .write(true)
        .open(f.tree.path().join("managed/config"))
        .unwrap();
    let path_only = File::from(
        rustix::fs::openat(
            &f.parent,
            "config",
            OFlags::PATH | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap(),
    );
    for file in [&writable, &path_only] {
        let mut sources = f.sources();
        sources[0].object = file;
        assert!(matches!(
            capture(&sources, &f.root, limits(), &|| false)
                .unwrap_err()
                .failure,
            Failure::WritableDescriptor
        ));
    }
    fs::hard_link(
        f.tree.path().join("managed/config"),
        f.tree.path().join("managed/alias"),
    )
    .unwrap();
    assert!(matches!(
        capture(&f.sources(), &f.root, limits(), &|| false)
            .unwrap_err()
            .failure,
        Failure::UnsupportedSource
    ));
    fs::remove_file(f.tree.path().join("managed/alias")).unwrap();
    fs::set_permissions(
        f.tree.path().join("managed/config"),
        fs::Permissions::from_mode(0o4640),
    )
    .unwrap();
    assert!(matches!(
        capture(&f.sources(), &f.root, limits(), &|| false)
            .unwrap_err()
            .failure,
        Failure::UnsupportedMetadata
    ));
    fs::set_permissions(
        f.tree.path().join("managed/config"),
        fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    rustix::fs::fsetxattr(
        &writable,
        "user.before-image-regression",
        b"must not be lost",
        rustix::fs::XattrFlags::empty(),
    )
    .unwrap();
    assert!(matches!(
        capture(&f.sources(), &f.root, limits(), &|| false)
            .unwrap_err()
            .failure,
        Failure::UnsupportedMetadata
    ));
    rustix::fs::fremovexattr(&writable, "user.before-image-regression").unwrap();
    rustix::fs::mknodat(
        &f.parent,
        "pipe",
        FileType::Fifo,
        Mode::from_raw_mode(0o600),
        0,
    )
    .unwrap();
    let fifo = File::from(
        rustix::fs::openat(
            &f.parent,
            "pipe",
            OFlags::RDONLY | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .unwrap(),
    );
    let sources = [Source {
        resource: "app",
        relative_path: "pipe",
        parent: &f.parent,
        name: "pipe",
        object: &fifo,
    }];
    assert!(matches!(
        capture(&sources, &f.root, limits(), &|| false)
            .unwrap_err()
            .failure,
        Failure::UnsupportedSource
    ));
    f.assert_clean();
}

#[test]
fn changed_or_symlink_name_and_private_root_overlap_refuse() {
    let f = Fixture::new(13);
    fs::rename(
        f.tree.path().join("managed/config"),
        f.tree.path().join("managed/retained"),
    )
    .unwrap();
    fs::write(f.tree.path().join("managed/config"), &f.expected).unwrap();
    assert!(matches!(
        capture(&f.sources(), &f.root, limits(), &|| false)
            .unwrap_err()
            .failure,
        Failure::ChangedSource
    ));
    fs::remove_file(f.tree.path().join("managed/config")).unwrap();
    symlink("retained", f.tree.path().join("managed/config")).unwrap();
    assert!(matches!(
        capture(&f.sources(), &f.root, limits(), &|| false)
            .unwrap_err()
            .failure,
        Failure::ChangedSource
    ));
    let private = File::open(f.private()).unwrap();
    let outer = File::open(f.tree.path()).unwrap();
    let sources = [Source {
        resource: "app",
        relative_path: "private",
        parent: &outer,
        name: "private",
        object: &private,
    }];
    assert!(matches!(
        capture(&sources, &f.root, limits(), &|| false)
            .unwrap_err()
            .failure,
        Failure::InvalidSelection
    ));
    assert_eq!(fs::read_dir(f.private()).unwrap().count(), 0);
}

#[test]
fn protected_root_and_retained_rename() {
    let f = Fixture::new(13);
    let alias = f.tree.path().join("alias");
    symlink(f.private(), &alias).unwrap();
    assert!(matches!(
        BeforeImageRoot::open(&alias).unwrap_err().failure,
        Failure::UnsafeRoot
    ));
    assert!(BeforeImageRoot::open(&f.tree.path().join("managed/config")).is_err());
    fs::set_permissions(f.private(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(BeforeImageRoot::open(&f.private()).is_err());
    fs::set_permissions(f.private(), fs::Permissions::from_mode(0o700)).unwrap();
    rustix::fs::fsetxattr(
        &f.root.directory,
        "user.private-policy",
        b"unknown",
        rustix::fs::XattrFlags::empty(),
    )
    .unwrap();
    assert!(BeforeImageRoot::open(&f.private()).is_err());
    rustix::fs::fremovexattr(&f.root.directory, "user.private-policy").unwrap();
    let moved = f.tree.path().join("held-private");
    fs::rename(f.private(), &moved).unwrap();
    fs::create_dir(f.private()).unwrap();
    fs::set_permissions(f.private(), fs::Permissions::from_mode(0o700)).unwrap();
    let owner = capture(&f.sources(), &f.root, limits(), &|| false).unwrap();
    assert_eq!(fs::read_dir(f.private()).unwrap().count(), 0);
    assert_eq!(fs::read_dir(&moved).unwrap().count(), 1);
    owner.discard().unwrap();
    assert_eq!(fs::read_dir(&moved).unwrap().count(), 0);
}

struct Fault {
    write: Option<io::ErrorKind>,
    errno: Option<i32>,
    zero: bool,
    short: bool,
    interrupted: bool,
    corrupt: bool,
    sync: Option<Barrier>,
    cleanup: bool,
    created_stat: Option<usize>,
    stat_count: usize,
    mutate: Option<(Point, std::path::PathBuf, Vec<u8>)>,
    cancel: Option<(Point, Cell<bool>)>,
}
impl Fault {
    fn new() -> Self {
        Self {
            write: None,
            errno: None,
            zero: false,
            short: false,
            interrupted: false,
            corrupt: false,
            sync: None,
            cleanup: false,
            created_stat: None,
            stat_count: 0,
            mutate: None,
            cancel: None,
        }
    }
}
impl CaptureIo for Fault {
    fn write(&mut self, file: &mut File, input: &[u8]) -> io::Result<usize> {
        if self.interrupted {
            self.interrupted = false;
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.zero {
            return Ok(0);
        }
        if let Some(errno) = self.errno {
            return Err(io::Error::from_raw_os_error(errno));
        }
        if let Some(kind) = self.write {
            return Err(kind.into());
        }
        if self.corrupt {
            return file.write(&vec![0; input.len()]);
        }
        file.write(if self.short {
            &input[..input.len().min(3)]
        } else {
            input
        })
    }
    fn sync(&mut self, file: &File, barrier: Barrier) -> io::Result<()> {
        if self.sync == Some(barrier) || (self.cleanup && barrier == Barrier::Cleanup) {
            return Err(io::Error::from_raw_os_error(5));
        }
        file.sync_all()
    }
    fn stat_created(&mut self, file: &File) -> io::Result<Stat> {
        self.stat_count += 1;
        if self.created_stat == Some(self.stat_count) {
            return Err(io::Error::from_raw_os_error(5));
        }
        rustix::fs::fstat(file).map_err(Into::into)
    }
    fn checkpoint(&mut self, point: Point, _: &File) -> Result<(), Failure> {
        if self.mutate.as_ref().is_some_and(|(at, _, _)| *at == point) {
            let (_, path, bytes) = self.mutate.take().unwrap();
            fs::write(path, bytes).unwrap();
        }
        if let Some((at, cancelled)) = &self.cancel {
            if *at == point {
                cancelled.set(true);
            }
        }
        Ok(())
    }
}

#[test]
fn interrupted_and_short_writes_produce_complete_independent_copy() {
    let f = Fixture::new(CHUNK + 1);
    let mut fault = Fault::new();
    fault.short = true;
    fault.interrupted = true;
    let owner = capture_with_io(&f.sources(), &f.root, limits(), &|| false, &mut fault).unwrap();
    let entry = owner.entries().next().unwrap();
    let mut out = Vec::new();
    entry
        .reader()
        .unwrap()
        .unwrap()
        .read_to_end(&mut out)
        .unwrap();
    assert_eq!(out, f.expected);
    owner.discard().unwrap();
    f.assert_clean();
}

#[test]
fn write_zero_enospc_corruption_and_every_sync_barrier_refuse() {
    let f = Fixture::new(17);
    for choice in 0..7 {
        let mut fault = Fault::new();
        match choice {
            0 => fault.zero = true,
            1 => fault.errno = Some(28),
            2 => fault.corrupt = true,
            3 => fault.sync = Some(Barrier::File),
            4 => fault.sync = Some(Barrier::Directory),
            5 => fault.sync = Some(Barrier::Attempt),
            _ => fault.sync = Some(Barrier::Root),
        }
        let error =
            capture_with_io(&f.sources(), &f.root, limits(), &|| false, &mut fault).unwrap_err();
        assert!(error.cleanup.is_none(), "{error:?}");
        f.assert_clean();
    }
    assert!(matches!(
        capture(
            &f.sources(),
            &f.root,
            CaptureLimits {
                max_record_bytes: 1,
                ..limits()
            },
            &|| false
        )
        .unwrap_err()
        .failure,
        Failure::Limit(Limit::RecordBytes)
    ));
    f.assert_clean();
}

#[test]
fn primary_errno_and_cleanup_errno_survive_without_drop_retry() {
    let f = Fixture::new(17);
    let mut fault = Fault::new();
    fault.errno = Some(28);
    fault.cleanup = true;
    let error =
        capture_with_io(&f.sources(), &f.root, limits(), &|| false, &mut fault).unwrap_err();
    assert!(
        matches!(error.failure, Failure::Io { source, .. } if source.raw_os_error() == Some(28))
    );
    assert!(
        matches!(error.cleanup.unwrap().failure, CleanupFailure::Io(source) if source.raw_os_error() == Some(5))
    );
    f.assert_clean();
}

#[test]
fn unverified_created_attempt_or_object_is_retained() {
    for count in [1, 2] {
        let f = Fixture::new(17);
        let mut fault = Fault::new();
        fault.created_stat = Some(count);
        let error =
            capture_with_io(&f.sources(), &f.root, limits(), &|| false, &mut fault).unwrap_err();
        assert!(matches!(
            error.cleanup.unwrap().failure,
            CleanupFailure::UnverifiedObject
        ));
        assert_eq!(fs::read_dir(f.private()).unwrap().count(), 1);
        if count == 2 {
            assert_eq!(fs::read_dir(f.attempt()).unwrap().count(), 1);
        }
    }
}

#[test]
fn late_source_mutation_append_and_truncation_refuse() {
    for bytes in [vec![5; 17], vec![5; 18], vec![5; 16]] {
        let f = Fixture::new(17);
        let mut fault = Fault::new();
        fault.mutate = Some((
            Point::RootSynced,
            f.tree.path().join("managed/config"),
            bytes,
        ));
        assert!(matches!(
            capture_with_io(&f.sources(), &f.root, limits(), &|| false, &mut fault)
                .unwrap_err()
                .failure,
            Failure::ChangedSource
        ));
        assert_eq!(fs::read_dir(f.private()).unwrap().count(), 0);
    }
}

#[test]
fn late_source_metadata_name_and_directory_changes_refuse() {
    struct Change<'a> {
        fixture: &'a Fixture,
        choice: u8,
    }
    impl CaptureIo for Change<'_> {
        fn checkpoint(&mut self, point: Point, _: &File) -> Result<(), Failure> {
            if point == Point::RootSynced {
                let path = self.fixture.tree.path().join("managed/config");
                match self.choice {
                    0 => fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap(),
                    1 => {
                        fs::rename(&path, self.fixture.tree.path().join("managed/old")).unwrap();
                        fs::write(&path, &self.fixture.expected).unwrap();
                    }
                    2 => {
                        let writable = OpenOptions::new().write(true).open(path).unwrap();
                        rustix::fs::fsetxattr(
                            &writable,
                            "user.late",
                            b"changed",
                            rustix::fs::XattrFlags::empty(),
                        )
                        .unwrap();
                    }
                    _ => fs::write(
                        self.fixture.tree.path().join("managed/subdir/new"),
                        b"late child",
                    )
                    .unwrap(),
                }
            }
            Ok(())
        }
    }
    for choice in 0..4 {
        let f = Fixture::new(17);
        let mut hook = Change {
            fixture: &f,
            choice,
        };
        assert!(matches!(
            capture_with_io(&f.sources(), &f.root, limits(), &|| false, &mut hook)
                .unwrap_err()
                .failure,
            Failure::ChangedSource
        ));
        assert_eq!(fs::read_dir(f.private()).unwrap().count(), 0);
    }
}

#[test]
fn short_interrupted_and_failed_reads_and_precreation_allocation_refusal() {
    struct ReadIo {
        short: bool,
        interrupt: bool,
        failure: Option<io::ErrorKind>,
    }
    impl CaptureIo for ReadIo {
        fn read_at(&mut self, file: &File, out: &mut [u8], offset: u64) -> io::Result<usize> {
            if self.interrupt {
                self.interrupt = false;
                return Err(io::ErrorKind::Interrupted.into());
            }
            if let Some(failure) = self.failure {
                return Err(failure.into());
            }
            let count = if self.short {
                out.len().min(3)
            } else {
                out.len()
            };
            file.read_at(&mut out[..count], offset)
        }
    }
    let f = Fixture::new(17);
    let mut hook = ReadIo {
        short: true,
        interrupt: true,
        failure: None,
    };
    capture_with_io(&f.sources(), &f.root, limits(), &|| false, &mut hook)
        .unwrap()
        .discard()
        .unwrap();
    let mut hook = ReadIo {
        short: false,
        interrupt: false,
        failure: Some(io::ErrorKind::Other),
    };
    assert!(matches!(
        capture_with_io(&f.sources(), &f.root, limits(), &|| false, &mut hook)
            .unwrap_err()
            .failure,
        Failure::Io { .. }
    ));
    struct NoAllocation;
    impl CaptureIo for NoAllocation {
        fn reserve<T>(&mut self, _: &mut Vec<T>, _: usize) -> Result<(), Failure> {
            Err(Failure::Allocation)
        }
    }
    assert!(matches!(
        capture_with_io(
            &f.sources(),
            &f.root,
            limits(),
            &|| false,
            &mut NoAllocation
        )
        .unwrap_err()
        .failure,
        Failure::Allocation
    ));
    f.assert_clean();
}

#[test]
fn cancellation_at_each_lifecycle_point_cleans_owned_objects() {
    let f = Fixture::new(17);
    assert!(matches!(
        capture(&f.sources(), &f.root, limits(), &|| true)
            .unwrap_err()
            .failure,
        Failure::Cancelled
    ));
    for point in [
        Point::AttemptCreated,
        Point::ObjectCreated,
        Point::Copied,
        Point::Sealed,
        Point::AttemptSynced,
        Point::RootSynced,
    ] {
        let mut fault = Fault::new();
        fault.cancel = Some((point, Cell::new(false)));
        // Keep the cancellation flag separate from the mutably borrowed I/O hook.
        struct CancelIo<'a> {
            point: Point,
            flag: &'a Cell<bool>,
        }
        impl CaptureIo for CancelIo<'_> {
            fn checkpoint(&mut self, point: Point, _: &File) -> Result<(), Failure> {
                if point == self.point {
                    self.flag.set(true);
                }
                Ok(())
            }
        }
        let flag = Cell::new(false);
        let mut hook = CancelIo { point, flag: &flag };
        assert!(matches!(
            capture_with_io(&f.sources(), &f.root, limits(), &|| flag.get(), &mut hook)
                .unwrap_err()
                .failure,
            Failure::Cancelled
        ));
        f.assert_clean();
    }
}

#[test]
fn foreign_and_replaced_private_objects_are_preserved() {
    for choice in 0..4 {
        let f = Fixture::new(17);
        let owner = capture(&f.sources(), &f.root, limits(), &|| false).unwrap();
        let attempt = f.attempt();
        let sentinel = match choice {
            0 => {
                let path = attempt.join("foreign");
                fs::write(&path, b"foreign").unwrap();
                path
            }
            1 => {
                let path = attempt.join("object-00000000");
                fs::rename(&path, attempt.join("held")).unwrap();
                fs::write(&path, b"foreign").unwrap();
                path
            }
            2 => {
                let path = attempt.join("object-00000001/foreign");
                fs::set_permissions(
                    attempt.join("object-00000001"),
                    fs::Permissions::from_mode(0o700),
                )
                .unwrap();
                fs::write(&path, b"foreign").unwrap();
                path
            }
            _ => {
                let moved = f.private().join("held");
                fs::rename(&attempt, &moved).unwrap();
                fs::create_dir(&attempt).unwrap();
                let path = attempt.join("foreign");
                fs::write(&path, b"foreign").unwrap();
                path
            }
        };
        assert!(owner.revalidate(&|| false).is_err());
        let error = owner.discard().unwrap_err();
        assert!(error.cleanup.is_some());
        assert_eq!(fs::read(sentinel).unwrap(), b"foreign");
        assert!(attempt.exists());
    }
}

#[test]
fn private_reader_mutation_latches_and_revalidate_cancellation_refuses() {
    let f = Fixture::new(17);
    let owner = capture(&f.sources(), &f.root, limits(), &|| false).unwrap();
    assert!(matches!(
        owner.revalidate(&|| true).unwrap_err().failure,
        Failure::Cancelled
    ));
    let entry = owner.entries().next().unwrap();
    let mut reader = entry.reader().unwrap().unwrap();
    let mut first = [0; 1];
    reader.read_exact(&mut first).unwrap();
    let path = f.attempt().join("object-00000000");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&path, &f.expected).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
    assert!(reader.read(&mut first).is_err());
    assert!(reader.read(&mut first).is_err());
    assert!(owner.revalidate(&|| false).is_err());
    owner.discard().unwrap();
    f.assert_clean();
}

#[test]
fn maximum_object_fixture_reports_actual_descriptor_and_record_cost() {
    // /proc/self/fd counts the whole process, including parallel test owners.
    // Keep exact retention and release assertions in an owned isolated process.
    const FLAG: &str = "LIMEOS_BEFORE_IMAGE_FD_PROBE";
    if std::env::var_os(FLAG).is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "before_images::tests::maximum_object_fixture_reports_actual_descriptor_and_record_cost",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(FLAG, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        print!("{}", String::from_utf8_lossy(&output.stdout));
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        return;
    }
    let f = Fixture::new(17);
    let mut names = Vec::new();
    let mut files = Vec::new();
    for i in 0..MAX_OBJECTS {
        let name = format!("small-{i}");
        fs::write(f.tree.path().join("managed").join(&name), b"abcdefgh").unwrap();
        files.push(File::open(f.tree.path().join("managed").join(&name)).unwrap());
        names.push(name);
    }
    let sources: Vec<_> = names
        .iter()
        .zip(&files)
        .map(|(name, object)| Source {
            resource: "many",
            relative_path: name,
            parent: &f.parent,
            name,
            object,
        })
        .collect();
    let descriptors = || fs::read_dir("/proc/self/fd").unwrap().count();
    let before = descriptors();
    let planned = plan(
        &sources,
        &f.root,
        CaptureLimits {
            max_objects: MAX_OBJECTS,
            max_record_bytes: MAX_RECORD_BYTES,
            ..limits()
        },
        &|| false,
    )
    .unwrap();
    assert_eq!(descriptors(), before);
    let requirements = planned.requirements();
    assert_eq!(requirements.objects, MAX_OBJECTS);
    assert_eq!(requirements.retained_descriptors, MAX_OBJECTS + 2);
    assert!(requirements.retained_descriptors + requirements.transient_descriptors > 256);
    let owner = capture_planned(planned, &|| false).unwrap();
    let additional = descriptors() - before;
    assert_eq!(additional, requirements.retained_descriptors as usize);
    eprintln!(
        "before-image fixture: objects={}, logical_bytes={}, added_retained_fds={}, record_bytes={}",
        MAX_OBJECTS,
        MAX_OBJECTS * 8,
        additional,
        owner.record().len()
    );
    owner.discard().unwrap();
    assert_eq!(descriptors(), before);
    f.assert_clean();
}

#[test]
fn crash_child() {
    let Ok(base) = std::env::var("LIMEOS_BEFORE_IMAGE_CHILD_ROOT") else {
        return;
    };
    let point = std::env::var("LIMEOS_BEFORE_IMAGE_CHILD_POINT").unwrap();
    let base = std::path::Path::new(&base);
    let parent = File::open(base.join("managed")).unwrap();
    let file = File::open(base.join("managed/config")).unwrap();
    let root = BeforeImageRoot::open(&base.join("private")).unwrap();
    struct CrashIo {
        point: String,
        marker: std::path::PathBuf,
    }
    impl CaptureIo for CrashIo {
        fn checkpoint(&mut self, point: Point, _: &File) -> Result<(), Failure> {
            if format!("{point:?}") == self.point {
                fs::write(&self.marker, b"barrier").unwrap();
                loop {
                    std::thread::park_timeout(Duration::from_millis(100));
                }
            }
            Ok(())
        }
    }
    let source = [Source {
        resource: "app",
        relative_path: "config",
        parent: &parent,
        name: "config",
        object: &file,
    }];
    let mut hook = CrashIo {
        point,
        marker: base.join("barrier"),
    };
    let _owner = capture_with_io(&source, &root, limits(), &|| false, &mut hook).unwrap();
    panic!("crash checkpoint not reached");
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn sigkill_leaves_inert_orphan_and_fresh_capture_never_adopts_it() {
    use std::os::unix::process::ExitStatusExt;
    for point in ["AttemptCreated", "Sealed", "AttemptSynced", "RootSynced"] {
        let f = Fixture::new(17);
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "before_images::tests::crash_child",
                "--nocapture",
            ])
            .env("LIMEOS_BEFORE_IMAGE_CHILD_ROOT", f.tree.path())
            .env("LIMEOS_BEFORE_IMAGE_CHILD_POINT", point)
            .spawn()
            .unwrap();
        let mut child = OwnedChild(child);
        let start = Instant::now();
        while !f.tree.path().join("barrier").exists() {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "checkpoint timeout {point}"
            );
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "child exited before {point}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let orphan = f.attempt();
        let inode = rustix::fs::stat(&orphan).unwrap().st_ino;
        child.0.kill().unwrap();
        assert_eq!(child.0.wait().unwrap().signal(), Some(9));
        let owner = capture(&f.sources(), &f.root, limits(), &|| false).unwrap();
        assert_eq!(fs::read_dir(f.private()).unwrap().count(), 2);
        assert_eq!(rustix::fs::stat(&orphan).unwrap().st_ino, inode);
        assert_eq!(
            fs::read_dir(&orphan)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|e| e.file_name().to_string_lossy().contains("record"))
                .count(),
            0
        );
        owner.discard().unwrap();
        assert_eq!(fs::read_dir(f.private()).unwrap().count(), 1);
        assert_eq!(
            fs::read(f.tree.path().join("managed/config")).unwrap(),
            f.expected
        );
        eprintln!(
            "before-image SIGKILL {point}: child reaped; orphan inode retained; fresh owner discarded"
        );
    }
}
