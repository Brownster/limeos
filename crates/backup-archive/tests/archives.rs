//! Adversarial and valid archives through the real decode and inspection path.
//!
//! Archives are built byte by byte so hostile headers can be expressed, then
//! compressed with real gzip and zstd encoders. Test quotas are small so a
//! decompression-bomb regression stays bounded.

use limeos_backup_archive::staging::{StagingError, StagingRoot, admit_for_staging, replay};
use limeos_backup_archive::{inspect, inspect_and_admit};
use limeos_domain::backups::{
    AdmissionPolicy, ArchiveFormat, ArchiveLimits, EntryKind, FindingCode, LegacyMapping,
    ManagedResource, Rejection, RestoreManifest,
};
use sha2::{Digest, Sha256};
use std::{
    cell::Cell,
    collections::BTreeMap,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use tar::{EntryType, Header};

use std::os::unix::fs::{PermissionsExt, symlink};

fn private_tempdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

fn empty(root: &Path) -> bool {
    std::fs::read_dir(root).unwrap().next().is_none()
}

fn verify_catalog(catalog: &limeos_backup_archive::staging::VerifiedCatalog) {
    for entry in catalog.entries() {
        if let Some(mut reader) = entry.reader() {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes.len() as u64, entry.metadata().size);
            assert_eq!(
                entry.metadata().sha256.as_deref(),
                Some(hex::encode(Sha256::digest(&bytes)).as_str())
            );
            // Independent readers have independent positions.
            let mut second = entry.reader().unwrap();
            let mut again = Vec::new();
            second.read_to_end(&mut again).unwrap();
            assert_eq!(bytes, again);
        } else {
            assert_eq!(entry.metadata().kind, EntryKind::Directory);
        }
    }
}

#[test]
fn staging_replays_real_formats_and_gnu_fixtures_to_flat_private_files() {
    let managed = private_tempdir();
    std::fs::write(
        managed.path().join("sentinel"),
        b"managed bytes stay unchanged",
    )
    .unwrap();
    for archive in [
        gz(&sample()),
        zst(&sample()),
        std::fs::read(fixtures().join("legacy-valid.tar.gz")).unwrap(),
        std::fs::read(fixtures().join("legacy-valid.tar.zst")).unwrap(),
        std::fs::read(fixtures().join("legacy-posix.tar.gz")).unwrap(),
        std::fs::read(fixtures().join("legacy-primary-overlap.tar.zst")).unwrap(),
    ] {
        let mut policy = policy();
        policy.resources[0].destination_root = managed.path().to_str().unwrap().into();
        let admitted = admit_for_staging(&archive[..], policy.clone(), &never).unwrap();
        let root = private_tempdir();
        let trusted = StagingRoot::open(root.path()).unwrap();
        let catalog = replay(
            &archive[..],
            &policy,
            admitted.manifest(),
            admitted.policy_snapshot(),
            &trusted,
            &never,
        )
        .unwrap();
        assert_eq!(catalog.manifest(), admitted.manifest());
        verify_catalog(&catalog);
        let attempts: Vec<_> = std::fs::read_dir(root.path())
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(attempts.len(), 1);
        assert!(
            attempts[0]
                .file_name()
                .to_str()
                .unwrap()
                .starts_with("incomplete-")
        );
        assert_eq!(
            attempts[0].metadata().unwrap().permissions().mode() & 0o7777,
            0o700
        );
        let files: Vec<_> = std::fs::read_dir(attempts[0].path())
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(files.len() as u64, admitted.manifest().file_count);
        for file in files {
            assert!(file.file_name().to_str().unwrap().starts_with("file-"));
            assert!(file.file_type().unwrap().is_file());
            assert_eq!(
                file.metadata().unwrap().permissions().mode() & 0o7777,
                0o400
            );
        }
        drop(catalog);
        assert!(empty(root.path()));
        assert_eq!(
            std::fs::read(managed.path().join("sentinel")).unwrap(),
            b"managed bytes stay unchanged"
        );
    }
}

#[test]
fn staging_refuses_full_policy_edits_even_with_the_same_revision_before_reads() {
    let archive = gz(&sample());
    let policy = policy();
    let admitted = admit_for_staging(&archive[..], policy.clone(), &never).unwrap();
    let mut changes = Vec::new();
    let mut p = policy.clone();
    p.revision += 1;
    changes.push(p);
    let mut p = policy.clone();
    p.limits.max_entries += 1;
    changes.push(p);
    let mut p = policy.clone();
    p.limits.max_zstd_window_log += 1;
    changes.push(p);
    let mut p = policy.clone();
    p.formats.reverse();
    changes.push(p);
    let mut p = policy.clone();
    p.resources[0].destination_root = "/private/other".into();
    changes.push(p);
    let mut p = policy.clone();
    p.legacy_mappings[0].resource_prefix = "other".into();
    changes.push(p);
    let root = private_tempdir();
    let trusted = StagingRoot::open(root.path()).unwrap();
    struct MustNotRead;
    impl Read for MustNotRead {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("policy refusal must precede archive reads")
        }
    }
    for changed in changes {
        assert!(matches!(
            replay(
                MustNotRead,
                &changed,
                admitted.manifest(),
                admitted.policy_snapshot(),
                &trusted,
                &never
            ),
            Err(StagingError::PolicyChanged)
        ));
        assert!(empty(root.path()));
    }
}

