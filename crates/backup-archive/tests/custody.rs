//! Private archive custody through the public API, on real private scratch
//! directories with real gzip and zstd encoders. Fault-injection and SIGKILL
//! cases that need the private I/O seam live in `src/custody/tests.rs`.

use limeos_backup_archive::custody::{
    CleanupFailure, CustodyBinding, CustodyError, CustodyLimit, CustodyLimits, CustodyRoot,
    RetainedArchive, recover, retain,
};
use limeos_backup_archive::inspect_and_admit;
use limeos_backup_archive::staging::{StagingError, StagingRoot, VerifiedCatalog};
use limeos_domain::backups::{
    AdmissionPolicy, ArchiveFormat, ArchiveLimits, EntryKind, FindingCode, LegacyMapping,
    ManagedResource, RestoreManifest,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::{Path, PathBuf},
};

const MIB: u64 = 1 << 20;

fn private_tempdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

fn never() -> bool {
    false
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
        max_zstd_window_log: 21,
    }
}

fn policy_with(limits: ArchiveLimits) -> AdmissionPolicy {
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
        limits,
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

fn policy() -> AdmissionPolicy {
    policy_with(limits())
}

fn custody_limits() -> CustodyLimits {
    CustodyLimits {
        max_archive_bytes: 96 * MIB,
        max_record_bytes: MIB,
    }
}

/// Deterministic GNU tar members with fixed metadata.
fn tar(members: &[(&str, Option<&[u8]>)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, data) in members {
        let mut header = tar::Header::new_gnu();
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        match data {
            Some(bytes) => {
                header.set_entry_type(tar::EntryType::Regular);
                header.set_mode(0o644);
                header.set_size(bytes.len() as u64);
                header.set_cksum();
                builder.append_data(&mut header, name, *bytes).unwrap();
            }
            None => {
                header.set_entry_type(tar::EntryType::Directory);
                header.set_mode(0o755);
                header.set_size(0);
                header.set_cksum();
                builder.append_data(&mut header, name, &[][..]).unwrap();
            }
        }
    }
    builder.into_inner().unwrap()
}

fn sample() -> Vec<u8> {
    tar(&[
        ("etc/limeos/", None),
        ("etc/limeos/core.json", Some(br#"{"version":1}"#)),
        ("opt/stacks/media/", None),
        ("opt/stacks/media/compose.yaml", Some(b"services: {}\n")),
        ("home/pi/docker/sonarr.xml", Some(b"<Config/>")),
    ])
}

fn gz_level(tar: &[u8], level: flate2::Compression) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), level);
    encoder.write_all(tar).unwrap();
    encoder.finish().unwrap()
}

fn gz(tar: &[u8]) -> Vec<u8> {
    gz_level(tar, flate2::Compression::default())
}

/// Stored deflate blocks: compressed size depends only on the tar length.
fn stored_gz(tar: &[u8]) -> Vec<u8> {
    gz_level(tar, flate2::Compression::none())
}

fn zst(tar: &[u8]) -> Vec<u8> {
    zstd::stream::encode_all(tar, 3).unwrap()
}

/// SHA-256 of little-endian counters: deterministic and incompressible.
fn pseudo_random(len: usize) -> Vec<u8> {
    let mut data = Vec::with_capacity(len + 32);
    let mut n = 0u64;
    while data.len() < len {
        data.extend_from_slice(&Sha256::digest(n.to_le_bytes()));
        n += 1;
    }
    data.truncate(len);
    data
}

fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// A selected source file outside the custody root. The extension is
/// deliberately misleading: format detection must use content.
fn source(dir: &Path, bytes: &[u8]) -> PathBuf {
    let path = dir.join("selected-backup.tar.bz2");
    fs::write(&path, bytes).unwrap();
    path
}

fn open(path: &Path) -> File {
    File::open(path).unwrap()
}

struct Scene {
    _temp: tempfile::TempDir,
    custody: PathBuf,
    staging: PathBuf,
    outside: PathBuf,
}

/// Custody root, staging root and an unrelated outside tree with sentinels.
fn scene() -> Scene {
    let temp = private_tempdir();
    let make = |name: &str| {
        let path = temp.path().join(name);
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    };
    let (custody, staging, outside) = (make("custody"), make("staging"), make("outside"));
    fs::write(outside.join("sentinel"), b"unrelated bytes").unwrap();
    fs::create_dir(outside.join("etc")).unwrap();
    fs::write(outside.join("etc/limeos.json"), b"{}").unwrap();
    Scene {
        _temp: temp,
        custody,
        staging,
        outside,
    }
}

