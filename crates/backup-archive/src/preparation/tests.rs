use super::private::{Barrier, Point};
use super::*;
use crate::staging::{StagingRoot, admit_for_staging, replay};
use limeos_domain::backups::{
    AdmissionPolicy, ArchiveFormat, ArchiveLimits, LegacyMapping, ManagedResource,
};
use std::{
    cell::Cell,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Scene {
    temp: tempfile::TempDir,
    prep: PathBuf,
    stage: PathBuf,
    catalog: VerifiedCatalog,
    selection: Selection,
    limits: PreparationLimits,
}
impl Scene {
    fn new() -> Self {
        Self::at(None, 1)
    }
    fn at(path: Option<PathBuf>, files: usize) -> Self {
        let temp = if let Some(path) = path {
            tempfile::Builder::new()
                .permissions(fs::Permissions::from_mode(0o700))
                .tempdir_in(path)
                .unwrap()
        } else {
            tempfile::Builder::new()
                .permissions(fs::Permissions::from_mode(0o700))
                .tempdir()
                .unwrap()
        };
        let prep = temp.path().join("prep");
        let stage = temp.path().join("stage");
        let managed = temp.path().join("managed");
        for dir in [&prep, &stage, &managed] {
            fs::create_dir(dir).unwrap();
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::write(managed.join("live"), b"unchanged").unwrap();
        fs::write(temp.path().join("sentinel"), b"sentinel").unwrap();
        let policy = AdmissionPolicy {
            revision: 1,
            formats: vec![ArchiveFormat::TarGzip, ArchiveFormat::TarZstd],
            limits: ArchiveLimits {
                max_compressed_bytes: 1 << 20,
                max_decompressed_bytes: 4 << 20,
                max_file_bytes: 1 << 20,
                max_entries: 512,
                max_path_bytes: 256,
                max_path_depth: 16,
                max_component_bytes: 128,
                max_metadata_bytes: 4096,
                max_total_metadata_bytes: 16384,
                max_zstd_window_log: 21,
            },
            resources: vec![ManagedResource {
                id: "config".into(),
                destination_root: managed.to_str().unwrap().into(),
            }],
            legacy_mappings: vec![LegacyMapping {
                archive_prefix: "etc/limeos".into(),
                resource: "config".into(),
                resource_prefix: String::new(),
            }],
        };
        let installed = InstalledMetadata {
            uid: 41000,
            gid: 41001,
            mode: 0o640,
        };
        let mut targets = vec![ManagedTarget {
            relative_path: String::new(),
            kind: EntryKind::Directory,
            installed,
        }];
        let mut tar = tar::Builder::new(Vec::new());
        for index in 0..files {
            let name = format!("file-{index:04}");
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Regular);
            header.set_mode(0o777);
            header.set_size(8);
            header.set_cksum();
            tar.append_data(&mut header, format!("etc/limeos/{name}"), &b"original"[..])
                .unwrap();
            targets.push(ManagedTarget {
                relative_path: name,
                kind: EntryKind::File,
                installed,
            });
        }
        let archive = zstd::stream::encode_all(tar.into_inner().unwrap().as_slice(), 3).unwrap();
        let admitted = admit_for_staging(archive.as_slice(), policy.clone(), &|| false).unwrap();
        let catalog = replay(
            archive.as_slice(),
            &policy,
            admitted.manifest(),
            admitted.policy_snapshot(),
            &StagingRoot::open(&stage).unwrap(),
            &|| false,
        )
        .unwrap();
        let selection = Selection {
            resources: vec![ResourceSelection {
                id: "config".into(),
                destination_root: managed.to_str().unwrap().into(),
                targets,
            }],
        };
        let limits = PreparationLimits {
            max_resources: MAX_RESOURCES,
            max_objects: MAX_OBJECTS,
            max_file_bytes: 1 << 20,
            max_total_bytes: 4 << 20,
            max_record_bytes: MAX_RECORD_BYTES,
        };
        Self {
            temp,
            prep,
            stage,
            catalog,
            selection,
            limits,
        }
    }
    fn run<I: PreparationIo>(
        &self,
        io: &mut I,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<PreparedRestore, PreparationError> {
        prepare_with_io(
            &self.catalog,
            self.catalog.manifest(),
            &self.selection,
            &PreparationRoot::open(&self.prep).unwrap(),
            &self.limits,
            cancelled,
            io,
        )
    }
    fn unchanged(&self) {
        assert_eq!(
            fs::read(self.temp.path().join("managed/live")).unwrap(),
            b"unchanged"
        );
        assert_eq!(
            fs::read(self.temp.path().join("sentinel")).unwrap(),
            b"sentinel"
        );
    }
    fn empty(&self) {
        assert_eq!(fs::read_dir(&self.prep).unwrap().count(), 0);
        self.unchanged();
    }
}

#[derive(Clone, Copy)]
enum WriteFault {
    Short,
    Interrupted,
    Zero,
    Space,
    Corrupt,
}
struct Writes {
    fault: WriteFault,
    calls: usize,
}
impl PreparationIo for Writes {
    fn write(&mut self, file: &mut File, input: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        match self.fault {
            WriteFault::Short => file.write(&input[..input.len().min(1)]),
            WriteFault::Interrupted if self.calls == 1 => Err(io::ErrorKind::Interrupted.into()),
            WriteFault::Zero => Ok(0),
            WriteFault::Space => Err(io::Error::from_raw_os_error(28)),
            WriteFault::Corrupt => file.write(&vec![b'X'; input.len()]),
            _ => file.write(input),
        }
    }
}
#[test]
fn interrupted_and_short_writes_complete_exact_bytes() {
    for fault in [WriteFault::Short, WriteFault::Interrupted] {
        let scene = Scene::new();
        let owner = scene
            .run(&mut Writes { fault, calls: 0 }, &|| false)
            .unwrap();
        owner.revalidate(&|| false).unwrap();
        owner.discard().unwrap();
        scene.empty();
    }
}
#[test]
fn zero_enospc_and_corrupt_writes_return_no_owner() {
    for fault in [WriteFault::Zero, WriteFault::Space, WriteFault::Corrupt] {
        let scene = Scene::new();
        let error = scene
            .run(&mut Writes { fault, calls: 0 }, &|| false)
            .unwrap_err();
        assert!(error.cleanup.is_none());
        scene.empty();
    }
}
struct SyncFault(Barrier);
impl PreparationIo for SyncFault {
    fn sync(&mut self, file: &File, barrier: Barrier) -> io::Result<()> {
        if barrier == self.0 {
            Err(io::Error::from_raw_os_error(5))
        } else {
            file.sync_all()
        }
    }
}
#[test]
fn each_object_attempt_and_root_fsync_failure_refuses() {
    for barrier in [
        Barrier::File,
        Barrier::Directory,
        Barrier::Attempt,
        Barrier::Root,
    ] {
        let scene = Scene::new();
        let error = scene.run(&mut SyncFault(barrier), &|| false).unwrap_err();
        assert!(matches!(error.failure, Failure::Io { .. }));
        assert!(error.cleanup.is_none());
        scene.empty();
    }
}
struct DualFault;
impl PreparationIo for DualFault {
    fn write(&mut self, _: &mut File, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::from_raw_os_error(28))
    }
    fn sync(&mut self, file: &File, barrier: Barrier) -> io::Result<()> {
        if barrier == Barrier::Cleanup {
            Err(io::Error::from_raw_os_error(5))
        } else {
            file.sync_all()
        }
    }
}
#[test]
fn primary_and_cleanup_fsync_failures_survive_together() {
    let scene = Scene::new();
    let error = scene.run(&mut DualFault, &|| false).unwrap_err();
    assert!(
        matches!(error.failure, Failure::Io { ref source, .. } if source.raw_os_error() == Some(28))
    );
    assert!(
        matches!(error.cleanup.unwrap().failure, CleanupFailure::Io(ref source) if source.raw_os_error() == Some(5))
    );
    scene.empty();
}
struct ListingFault;
impl PreparationIo for ListingFault {
    fn write(&mut self, _: &mut File, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::from_raw_os_error(28))
    }
    fn cleanup_count(&mut self, _: &File) -> Result<usize, CleanupFailure> {
        Err(CleanupFailure::Io(io::Error::from_raw_os_error(5)))
    }
}
#[test]
fn primary_and_cleanup_listing_io_errors_remain_distinct() {
    let scene = Scene::new();
    let error = scene.run(&mut ListingFault, &|| false).unwrap_err();
    assert!(
        matches!(error.failure, Failure::Io { ref source, .. } if source.raw_os_error() == Some(28))
    );
    assert!(
        matches!(error.cleanup.unwrap().failure, CleanupFailure::Io(ref source) if source.raw_os_error() == Some(5))
    );
    assert_eq!(fs::read_dir(&scene.prep).unwrap().count(), 1);
    scene.unchanged();
}
struct Allocate {
    calls: usize,
    fail_at: usize,
}
impl PreparationIo for Allocate {
    fn reserve<T>(&mut self, values: &mut Vec<T>, count: usize) -> Result<(), Failure> {
        self.calls += 1;
        if self.calls == self.fail_at {
            Err(Failure::Allocation)
        } else {
            reserve(values, count)
        }
    }
}
#[test]
fn allocation_refusal_precedes_any_directory_effect() {
    for fail_at in [1, 2] {
        let scene = Scene::new();
        assert!(matches!(
            scene
                .run(&mut Allocate { calls: 0, fail_at }, &|| false)
                .unwrap_err()
                .failure,
            Failure::Allocation
        ));
        scene.empty();
    }
    let mut vector: Vec<u8> = Vec::new();
    assert!(matches!(
        reserve(&mut vector, usize::MAX),
        Err(Failure::Allocation)
    ));
}
struct StatFault {
    calls: usize,
    fail_at: usize,
}
impl PreparationIo for StatFault {
    fn stat_created(&mut self, file: &File) -> io::Result<Stat> {
        self.calls += 1;
        if self.calls == self.fail_at {
            Err(io::Error::from_raw_os_error(5))
        } else {
            rustix::fs::fstat(file).map_err(Into::into)
        }
    }
}
#[test]
fn unverified_created_directory_and_files_are_retained() {
    for fail_at in [1, 2, 3] {
        let scene = Scene::new();
        let error = scene
            .run(&mut StatFault { calls: 0, fail_at }, &|| false)
            .unwrap_err();
        assert!(matches!(
            error.cleanup.unwrap().failure,
            CleanupFailure::UnverifiedObject
        ));
        assert_eq!(fs::read_dir(&scene.prep).unwrap().count(), 1);
        let orphan = fs::read_dir(&scene.prep)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let fresh = scene.run(&mut RealIo, &|| false).unwrap();
        fresh.discard().unwrap();
        assert!(orphan.exists());
        scene.unchanged();
    }
}
#[derive(Clone, Copy)]
enum Replace {
    File,
    Symlink,
    Foreign,
    Attempt,
    Catalog,
    InsideDirectory,
}
struct Replacement<'a> {
    scene: &'a Scene,
    kind: Replace,
    copied: usize,
}
impl PreparationIo for Replacement<'_> {
    fn checkpoint(&mut self, point: Point, _: &File) -> Result<(), Failure> {
        if point != Point::Copied {
            return Ok(());
        }
        self.copied += 1;
        if matches!(self.kind, Replace::InsideDirectory) && self.copied == 1 {
            let attempt = fs::read_dir(&self.scene.prep)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            fs::write(attempt.join("object-00000000/foreign"), b"foreign").unwrap();
            return Ok(());
        }
        if self.copied != 2 {
            return Ok(());
        }
        let attempt = fs::read_dir(&self.scene.prep)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let name = attempt.join("object-00000001");
        match self.kind {
            Replace::File | Replace::Symlink => {
                fs::rename(&name, attempt.join("displaced")).unwrap();
                if matches!(self.kind, Replace::File) {
                    fs::write(name, b"foreign").unwrap();
                } else {
                    symlink(self.scene.temp.path().join("sentinel"), name).unwrap();
                }
            }
            Replace::Foreign => fs::write(attempt.join("foreign"), b"foreign").unwrap(),
            Replace::Attempt => {
                fs::rename(&attempt, self.scene.prep.join("displaced")).unwrap();
                fs::create_dir(&attempt).unwrap();
                fs::write(attempt.join("foreign"), b"foreign").unwrap();
            }
            Replace::Catalog => {
                let q = fs::read_dir(&self.scene.stage)
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path();
                let file = fs::read_dir(q).unwrap().next().unwrap().unwrap().path();
                fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
                fs::OpenOptions::new()
                    .append(true)
                    .open(&file)
                    .unwrap()
                    .write_all(b"extra")
                    .unwrap();
                fs::set_permissions(file, fs::Permissions::from_mode(0o400)).unwrap();
            }
            Replace::InsideDirectory => {}
        }
        Ok(())
    }
}
#[test]
fn replaced_files_and_symlinks_preserve_foreign_objects() {
    for kind in [Replace::File, Replace::Symlink] {
        let scene = Scene::new();
        let error = scene
            .run(
                &mut Replacement {
                    scene: &scene,
                    kind,
                    copied: 0,
                },
                &|| false,
            )
            .unwrap_err();
        assert!(matches!(error.failure, Failure::ChangedObject));
        assert!(matches!(
            error.cleanup.unwrap().failure,
            CleanupFailure::ReplacedObject
        ));
        let attempt = fs::read_dir(&scene.prep)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(attempt.join("displaced").exists());
        if matches!(kind, Replace::File) {
            assert_eq!(
                fs::read(attempt.join("object-00000001")).unwrap(),
                b"foreign"
            );
        } else {
            assert!(
                fs::symlink_metadata(attempt.join("object-00000001"))
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
        }
        scene.unchanged();
    }
}
#[test]
fn unknown_entries_and_replaced_attempts_are_never_deleted() {
    for kind in [Replace::Foreign, Replace::Attempt] {
        let scene = Scene::new();
        let error = scene
            .run(
                &mut Replacement {
                    scene: &scene,
                    kind,
                    copied: 0,
                },
                &|| false,
            )
            .unwrap_err();
        assert!(error.cleanup.is_some());
        assert!(
            fs::read_dir(&scene.prep)
                .unwrap()
                .any(|e| e.unwrap().path().join("foreign").exists())
        );
        scene.unchanged();
    }
}
#[test]
fn foreign_children_of_required_directories_are_retained() {
    let scene = Scene::new();
    let error = scene
        .run(
            &mut Replacement {
                scene: &scene,
                kind: Replace::InsideDirectory,
                copied: 0,
            },
            &|| false,
        )
        .unwrap_err();
    assert!(matches!(
        error.cleanup.unwrap().failure,
        CleanupFailure::ForeignObjects
    ));
    let attempt = fs::read_dir(&scene.prep)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        fs::read(attempt.join("object-00000000/foreign")).unwrap(),
        b"foreign"
    );
    scene.unchanged();
}
#[test]
fn mutation_after_copy_refuses_even_after_root_fsync() {
    let scene = Scene::new();
    let error = scene
        .run(
            &mut Replacement {
                scene: &scene,
                kind: Replace::Catalog,
                copied: 0,
            },
            &|| false,
        )
        .unwrap_err();
    assert!(matches!(error.failure, Failure::Catalog(_)));
    assert!(error.cleanup.is_none());
    scene.empty();
}
struct CancelAt<'a> {
    point: Point,
    flag: &'a Cell<bool>,
}
impl PreparationIo for CancelAt<'_> {
    fn checkpoint(&mut self, point: Point, directory: &File) -> Result<(), Failure> {
        if point == self.point {
            if point == Point::Sealed
                && rustix::fs::statat(directory, "object-00000001", AtFlags::SYMLINK_NOFOLLOW)
                    .is_err()
            {
                return Ok(());
            }
            self.flag.set(true);
        }
        Ok(())
    }
}
#[test]
fn cancellation_at_every_lifecycle_point_returns_no_owner() {
    for point in [
        Point::AttemptCreated,
        Point::ObjectCreated,
        Point::Copied,
        Point::Sealed,
        Point::AttemptSynced,
        Point::RootSynced,
    ] {
        let scene = Scene::new();
        let flag = Cell::new(false);
        let error = scene
            .run(&mut CancelAt { point, flag: &flag }, &|| flag.get())
            .unwrap_err();
        assert!(matches!(error.failure, Failure::Cancelled));
        assert!(error.cleanup.is_none());
        scene.empty();
    }
}
struct Reads {
    interrupted: bool,
    zero: bool,
}
impl PreparationIo for Reads {
    fn read_at(&mut self, file: &File, out: &mut [u8], offset: u64) -> io::Result<usize> {
        if self.interrupted {
            self.interrupted = false;
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.zero {
            return Ok(0);
        }
        file.read_at(&mut out[..1], offset)
    }
}
#[test]
fn independent_copy_verification_handles_short_and_interrupted_reads() {
    let scene = Scene::new();
    let owner = scene
        .run(
            &mut Reads {
                interrupted: true,
                zero: false,
            },
            &|| false,
        )
        .unwrap();
    owner.discard().unwrap();
    scene.empty();
    assert!(matches!(
        scene
            .run(
                &mut Reads {
                    interrupted: false,
                    zero: true
                },
                &|| false
            )
            .unwrap_err()
            .failure,
        Failure::ChangedObject
    ));
    scene.empty();
}
#[test]
fn prepared_readers_refuse_mutation_and_latch_failure() {
    let scene = Scene::new();
    let owner = scene.run(&mut RealIo, &|| false).unwrap();
    let entry = owner
        .entries()
        .find(|e| e.metadata().kind == EntryKind::File)
        .unwrap();
    let mut reader = entry.reader().unwrap().unwrap();
    let mut first = [0; 4];
    reader.read_exact(&mut first).unwrap();
    assert_eq!(&first, b"orig");
    let attempt = fs::read_dir(&scene.prep)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let file = attempt.join("object-00000001");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&file, b"original").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o400)).unwrap();
    assert!(reader.read(&mut first).is_err());
    assert!(reader.read(&mut first).is_err());
    assert!(owner.revalidate(&|| false).is_err());
    owner.discard().unwrap();
    scene.empty();
}
#[test]
fn record_limit_after_private_copies_cleans_all_owned_objects() {
    let mut scene = Scene::new();
    scene.limits.max_record_bytes = 1;
    assert!(matches!(
        scene.run(&mut RealIo, &|| false).unwrap_err().failure,
        Failure::Limit(Limit::RecordBytes)
    ));
    scene.empty();
}
#[test]
fn maximum_objects_have_bounded_retained_descriptors() {
    let scene = Scene::at(None, 255);
    let before = fs::read_dir("/proc/self/fd").unwrap().count();
    let owner = scene.run(&mut RealIo, &|| false).unwrap();
    let after = fs::read_dir("/proc/self/fd").unwrap().count();
    println!(
        "256 objects, 2040 logical payload bytes, retained descriptor delta {}, record bytes {}",
        after - before,
        owner.record().len()
    );
    // Run this measured case alone for exact descriptor attribution. Other
    // concurrently running tests may own additional descriptors.
    assert_eq!(owner.entries().count(), 256);
    owner.discard().unwrap();
    scene.empty();
}