#[test]
fn staging_revalidates_every_manifest_field_and_never_uses_forged_paths() {
    let archive = gz(&sample());
    let policy = policy();
    let admitted = admit_for_staging(&archive[..], policy.clone(), &never).unwrap();
    let base = admitted.manifest();
    type Mutation = Box<dyn Fn(&mut RestoreManifest)>;
    let mutations: Vec<Mutation> = vec![
        Box::new(|m| m.manifest_version = 1),
        Box::new(|m| m.policy_revision += 1),
        Box::new(|m| m.archive_sha256 = "a".repeat(64)),
        Box::new(|m| m.format = ArchiveFormat::TarZstd),
        Box::new(|m| m.compressed_bytes += 1),
        Box::new(|m| m.decompressed_bytes += 512),
        Box::new(|m| m.header_count += 1),
        Box::new(|m| m.coalesced_self_hardlinks += 1),
        Box::new(|m| m.file_count += 1),
        Box::new(|m| m.directory_count += 1),
        Box::new(|m| m.file_bytes += 1),
        Box::new(|m| m.entries[0].resource = "forged-resource".into()),
        Box::new(|m| m.entries[0].relative_path = "../../foreign-sentinel".into()),
        Box::new(|m| m.entries[0].archive_path = "/foreign-sentinel".into()),
        Box::new(|m| m.entries[0].sha256 = Some("a".repeat(64))),
        Box::new(|m| m.entries[0].size += 1),
        Box::new(|m| m.entries[0].archived_permissions = 0o4777),
        Box::new(|m| m.entries.reverse()),
        Box::new(|m| m.entries.push(m.entries[0].clone())),
        Box::new(|m| m.file_bytes = u64::MAX),
        Box::new(|m| m.header_count = u64::MAX),
    ];
    let root = private_tempdir();
    let trusted = StagingRoot::open(root.path()).unwrap();
    let foreign = root.path().join("foreign-sentinel");
    std::fs::write(&foreign, b"foreign").unwrap();
    for (index, mutation) in mutations.iter().enumerate() {
        let mut forged = base.clone();
        mutation(&mut forged);
        assert!(
            replay(
                &archive[..],
                &policy,
                &forged,
                admitted.policy_snapshot(),
                &trusted,
                &never
            )
            .is_err(),
            "forged field {index}"
        );
        assert_eq!(std::fs::read(&foreign).unwrap(), b"foreign");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
}

#[test]
fn staging_changed_payload_order_header_format_and_matching_supplied_digest_refuse() {
    let original = Tar::default()
        .file("etc/limeos/a", b"one")
        .file("etc/limeos/b", b"two")
        .end();
    let archive = gz(&original);
    let policy = policy();
    let admitted = admit_for_staging(&archive[..], policy.clone(), &never).unwrap();
    let mut changed_mode = Tar::header(EntryType::Regular, b"etc/limeos/a", 3);
    changed_mode.set_mode(0o600);
    changed_mode.set_cksum();
    let changed = [
        gz(&Tar::default()
            .file("etc/limeos/a", b"new")
            .file("etc/limeos/b", b"two")
            .end()),
        gz(&Tar::default()
            .file("etc/limeos/b", b"two")
            .file("etc/limeos/a", b"one")
            .end()),
        gz(&Tar::default()
            .push(&changed_mode, b"one")
            .file("etc/limeos/b", b"two")
            .end()),
        zst(&original),
    ];
    let root = private_tempdir();
    let trusted = StagingRoot::open(root.path()).unwrap();
    for bytes in changed {
        let mut forged = admitted.manifest().clone();
        // Even a correct supplied compressed digest is not sufficient proof.
        forged.archive_sha256 = hex::encode(Sha256::digest(&bytes));
        assert!(
            replay(
                &bytes[..],
                &policy,
                &forged,
                admitted.policy_snapshot(),
                &trusted,
                &never
            )
            .is_err()
        );
        assert!(empty(root.path()));
    }
}

#[test]
fn staging_corrupt_truncated_trailing_and_hostile_streams_clean_private_attempts() {
    let archive = gz(&sample());
    let policy = policy();
    let admitted = admit_for_staging(&archive[..], policy.clone(), &never).unwrap();
    let mut corrupt = archive.clone();
    let at = corrupt.len() - 8;
    corrupt[at] ^= 0x55;
    let mut trailing = archive.clone();
    trailing.extend_from_slice(b"trailing");
    let mut inputs = vec![corrupt, archive[..archive.len() - 5].to_vec(), trailing];
    for name in [
        "parent-name.tar.gz",
        "absolute-name.tar.gz",
        "symlink-escape.tar.gz",
        "sparse.tar.gz",
        "fifo.tar.gz",
    ] {
        inputs.push(std::fs::read(fixtures().join(name)).unwrap());
    }
    for tar in [
        Tar::default()
            .typed(EntryType::Link, "etc/limeos/core.json", "../../foreign")
            .end(),
        Tar::default()
            .typed(EntryType::Char, "etc/limeos/device", "")
            .end(),
        Tar::default()
            .file("etc/limeos/core.json", b"x")
            .file("etc/limeos/core.json", b"y")
            .end(),
        Tar::default()
            .pax(&[("size", b"100")])
            .file("etc/limeos/core.json", b"x")
            .end(),
    ] {
        inputs.extend([gz(&tar), zst(&tar)]);
    }
    let root = private_tempdir();
    let trusted = StagingRoot::open(root.path()).unwrap();
    for input in inputs {
        assert!(
            replay(
                &input[..],
                &policy,
                admitted.manifest(),
                admitted.policy_snapshot(),
                &trusted,
                &never
            )
            .is_err()
        );
        assert!(empty(root.path()));
    }
}

#[test]
fn staging_short_interrupted_reads_and_cancellation_mid_stream_and_near_eof() {
    let policy = policy();
    for archive in [gz(&sample()), zst(&sample())] {
        let admitted = admit_for_staging(&archive[..], policy.clone(), &never).unwrap();
        let root = private_tempdir();
        let trusted = StagingRoot::open(root.path()).unwrap();
        let source = Fragmented {
            inner: &archive[..],
            maximum: 1,
            interrupt: true,
        };
        let catalog = replay(
            source,
            &policy,
            admitted.manifest(),
            admitted.policy_snapshot(),
            &trusted,
            &never,
        )
        .unwrap();
        verify_catalog(&catalog);
        drop(catalog);
        assert!(empty(root.path()));
        for threshold in [0, 5, archive.len() / 2, archive.len() - 1, archive.len()] {
            struct Counted<'a> {
                bytes: &'a [u8],
                served: &'a Cell<usize>,
            }
            impl Read for Counted<'_> {
                fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                    let n = self.bytes.read(&mut buf[..1])?;
                    self.served.set(self.served.get() + n);
                    Ok(n)
                }
            }
            let served = Cell::new(0);
            let source = Counted {
                bytes: &archive,
                served: &served,
            };
            assert!(matches!(
                replay(
                    source,
                    &policy,
                    admitted.manifest(),
                    admitted.policy_snapshot(),
                    &trusted,
                    &|| served.get() >= threshold
                ),
                Err(StagingError::Cancelled)
            ));
            assert!(empty(root.path()));
        }
    }
}

#[test]
fn staging_root_symlink_and_foreign_path_attacks_refuse_without_overwrite() {
    let parent = private_tempdir();
    let private = parent.path().join("private");
    std::fs::create_dir(&private).unwrap();
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(private.join("sentinel"), b"foreign").unwrap();
    let link = parent.path().join("link");
    symlink(&private, &link).unwrap();
    assert!(matches!(
        StagingRoot::open(&link),
        Err(StagingError::UnsafeRoot)
    ));
    let nested = private.join("nested");
    std::fs::create_dir(&nested).unwrap();
    std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(StagingRoot::open(&link.join("nested")).is_err());
    assert!(StagingRoot::open(&private.join("sentinel")).is_err());
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(StagingRoot::open(&private).is_err());
    assert_eq!(std::fs::read(private.join("sentinel")).unwrap(), b"foreign");
}

#[test]
fn staging_attempts_are_independent_and_cleanup_uses_held_root_descriptor() {
    let parent = private_tempdir();
    let path = parent.path().join("private");
    std::fs::create_dir(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let root = StagingRoot::open(&path).unwrap();
    let archive = gz(&sample());
    let policy = policy();
    let admitted = admit_for_staging(&archive[..], policy.clone(), &never).unwrap();
    let a = replay(
        &archive[..],
        &policy,
        admitted.manifest(),
        admitted.policy_snapshot(),
        &root,
        &never,
    )
    .unwrap();
    let b = replay(
        &archive[..],
        &policy,
        admitted.manifest(),
        admitted.policy_snapshot(),
        &root,
        &never,
    )
    .unwrap();
    assert_eq!(std::fs::read_dir(&path).unwrap().count(), 2);
    let moved = parent.path().join("moved");
    std::fs::rename(&path, &moved).unwrap();
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("foreign"), b"untouched").unwrap();
    a.discard().unwrap();
    assert_eq!(std::fs::read_dir(&moved).unwrap().count(), 1);
    verify_catalog(&b);
    drop(b);
    assert!(empty(&moved));
    assert_eq!(std::fs::read(path.join("foreign")).unwrap(), b"untouched");
}