impl Scene {
    fn root(&self) -> CustodyRoot {
        CustodyRoot::open(&self.custody).unwrap()
    }
    fn staging_root(&self) -> StagingRoot {
        StagingRoot::open(&self.staging).unwrap()
    }
    fn retain(&self, bytes: &[u8], policy: AdmissionPolicy) -> RetainedArchive {
        let path = source(&self.outside, bytes);
        let retained = retain(open(&path), policy, &self.root(), &custody_limits(), &never)
            .expect("archive must be retained");
        fs::remove_file(path).unwrap();
        retained
    }
    fn recover(&self, binding: &CustodyBinding, policy: AdmissionPolicy) -> RetainedArchive {
        recover(&self.root(), binding, policy, &custody_limits(), &never)
            .expect("record must be recovered")
    }
    fn recover_err(&self, binding: &CustodyBinding, policy: AdmissionPolicy) -> CustodyError {
        recover(&self.root(), binding, policy, &custody_limits(), &never)
            .expect_err("recovery must refuse")
    }
    fn record_dir(&self, binding: &CustodyBinding) -> PathBuf {
        self.custody.join(format!("retained-{}", binding.record_id))
    }
}

/// Every path below `root` with its type, mode and bytes.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, (u32, Vec<u8>)> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            let bytes = if meta.is_file() {
                fs::read(&path).unwrap()
            } else {
                Vec::new()
            };
            if meta.is_dir() {
                stack.push(path.clone());
            }
            out.insert(path, (meta.mode(), bytes));
        }
    }
    out
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().mode() & 0o7777
}

/// Independently read every catalog reader and compare it with the manifest.
fn verify_catalog(catalog: &VerifiedCatalog, expected: &RestoreManifest) {
    assert_eq!(catalog.manifest(), expected);
    let mut files = 0;
    for entry in catalog.entries() {
        match entry.reader() {
            Some(mut reader) => {
                let mut hasher = Sha256::new();
                let mut chunk = vec![0; 64 * 1024];
                let mut size = 0u64;
                loop {
                    let n = reader.read(&mut chunk).unwrap();
                    if n == 0 {
                        break;
                    }
                    hasher.update(&chunk[..n]);
                    size += n as u64;
                }
                assert_eq!(size, entry.metadata().size);
                assert_eq!(
                    entry.metadata().sha256.as_deref(),
                    Some(hex::encode(hasher.finalize()).as_str())
                );
                files += 1;
            }
            None => assert_eq!(entry.metadata().kind, EntryKind::Directory),
        }
    }
    assert_eq!(files, expected.file_count);
}

/// Owner write access for a same-UID tamper, then the original mode back.
fn tamper(path: &Path, edit: impl FnOnce(&Path)) {
    let original = mode(path);
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    edit(path);
    fs::set_permissions(path, fs::Permissions::from_mode(original)).unwrap();
}

fn replace_bytes(path: &Path, bytes: &[u8]) {
    tamper(path, |p| fs::write(p, bytes).unwrap());
}

/// Full lifecycle for one archive: retain, close, exact-binding recovery,
/// fresh staging with independent reader checks, repeat, then discard.
fn lifecycle(scene: &Scene, archive: &[u8], policy: &AdmissionPolicy) -> RestoreManifest {
    let outside = snapshot(&scene.outside);
    let direct = inspect_and_admit(archive, policy, &never).unwrap();
    let retained = scene.retain(archive, policy.clone());
    let binding = retained.binding().clone();
    assert_eq!(retained.manifest(), &direct);
    assert_eq!(binding.format_version, 1);
    assert_eq!(binding.record_id.len(), 32);
    assert_eq!(binding.archive_sha256, sha(archive));
    assert_eq!(binding.archive_bytes, archive.len() as u64);
    let dir = scene.record_dir(&binding);
    assert_eq!(
        names(&scene.custody),
        [format!("retained-{}", binding.record_id)]
    );
    assert_eq!(names(&dir), ["archive", "record.json"]);
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(&dir.join("archive")), 0o400);
    assert_eq!(mode(&dir.join("record.json")), 0o400);
    assert_eq!(fs::read(dir.join("archive")).unwrap(), archive);
    // Successful Drop closes handles and keeps the record for restart.
    drop(retained);
    assert_eq!(names(&dir), ["archive", "record.json"]);

    let mut recovered = scene.recover(&binding, policy.clone());
    assert_eq!(recovered.binding(), &binding);
    assert_eq!(recovered.manifest(), &direct);
    let catalog = recovered
        .stage(policy, &scene.staging_root(), &never)
        .unwrap();
    verify_catalog(&catalog, &direct);
    catalog.discard().unwrap();
    assert!(names(&scene.staging).is_empty());
    drop(recovered);

    // Repeated recovery creates another fresh catalog; Drop cleans it.
    let mut again = scene.recover(&binding, policy.clone());
    let catalog = again.stage(policy, &scene.staging_root(), &never).unwrap();
    verify_catalog(&catalog, &direct);
    assert_eq!(names(&scene.staging).len(), 1);
    drop(catalog);
    assert!(names(&scene.staging).is_empty());
    again.discard().unwrap();
    assert!(names(&scene.custody).is_empty());
    assert!(matches!(
        scene.recover_err(&binding, policy.clone()),
        CustodyError::NotFound
    ));
    assert_eq!(snapshot(&scene.outside), outside);
    direct
}

