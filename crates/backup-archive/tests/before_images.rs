//! Genuine replacement preparation and independent current-target capture.
use limeos_backup_archive::{
    before_images::{BeforeImageRoot, CaptureLimits, Source, capture},
    preparation::{
        self, InstalledMetadata, ManagedTarget, PreparationLimits, PreparationRoot,
        ResourceSelection, Selection,
    },
    staging::{StagingRoot, admit_for_staging, replay},
};
use limeos_domain::backups::{
    AdmissionPolicy, ArchiveFormat, ArchiveLimits, EntryKind, LegacyMapping, ManagedResource,
};
use std::{
    fs::{self, File},
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
};

fn capture_limits() -> CaptureLimits {
    CaptureLimits {
        max_objects: 8,
        max_file_bytes: 1024 * 1024,
        max_total_bytes: 2 * 1024 * 1024,
        max_record_bytes: 65536,
    }
}
fn archive(zstd: bool) -> Vec<u8> {
    let data = b"replacement configuration";
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_mode(0o777);
    header.set_size(data.len() as u64);
    header.set_cksum();
    builder
        .append_data(&mut header, "etc/limeos/core.json", &data[..])
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

#[test]
fn genuine_gzip_and_zstd_replacements_remain_distinct_from_before_images() {
    for zstd in [false, true] {
        let tree = tempfile::tempdir().unwrap();
        for name in ["managed", "staging", "replacement", "before"] {
            fs::create_dir(tree.path().join(name)).unwrap();
            fs::set_permissions(tree.path().join(name), fs::Permissions::from_mode(0o700)).unwrap();
        }
        let destination = tree.path().join("managed");
        fs::write(destination.join("core.json"), b"existing configuration").unwrap();
        fs::set_permissions(
            destination.join("core.json"),
            fs::Permissions::from_mode(0o640),
        )
        .unwrap();
        let policy = AdmissionPolicy {
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
        };
        let bytes = archive(zstd);
        let admitted = admit_for_staging(bytes.as_slice(), policy.clone(), &|| false).unwrap();
        let catalog = replay(
            bytes.as_slice(),
            &policy,
            admitted.manifest(),
            admitted.policy_snapshot(),
            &StagingRoot::open(&tree.path().join("staging")).unwrap(),
            &|| false,
        )
        .unwrap();
        let selection = Selection {
            resources: vec![ResourceSelection {
                id: "configuration".into(),
                destination_root: destination.to_str().unwrap().into(),
                targets: vec![ManagedTarget {
                    relative_path: "core.json".into(),
                    kind: EntryKind::File,
                    installed: InstalledMetadata {
                        uid: 42000,
                        gid: 42001,
                        mode: 0o600,
                    },
                }],
            }],
        };
        let limits = PreparationLimits {
            max_resources: 1,
            max_objects: 1,
            max_file_bytes: 1 << 20,
            max_total_bytes: 2 << 20,
            max_record_bytes: 65536,
        };
        let plan = preparation::plan(
            &catalog,
            catalog.manifest().clone(),
            selection.clone(),
            limits,
        )
        .unwrap();
        let replacement = preparation::prepare(
            &catalog,
            &plan,
            &selection,
            &PreparationRoot::open(&tree.path().join("replacement")).unwrap(),
            &limits,
            &|| false,
        )
        .unwrap();
        let parent = File::open(&destination).unwrap();
        let file = File::open(destination.join("core.json")).unwrap();
        let sources = [Source {
            resource: "configuration",
            relative_path: "core.json",
            parent: &parent,
            name: "core.json",
            object: &file,
        }];
        let before = capture(
            &sources,
            &BeforeImageRoot::open(&tree.path().join("before")).unwrap(),
            capture_limits(),
            &|| false,
        )
        .unwrap();
        assert_eq!(
            before.entries().next().unwrap().metadata().source.mode,
            0o640
        );
        assert_eq!(
            replacement
                .entries()
                .next()
                .unwrap()
                .metadata()
                .installed
                .mode,
            0o600
        );
        let mut existing = Vec::new();
        before
            .entries()
            .next()
            .unwrap()
            .reader()
            .unwrap()
            .unwrap()
            .read_to_end(&mut existing)
            .unwrap();
        let mut future = Vec::new();
        replacement
            .entries()
            .next()
            .unwrap()
            .reader()
            .unwrap()
            .unwrap()
            .read_to_end(&mut future)
            .unwrap();
        assert_eq!(existing, b"existing configuration");
        assert_eq!(future, b"replacement configuration");
        catalog.discard().unwrap();
        replacement.discard().unwrap();
        // Independent copies remain valid after caller closes original handles.
        drop(file);
        drop(parent);
        before.revalidate(&|| false).unwrap();
        before.discard().unwrap();
        assert_eq!(
            fs::read(destination.join("core.json")).unwrap(),
            b"existing configuration"
        );
        for name in ["staging", "replacement", "before"] {
            assert_eq!(fs::read_dir(tree.path().join(name)).unwrap().count(), 0);
        }
    }
}

#[test]
fn frozen_layout_configuration_bytes_are_preserved_without_rewriting_ownership() {
    let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../tests/fixtures/storage-contracts.json"
    ))
    .unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 3);
    for case in cases {
        let tree = tempfile::tempdir().unwrap();
        let private = tree.path().join("private");
        fs::create_dir(&private).unwrap();
        fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
        let original = serde_json::to_vec(&case["contract"]).unwrap();
        fs::write(tree.path().join("contract.json"), &original).unwrap();
        let parent = File::open(tree.path()).unwrap();
        let file = File::open(tree.path().join("contract.json")).unwrap();
        let sources = [Source {
            resource: case["contract"]["profile"].as_str().unwrap(),
            relative_path: "contract.json",
            parent: &parent,
            name: "contract.json",
            object: &file,
        }];
        let before = capture(
            &sources,
            &BeforeImageRoot::open(&private).unwrap(),
            capture_limits(),
            &|| false,
        )
        .unwrap();
        let mut copied = Vec::new();
        before
            .entries()
            .next()
            .unwrap()
            .reader()
            .unwrap()
            .unwrap()
            .read_to_end(&mut copied)
            .unwrap();
        assert_eq!(copied, original);
        let parsed: serde_json::Value = serde_json::from_slice(&copied).unwrap();
        assert_eq!(parsed["media_identity"], case["contract"]["media_identity"]);
        assert_eq!(parsed["devices"], case["contract"]["devices"]);
        before.discard().unwrap();
        assert_eq!(fs::read_dir(private).unwrap().count(), 0);
    }
}