// Deterministic incompressible data keeps the decoder asking for source reads
// after at least one 64 KiB chunk has actually reached quarantine.
fn staging_large_archive() -> Vec<u8> {
    let mut data = Vec::with_capacity(256 * 1024);
    for n in 0u64..8192 {
        data.extend_from_slice(&Sha256::digest(n.to_le_bytes()));
    }
    gz(&Tar::default().file("etc/limeos/MixedCase", &data).end())
}

fn tentative_bytes(root: &Path) -> u64 {
    std::fs::read_dir(root)
        .unwrap()
        .map(Result::unwrap)
        .filter(|e| e.file_type().unwrap().is_dir())
        .flat_map(|e| std::fs::read_dir(e.path()).unwrap().map(Result::unwrap))
        .map(|e| e.metadata().unwrap().len())
        .sum()
}

#[test]
fn staging_footer_failure_and_mid_file_cancellation_follow_tentative_writes() {
    let archive = staging_large_archive();
    let policy = policy();
    let admitted = admit_for_staging(&archive[..], policy.clone(), &never).unwrap();
    let mut corrupt = archive.clone();
    let at = corrupt.len() - 8;
    corrupt[at] ^= 1;
    let truncated = archive[..archive.len() - 5].to_vec();
    let mut trailing = archive.clone();
    trailing.extend_from_slice(b"trailing");
    for (cancel, input) in [
        (true, archive.clone()),
        (false, corrupt),
        (false, truncated),
        (false, trailing),
    ] {
        let root = private_tempdir();
        let trusted = StagingRoot::open(root.path()).unwrap();
        let observed = Cell::new(false);
        struct Observe<'a> {
            bytes: &'a [u8],
            root: &'a Path,
            observed: &'a Cell<bool>,
        }
        impl Read for Observe<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                if tentative_bytes(self.root) > 0 {
                    self.observed.set(true);
                }
                let n = buffer.len().min(4096);
                self.bytes.read(&mut buffer[..n])
            }
        }
        let source = Observe {
            bytes: &input,
            root: root.path(),
            observed: &observed,
        };
        let result = replay(
            source,
            &policy,
            admitted.manifest(),
            admitted.policy_snapshot(),
            &trusted,
            &|| cancel && observed.get(),
        );
        assert!(result.is_err());
        if cancel {
            assert!(matches!(result, Err(StagingError::Cancelled)));
        }
        assert!(observed.get(), "must write tentative bytes before refusal");
        assert!(empty(root.path()));
    }
}

#[test]
fn staging_repeated_interrupted_reads_preserve_bytes() {
    struct Alternating<'a> {
        bytes: &'a [u8],
        interrupt: bool,
    }
    impl Read for Alternating<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.interrupt = !self.interrupt;
            if self.interrupt {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let n = buffer.len().min(1);
            self.bytes.read(&mut buffer[..n])
        }
    }
    for archive in [gz(&sample()), zst(&sample())] {
        let policy = policy();
        let admitted = admit_for_staging(&archive[..], policy.clone(), &never).unwrap();
        let root = private_tempdir();
        let trusted = StagingRoot::open(root.path()).unwrap();
        let catalog = replay(
            Alternating {
                bytes: &archive,
                interrupt: false,
            },
            &policy,
            admitted.manifest(),
            admitted.policy_snapshot(),
            &trusted,
            &never,
        )
        .unwrap();
        verify_catalog(&catalog);
        catalog.discard().unwrap();
        assert!(empty(root.path()));
    }
}

#[test]
fn staging_enforces_declared_metadata_file_entry_and_decoder_limits() {
    let huge_name = Tar::header(EntryType::GNULongName, b"././@LongLink", 1 << 30);
    let huge_file = Tar::header(EntryType::Regular, b"opt/stacks/big", 1 << 40);
    let mut flood = Tar::default();
    for i in 0..65 {
        flood = flood.dir(&format!("opt/stacks/{i}/"));
    }
    let mut bomb = Tar::default();
    for i in 0..5 {
        bomb = bomb.file(&format!("opt/stacks/zero-{i}"), &vec![0; 900 * 1024]);
    }
    for format in [ArchiveFormat::TarGzip, ArchiveFormat::TarZstd] {
        let encode = |data: &[u8]| match format {
            ArchiveFormat::TarGzip => gz(data),
            ArchiveFormat::TarZstd => zst(data),
        };
        // Roomy baseline totals let declarations reach the trusted per-file
        // and metadata guards; the expected totals still cap aggregate output.
        let mut baseline = Tar::default();
        for i in 0..64 {
            baseline = baseline.dir(&format!("opt/stacks/{i}/"));
        }
        let mut baseline = baseline.end();
        baseline.extend(std::iter::repeat_n(0, 3 * MIB as usize));
        let archive = encode(&baseline);
        let policy = policy();
        let admitted = admit_for_staging(&archive[..], policy.clone(), &never).unwrap();
        let root = private_tempdir();
        let trusted = StagingRoot::open(root.path()).unwrap();
        for data in [
            Tar::default().push(&huge_name, &[]).end(),
            Tar::default().push(&huge_file, &[]).end(),
            flood.0.clone(),
            bomb.0.clone(),
        ] {
            let input = encode(&data);
            assert!(
                replay(
                    &input[..],
                    &policy,
                    admitted.manifest(),
                    admitted.policy_snapshot(),
                    &trusted,
                    &never
                )
                .is_err()
            );
            assert!(empty(root.path()));
        }
    }
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
    encoder
        .set_parameter(zstd::stream::raw::CParameter::WindowLog(23))
        .unwrap();
    encoder.write_all(&sample()).unwrap();
    let oversized_window = encoder.finish().unwrap();
    let archive = zst(&sample());
    let policy = policy();
    let admitted = admit_for_staging(&archive[..], policy.clone(), &never).unwrap();
    let root = private_tempdir();
    let trusted = StagingRoot::open(root.path()).unwrap();
    assert!(
        matches!(replay(&oversized_window[..], &policy, admitted.manifest(),
        admitted.policy_snapshot(), &trusted, &never), Err(StagingError::Admission(r))
        if r.codes().contains(&FindingCode::DecoderMemoryLimit))
    );
    assert!(empty(root.path()));
}

#[test]
fn staging_crash_child() {
    let Some(path) = std::env::var_os("LIMEOS_STAGING_CRASH_ROOT") else {
        return;
    };
    let path = PathBuf::from(path);
    let archive = staging_large_archive();
    let policy = policy();
    let admitted = admit_for_staging(&archive[..], policy.clone(), &never).unwrap();
    let root = StagingRoot::open(&path).unwrap();
    struct Block<'a> {
        bytes: &'a [u8],
        root: &'a Path,
    }
    impl Read for Block<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if tentative_bytes(self.root) > 0 {
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
            let n = buffer.len().min(4096);
            self.bytes.read(&mut buffer[..n])
        }
    }
    let _catalog = replay(
        Block {
            bytes: &archive,
            root: &path,
        },
        &policy,
        admitted.manifest(),
        admitted.policy_snapshot(),
        &root,
        &never,
    )
    .unwrap();
    panic!("child must be killed before verified completion");
}

