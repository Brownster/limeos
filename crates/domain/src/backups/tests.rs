use super::*;

const DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn limits() -> ArchiveLimits {
    ArchiveLimits {
        max_compressed_bytes: 1 << 20,
        max_decompressed_bytes: 1 << 22,
        max_file_bytes: 1 << 20,
        max_entries: 64,
        max_path_bytes: 256,
        max_path_depth: 8,
        max_component_bytes: 64,
        max_metadata_bytes: 4096,
        max_total_metadata_bytes: 16384,
        max_zstd_window_log: 20,
    }
}

fn policy() -> AdmissionPolicy {
    AdmissionPolicy {
        revision: 7,
        formats: vec![ArchiveFormat::TarGzip, ArchiveFormat::TarZstd],
        limits: limits(),
        resources: vec![
            ManagedResource {
                id: "limeos-config".into(),
                destination_root: "/etc/limeos".into(),
            },
            ManagedResource {
                id: "stacks".into(),
                destination_root: "/opt/stacks".into(),
            },
            ManagedResource {
                id: "credentials".into(),
                destination_root: "/etc/limeos-credentials".into(),
            },
        ],
        legacy_mappings: vec![
            LegacyMapping {
                archive_prefix: "etc/limeos".into(),
                resource: "limeos-config".into(),
                resource_prefix: "".into(),
            },
            LegacyMapping {
                archive_prefix: "opt/stacks".into(),
                resource: "stacks".into(),
                resource_prefix: "".into(),
            },
            LegacyMapping {
                archive_prefix: "etc/limeos/credentials.env".into(),
                resource: "credentials".into(),
                resource_prefix: "legacy/credentials.env".into(),
            },
        ],
    }
}

fn file(index: u64, path: &str, size: u64) -> InspectedEntry {
    InspectedEntry {
        header_index: index,
        archive_path: path.into(),
        kind: EntryKind::File,
        size,
        sha256: Some(DIGEST.into()),
        archived_mode: 0o644,
    }
}

fn dir(index: u64, path: &str) -> InspectedEntry {
    InspectedEntry {
        header_index: index,
        archive_path: path.into(),
        kind: EntryKind::Directory,
        size: 0,
        sha256: None,
        archived_mode: 0o755,
    }
}

fn report(entries: Vec<InspectedEntry>) -> InspectionReport {
    let header_count = entries
        .iter()
        .map(|e| e.header_index + 1)
        .max()
        .unwrap_or(0);
    InspectionReport {
        format: ArchiveFormat::TarZstd,
        archive_sha256: DIGEST.replace('0', "a"),
        compressed_bytes: 100,
        decompressed_bytes: 10240,
        header_count,
        metadata_bytes: 0,
        entries,
    }
}

fn codes(result: Result<RestoreManifest, Rejection>) -> Vec<FindingCode> {
    result.expect_err("must be rejected").codes()
}

#[test]
fn representative_legacy_layout_maps_to_managed_resources_deterministically() {
    let entries = vec![
        dir(0, "etc/limeos/"),
        file(1, "etc/limeos/core.json", 10),
        file(2, "etc/limeos/credentials.env", 4),
        dir(3, "opt/stacks/"),
        dir(4, "opt/stacks/media/"),
        file(5, "opt/stacks/media/compose.yaml", 20),
    ];
    let manifest = admit(&policy(), &report(entries.clone())).unwrap();
    let pairs: Vec<_> = manifest
        .entries
        .iter()
        .map(|e| (e.resource.as_str(), e.relative_path.as_str()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("credentials", "legacy/credentials.env"),
            ("limeos-config", ""),
            ("limeos-config", "core.json"),
            ("stacks", ""),
            ("stacks", "media"),
            ("stacks", "media/compose.yaml"),
        ]
    );
    assert_eq!(
        (
            manifest.file_count,
            manifest.directory_count,
            manifest.file_bytes
        ),
        (3, 3, 34)
    );
    assert_eq!(manifest.policy_revision, 7);
    assert_eq!(manifest.archive_sha256, DIGEST.replace('0', "a"));
    // Order of members does not change the manifest.
    let mut reversed = entries;
    reversed.reverse();
    for (i, e) in reversed.iter_mut().enumerate() {
        e.header_index = i as u64;
    }
    assert_eq!(
        admit(&policy(), &report(reversed)).unwrap().entries,
        manifest.entries
    );
}

