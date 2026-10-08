//! Real catalog identity regressions for private preparation.

use limeos_backup_archive::staging::{StagingRoot, admit_for_staging, replay};
use limeos_domain::backups::{
    AdmissionPolicy, ArchiveFormat, ArchiveLimits, LegacyMapping, ManagedResource,
};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};

fn policy(destination: &Path) -> AdmissionPolicy {
    AdmissionPolicy {
        revision: 1,
        formats: vec![ArchiveFormat::TarGzip, ArchiveFormat::TarZstd],
        limits: ArchiveLimits {
            max_compressed_bytes: 1 << 20,
            max_decompressed_bytes: 4 << 20,
            max_file_bytes: 1 << 20,
            max_entries: 16,
            max_path_bytes: 256,
            max_path_depth: 16,
            max_component_bytes: 128,
            max_metadata_bytes: 4096,
            max_total_metadata_bytes: 16384,
            max_zstd_window_log: 21,
        },
        resources: vec![ManagedResource {
            id: "configuration".into(),
            destination_root: destination.to_str().unwrap().into(),
        }],
        legacy_mappings: vec![LegacyMapping {
            archive_prefix: "etc/limeos".into(),
            resource: "configuration".into(),
            resource_prefix: String::new(),
        }],
    }
}

fn archive(zstd: bool) -> Vec<u8> {
    archive_data(zstd, b"original")
}
fn archive_data(zstd: bool, data: &[u8]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_mode(0o644);
    header.set_size(data.len() as u64);
    header.set_cksum();
    builder
        .append_data(&mut header, "etc/limeos/core.json", data)
        .unwrap();
    let tar = builder.into_inner().unwrap();
    if zstd {
        zstd::stream::encode_all(tar.as_slice(), 3).unwrap()
    } else {
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(&tar).unwrap();
        gzip.finish().unwrap()
    }
}

fn probe(zstd: bool, replace: bool) {
    let temp = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let destination = temp.path().join("managed");
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("core.json"), b"live unchanged").unwrap();
    fs::write(temp.path().join("unrelated"), b"sentinel").unwrap();
    let staging = temp.path().join("staging");
    fs::create_dir(&staging).unwrap();
    fs::set_permissions(&staging, fs::Permissions::from_mode(0o700)).unwrap();
    let bytes = archive(zstd);
    let policy = policy(&destination);
    let admitted = admit_for_staging(bytes.as_slice(), policy.clone(), &|| false).unwrap();
    let catalog = replay(
        bytes.as_slice(),
        &policy,
        admitted.manifest(),
        admitted.policy_snapshot(),
        &StagingRoot::open(&staging).unwrap(),
        &|| false,
    )
    .unwrap();
    let attempt = fs::read_dir(&staging)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let file = fs::read_dir(&attempt)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let original_inode = fs::metadata(&file).unwrap().ino();
    if replace {
        fs::rename(&file, attempt.join("displaced-original")).unwrap();
        fs::write(&file, b"foreign replacement").unwrap();
        assert_ne!(fs::metadata(&file).unwrap().ino(), original_inode);
    } else {
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        OpenOptions::new()
            .append(true)
            .open(&file)
            .unwrap()
            .write_all(b" appended after staging")
            .unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o400)).unwrap();
        assert_eq!(fs::metadata(&file).unwrap().len(), 31);
    }
    let mut observed = Vec::new();
    catalog
        .entries()
        .next()
        .unwrap()
        .reader()
        .unwrap()
        .read_to_end(&mut observed)
        .unwrap();
    assert_eq!(observed, b"original");
    assert!(catalog.revalidate(&|| false).is_err());
    println!(
        "{} {}: catalog reader returns original 8 bytes without reporting changed object",
        if zstd { "zstd" } else { "gzip" },
        if replace {
            "replaced name"
        } else {
            "appended payload"
        }
    );
    if replace {
        assert!(catalog.discard().is_err());
        assert_eq!(fs::read(&file).unwrap(), b"foreign replacement");
    } else {
        catalog.discard().unwrap();
        assert_eq!(fs::read_dir(&staging).unwrap().count(), 0);
    }
    assert_eq!(
        fs::read(destination.join("core.json")).unwrap(),
        b"live unchanged"
    );
    assert_eq!(
        fs::read(temp.path().join("unrelated")).unwrap(),
        b"sentinel"
    );
}

#[test]
fn gzip_reader_does_not_report_appended_payload() {
    probe(false, false);
}

#[test]
fn zstd_reader_does_not_report_appended_payload() {
    probe(true, false);
}

#[test]
fn borrowed_reader_does_not_report_replaced_name() {
    probe(false, true);
}