#[test]
fn staging_sigkill_leaves_private_incomplete_artifacts_and_never_resumes_them() {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let temp = private_tempdir();
    let mut child = Child(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "staging_crash_child", "--nocapture"])
            .env("LIMEOS_STAGING_CRASH_ROOT", temp.path())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let start = Instant::now();
    while tentative_bytes(temp.path()) == 0 {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child exited before tentative write"
        );
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "child made no progress"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    child.0.kill().unwrap();
    let status = child.0.wait().unwrap();
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(status.signal(), Some(9));
    let attempts: Vec<_> = std::fs::read_dir(temp.path())
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(attempts.len(), 1);
    assert!(
        attempts[0]
            .file_name()
            .to_str()
            .unwrap()
            .starts_with("incomplete-")
    );
    assert_eq!(
        attempts[0].metadata().unwrap().permissions().mode() & 0o7777,
        0o700
    );
    for file in std::fs::read_dir(attempts[0].path())
        .unwrap()
        .map(Result::unwrap)
    {
        assert!(file.file_name().to_str().unwrap().starts_with("file-"));
        assert_eq!(
            file.metadata().unwrap().permissions().mode() & 0o7777,
            0o600
        );
    }
    let old_bytes = tentative_bytes(temp.path());
    let archive = gz(&sample());
    let policy = policy();
    let admitted = admit_for_staging(&archive[..], policy.clone(), &never).unwrap();
    let root = StagingRoot::open(temp.path()).unwrap();
    let catalog = replay(
        &archive[..],
        &policy,
        admitted.manifest(),
        admitted.policy_snapshot(),
        &root,
        &never,
    )
    .unwrap();
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 2);
    verify_catalog(&catalog);
    catalog.discard().unwrap();
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    assert_eq!(tentative_bytes(temp.path()), old_bytes);
}

const MIB: u64 = 1 << 20;

struct Fragmented<R> {
    inner: R,
    maximum: usize,
    interrupt: bool,
}

impl<R: Read> Read for Fragmented<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.interrupt {
            self.interrupt = false;
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        let count = buffer.len().min(self.maximum);
        self.inner.read(&mut buffer[..count])
    }
}

fn limits() -> ArchiveLimits {
    ArchiveLimits {
        max_compressed_bytes: MIB,
        max_decompressed_bytes: 4 * MIB,
        max_file_bytes: MIB,
        max_entries: 64,
        max_path_bytes: 256,
        max_path_depth: 16,
        max_component_bytes: 128,
        max_metadata_bytes: 4096,
        max_total_metadata_bytes: 16384,
        // zstd's default level (what the frozen helper used) needs 2^21.
        max_zstd_window_log: 21,
    }
}

fn policy() -> AdmissionPolicy {
    let resource = |id: &str, root: &str| ManagedResource {
        id: id.into(),
        destination_root: root.into(),
    };
    let map = |prefix: &str, id: &str| LegacyMapping {
        archive_prefix: prefix.into(),
        resource: id.into(),
        resource_prefix: String::new(),
    };
    AdmissionPolicy {
        revision: 3,
        formats: vec![ArchiveFormat::TarGzip, ArchiveFormat::TarZstd],
        limits: limits(),
        resources: vec![
            resource("limeos-config", "/etc/limeos"),
            resource("limeos-state", "/var/lib/limeos"),
            resource("stacks", "/opt/stacks"),
            resource("app-config", "/home/pi/docker"),
        ],
        legacy_mappings: vec![
            map("etc/limeos", "limeos-config"),
            map("var/lib/limeos", "limeos-state"),
            map("opt/stacks", "stacks"),
            map("home/pi/docker", "app-config"),
        ],
    }
}

fn never() -> bool {
    false
}

/// Raw tar stream builder. Names are written into the header verbatim so
/// traversal, absolute and non-UTF-8 names can be expressed.
#[derive(Default, Clone)]
struct Tar(Vec<u8>);

impl Tar {
    fn header(kind: EntryType, name: &[u8], size: u64) -> Header {
        let mut header = Header::new_gnu();
        header.set_entry_type(kind);
        header.set_size(size);
        header.set_mode(if kind == EntryType::Directory {
            0o755
        } else {
            0o644
        });
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        let gnu = header.as_gnu_mut().unwrap();
        gnu.name = [0; 100];
        gnu.name[..name.len()].copy_from_slice(name);
        header.set_cksum();
        header
    }

    fn push(mut self, header: &Header, data: &[u8]) -> Self {
        self.0.extend_from_slice(header.as_bytes());
        self.0.extend_from_slice(data);
        self.0.resize(self.0.len().div_ceil(512) * 512, 0);
        self
    }

    fn file(self, name: &str, data: &[u8]) -> Self {
        let header = Self::header(EntryType::Regular, name.as_bytes(), data.len() as u64);
        self.push(&header, data)
    }

    fn dir(self, name: &str) -> Self {
        let header = Self::header(EntryType::Directory, name.as_bytes(), 0);
        self.push(&header, &[])
    }

    fn typed(self, kind: EntryType, name: &str, link: &str) -> Self {
        let mut header = Self::header(kind, name.as_bytes(), 0);
        let gnu = header.as_gnu_mut().unwrap();
        gnu.linkname[..link.len()].copy_from_slice(link.as_bytes());
        header.set_cksum();
        self.push(&header, &[])
    }

    fn extension(self, kind: EntryType, body: &[u8]) -> Self {
        let header = Self::header(kind, b"././@LongLink", body.len() as u64);
        self.push(&header, body)
    }

    fn long_name(self, name: &[u8]) -> Self {
        let mut body = name.to_vec();
        body.push(0);
        self.extension(EntryType::GNULongName, &body)
    }

    fn pax(self, records: &[(&str, &[u8])]) -> Self {
        self.extension(EntryType::XHeader, &pax_body(records))
    }

    fn end(mut self) -> Vec<u8> {
        self.0.extend_from_slice(&[0; 1024]);
        self.0
    }
}

fn pax_body(records: &[(&str, &[u8])]) -> Vec<u8> {
    let mut body = Vec::new();
    for (key, value) in records {
        let rest = key.len() + value.len() + 3;
        let mut len = rest + 1;
        while len.to_string().len() + rest != len {
            len += 1;
        }
        body.extend_from_slice(format!("{len} {key}=").as_bytes());
        body.extend_from_slice(value);
        body.push(b'\n');
    }
    body
}

fn gz(tar: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(tar).unwrap();
    encoder.finish().unwrap()
}

fn zst(tar: &[u8]) -> Vec<u8> {
    zstd::stream::encode_all(tar, 3).unwrap()
}

fn admitted(archive: &[u8]) -> RestoreManifest {
    inspect_and_admit(archive, &policy(), &never).expect("archive must be admitted")
}

fn rejected_with(archive: &[u8], policy: &AdmissionPolicy) -> Rejection {
    inspect_and_admit(archive, policy, &never).expect_err("archive must be rejected")
}