#[test]
fn unsafe_names_are_rejected_with_specific_findings() {
    let cases = [
        ("/etc/limeos/core.json", FindingCode::AbsolutePath),
        ("etc/limeos/../shadow", FindingCode::ParentComponent),
        ("../etc/limeos/core.json", FindingCode::ParentComponent),
        ("etc/limeos/./core.json", FindingCode::DotComponent),
        ("./etc/limeos/core.json", FindingCode::DotComponent),
        ("etc//limeos/core.json", FindingCode::EmptyComponent),
        (
            "etc/limeos/core\u{1b}[31m.json",
            FindingCode::UnsafeCharacter,
        ),
        ("etc/limeos/core\u{85}.json", FindingCode::UnsafeCharacter),
        ("etc/limeos/a\\b.json", FindingCode::UnsafeCharacter),
        ("etc/limeos/core.json/", FindingCode::TrailingSlashOnFile),
        ("etc/limeos/x", FindingCode::Unmapped),
    ];
    for (path, expected) in cases {
        let mut entries = vec![file(0, path, 1)];
        if path == "etc/limeos/x" {
            entries = vec![file(0, "etc/limeos2/x", 1)];
        }
        assert_eq!(
            codes(admit(&policy(), &report(entries))),
            [expected],
            "{path}"
        );
    }
    assert_eq!(
        codes(admit(&policy(), &report(vec![dir(0, "/")]))),
        [FindingCode::EmptyComponent]
    );
}

#[test]
fn name_limits_are_enforced_from_policy() {
    let long = format!("opt/stacks/{}", "a".repeat(65));
    assert_eq!(
        codes(admit(&policy(), &report(vec![file(0, &long, 1)]))),
        [FindingCode::ComponentTooLong]
    );
    let deep = format!("opt/stacks/{}", ["d"; 8].join("/"));
    assert_eq!(
        codes(admit(&policy(), &report(vec![file(0, &deep, 1)]))),
        [FindingCode::PathTooDeep]
    );
    let huge = format!("opt/stacks/{}", ["abcdefgh"; 30].join("/"));
    assert_eq!(
        codes(admit(&policy(), &report(vec![file(0, &huge, 1)]))),
        [FindingCode::PathTooLong]
    );
}

#[test]
fn literal_case_sensitive_paths_are_preserved() {
    let manifest = admit(
        &policy(),
        &report(vec![
            file(0, "opt/stacks/Media.yaml", 1),
            file(1, "opt/stacks/media.yaml", 1),
        ]),
    )
    .unwrap();
    let paths: Vec<_> = manifest
        .entries
        .iter()
        .map(|e| e.relative_path.as_str())
        .collect();
    assert_eq!(paths, ["Media.yaml", "media.yaml"]);
}

#[test]
fn duplicates_and_collisions_are_rejected_in_either_order() {
    let dup = vec![file(0, "opt/stacks/a", 1), file(1, "opt/stacks/a", 1)];
    assert_eq!(
        codes(admit(&policy(), &report(dup))),
        [FindingCode::DuplicateDestination]
    );
    let dup_dir = vec![dir(0, "opt/stacks/a/"), dir(1, "opt/stacks/a/")];
    assert_eq!(
        codes(admit(&policy(), &report(dup_dir))),
        [FindingCode::DuplicateDestination]
    );
    for entries in [
        vec![file(0, "opt/stacks/a", 1), dir(1, "opt/stacks/a/")],
        vec![dir(0, "opt/stacks/a/"), file(1, "opt/stacks/a", 1)],
    ] {
        assert_eq!(
            codes(admit(&policy(), &report(entries))),
            [FindingCode::FileDirectoryCollision]
        );
    }
    for entries in [
        vec![file(0, "opt/stacks/a", 1), file(1, "opt/stacks/a/b/c", 1)],
        vec![file(0, "opt/stacks/a/b/c", 1), file(1, "opt/stacks/a", 1)],
    ] {
        assert_eq!(
            codes(admit(&policy(), &report(entries))),
            [FindingCode::ParentIsFile]
        );
    }
    // A file may not replace a resource root, through any mapping.
    assert_eq!(
        codes(admit(&policy(), &report(vec![file(0, "opt/stacks", 1)]))),
        [FindingCode::FileDirectoryCollision]
    );
}

#[test]
fn every_destination_finding_is_reported_and_no_manifest_returned() {
    let entries = vec![
        file(0, "/etc/passwd", 1),
        file(1, "home/pi/.ssh/authorized_keys", 1),
        file(2, "opt/stacks/a", 1),
        file(3, "opt/stacks/a", 1),
    ];
    let rejection = admit(&policy(), &report(entries)).unwrap_err();
    assert_eq!(
        rejection.codes(),
        [
            FindingCode::AbsolutePath,
            FindingCode::Unmapped,
            FindingCode::DuplicateDestination
        ]
    );
    assert_eq!(rejection.findings[1].entry, Some(1));
}

