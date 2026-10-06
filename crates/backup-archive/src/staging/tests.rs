use super::*;
use limeos_domain::backups::{ArchiveFormat, ArchiveLimits, LegacyMapping, ManagedResource};
use std::os::unix::fs::{PermissionsExt, symlink};

fn private_tempdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

fn policy() -> AdmissionPolicy {
    AdmissionPolicy {
        revision: 4,
        formats: vec![ArchiveFormat::TarGzip, ArchiveFormat::TarZstd],
        limits: ArchiveLimits {
            max_compressed_bytes: 1024 * 1024,
            max_decompressed_bytes: 4 * 1024 * 1024,
            max_file_bytes: 1024 * 1024,
            max_entries: 64,
            max_path_bytes: 256,
            max_path_depth: 16,
            max_component_bytes: 128,
            max_metadata_bytes: 4096,
            max_total_metadata_bytes: 16384,
            max_zstd_window_log: 21,
        },
        resources: vec![ManagedResource {
            id: "config".into(),
            destination_root: "/synthetic/config".into(),
        }],
        legacy_mappings: vec![LegacyMapping {
            archive_prefix: "etc/limeos".into(),
            resource: "config".into(),
            resource_prefix: String::new(),
        }],
    }
}

fn archive() -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut tar = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_size(3);
    header.set_mode(0o7777);
    header.set_uid(12345);
    header.set_gid(12345);
    header.set_cksum();
    tar.append_data(&mut header, "etc/limeos/Case", &b"abc"[..])
        .unwrap();
    tar.into_inner().unwrap().finish().unwrap()
}

fn empty(path: &Path) -> bool {
    std::fs::read_dir(path).unwrap().next().is_none()
}

#[test]
fn partial_write_and_each_durability_failure_are_typed_and_cleaned() {
    struct Fault {
        write: bool,
        sync_at: usize,
        syncs: usize,
    }
    impl FileIo for Fault {
        fn write(&mut self, file: &mut File, data: &[u8]) -> io::Result<usize> {
            if self.write {
                file.write_all(&data[..1])?;
                return Err(rustix::io::Errno::NOSPC.into());
            }
            file.write(data)
        }
        fn sync(&mut self, file: &File) -> io::Result<()> {
            self.syncs += 1;
            if self.syncs == self.sync_at {
                Err(rustix::io::Errno::IO.into())
            } else {
                file.sync_all()
            }
        }
    }
    let bytes = archive();
    let policy = policy();
    let admitted = admit_for_staging(&bytes[..], policy.clone(), &|| false).unwrap();
    for (write, sync_at) in [(true, 0), (false, 1), (false, 2), (false, 3)] {
        let temp = private_tempdir();
        let root = StagingRoot::open(temp.path()).unwrap();
        let mut fault = Fault {
            write,
            sync_at,
            syncs: 0,
        };
        assert!(matches!(
            replay_with_io(
                &bytes[..],
                &policy,
                admitted.manifest(),
                admitted.policy_snapshot(),
                &root,
                &|| false,
                &mut fault
            ),
            Err(StagingError::Io { .. })
        ));
        assert!(empty(temp.path()));
    }
}

#[test]
fn independent_size_and_hash_verification_refuse_a_lying_or_corrupt_writer() {
    struct BadWriter {
        omit: bool,
    }
    impl FileIo for BadWriter {
        fn write(&mut self, file: &mut File, bytes: &[u8]) -> io::Result<usize> {
            if !self.omit {
                file.write_all(&vec![b'x'; bytes.len()])?;
            }
            Ok(bytes.len())
        }
    }
    let bytes = archive();
    let policy = policy();
    let admitted = admit_for_staging(&bytes[..], policy.clone(), &|| false).unwrap();
    for omit in [true, false] {
        let temp = private_tempdir();
        let root = StagingRoot::open(temp.path()).unwrap();
        assert!(matches!(
            replay_with_io(
                &bytes[..],
                &policy,
                admitted.manifest(),
                admitted.policy_snapshot(),
                &root,
                &|| false,
                &mut BadWriter { omit }
            ),
            Err(StagingError::ManifestMismatch)
        ));
        assert!(empty(temp.path()));
    }
}

#[test]
fn allocation_and_total_file_byte_quotas_fail_before_creating_files() {
    let temp = private_tempdir();
    let root = StagingRoot::open(temp.path()).unwrap();
    assert!(matches!(
        Quarantine::create(&root, u64::MAX),
        Err(StagingError::Allocation)
    ));
    assert!(empty(temp.path()));
    assert_eq!(
        buffer(usize::MAX, None).unwrap_err().codes(),
        [FindingCode::Allocation]
    );
    let mut quarantine = Quarantine::create(&root, 1).unwrap();
    let mut io = RealIo;
    let mut sink = QuarantineSink {
        quarantine: &mut quarantine,
        io: &mut io,
        cancelled: &|| false,
        max_files: 1,
        max_bytes: 3,
        declared: 0,
        current: None,
    };
    assert!(matches!(
        sink.begin_file(0, "not-a-host-name", 4),
        Err(StagingError::Quota)
    ));
    assert!(sink.quarantine.files.is_empty());
    sink.declared = 0;
    sink.begin_file(0, "metadata-only", 3).unwrap();
    sink.chunk(b"abc").unwrap();
    sink.end_file().unwrap();
    assert!(matches!(
        sink.begin_file(1, "other-metadata", 0),
        Err(StagingError::Quota)
    ));
    drop(quarantine);
    assert!(empty(temp.path()));
}