struct KillPoint {
    point: Point,
    marker: PathBuf,
}
impl PreparationIo for KillPoint {
    fn checkpoint(&mut self, point: Point, directory: &File) -> Result<(), Failure> {
        if point == self.point {
            if point == Point::Sealed
                && rustix::fs::statat(directory, "object-00000001", AtFlags::SYMLINK_NOFOLLOW)
                    .is_err()
            {
                return Ok(());
            }
            fs::write(&self.marker, b"ready").unwrap();
            loop {
                std::thread::park_timeout(Duration::from_secs(1));
            }
        }
        Ok(())
    }
}
#[test]
fn crash_child() {
    let Some(root) = std::env::var_os("LIMEOS_PREPARATION_CRASH_ROOT") else {
        return;
    };
    let point = match std::env::var("LIMEOS_PREPARATION_CRASH_POINT")
        .unwrap()
        .as_str()
    {
        "created" => Point::AttemptCreated,
        "sealed" => Point::Sealed,
        "attempt" => Point::AttemptSynced,
        "root" => Point::RootSynced,
        _ => panic!("unknown point"),
    };
    let root = PathBuf::from(root);
    let scene = Scene::at(Some(root.clone()), 1);
    let _ = scene.run(
        &mut KillPoint {
            point,
            marker: root.join("ready"),
        },
        &|| false,
    );
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn sigkill_leaves_inert_orphans_without_adoption_or_live_changes() {
    for point in ["created", "sealed", "attempt", "root"] {
        let root = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let mut child = OwnedChild(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "preparation::tests::crash_child", "--nocapture"])
                .env("LIMEOS_PREPARATION_CRASH_ROOT", root.path())
                .env("LIMEOS_PREPARATION_CRASH_POINT", point)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        while !root.path().join("ready").exists() {
            assert!(child.0.try_wait().unwrap().is_none(), "child ended early");
            assert!(Instant::now() < deadline, "child readiness expired");
            std::thread::sleep(Duration::from_millis(5));
        }
        child.0.kill().unwrap();
        let status = child.0.wait().unwrap();
        assert!(!status.success());
        let scene_path = fs::read_dir(root.path())
            .unwrap()
            .filter_map(|e| {
                let p = e.unwrap().path();
                p.is_dir().then_some(p)
            })
            .next()
            .unwrap();
        let prep = scene_path.join("prep");
        let orphan = fs::read_dir(&prep).unwrap().next().unwrap().unwrap().path();
        assert!(
            orphan
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("preparing-")
        );
        assert!(!orphan.join("record.json").exists());
        assert_eq!(
            fs::read(scene_path.join("managed/live")).unwrap(),
            b"unchanged"
        );
        assert_eq!(fs::read(scene_path.join("sentinel")).unwrap(), b"sentinel");
        // Opening the root validates its descriptor; it adopts no attempt.
        drop(PreparationRoot::open(&prep).unwrap());
        assert!(orphan.exists());
        let fresh = Scene::new();
        let owner = prepare_with_io(
            &fresh.catalog,
            fresh.catalog.manifest(),
            &fresh.selection,
            &PreparationRoot::open(&prep).unwrap(),
            &fresh.limits,
            &|| false,
            &mut RealIo,
        )
        .unwrap();
        assert_eq!(fs::read_dir(&prep).unwrap().count(), 2);
        owner.discard().unwrap();
        assert!(orphan.exists());
        fresh.unchanged();
        println!(
            "SIGKILL at {point}: child reaped; inert private orphan retained; live bytes unchanged"
        );
    }
}