fn first(archive: &[u8]) -> FindingCode {
    rejected_with(archive, &policy()).findings[0].code
}

/// Both compressions must reach the same decision for the same tar stream.
fn both(tar: &[u8]) -> FindingCode {
    let (a, b) = (first(&gz(tar)), first(&zst(tar)));
    assert_eq!(a, b, "gzip and zstd disagree");
    a
}

fn sample() -> Vec<u8> {
    Tar::default()
        .dir("etc/limeos/")
        .file("etc/limeos/core.json", br#"{"version":1}"#)
        .dir("opt/stacks/media/")
        .file("opt/stacks/media/compose.yaml", b"services: {}\n")
        .file("home/pi/docker/sonarr.xml", b"<Config/>")
        .end()
}

#[test]
fn valid_archives_yield_identical_deterministic_manifests_in_both_formats() {
    let tar = sample();
    let (g, z) = (admitted(&gz(&tar)), admitted(&zst(&tar)));
    assert_eq!(g.entries, z.entries);
    assert_eq!(
        (g.format, z.format),
        (ArchiveFormat::TarGzip, ArchiveFormat::TarZstd)
    );
    assert_eq!(g.decompressed_bytes, tar.len() as u64);
    assert_eq!((g.header_count, g.file_count, g.directory_count), (5, 3, 2));
    assert_eq!(
        admitted(&gz(&tar)),
        g,
        "repeat inspection must be identical"
    );
    let resources: Vec<_> = g
        .entries
        .iter()
        .map(|e| (e.resource.as_str(), e.relative_path.as_str()))
        .collect();
    assert_eq!(
        resources,
        [
            ("app-config", "sonarr.xml"),
            ("limeos-config", ""),
            ("limeos-config", "core.json"),
            ("stacks", "media"),
            ("stacks", "media/compose.yaml"),
        ]
    );
}

#[test]
fn short_and_interrupted_reads_preserve_format_digest_and_complete_admission() {
    for archive in [gz(&sample()), zst(&sample())] {
        let expected = admitted(&archive);
        for maximum in [1, 2, 3, 7, 1024] {
            for interrupt in [false, true] {
                let mut source = Fragmented {
                    inner: io::Cursor::new(&archive),
                    maximum,
                    interrupt,
                };
                let actual = inspect_and_admit(&mut source, &policy(), &never).unwrap();
                assert_eq!(actual, expected, "chunk size {maximum}");
                assert_eq!(source.inner.position(), archive.len() as u64);
            }
        }
    }
}

#[test]
fn manifest_is_bound_to_the_exact_archive_and_file_contents() {
    let archive = zst(&sample());
    let manifest = admitted(&archive);
    assert_eq!(
        manifest.archive_sha256,
        hex::encode(Sha256::digest(&archive))
    );
    assert_eq!(manifest.compressed_bytes, archive.len() as u64);
    assert_eq!(manifest.policy_revision, 3);
    let core = manifest
        .entries
        .iter()
        .find(|e| e.relative_path == "core.json")
        .unwrap();
    assert_eq!(
        core.sha256.as_deref(),
        Some(hex::encode(Sha256::digest(br#"{"version":1}"#)).as_str())
    );
    assert_eq!(core.kind, EntryKind::File);
    // A single changed byte changes the binding.
    let other = zst(&Tar::default()
        .file("etc/limeos/core.json", br#"{"version":2}"#)
        .end());
    assert_ne!(admitted(&other).archive_sha256, manifest.archive_sha256);
}

#[test]
fn traversal_and_absolute_names_are_rejected() {
    for (name, code) in [
        ("../etc/limeos/core.json", FindingCode::ParentComponent),
        ("etc/limeos/../../shadow", FindingCode::ParentComponent),
        ("/etc/limeos/core.json", FindingCode::AbsolutePath),
        ("./etc/limeos/core.json", FindingCode::DotComponent),
        ("etc//limeos/core.json", FindingCode::EmptyComponent),
        ("etc/shadow", FindingCode::Unmapped),
        ("home/pi/.ssh/authorized_keys", FindingCode::Unmapped),
        ("opt/stacks/a\u{1b}[2J", FindingCode::UnsafeCharacter),
        ("opt/stacks/file/", FindingCode::TrailingSlashOnFile),
    ] {
        assert_eq!(both(&Tar::default().file(name, b"x").end()), code, "{name}");
    }
    let mut header = Tar::header(EntryType::Regular, b"opt/stacks/\xff\xfe", 1);
    header.set_cksum();
    assert_eq!(
        both(&Tar::default().push(&header, b"x").end()),
        FindingCode::NonUtf8Name
    );
}

#[test]
fn extension_records_set_the_effective_name_that_is_checked() {
    // The innocent header name is overridden; the override is what counts.
    let gnu = Tar::default()
        .long_name(b"../../etc/shadow")
        .file("opt/stacks/safe", b"x")
        .end();
    assert_eq!(both(&gnu), FindingCode::ParentComponent);
    let pax = Tar::default()
        .pax(&[("path", b"/etc/shadow")])
        .file("opt/stacks/safe", b"x")
        .end();
    assert_eq!(both(&pax), FindingCode::AbsolutePath);
    let long = format!("opt/stacks/{}/settings.json", "d".repeat(120));
    let ok = Tar::default()
        .long_name(long.as_bytes())
        .file("opt/stacks/short", b"x")
        .end();
    assert_eq!(admitted(&gz(&ok)).entries[0].archive_path, long);
    let pax_ok = Tar::default()
        .pax(&[
            ("path", b"opt/stacks/real.yaml"),
            ("mtime", b"1767225600.5"),
            ("uname", b"root"),
        ])
        .file("opt/stacks/placeholder", b"x")
        .end();
    assert_eq!(
        admitted(&zst(&pax_ok)).entries[0].relative_path,
        "real.yaml"
    );
}

#[test]
fn ambiguous_or_unsupported_extension_records_are_rejected() {
    let cases: Vec<(Vec<u8>, FindingCode)> = vec![
        (
            Tar::default()
                .long_name(b"opt/stacks/a")
                .pax(&[("path", b"opt/stacks/b")])
                .file("opt/stacks/c", b"x")
                .end(),
            FindingCode::AmbiguousName,
        ),
        (
            Tar::default()
                .long_name(b"opt/stacks/a")
                .long_name(b"opt/stacks/b")
                .file("opt/stacks/c", b"x")
                .end(),
            FindingCode::AmbiguousName,
        ),
        (
            Tar::default()
                .pax(&[("mtime", b"1")])
                .pax(&[("mtime", b"2")])
                .file("opt/stacks/c", b"x")
                .end(),
            FindingCode::AmbiguousName,
        ),
        (
            Tar::default()
                .pax(&[("path", b"opt/stacks/a"), ("path", b"opt/stacks/b")])
                .file("opt/stacks/c", b"x")
                .end(),
            FindingCode::AmbiguousName,
        ),
        // A size override changes framing; never trusted, never applied.
        (
            Tar::default()
                .pax(&[("size", b"0")])
                .file("opt/stacks/c", b"x")
                .end(),
            FindingCode::UnsupportedExtension,
        ),
        (
            Tar::default()
                .pax(&[("SCHILY.xattr.security.capability", b"\x01")])
                .file("opt/stacks/c", b"x")
                .end(),
            FindingCode::UnsupportedExtension,
        ),
        (
            Tar::default()
                .pax(&[("GNU.sparse.size", b"9")])
                .file("opt/stacks/c", b"x")
                .end(),
            FindingCode::Sparse,
        ),
        (
            Tar::default()
                .extension(EntryType::XGlobalHeader, &pax_body(&[("comment", b"x")]))
                .end(),
            FindingCode::UnsupportedExtension,
        ),
        (
            Tar::default().pax(&[("mtime", b"1")]).end(),
            FindingCode::ExtensionWithoutMember,
        ),
        (
            Tar::default()
                .long_name(b"opt/stacks/x\0y")
                .file("opt/stacks/c", b"x")
                .end(),
            FindingCode::UnsafeCharacter,
        ),
        (
            Tar::default()
                .extension(EntryType::XHeader, b"5 x\n")
                .file("opt/stacks/c", b"x")
                .end(),
            FindingCode::Malformed,
        ),
    ];
    for (i, (tar, code)) in cases.iter().enumerate() {
        assert_eq!(both(tar), *code, "case {i}");
    }
}

#[test]
fn links_devices_and_other_member_types_are_rejected() {
    let cases = [
        (
            EntryType::Symlink,
            "opt/stacks/escape",
            "../../../etc/shadow",
            FindingCode::Symlink,
        ),
        (
            EntryType::Symlink,
            "opt/stacks/abs",
            "/etc/shadow",
            FindingCode::Symlink,
        ),
        (
            EntryType::Link,
            "opt/stacks/hard",
            "etc/shadow",
            FindingCode::Hardlink,
        ),
        (
            EntryType::Link,
            "opt/stacks/self",
            "opt/stacks/self",
            FindingCode::LegacySelfHardlink,
        ),
        (EntryType::Char, "opt/stacks/null", "", FindingCode::Device),
        (EntryType::Block, "opt/stacks/sda", "", FindingCode::Device),
        (EntryType::Fifo, "opt/stacks/pipe", "", FindingCode::Fifo),
        (
            EntryType::GNUSparse,
            "opt/stacks/sparse",
            "",
            FindingCode::Sparse,
        ),
        (
            EntryType::Continuous,
            "opt/stacks/contig",
            "",
            FindingCode::UnsupportedEntryType,
        ),
        (
            EntryType::new(b'V'),
            "opt/stacks/volume",
            "",
            FindingCode::UnsupportedEntryType,
        ),
        (
            EntryType::new(b'D'),
            "opt/stacks/dumpdir",
            "",
            FindingCode::UnsupportedEntryType,
        ),
    ];
    for (kind, name, link, code) in cases {
        assert_eq!(
            both(&Tar::default().typed(kind, name, link).end()),
            code,
            "{name}"
        );
    }
    // A chain stops at its first link; the escape is never followed.
    let chain = Tar::default()
        .typed(EntryType::Symlink, "opt/stacks/a", "b")
        .typed(EntryType::Symlink, "opt/stacks/b", "/etc")
        .file("opt/stacks/a/passwd", b"x")
        .end();
    assert_eq!(both(&chain), FindingCode::Symlink);
    // A long link name is read under the metadata bound and still rejected.
    let long_link = Tar::default()
        .extension(EntryType::GNULongLink, b"opt/stacks/self\0")
        .typed(EntryType::Link, "opt/stacks/self", "ignored")
        .end();
    assert_eq!(both(&long_link), FindingCode::LegacySelfHardlink);
    let stray_link = Tar::default()
        .extension(EntryType::GNULongLink, b"x\0")
        .file("opt/stacks/f", b"x")
        .end();
    assert_eq!(both(&stray_link), FindingCode::AmbiguousName);
}

#[test]
fn duplicate_and_colliding_destinations_are_rejected() {
    let dup = Tar::default()
        .file("opt/stacks/a", b"1")
        .file("opt/stacks/a", b"2")
        .end();
    assert_eq!(both(&dup), FindingCode::DuplicateDestination);
    let parent = Tar::default()
        .file("opt/stacks/a", b"1")
        .dir("opt/stacks/a/b/")
        .file("opt/stacks/a/b/c", b"2")
        .end();
    assert_eq!(both(&parent), FindingCode::ParentIsFile);
    let kind = Tar::default()
        .dir("opt/stacks/a/")
        .file("opt/stacks/a", b"1")
        .end();
    assert_eq!(both(&kind), FindingCode::FileDirectoryCollision);
    let root = Tar::default().file("opt/stacks", b"1").end();
    assert_eq!(both(&root), FindingCode::FileDirectoryCollision);
    let mut dir_data = Tar::header(EntryType::Directory, b"opt/stacks/d/", 3);
    dir_data.set_cksum();
    assert_eq!(
        both(&Tar::default().push(&dir_data, b"abc").end()),
        FindingCode::DirectoryWithData
    );
}

#[test]
fn malformed_and_truncated_streams_are_rejected() {
    let good = Tar::default().file("opt/stacks/a", &[7; 2000]).end();
    let mut checksum = good.clone();
    checksum[0] ^= 1;
    assert_eq!(both(&checksum), FindingCode::Malformed);
    let mut size = Tar::header(EntryType::Regular, b"opt/stacks/a", 1);
    size.as_gnu_mut().unwrap().size = *b"zzzzzzzzzzz\0";
    size.set_cksum();
    assert_eq!(
        both(&Tar::default().push(&size, b"x").end()),
        FindingCode::Malformed
    );
    let mut mode = Tar::header(EntryType::Regular, b"opt/stacks/a", 1);
    mode.as_gnu_mut().unwrap().mode = *b"9999999\0";
    mode.set_cksum();
    assert_eq!(
        both(&Tar::default().push(&mode, b"x").end()),
        FindingCode::Malformed
    );
    // Cut inside a header, inside file data, and before the end marker.
    assert_eq!(both(&good[..300]), FindingCode::Truncated);
    assert_eq!(both(&good[..512 + 1000]), FindingCode::Truncated);
    assert_eq!(both(&good[..512 + 2048]), FindingCode::Truncated);
    let one_zero_block = &good[..good.len() - 512];
    assert_eq!(both(one_zero_block), FindingCode::Truncated);
    let mut hidden = good.clone();
    hidden.extend_from_slice(
        Tar::default()
            .file("opt/stacks/hidden", b"x")
            .end()
            .as_slice(),
    );
    assert_eq!(both(&hidden), FindingCode::TrailingData);
    // Compressed streams cut short or followed by junk.
    for archive in [gz(&good), zst(&good)] {
        assert_eq!(first(&archive[..archive.len() - 6]), FindingCode::Truncated);
        let mut junk = archive.clone();
        junk.extend_from_slice(&[b'x'; 32]);
        assert_eq!(first(&junk), FindingCode::Malformed);
    }
    let mut v7 = Header::new_old();
    v7.as_old_mut().name[..12].copy_from_slice(b"opt/stacks/a");
    v7.set_size(1);
    v7.set_cksum();
    assert_eq!(
        both(&Tar::default().push(&v7, b"x").end()),
        FindingCode::UnsupportedHeaderFormat
    );
    assert_eq!(
        first(&good),
        FindingCode::UnrecognizedCompression,
        "plain tar has no compression magic"
    );
    assert_eq!(first(b"BZh91AY&SY"), FindingCode::UnrecognizedCompression);
    assert_eq!(first(b""), FindingCode::Truncated);
}

#[test]
fn oversized_declarations_are_refused_before_any_read() {
    // The headers declare far more than the stream carries; refusal must not
    // depend on reading (or allocating) the declared bytes.
    let huge_name = Tar::header(EntryType::GNULongName, b"././@LongLink", 1 << 30);
    assert_eq!(
        both(&Tar::default().push(&huge_name, &[]).end()),
        FindingCode::MetadataLimit
    );
    let huge_pax = Tar::header(EntryType::XHeader, b"pax", 4097);
    assert_eq!(
        both(&Tar::default().push(&huge_pax, &[]).end()),
        FindingCode::MetadataLimit
    );
    let huge_file = Tar::header(EntryType::Regular, b"opt/stacks/big", 1 << 40);
    assert_eq!(
        both(&Tar::default().push(&huge_file, &[]).end()),
        FindingCode::FileSizeLimit
    );
    // 2^63 survives the library's checked arithmetic; the policy refuses it.
    let mut top = Tar::header(EntryType::Regular, b"opt/stacks/top", 0);
    top.set_size(1 << 63);
    top.set_cksum();
    assert_eq!(
        both(&Tar::default().push(&top, &[]).end()),
        FindingCode::FileSizeLimit
    );
    // u64::MAX overflows the next-header computation: refused as malformed.
    let mut overflow = Tar::header(EntryType::Regular, b"opt/stacks/overflow", 0);
    overflow.set_size(u64::MAX);
    overflow.set_cksum();
    assert_eq!(
        both(&Tar::default().push(&overflow, &[]).end()),
        FindingCode::Malformed
    );
    // Many individually small records exceed the total metadata allowance.
    let mut tar = Tar::default();
    for i in 0..5 {
        let name = format!("opt/stacks/{i}{}", "n".repeat(3900));
        tar = tar.long_name(name.as_bytes()).file("opt/stacks/x", b"x");
    }
    assert_eq!(both(&tar.end()), FindingCode::TotalMetadataLimit);
}

#[test]
fn entry_floods_and_decompression_bombs_stop_at_policy_limits() {
    let mut flood = Tar::default();
    for i in 0..65 {
        flood = flood.dir(&format!("opt/stacks/{i}/"));
    }
    assert_eq!(both(&flood.end()), FindingCode::EntryLimit);

    // Five 900 KiB files of zeros: each within the file limit, together past
    // the 4 MiB decompressed limit. The compressed form stays tiny.
    let mut bomb = Tar::default();
    for i in 0..5 {
        bomb = bomb.file(&format!("opt/stacks/zero-{i}"), &vec![0; 900 * 1024]);
    }
    let bomb = bomb.end();
    for archive in [gz(&bomb), zst(&bomb)] {
        assert!(
            archive.len() < 64 * 1024,
            "fixture must be a high-expansion archive"
        );
        let rejection = rejected_with(&archive, &policy());
        assert_eq!(rejection.findings[0].code, FindingCode::DecompressedLimit);
    }
    // Zero padding after the end marker counts too.
    let mut padded = sample();
    padded.extend(std::iter::repeat_n(0, 8 * MIB as usize));
    assert_eq!(both(&padded), FindingCode::DecompressedLimit);

    let mut tight = policy();
    tight.limits.max_compressed_bytes = 64;
    assert_eq!(
        rejected_with(&zst(&sample()), &tight).findings[0].code,
        FindingCode::CompressedLimit
    );
}

#[test]
fn zstd_windows_beyond_the_decoder_memory_limit_are_refused() {
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
    encoder
        .set_parameter(zstd::stream::raw::CParameter::WindowLog(23))
        .unwrap();
    encoder.write_all(&sample()).unwrap();
    let archive = encoder.finish().unwrap();
    assert_eq!(first(&archive), FindingCode::DecoderMemoryLimit);
    let mut roomy = policy();
    roomy.limits.max_zstd_window_log = 23;
    assert!(inspect_and_admit(archive.as_slice(), &roomy, &never).is_ok());
}

#[test]
fn quota_boundaries_admit_exactly_the_limit() {
    let at = Tar::default()
        .file("opt/stacks/a", &vec![1; MIB as usize])
        .end();
    assert!(inspect_and_admit(gz(&at).as_slice(), &policy(), &never).is_ok());
    let over = Tar::default()
        .file("opt/stacks/a", &vec![1; MIB as usize + 1])
        .end();
    assert_eq!(both(&over), FindingCode::FileSizeLimit);

    let mut exact = Tar::default();
    for i in 0..64 {
        exact = exact.dir(&format!("opt/stacks/{i}/"));
    }
    assert_eq!(admitted(&zst(&exact.end())).header_count, 64);

    let archive = zst(&sample());
    let report = inspect(archive.as_slice(), &policy(), &never).unwrap();
    let mut limit = policy();
    limit.limits.max_decompressed_bytes = report.decompressed_bytes;
    limit.limits.max_file_bytes = report.decompressed_bytes;
    assert!(inspect_and_admit(archive.as_slice(), &limit, &never).is_ok());
    limit.limits.max_decompressed_bytes -= 1;
    limit.limits.max_file_bytes -= 1;
    assert_eq!(
        rejected_with(&archive, &limit).findings[0].code,
        FindingCode::DecompressedLimit
    );
    let mut compressed = policy();
    compressed.limits.max_compressed_bytes = archive.len() as u64;
    assert!(inspect_and_admit(archive.as_slice(), &compressed, &never).is_ok());
    compressed.limits.max_compressed_bytes -= 1;
    assert_eq!(
        rejected_with(&archive, &compressed).findings[0].code,
        FindingCode::CompressedLimit
    );
}

#[test]
fn cancellation_returns_no_report() {
    let mut big = Tar::default();
    for i in 0..8 {
        big = big.file(&format!("opt/stacks/f{i}"), &vec![i as u8; 200 * 1024]);
    }
    let archive = gz(&big.end());
    let calls = Cell::new(0u32);
    let cancel_later = || {
        calls.set(calls.get() + 1);
        calls.get() > 6
    };
    let rejection = inspect_and_admit(archive.as_slice(), &policy(), &cancel_later).unwrap_err();
    assert_eq!(rejection.codes(), [FindingCode::Cancelled]);
    assert!(inspect(archive.as_slice(), &policy(), &|| true).is_err());
}

#[test]
fn format_policy_and_invalid_policy_are_enforced_before_decoding() {
    let mut zstd_only = policy();
    zstd_only.formats = vec![ArchiveFormat::TarZstd];
    assert_eq!(
        rejected_with(&gz(&sample()), &zstd_only).findings[0].code,
        FindingCode::FormatNotAllowed
    );
    let mut broad = policy();
    broad.legacy_mappings[0].archive_prefix = "etc".into();
    assert_eq!(
        rejected_with(&gz(&sample()), &broad).findings[0].code,
        FindingCode::InvalidPolicy
    );
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = std::fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                stack.push(path.clone());
                out.insert(path, b"<dir>".to_vec());
            } else {
                out.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

#[test]
fn rejections_produce_no_manifest_and_change_no_managed_destination() {
    // Stand-in managed destinations with sentinel contents.
    let destinations = tempfile::tempdir().unwrap();
    for (path, body) in [
        ("etc/limeos/core.json", "sentinel"),
        ("opt/stacks/media/compose.yaml", "sentinel"),
    ] {
        let path = destinations.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
    let before = snapshot(destinations.path());
    let hostile = [
        Tar::default()
            .file("../../etc/limeos/core.json", b"pwn")
            .end(),
        Tar::default().file("/etc/limeos/core.json", b"pwn").end(),
        Tar::default()
            .typed(EntryType::Symlink, "opt/stacks/media", "/etc")
            .end(),
        Tar::default()
            .typed(EntryType::Link, "opt/stacks/x", "etc/shadow")
            .end(),
        Tar::default()
            .typed(EntryType::Char, "opt/stacks/null", "")
            .end(),
        Tar::default()
            .pax(&[("path", b"../etc/passwd")])
            .file("opt/stacks/a", b"pwn")
            .end(),
        Tar::default()
            .file("opt/stacks/a", b"1")
            .file("opt/stacks/a", b"2")
            .end(),
        Tar::default().file("opt/stacks/a", &[0; 2000]).end()[..900].to_vec(),
    ];
    for tar in hostile {
        for archive in [gz(&tar), zst(&tar)] {
            assert!(inspect_and_admit(archive.as_slice(), &policy(), &never).is_err());
        }
    }
    assert_eq!(snapshot(destinations.path()), before);
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/backup-archives")
}

#[test]
fn completed_regular_file_self_links_are_inert_legacy_repeats() {
    let name = "etc/limeos/media_layout.json";
    let original = Tar::default().file(name, b"legacy config");
    let repeated = original
        .clone()
        .typed(EntryType::Link, name, name)
        .typed(EntryType::Link, name, name)
        .end();
    for compress in [gz, zst] {
        let original = compress(&original.clone().end());
        let repeated = compress(&repeated);
        let before = inspect_and_admit(original.as_slice(), &policy(), &never).unwrap();
        let after = inspect_and_admit(repeated.as_slice(), &policy(), &never).unwrap();
        assert_eq!(before.entries, after.entries);
        assert_eq!(after.file_count, 1);
        assert_eq!(before.file_bytes, after.file_bytes);
        assert_eq!(after.header_count, 3);
        assert_eq!(before.coalesced_self_hardlinks, 0);
        assert_eq!(after.coalesced_self_hardlinks, 2);
        assert_eq!(after.manifest_version, 2);
        assert_ne!(before.archive_sha256, after.archive_sha256);
        // A truncated or corrupted ending still cannot publish a manifest.
        let mut incomplete = repeated.clone();
        incomplete.truncate(incomplete.len() - 8);
        assert!(inspect_and_admit(incomplete.as_slice(), &policy(), &never).is_err());
    }
}

#[test]
fn legacy_repeat_cannot_create_links_or_bypass_destination_checks() {
    let name = "etc/limeos/core.json";
    let mut nonempty = Tar::header(EntryType::Link, name.as_bytes(), 1);
    nonempty.as_gnu_mut().unwrap().linkname[..name.len()].copy_from_slice(name.as_bytes());
    nonempty.set_cksum();
    let cases = [
        (
            Tar::default()
                .typed(EntryType::Link, name, name)
                .file(name, b"1"),
            FindingCode::LegacySelfHardlink,
        ),
        (
            Tar::default().dir(name).typed(EntryType::Link, name, name),
            FindingCode::LegacySelfHardlink,
        ),
        (
            Tar::default().file(name, b"1").push(&nonempty, b"x"),
            FindingCode::LegacySelfHardlink,
        ),
        (
            Tar::default()
                .file(name, b"1")
                .typed(EntryType::Link, "etc/limeos/other", name),
            FindingCode::Hardlink,
        ),
        (
            Tar::default()
                .file(name, b"1")
                .typed(EntryType::Link, name, "etc/limeos/./core.json"),
            FindingCode::Hardlink,
        ),
        (
            Tar::default()
                .file(name, b"1")
                .typed(EntryType::Symlink, name, name),
            FindingCode::Symlink,
        ),
        (
            Tar::default()
                .file(name, b"1")
                .typed(EntryType::Link, name, name)
                .file(name, b"2"),
            FindingCode::DuplicateDestination,
        ),
        (
            Tar::default().file("etc/shadow", b"secret").typed(
                EntryType::Link,
                "etc/shadow",
                "etc/shadow",
            ),
            FindingCode::Unmapped,
        ),
        (
            Tar::default()
                .file("etc/limeos/../shadow", b"secret")
                .typed(
                    EntryType::Link,
                    "etc/limeos/../shadow",
                    "etc/limeos/../shadow",
                ),
            FindingCode::ParentComponent,
        ),
    ];
    for (tar, code) in cases {
        assert_eq!(both(&tar.end()), code);
    }
}

#[test]
fn gnu_tar_fixtures_match_their_recorded_hashes_and_expected_decisions() {
    let sums = std::fs::read_to_string(fixtures().join("SHA256SUMS")).unwrap();
    let mut seen = 0;
    for line in sums.lines() {
        let (digest, name) = line.split_once("  ./").unwrap();
        let archive = std::fs::read(fixtures().join(name)).unwrap();
        assert_eq!(
            hex::encode(Sha256::digest(&archive)),
            digest,
            "{name} changed; regenerate"
        );
        let result = inspect_and_admit(archive.as_slice(), &policy(), &never);
        let expected = match name {
            "legacy-valid.tar.gz" | "legacy-valid.tar.zst" | "legacy-posix.tar.gz" => None,
            "legacy-primary-overlap.tar.zst" => None,
            "symlink-escape.tar.gz" => Some(FindingCode::Symlink),
            "fifo.tar.gz" => Some(FindingCode::Fifo),
            "sparse.tar.gz" => Some(FindingCode::Sparse),
            "absolute-name.tar.gz" => Some(FindingCode::AbsolutePath),
            "parent-name.tar.gz" => Some(FindingCode::ParentComponent),
            "zeros-64m.tar.zst" => Some(FindingCode::FileSizeLimit),
            other => panic!("unexpected fixture {other}"),
        };
        match (expected, result) {
            (None, Ok(manifest)) => {
                assert_eq!(
                    manifest.coalesced_self_hardlinks,
                    if name == "legacy-primary-overlap.tar.zst" {
                        2
                    } else {
                        0
                    },
                    "{name}"
                );
                assert!(manifest.file_count >= 4, "{name}");
                assert!(
                    manifest.entries.iter().any(|e| e.archive_path.len() > 100),
                    "{name} lacks a GNU long name"
                );
            }
            (Some(code), Err(rejection)) => assert_eq!(rejection.findings[0].code, code, "{name}"),
            (expected, result) => panic!("{name}: expected {expected:?}, got {result:?}"),
        }
        seen += 1;
    }
    assert_eq!(seen, 10);
}