#[test]
fn existing_attempt_and_file_symlink_attacks_never_overwrite_or_delete_foreign_files() {
    let temp = private_tempdir();
    let root = StagingRoot::open(temp.path()).unwrap();
    let existing = temp.path().join("incomplete-fixture");
    std::fs::create_dir(&existing).unwrap();
    std::fs::write(existing.join("sentinel"), b"foreign directory").unwrap();
    assert!(Quarantine::create_named(&root, "incomplete-fixture".into(), Vec::new()).is_err());
    assert_eq!(
        std::fs::read(existing.join("sentinel")).unwrap(),
        b"foreign directory"
    );
    let outside = temp.path().join("foreign-file");
    std::fs::write(&outside, b"foreign bytes").unwrap();
    let mut quarantine = Quarantine::create(&root, 1).unwrap();
    symlink(
        &outside,
        temp.path()
            .join(&quarantine.name)
            .join("file-0000000000000000"),
    )
    .unwrap();
    let mut io = RealIo;
    let mut sink = QuarantineSink {
        quarantine: &mut quarantine,
        io: &mut io,
        cancelled: &|| false,
        max_files: 1,
        max_bytes: 3,
        declared: 0,
        current: None,
    };
    assert!(matches!(
        sink.begin_file(0, "etc/limeos/Case", 3),
        Err(StagingError::Io { .. })
    ));
    // A foreign entry prevents empty-directory removal. Cleanup refuses to
    // recurse into it, reports failure and leaves it private/incomplete.
    assert!(quarantine.cleanup().is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), b"foreign bytes");
    drop(quarantine);
    assert_eq!(std::fs::read(&outside).unwrap(), b"foreign bytes");
}

#[test]
fn cleanup_refuses_a_replaced_attempt_name_and_keeps_foreign_directory() {
    let temp = private_tempdir();
    let root = StagingRoot::open(temp.path()).unwrap();
    let mut quarantine = Quarantine::create(&root, 0).unwrap();
    let old = temp.path().join(&quarantine.name);
    std::fs::rename(&old, temp.path().join("moved-attempt")).unwrap();
    std::fs::create_dir(&old).unwrap();
    std::fs::write(old.join("sentinel"), b"foreign").unwrap();
    assert!(quarantine.cleanup().is_err());
    drop(quarantine);
    assert_eq!(std::fs::read(old.join("sentinel")).unwrap(), b"foreign");
}

#[test]
fn input_io_failure_is_not_misclassified_as_archive_syntax() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
    }
    let bytes = archive();
    let policy = policy();
    let admitted = admit_for_staging(&bytes[..], policy.clone(), &|| false).unwrap();
    let temp = private_tempdir();
    let root = StagingRoot::open(temp.path()).unwrap();
    match replay(
        Broken,
        &policy,
        admitted.manifest(),
        admitted.policy_snapshot(),
        &root,
        &|| false,
    ) {
        Err(StagingError::Admission(r)) => assert_eq!(r.codes(), [FindingCode::Io]),
        result => panic!("expected typed input I/O refusal, got {result:?}"),
    }
    assert!(empty(temp.path()));
}

#[test]
fn short_interrupted_zero_writes_and_cancellation_before_completion() {
    use std::cell::Cell;
    struct Writes<'a> {
        calls: usize,
        syncs: usize,
        zero: bool,
        cancel_on_sync: bool,
        cancelled: &'a Cell<bool>,
    }
    impl FileIo for Writes<'_> {
        fn write(&mut self, file: &mut File, bytes: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            if self.zero {
                return Ok(0);
            }
            if self.calls % 2 == 1 {
                return Err(io::ErrorKind::Interrupted.into());
            }
            file.write(&bytes[..1])
        }
        fn sync(&mut self, file: &File) -> io::Result<()> {
            file.sync_all()?;
            self.syncs += 1;
            if self.cancel_on_sync && self.syncs == 3 {
                self.cancelled.set(true);
            }
            Ok(())
        }
    }
    let bytes = archive();
    let policy = policy();
    let admitted = admit_for_staging(&bytes[..], policy.clone(), &|| false).unwrap();
    for (zero, cancel_on_sync) in [(false, false), (true, false), (false, true)] {
        let temp = private_tempdir();
        let root = StagingRoot::open(temp.path()).unwrap();
        let cancelled = Cell::new(false);
        let mut io = Writes {
            calls: 0,
            syncs: 0,
            zero,
            cancel_on_sync,
            cancelled: &cancelled,
        };
        let result = replay_with_io(
            &bytes[..],
            &policy,
            admitted.manifest(),
            admitted.policy_snapshot(),
            &root,
            &|| cancelled.get(),
            &mut io,
        );
        if zero {
            assert!(matches!(result, Err(StagingError::Io { source, .. })
                if source.kind() == io::ErrorKind::WriteZero));
        } else if cancel_on_sync {
            assert!(matches!(result, Err(StagingError::Cancelled)));
        } else {
            let catalog = result.unwrap();
            let entry = catalog.entries().next().unwrap();
            let mut contents = Vec::new();
            entry.reader().unwrap().read_to_end(&mut contents).unwrap();
            assert_eq!(contents, b"abc");
            catalog.discard().unwrap();
        }
        assert!(empty(temp.path()));
    }
}