use limeos_backup_archive::preparation::{
    Failure, InstalledMetadata, ManagedTarget, PreparationLimits, PreparationRoot,
    ResourceSelection, Selection, plan, prepare,
};
use limeos_backup_archive::staging::VerifiedCatalog;
use limeos_domain::backups::EntryKind;
use std::{
    os::{fd::AsRawFd, unix::fs::symlink},
    path::PathBuf,
};

struct Scene {
    temp: tempfile::TempDir,
    staging: PathBuf,
    private: PathBuf,
    destination: PathBuf,
    policy: AdmissionPolicy,
    catalog: VerifiedCatalog,
    selection: Selection,
    limits: PreparationLimits,
}
impl Scene {
    fn new(zstd: bool, data: &[u8]) -> Self {
        let temp = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let destination = temp.path().join("managed");
        let staging = temp.path().join("staging");
        let private = temp.path().join("preparation");
        for path in [&destination, &staging, &private] {
            fs::create_dir(path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::write(destination.join("core.json"), b"live unchanged").unwrap();
        fs::write(temp.path().join("unrelated"), b"sentinel").unwrap();
        let policy = policy(&destination);
        let bytes = archive_data(zstd, data);
        let admitted = admit_for_staging(bytes.as_slice(), policy.clone(), &|| false).unwrap();
        let catalog = replay(
            bytes.as_slice(),
            &policy,
            admitted.manifest(),
            admitted.policy_snapshot(),
            &StagingRoot::open(&staging).unwrap(),
            &|| false,
        )
        .unwrap();
        let target = |relative_path: &str, kind| ManagedTarget {
            relative_path: relative_path.into(),
            kind,
            installed: InstalledMetadata {
                uid: 42000,
                gid: 42001,
                mode: if kind == EntryKind::File {
                    0o640
                } else {
                    0o750
                },
            },
        };
        let selection = Selection {
            resources: vec![ResourceSelection {
                id: "configuration".into(),
                destination_root: destination.to_str().unwrap().into(),
                targets: vec![
                    target("", EntryKind::Directory),
                    target("core.json", EntryKind::File),
                ],
            }],
        };
        let limits = PreparationLimits {
            max_resources: 4,
            max_objects: 16,
            max_file_bytes: 1 << 20,
            max_total_bytes: 2 << 20,
            max_record_bytes: 1 << 20,
        };
        Self {
            temp,
            staging,
            private,
            destination,
            policy,
            catalog,
            selection,
            limits,
        }
    }
    fn unchanged(&self) {
        assert_eq!(
            fs::read(self.destination.join("core.json")).unwrap(),
            b"live unchanged"
        );
        assert_eq!(
            fs::read(self.temp.path().join("unrelated")).unwrap(),
            b"sentinel"
        );
    }
}

#[test]
fn gzip_and_zstd_copies_bind_trusted_metadata_and_are_independent() {
    for zstd in [false, true] {
        let scene = Scene::new(zstd, b"replacement bytes");
        let root = PreparationRoot::open(&scene.private).unwrap();
        let plan = plan(
            &scene.catalog,
            scene.catalog.manifest().clone(),
            scene.selection.clone(),
            scene.limits,
        )
        .unwrap();
        let first = prepare(
            &scene.catalog,
            &plan,
            &scene.selection,
            &root,
            &scene.limits,
            &|| false,
        )
        .unwrap();
        let second = prepare(
            &scene.catalog,
            &plan,
            &scene.selection,
            &root,
            &scene.limits,
            &|| false,
        )
        .unwrap();
        assert_eq!(first.record(), second.record());
        assert_eq!(first.record_sha256(), second.record_sha256());
        assert_ne!(plan.identity(), first.record_sha256());
        first.revalidate(&|| false).unwrap();
        let entries: Vec<_> = first.entries().collect();
        assert!(entries[0].reader().unwrap().is_none());
        let metadata = entries[1].metadata();
        assert_eq!(metadata.installed.uid, 42000);
        assert_eq!(metadata.installed.gid, 42001);
        assert_eq!(metadata.installed.mode, 0o640);
        assert_eq!(metadata.private_uid, rustix::process::geteuid().as_raw());
        assert_eq!(metadata.private_mode, 0o400);
        let mut bytes = Vec::new();
        entries[1]
            .reader()
            .unwrap()
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, b"replacement bytes");
        scene.unchanged();
        drop(entries);
        first.discard().unwrap();
        drop(second);
        assert_eq!(fs::read_dir(&scene.private).unwrap().count(), 0);
    }
}

#[test]
fn empty_exact_and_chunk_boundary_payloads() {
    for size in [0, 1, 65535, 65536, 65537] {
        let data = vec![b'X'; size];
        let scene = Scene::new(size % 2 == 0, &data);
        let mut limits = scene.limits;
        limits.max_file_bytes = size.max(1) as u64;
        limits.max_total_bytes = size.max(1) as u64;
        let bound = plan(
            &scene.catalog,
            scene.catalog.manifest().clone(),
            scene.selection.clone(),
            limits,
        )
        .unwrap();
        let prepared = prepare(
            &scene.catalog,
            &bound,
            &scene.selection,
            &PreparationRoot::open(&scene.private).unwrap(),
            &limits,
            &|| false,
        )
        .unwrap();
        let entry = prepared
            .entries()
            .find(|e| e.metadata().kind == EntryKind::File)
            .unwrap();
        let mut actual = Vec::new();
        entry
            .reader()
            .unwrap()
            .unwrap()
            .read_to_end(&mut actual)
            .unwrap();
        assert_eq!(actual, data);
        scene.unchanged();
    }
}

#[test]
fn closed_selection_rejects_unknown_duplicate_absent_and_unsafe_targets() {
    let scene = Scene::new(false, b"original");
    for case in 0..11 {
        let mut selection = scene.selection.clone();
        let resource = &mut selection.resources[0];
        match case {
            0 => resource.id = "unknown".into(),
            1 => {
                let duplicate = resource.clone();
                selection.resources.push(duplicate);
            }
            2 => resource.targets.pop().map(|_| ()).unwrap(),
            3 => resource.targets.push(resource.targets[1].clone()),
            4 => resource.targets[1].relative_path = "missing".into(),
            5 => resource.targets[1].kind = EntryKind::Directory,
            6 => resource.targets[1].relative_path = "../escape".into(),
            7 => resource.targets[1].relative_path = "/absolute".into(),
            8 => resource.targets[1].installed.mode = 0o4640,
            9 => resource.targets[1].installed.uid = u32::MAX,
            10 => resource.destination_root = "/".into(),
            _ => unreachable!(),
        }
        assert!(
            plan(
                &scene.catalog,
                scene.catalog.manifest().clone(),
                selection,
                scene.limits
            )
            .is_err(),
            "case {case}"
        );
        assert_eq!(fs::read_dir(&scene.private).unwrap().count(), 0);
        scene.unchanged();
    }
}

#[test]
fn changed_policy_and_manifest_refuse_before_private_objects() {
    let scene = Scene::new(false, b"original");
    let bound = plan(
        &scene.catalog,
        scene.catalog.manifest().clone(),
        scene.selection.clone(),
        scene.limits,
    )
    .unwrap();
    let root = PreparationRoot::open(&scene.private).unwrap();
    let mut current = scene.selection.clone();
    current.resources[0].targets[1].installed.mode ^= 0o010;
    assert!(matches!(
        prepare(
            &scene.catalog,
            &bound,
            &current,
            &root,
            &scene.limits,
            &|| false
        )
        .unwrap_err()
        .failure,
        Failure::PolicyChanged
    ));
    let mut limits = scene.limits;
    limits.max_total_bytes += 1;
    assert!(matches!(
        prepare(
            &scene.catalog,
            &bound,
            &scene.selection,
            &root,
            &limits,
            &|| false
        )
        .unwrap_err()
        .failure,
        Failure::PolicyChanged
    ));
    let mut expected = scene.catalog.manifest().clone();
    expected.entries[0].archived_permissions ^= 0o010;
    assert!(matches!(
        plan(
            &scene.catalog,
            expected,
            scene.selection.clone(),
            scene.limits
        )
        .unwrap_err()
        .failure,
        Failure::ManifestMismatch
    ));
    assert_eq!(fs::read_dir(&scene.private).unwrap().count(), 0);
    scene.unchanged();
}

#[test]
fn root_protection_and_held_root_rename() {
    let scene = Scene::new(false, b"original");
    let root = PreparationRoot::open(&scene.private).unwrap();
    let alias = scene.temp.path().join("alias");
    symlink(&scene.private, &alias).unwrap();
    assert!(PreparationRoot::open(&alias).is_err());
    assert!(PreparationRoot::open(&alias.join("subdir")).is_err());
    assert!(PreparationRoot::open(Path::new("relative")).is_err());
    let directory_fd = std::fs::File::open(&scene.private).unwrap();
    assert!(
        PreparationRoot::open(Path::new(&format!(
            "/proc/self/fd/{}",
            directory_fd.as_raw_fd()
        )))
        .is_err()
    );
    assert!(PreparationRoot::open(&scene.destination.join("core.json")).is_err());
    fs::set_permissions(&scene.private, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(PreparationRoot::open(&scene.private).is_err());
    fs::set_permissions(&scene.private, fs::Permissions::from_mode(0o700)).unwrap();
    let moved = scene.temp.path().join("moved-private");
    fs::rename(&scene.private, &moved).unwrap();
    fs::create_dir(&scene.private).unwrap();
    fs::write(scene.private.join("foreign"), b"sentinel").unwrap();
    let bound = plan(
        &scene.catalog,
        scene.catalog.manifest().clone(),
        scene.selection.clone(),
        scene.limits,
    )
    .unwrap();
    let prepared = prepare(
        &scene.catalog,
        &bound,
        &scene.selection,
        &root,
        &scene.limits,
        &|| false,
    )
    .unwrap();
    assert_eq!(fs::read_dir(&moved).unwrap().count(), 1);
    assert_eq!(
        fs::read(scene.private.join("foreign")).unwrap(),
        b"sentinel"
    );
    prepared.discard().unwrap();
    scene.unchanged();
}

#[test]
fn size_object_and_record_limits_refuse_without_owner() {
    let scene = Scene::new(false, b"original");
    for case in 0..5 {
        let mut limits = scene.limits;
        match case {
            0 => limits.max_file_bytes = 7,
            1 => limits.max_total_bytes = 7,
            2 => limits.max_objects = 1,
            3 => limits.max_record_bytes = 1,
            4 => limits.max_total_bytes = u64::MAX,
            _ => unreachable!(),
        }
        assert!(
            plan(
                &scene.catalog,
                scene.catalog.manifest().clone(),
                scene.selection.clone(),
                limits
            )
            .is_err()
        );
        assert_eq!(fs::read_dir(&scene.private).unwrap().count(), 0);
        scene.unchanged();
    }
}
#[test]
fn private_root_must_be_disjoint_from_selected_managed_roots() {
    let scene = Scene::new(false, b"original");
    let mut selection = scene.selection.clone();
    selection.resources[0].destination_root = scene.private.to_str().unwrap().into();
    let bound = plan(
        &scene.catalog,
        scene.catalog.manifest().clone(),
        selection.clone(),
        scene.limits,
    )
    .unwrap();
    assert!(matches!(
        prepare(
            &scene.catalog,
            &bound,
            &selection,
            &PreparationRoot::open(&scene.private).unwrap(),
            &scene.limits,
            &|| false
        )
        .unwrap_err()
        .failure,
        Failure::InvalidPolicy
    ));
    assert_eq!(fs::read_dir(&scene.private).unwrap().count(), 0);
    scene.unchanged();
}

#[test]
fn genuine_catalog_from_fresh_custody_recovery_prepares() {
    use limeos_backup_archive::custody::{CustodyLimits, CustodyRoot, recover, retain};
    let scene = Scene::new(true, b"original");
    let custody_path = scene.temp.path().join("custody");
    fs::create_dir(&custody_path).unwrap();
    fs::set_permissions(&custody_path, fs::Permissions::from_mode(0o700)).unwrap();
    let source_path = scene.temp.path().join("archive");
    fs::write(&source_path, archive(true)).unwrap();
    let root = CustodyRoot::open(&custody_path).unwrap();
    let limits = CustodyLimits {
        max_archive_bytes: 1 << 20,
        max_record_bytes: 1 << 20,
    };
    let retained = retain(
        std::fs::File::open(source_path).unwrap(),
        scene.policy.clone(),
        &root,
        &limits,
        &|| false,
    )
    .unwrap();
    let binding = retained.binding().clone();
    drop(retained);
    let mut recovered = recover(&root, &binding, scene.policy.clone(), &limits, &|| false).unwrap();
    let catalog = recovered
        .stage(
            &scene.policy,
            &StagingRoot::open(&scene.staging).unwrap(),
            &|| false,
        )
        .unwrap();
    let bound = plan(
        &catalog,
        catalog.manifest().clone(),
        scene.selection.clone(),
        scene.limits,
    )
    .unwrap();
    let prepared = prepare(
        &catalog,
        &bound,
        &scene.selection,
        &PreparationRoot::open(&scene.private).unwrap(),
        &scene.limits,
        &|| false,
    )
    .unwrap();
    catalog.discard().unwrap();
    recovered.discard().unwrap();
    prepared.revalidate(&|| false).unwrap();
    let mut result = Vec::new();
    prepared
        .entries()
        .find(|e| e.metadata().kind == EntryKind::File)
        .unwrap()
        .reader()
        .unwrap()
        .unwrap()
        .read_to_end(&mut result)
        .unwrap();
    assert_eq!(result, b"original");
    prepared.discard().unwrap();
    scene.unchanged();
}