#[test]
fn custody_small_archives_retain_recover_stage_and_discard_in_both_formats() {
    let scene = scene();
    let gzip = lifecycle(&scene, &gz(&sample()), &policy());
    let zstd = lifecycle(&scene, &zst(&sample()), &policy());
    assert_eq!(gzip.format, ArchiveFormat::TarGzip);
    assert_eq!(zstd.format, ArchiveFormat::TarZstd);
    assert_eq!(gzip.entries, zstd.entries);
}

#[test]
fn custody_64_mib_compressed_and_expanded_archives_complete_the_lifecycle() {
    let large = policy_with(ArchiveLimits {
        max_compressed_bytes: 80 * MIB,
        max_decompressed_bytes: 80 * MIB,
        max_file_bytes: 64 * MIB,
        ..limits()
    });
    let scene = scene();
    // 64 MiB of incompressible payload stored in about 64 MiB of gzip.
    let random = pseudo_random(64 * MIB as usize);
    let archive = stored_gz(&tar(&[("opt/stacks/media/random.bin", Some(&random))]));
    assert!(archive.len() as u64 > 64 * MIB);
    let manifest = lifecycle(&scene, &archive, &large);
    assert_eq!(manifest.file_bytes, 64 * MIB);
    drop(random);
    // 64 MiB of zeros in a few kilobytes of zstd.
    let zeros = vec![0u8; 64 * MIB as usize];
    let archive = zst(&tar(&[("opt/stacks/media/zeros.bin", Some(&zeros))]));
    assert!((archive.len() as u64) < MIB);
    let manifest = lifecycle(&scene, &archive, &large);
    assert_eq!(manifest.file_bytes, 64 * MIB);
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/backup-archives")
}

#[test]
fn custody_preserves_current_fixture_decisions_and_the_inert_self_repeat() {
    let scene = scene();
    let overlap = fs::read(fixtures().join("legacy-primary-overlap.tar.zst")).unwrap();
    let manifest = lifecycle(&scene, &overlap, &policy());
    assert_eq!(manifest.coalesced_self_hardlinks, 2);
    // Staging the repeat creates one flat private file per regular file.
    let mut retained = scene.retain(&overlap, policy());
    let catalog = retained
        .stage(&policy(), &scene.staging_root(), &never)
        .unwrap();
    let attempt = scene.staging.join(&names(&scene.staging)[0]);
    let staged = names(&attempt);
    assert_eq!(staged.len() as u64, manifest.file_count);
    for name in staged {
        assert_eq!(fs::symlink_metadata(attempt.join(name)).unwrap().nlink(), 1);
    }
    catalog.discard().unwrap();
    retained.discard().unwrap();

    for (name, code) in [
        ("symlink-escape.tar.gz", FindingCode::Symlink),
        ("fifo.tar.gz", FindingCode::Fifo),
        ("sparse.tar.gz", FindingCode::Sparse),
        ("absolute-name.tar.gz", FindingCode::AbsolutePath),
        ("parent-name.tar.gz", FindingCode::ParentComponent),
        ("zeros-64m.tar.zst", FindingCode::FileSizeLimit),
    ] {
        let bytes = fs::read(fixtures().join(name)).unwrap();
        let path = source(&scene.outside, &bytes);
        match retain(
            open(&path),
            policy(),
            &scene.root(),
            &custody_limits(),
            &never,
        ) {
            Err(CustodyError::Admission(rejection)) => {
                assert_eq!(rejection.findings[0].code, code, "{name}")
            }
            other => panic!("{name}: expected admission refusal, got {other:?}"),
        }
        fs::remove_file(path).unwrap();
        assert!(
            names(&scene.custody).is_empty(),
            "{name} left custody state"
        );
    }
    for name in [
        "legacy-valid.tar.gz",
        "legacy-valid.tar.zst",
        "legacy-posix.tar.gz",
    ] {
        let bytes = fs::read(fixtures().join(name)).unwrap();
        lifecycle(&scene, &bytes, &policy());
    }
}