#[test]
fn archived_special_bits_and_ownership_never_reach_the_manifest() {
    let mut entry = file(0, "opt/stacks/tool", 1);
    entry.archived_mode = 0o6777;
    let manifest = admit(&policy(), &report(vec![entry])).unwrap();
    assert_eq!(manifest.entries[0].archived_permissions, 0o777);
    let json = serde_json::to_string(&manifest).unwrap();
    assert!(!json.contains("uid") && !json.contains("owner"));
}

#[test]
fn inconsistent_or_partial_reports_are_refused() {
    let mut bad = report(vec![file(0, "opt/stacks/a", 1)]);
    bad.archive_sha256 = "ABC".into();
    assert_eq!(
        codes(admit(&policy(), &bad)),
        [FindingCode::InconsistentReport]
    );
    let mut missing = report(vec![file(0, "opt/stacks/a", 1)]);
    missing.entries[0].sha256 = None;
    assert_eq!(
        codes(admit(&policy(), &missing)),
        [FindingCode::InconsistentReport]
    );
    let mut dir_data = report(vec![dir(0, "opt/stacks/a/")]);
    dir_data.entries[0].size = 1;
    assert_eq!(
        codes(admit(&policy(), &dir_data)),
        [FindingCode::InconsistentReport]
    );
    let mut order = report(vec![file(1, "opt/stacks/a", 1), file(0, "opt/stacks/b", 1)]);
    order.header_count = 2;
    assert_eq!(
        codes(admit(&policy(), &order)),
        [FindingCode::InconsistentReport]
    );
    let mut oversize = report(vec![file(0, "opt/stacks/a", 2 << 20)]);
    oversize.decompressed_bytes = 4 << 20;
    assert_eq!(
        codes(admit(&policy(), &oversize)),
        [FindingCode::InconsistentReport]
    );
    let mut overflow = report(vec![
        file(0, "opt/stacks/a", 1 << 20),
        file(1, "opt/stacks/b", 1 << 20),
    ]);
    overflow.decompressed_bytes = 1 << 20;
    assert_eq!(
        codes(admit(&policy(), &overflow)),
        [FindingCode::InconsistentReport]
    );
    let mut format = report(vec![]);
    format.format = ArchiveFormat::TarGzip;
    let mut zstd_only = policy();
    zstd_only.formats = vec![ArchiveFormat::TarZstd];
    assert_eq!(
        codes(admit(&zstd_only, &format)),
        [FindingCode::FormatNotAllowed]
    );
}

#[test]
fn broad_overlapping_or_open_policies_are_invalid() {
    let invalid =
        |p: &AdmissionPolicy| p.validate().unwrap_err().codes() == [FindingCode::InvalidPolicy];
    for prefix in [
        "etc",
        "home",
        "home/pi",
        "var/lib",
        "var/log",
        "",
        "/etc/limeos",
        "etc/../x",
    ] {
        let mut p = policy();
        p.legacy_mappings[0].archive_prefix = prefix.into();
        assert!(invalid(&p), "{prefix:?}");
    }
    let mut overlap = policy();
    overlap.resources[1].destination_root = "/etc/limeos/stacks".into();
    assert!(invalid(&overlap));
    let mut relative = policy();
    relative.resources[0].destination_root = "etc/limeos".into();
    assert!(invalid(&relative));
    let mut unknown = policy();
    unknown.legacy_mappings[0].resource = "nobody".into();
    assert!(invalid(&unknown));
    let mut twice = policy();
    twice.legacy_mappings.push(twice.legacy_mappings[0].clone());
    assert!(invalid(&twice));
    let mut zero = policy();
    zero.limits.max_entries = 0;
    assert!(invalid(&zero));
    let mut window = policy();
    window.limits.max_zstd_window_log = 31;
    assert!(invalid(&window));
    let mut none = policy();
    none.formats.clear();
    assert!(invalid(&none));
    assert!(policy().validate().is_ok());
}

#[test]
fn reports_render_hostile_paths_safely() {
    let rendered = display_path(&format!("a\u{1b}[2J\n{}", "x".repeat(2000)));
    assert!(!rendered.chars().any(|c| c.is_control()));
    assert!(rendered.len() <= 515);
}