/// Same-revision edits to every policy field, plus a revision change.
fn policy_mutations() -> Vec<(&'static str, AdmissionPolicy)> {
    let base = policy();
    let mut out: Vec<(&'static str, AdmissionPolicy)> = Vec::new();
    let mut edit = |name, change: &dyn Fn(&mut AdmissionPolicy)| {
        let mut changed = base.clone();
        change(&mut changed);
        assert_ne!(changed, base, "{name}");
        changed.validate().unwrap();
        out.push((name, changed));
    };
    edit("formats removed", &|p| {
        p.formats = vec![ArchiveFormat::TarZstd]
    });
    edit("formats reordered", &|p| p.formats.reverse());
    edit("compressed limit", &|p| p.limits.max_compressed_bytes += 1);
    edit("decompressed limit", &|p| {
        p.limits.max_decompressed_bytes += 1
    });
    edit("file limit", &|p| p.limits.max_file_bytes += 1);
    edit("entry limit", &|p| p.limits.max_entries += 1);
    edit("path limit", &|p| p.limits.max_path_bytes += 1);
    edit("depth limit", &|p| p.limits.max_path_depth += 1);
    edit("component limit", &|p| p.limits.max_component_bytes += 1);
    edit("metadata limit", &|p| p.limits.max_metadata_bytes += 1);
    edit("total metadata limit", &|p| {
        p.limits.max_total_metadata_bytes += 1
    });
    edit("zstd window", &|p| p.limits.max_zstd_window_log += 1);
    edit("registry root", &|p| {
        p.resources[2].destination_root = "/srv/stacks".into()
    });
    edit("resource order", &|p| p.resources.swap(0, 1));
    edit("mapping order", &|p| p.legacy_mappings.swap(0, 1));
    edit("mapping prefix", &|p| {
        p.legacy_mappings[2].resource_prefix = "media".into()
    });
    edit("mapping target", &|p| {
        p.legacy_mappings[3].resource = "stacks".into()
    });
    edit("extra mapping", &|p| {
        p.legacy_mappings.push(LegacyMapping {
            archive_prefix: "home/pi/extra".into(),
            resource: "app-config".into(),
            resource_prefix: "extra".into(),
        })
    });
    edit("revision", &|p| p.revision += 1);
    out
}

#[test]
fn custody_full_policy_changes_refuse_recovery_and_staging_even_with_the_same_revision() {
    let scene = scene();
    let mut retained = scene.retain(&zst(&sample()), policy());
    let binding = retained.binding().clone();
    for (name, changed) in policy_mutations() {
        assert!(
            matches!(
                scene.recover_err(&binding, changed.clone()),
                CustodyError::PolicyChanged
            ),
            "{name}: recovery"
        );
        assert!(
            matches!(
                retained.stage(&changed, &scene.staging_root(), &never),
                Err(CustodyError::PolicyChanged)
            ),
            "{name}: staging"
        );
        assert!(names(&scene.staging).is_empty(), "{name}");
    }
    // The unchanged policy still works after every refusal.
    let catalog = retained
        .stage(&policy(), &scene.staging_root(), &never)
        .unwrap();
    verify_catalog(&catalog, retained.manifest());
    drop(catalog);
    drop(retained);
    drop(scene.recover(&binding, policy()));
}

#[test]
fn custody_binding_substitution_is_refused_by_exact_binding_validation() {
    let scene = scene();
    let binding = scene.retain(&gz(&sample()), policy()).binding().clone();
    let other = "f".repeat(64);
    type Expect = fn(&CustodyError) -> bool;
    let cases: Vec<(&str, CustodyBinding, Expect)> = vec![
        (
            "unknown record id",
            CustodyBinding {
                record_id: "0".repeat(32),
                ..binding.clone()
            },
            |e| matches!(e, CustodyError::NotFound),
        ),
        (
            "uppercase record id",
            CustodyBinding {
                record_id: binding.record_id.to_uppercase(),
                ..binding.clone()
            },
            |e| matches!(e, CustodyError::MalformedBinding),
        ),
        (
            "short record id",
            CustodyBinding {
                record_id: binding.record_id[..31].into(),
                ..binding.clone()
            },
            |e| matches!(e, CustodyError::MalformedBinding),
        ),
        (
            "path-like record id",
            CustodyBinding {
                record_id: "../../../../../../../tmp/xxxxxx".into(),
                ..binding.clone()
            },
            |e| matches!(e, CustodyError::MalformedBinding),
        ),
        (
            "format version",
            CustodyBinding {
                format_version: 2,
                ..binding.clone()
            },
            |e| matches!(e, CustodyError::MalformedBinding),
        ),
        (
            "zero size",
            CustodyBinding {
                archive_bytes: 0,
                ..binding.clone()
            },
            |e| matches!(e, CustodyError::MalformedBinding),
        ),
        (
            "archive digest",
            CustodyBinding {
                archive_sha256: other.clone(),
                ..binding.clone()
            },
            |e| matches!(e, CustodyError::BindingMismatch),
        ),
        (
            "archive size",
            CustodyBinding {
                archive_bytes: binding.archive_bytes + 1,
                ..binding.clone()
            },
            |e| matches!(e, CustodyError::BindingMismatch),
        ),
        (
            "policy identity",
            CustodyBinding {
                policy_identity: other.clone(),
                ..binding.clone()
            },
            |e| matches!(e, CustodyError::BindingMismatch),
        ),
        (
            "manifest identity",
            CustodyBinding {
                manifest_identity: other.clone(),
                ..binding.clone()
            },
            |e| matches!(e, CustodyError::BindingMismatch),
        ),
        (
            "archive size over custody limit",
            CustodyBinding {
                archive_bytes: custody_limits().max_archive_bytes + 1,
                ..binding.clone()
            },
            |e| matches!(e, CustodyError::Limit(CustodyLimit::Archive)),
        ),
    ];
    for (name, forged, expected) in cases {
        let error = scene.recover_err(&forged, policy());
        assert!(expected(&error), "{name}: {error:?}");
    }
    // Serialized bindings are closed too.
    let json = serde_json::to_value(&binding).unwrap();
    let mut extra = json.clone();
    extra["authorized"] = true.into();
    assert!(serde_json::from_value::<CustodyBinding>(extra).is_err());
    let duplicated =
        serde_json::to_string(&binding)
            .unwrap()
            .replacen('{', r#"{"archive_bytes":1,"#, 1);
    assert!(serde_json::from_str::<CustodyBinding>(&duplicated).is_err());
    drop(scene.recover(&binding, policy()));
}

/// The record's closed shape, rebuilt from public domain types so a forged
/// record can be written in exactly the canonical encoding.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Mirror {
    binding: CustodyBinding,
    policy: AdmissionPolicy,
    manifest: RestoreManifest,
}

#[test]
fn custody_forged_malformed_oversized_and_foreign_records_are_refused() {
    let scene = scene();
    let binding = scene.retain(&gz(&sample()), policy()).binding().clone();
    let record = scene.record_dir(&binding).join("record.json");
    let original = fs::read(&record).unwrap();
    let mirror: Mirror = serde_json::from_slice(&original).unwrap();
    assert_eq!(
        serde_json::to_vec(&mirror).unwrap(),
        original,
        "mirror is canonical"
    );
    let text = String::from_utf8(original.clone()).unwrap();
    let edited = |change: &dyn Fn(&mut Mirror)| {
        let mut changed = mirror.clone();
        change(&mut changed);
        serde_json::to_vec(&changed).unwrap()
    };
    let policy_json = serde_json::to_string(&mirror.policy).unwrap();
    let malformed = [
        (
            "unknown field",
            text.replacen('{', r#"{"success":true,"#, 1).into_bytes(),
        ),
        (
            "unknown nested field",
            text.replacen(
                r#""manifest":{"#,
                r#""manifest":{"restore_authorized":true,"#,
                1,
            )
            .into_bytes(),
        ),
        (
            "duplicate field",
            text.replacen('{', &format!(r#"{{"policy":{policy_json},"#), 1)
                .into_bytes(),
        ),
        ("trailing data", [original.as_slice(), b"{}"].concat()),
        ("trailing whitespace", [original.as_slice(), b"\n"].concat()),
        ("truncated", original[..original.len() - 1].to_vec()),
        ("non-canonical", serde_json::to_vec_pretty(&mirror).unwrap()),
        ("wrong version", edited(&|m| m.binding.format_version = 2)),
        ("not json", b"retained".to_vec()),
    ];
    for (name, bytes) in malformed {
        replace_bytes(&record, &bytes);
        let error = scene.recover_err(&binding, policy());
        assert!(
            matches!(error, CustodyError::MalformedRecord),
            "{name}: {error:?}"
        );
    }
    let entries = mirror.manifest.entries.len();
    let inconsistent = [
        (
            "entry removed",
            edited(&|m| {
                m.manifest.entries.pop();
            }),
        ),
        ("file count", edited(&|m| m.manifest.file_count = 99)),
        (
            "entry checksum",
            edited(&|m| {
                let entry = m
                    .manifest
                    .entries
                    .iter_mut()
                    .find(|e| e.kind == EntryKind::File)
                    .unwrap();
                entry.sha256 = Some("0".repeat(64));
            }),
        ),
        (
            "saved policy root",
            edited(&|m| m.policy.resources[0].destination_root = "/".into()),
        ),
        (
            "saved policy order",
            edited(&|m| m.policy.legacy_mappings.swap(0, 1)),
        ),
        (
            "record identifier",
            edited(&|m| m.binding.record_id = "1".repeat(32)),
        ),
        (
            "record archive digest",
            edited(&|m| m.binding.archive_sha256 = "2".repeat(64)),
        ),
    ];
    assert!(entries > 1);
    for (name, bytes) in inconsistent {
        replace_bytes(&record, &bytes);
        let error = scene.recover_err(&binding, policy());
        assert!(
            matches!(error, CustodyError::BindingMismatch),
            "{name}: {error:?}"
        );
    }
    // A forged success marker or any other foreign entry blocks recovery.
    replace_bytes(&record, &original);
    let marker = scene.record_dir(&binding).join("complete");
    fs::write(&marker, b"verified").unwrap();
    assert!(matches!(
        scene.recover_err(&binding, policy()),
        CustodyError::ForeignEntry
    ));
    fs::remove_file(&marker).unwrap();
    // Oversized records are refused by size, before any parse.
    let small = CustodyLimits {
        max_record_bytes: original.len() as u64 - 1,
        ..custody_limits()
    };
    assert!(matches!(
        recover(&scene.root(), &binding, policy(), &small, &never),
        Err(CustodyError::Limit(CustodyLimit::Record))
    ));
    replace_bytes(&record, &vec![b' '; (MIB + 1) as usize]);
    assert!(matches!(
        scene.recover_err(&binding, policy()),
        CustodyError::Limit(CustodyLimit::Record)
    ));
    // Restoring the exact bytes restores recovery: each refusal was the edit.
    replace_bytes(&record, &original);
    drop(scene.recover(&binding, policy()));
}

#[test]
fn custody_source_replacement_cannot_redirect_the_selected_descriptor() {
    let scene = scene();
    let original = gz(&sample());
    let path = source(&scene.outside, &original);
    let selected = open(&path);
    // Replace the pathname after selection; the held descriptor still wins.
    let hostile = gz(&tar(&[("etc/limeos/core.json", Some(b"replaced"))]));
    let replacement = scene.outside.join("replacement");
    fs::write(&replacement, &hostile).unwrap();
    fs::rename(&replacement, &path).unwrap();
    let retained = retain(selected, policy(), &scene.root(), &custody_limits(), &never).unwrap();
    assert_eq!(retained.binding().archive_sha256, sha(&original));
    let dir = scene.record_dir(retained.binding());
    assert_eq!(fs::read(dir.join("archive")).unwrap(), original);
    assert_eq!(fs::read(&path).unwrap(), hostile);
    retained.discard().unwrap();
    // Only regular files are accepted as selected descriptors.
    for other in [
        File::open(&scene.outside).unwrap(),
        open(Path::new("/dev/null")),
    ] {
        assert!(matches!(
            retain(other, policy(), &scene.root(), &custody_limits(), &never),
            Err(CustodyError::UnsafeSource)
        ));
    }
    assert!(names(&scene.custody).is_empty());
}

#[test]
fn custody_sealed_byte_size_order_and_inode_changes_refuse() {
    let scene = scene();
    let members_ab = [
        ("etc/limeos/a.json", Some(&b"first"[..])),
        ("etc/limeos/b.json", Some(&b"other"[..])),
    ];
    let members_ba = [members_ab[1], members_ab[0]];
    let original = stored_gz(&tar(&members_ab));
    let reordered = stored_gz(&tar(&members_ba));
    assert_eq!(original.len(), reordered.len());
    let a = inspect_and_admit(original.as_slice(), &policy(), &never).unwrap();
    let b = inspect_and_admit(reordered.as_slice(), &policy(), &never).unwrap();
    assert_eq!(a.entries, b.entries, "sorted manifests must match");
    assert_ne!(a.archive_sha256, b.archive_sha256);

    let binding = scene.retain(&original, policy()).binding().clone();
    let archive = scene.record_dir(&binding).join("archive");
    let mut flipped = original.clone();
    let middle = flipped.len() / 2;
    flipped[middle] ^= 0xff;
    let cases: [(&str, Vec<u8>); 4] = [
        ("appended", [original.as_slice(), b"x"].concat()),
        ("truncated", original[..original.len() - 1].to_vec()),
        ("same-size reordered", reordered.clone()),
        ("same-size corrupted", flipped),
    ];
    for (name, bytes) in cases {
        replace_bytes(&archive, &bytes);
        let error = scene.recover_err(&binding, policy());
        assert!(
            matches!(
                error,
                CustodyError::UnsafeObject("archive")
                    | CustodyError::BindingMismatch
                    | CustodyError::Admission(_)
            ),
            "{name}: {error:?}"
        );
        if name == "same-size reordered" {
            assert!(matches!(error, CustodyError::BindingMismatch), "{name}");
        }
    }
    replace_bytes(&archive, &original);

    // While an owner holds the record, any name or byte change blocks staging.
    let mut held = scene.recover(&binding, policy());
    let swap = scene.record_dir(&binding).join("swap");
    fs::write(&swap, &original).unwrap();
    fs::set_permissions(&swap, fs::Permissions::from_mode(0o400)).unwrap();
    fs::rename(&swap, &archive).unwrap();
    assert!(matches!(
        held.stage(&policy(), &scene.staging_root(), &never),
        Err(CustodyError::UnsafeObject("archive"))
    ));
    assert!(names(&scene.staging).is_empty());
    // Discard leaves the foreign replacement and reports it.
    assert!(matches!(
        held.discard(),
        Err(CustodyError::Cleanup {
            failure: None,
            cleanup: CleanupFailure::Replaced("archive")
        })
    ));
    assert_eq!(fs::read(&archive).unwrap(), original);
    // The record name was removed first, so nothing remains recoverable.
    assert!(matches!(
        scene.recover_err(&binding, policy()),
        CustodyError::UnsafeObject("record")
    ));

    // In-place edits to a held inode are caught by replay's recomputation.
    let binding = scene.retain(&original, policy()).binding().clone();
    let archive = scene.record_dir(&binding).join("archive");
    let mut held = scene.recover(&binding, policy());
    replace_bytes(&archive, &reordered);
    assert!(matches!(
        held.stage(&policy(), &scene.staging_root(), &never),
        Err(CustodyError::Staging(StagingError::ManifestMismatch))
    ));
    assert!(names(&scene.staging).is_empty());
}

#[test]
fn custody_root_and_object_substitution_never_follows_clobbers_or_deletes() {
    let scene = scene();
    let parent = scene.custody.parent().unwrap().to_path_buf();
    // Root: relative, symlinked, magic-linked, wrong mode, wrong type, absent.
    let link = parent.join("custody-link");
    symlink(&scene.custody, &link).unwrap();
    let through = link.join(".");
    let wide = parent.join("wide");
    fs::create_dir(&wide).unwrap();
    fs::set_permissions(&wide, fs::Permissions::from_mode(0o750)).unwrap();
    let held = File::open(&scene.custody).unwrap();
    use std::os::fd::AsRawFd;
    let magic = PathBuf::from(format!("/proc/self/fd/{}", held.as_raw_fd()));
    for path in [
        Path::new("relative/custody"),
        &link,
        &through,
        &magic,
        &wide,
        &scene.outside.join("sentinel"),
        &parent.join("absent"),
    ] {
        assert!(
            matches!(CustodyRoot::open(path), Err(CustodyError::UnsafeRoot)),
            "{}",
            path.display()
        );
    }

    // A held root keeps working after its pathname moves.
    let root = scene.root();
    let moved = parent.join("custody-moved");
    fs::rename(&scene.custody, &moved).unwrap();
    let path = source(&scene.outside, &zst(&sample()));
    let retained = retain(open(&path), policy(), &root, &custody_limits(), &never).unwrap();
    let binding = retained.binding().clone();
    drop(retained);
    fs::rename(&moved, &scene.custody).unwrap();
    let dir = scene.record_dir(&binding);
    let pristine = parent.join("pristine");
    fs::create_dir(&pristine).unwrap();
    for name in ["archive", "record.json"] {
        fs::copy(dir.join(name), pristine.join(name)).unwrap();
        fs::set_permissions(pristine.join(name), fs::Permissions::from_mode(0o400)).unwrap();
    }
    fs::set_permissions(&pristine, fs::Permissions::from_mode(0o700)).unwrap();

    let refuse = |what: &str| {
        let error = scene.recover_err(&binding, policy());
        assert!(
            matches!(error, CustodyError::UnsafeObject(_)),
            "{what}: {error:?}"
        );
    };
    // Each object replaced by a symlink to a valid copy is never followed.
    for name in ["archive", "record.json"] {
        let aside = parent.join(format!("{name}.aside"));
        fs::rename(dir.join(name), &aside).unwrap();
        symlink(pristine.join(name), dir.join(name)).unwrap();
        refuse(name);
        fs::remove_file(dir.join(name)).unwrap();
        fs::rename(&aside, dir.join(name)).unwrap();
    }
    // Hard links, wrong modes and wrong types.
    let extra = parent.join("archive-link");
    fs::hard_link(dir.join("archive"), &extra).unwrap();
    refuse("hard link");
    fs::remove_file(&extra).unwrap();
    for (name, wrong) in [("archive", 0o444), ("record.json", 0o600)] {
        fs::set_permissions(dir.join(name), fs::Permissions::from_mode(wrong)).unwrap();
        refuse(name);
        fs::set_permissions(dir.join(name), fs::Permissions::from_mode(0o400)).unwrap();
    }
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    refuse("directory mode");
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    let aside = parent.join("record.aside");
    fs::rename(dir.join("record.json"), &aside).unwrap();
    fs::create_dir(dir.join("record.json")).unwrap();
    refuse("record directory in place of a file");
    fs::remove_dir(dir.join("record.json")).unwrap();
    fs::rename(&aside, dir.join("record.json")).unwrap();
    // The record directory itself replaced by a symlink to a valid copy.
    let aside = parent.join("record-dir.aside");
    fs::rename(&dir, &aside).unwrap();
    symlink(&pristine, &dir).unwrap();
    refuse("record directory symlink");
    fs::remove_file(&dir).unwrap();
    fs::rename(&aside, &dir).unwrap();

    // Foreign entries block recovery, and discard keeps them.
    let retained = scene.recover(&binding, policy());
    fs::write(dir.join("foreign"), b"not ours").unwrap();
    assert!(matches!(
        scene.recover_err(&binding, policy()),
        CustodyError::ForeignEntry
    ));
    assert!(matches!(
        retained.discard(),
        Err(CustodyError::Cleanup {
            failure: None,
            cleanup: CleanupFailure::Foreign(1)
        })
    ));
    assert_eq!(names(&dir), ["foreign"]);
    assert_eq!(fs::read(dir.join("foreign")).unwrap(), b"not ours");
    assert_eq!(
        fs::read(scene.outside.join("sentinel")).unwrap(),
        b"unrelated bytes"
    );
    assert_eq!(names(&pristine), ["archive", "record.json"]);
}

#[test]
fn custody_owners_are_independent_and_drop_never_removes_published_state() {
    let scene = scene();
    let retained = scene.retain(&gz(&sample()), policy());
    let binding = retained.binding().clone();
    let manifest = retained.manifest().clone();
    drop(retained);
    let mut first = scene.recover(&binding, policy());
    let mut second = scene.recover(&binding, policy());
    let one = first
        .stage(&policy(), &scene.staging_root(), &never)
        .unwrap();
    let two = second
        .stage(&policy(), &scene.staging_root(), &never)
        .unwrap();
    assert_eq!(names(&scene.staging).len(), 2);
    verify_catalog(&one, &manifest);
    drop(one);
    verify_catalog(&two, &manifest);
    two.discard().unwrap();
    assert!(names(&scene.staging).is_empty());
    // Catalog cleanup never touches the retained record.
    assert_eq!(
        names(&scene.record_dir(&binding)),
        ["archive", "record.json"]
    );
    first.discard().unwrap();
    assert!(names(&scene.custody).is_empty());
    // The other owner notices its names are gone and cannot stage.
    assert!(matches!(
        second.stage(&policy(), &scene.staging_root(), &never),
        Err(CustodyError::UnsafeObject("archive"))
    ));
    second.discard().unwrap();
    // A replaced record name is reported and retained by discard.
    let retained = scene.retain(&gz(&sample()), policy());
    let dir = scene.record_dir(retained.binding());
    let foreign = dir.join("foreign");
    fs::write(&foreign, b"foreign record").unwrap();
    fs::rename(&foreign, dir.join("record.json")).unwrap();
    assert!(matches!(
        retained.discard(),
        Err(CustodyError::Cleanup {
            failure: None,
            cleanup: CleanupFailure::Replaced("record")
        })
    ));
    assert_eq!(
        fs::read(dir.join("record.json")).unwrap(),
        b"foreign record"
    );
}

#[test]
fn custody_limits_and_policy_are_validated_before_any_custody_state() {
    let scene = scene();
    let path = source(&scene.outside, &gz(&sample()));
    let size = fs::metadata(&path).unwrap().len();
    for limits in [
        CustodyLimits {
            max_archive_bytes: 0,
            ..custody_limits()
        },
        CustodyLimits {
            max_record_bytes: 0,
            ..custody_limits()
        },
        CustodyLimits {
            max_record_bytes: 64 * MIB + 1,
            ..custody_limits()
        },
        CustodyLimits {
            max_archive_bytes: u64::MAX,
            ..custody_limits()
        },
    ] {
        assert!(matches!(
            retain(open(&path), policy(), &scene.root(), &limits, &never),
            Err(CustodyError::InvalidLimits)
        ));
    }
    // Exactly the copy limit is retained; one byte less is refused.
    let exact = CustodyLimits {
        max_archive_bytes: size,
        ..custody_limits()
    };
    retain(open(&path), policy(), &scene.root(), &exact, &never)
        .unwrap()
        .discard()
        .unwrap();
    let under = CustodyLimits {
        max_archive_bytes: size - 1,
        ..custody_limits()
    };
    assert!(matches!(
        retain(open(&path), policy(), &scene.root(), &under, &never),
        Err(CustodyError::Limit(CustodyLimit::Archive))
    ));
    let mut invalid = policy();
    invalid.formats.clear();
    assert!(matches!(
        retain(
            open(&path),
            invalid,
            &scene.root(),
            &custody_limits(),
            &never
        ),
        Err(CustodyError::Admission(_))
    ));
    assert!(matches!(
        retain(
            open(&path),
            policy(),
            &scene.root(),
            &custody_limits(),
            &|| true
        ),
        Err(CustodyError::Cancelled)
    ));
    assert!(names(&scene.custody).is_empty());
}
