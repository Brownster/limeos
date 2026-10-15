use super::*;
use serde_json::Value;

fn contract(index: usize) -> StorageContract {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/storage-contracts.json"
    ))
    .unwrap();
    serde_json::from_value(fixture["cases"][index]["contract"].clone()).unwrap()
}
fn identity(application: MediaService) -> ServiceIdentity {
    ServiceIdentity {
        stack: "media".into(),
        engine_id: "engine-a".into(),
        project: ProjectIdentity {
            name: "media".into(),
            working_directory: "/opt/stacks/Media".into(),
            config_files: vec![
                "/opt/stacks/Media/compose.yaml".into(),
                "/opt/stacks/Media/limeos.override.json".into(),
            ],
        },
        service: "selected-service".into(),
        application,
    }
}
fn bind(source: &str, target: &str, read_only: bool) -> ComposeMount {
    ComposeMount::Bind {
        source: source.into(),
        target: target.into(),
        read_only,
    }
}
fn context<'a>(
    c: &StorageContract,
    id: &'a ServiceIdentity,
    mounts: &'a [ComposeMount],
) -> SeedContext<'a> {
    SeedContext {
        identity: id,
        declared_media_identity: Some(c.media_identity.clone()),
        mounts,
    }
}
fn observation<'a>(id: &'a ServiceIdentity, records: Records<'a>) -> Observation<'a> {
    Observation {
        identity: id,
        sampled_at: 100,
        completeness: Completeness::Complete,
        records,
    }
}
fn run(
    c: &StorageContract,
    id: &ServiceIdentity,
    mounts: &[ComposeMount],
    records: Records<'_>,
) -> SeedPreview {
    preview(
        101,
        c,
        context(c, id, mounts),
        Some(observation(id, records)),
    )
    .unwrap()
}
fn roots() -> Vec<RootFolder> {
    vec![
        RootFolder {
            id: 1,
            path: "/downloads/complete/sonarr".into(),
        },
        RootFolder {
            id: 2,
            path: "/OtherLibrary".into(),
        },
    ]
}

#[test]
fn all_existing_profiles_keep_numeric_identity_and_canonical_paths() {
    for index in 0..3 {
        let c = contract(index);
        c.validate().unwrap();
        let id = identity(MediaService::Sonarr);
        let mounts = [
            bind(&c.locations.media_host, "/data/media", false),
            bind(&c.locations.downloads_host, "/data/downloads", false),
        ];
        let report = run(&c, &id, &mounts, Records::Roots(&[]));
        assert_eq!(report.outcome, Outcome::ChangesProposed);
        assert_eq!(
            report.additions[0].host,
            format!("{}/tv", c.locations.media_host)
        );
        assert_eq!(report.additions[0].container, "/data/media/tv");
        assert_eq!(report.media_identity, c.media_identity);
        assert_eq!(report.unresolved_gates, GATES);
    }
}
#[test]
fn frozen_arr_download_root_becomes_review_candidate_with_legacy_path() {
    let c = contract(0);
    let id = identity(MediaService::Sonarr);
    let mounts = [
        bind(&format!("{}/tv", c.locations.media_host), "/tv", false),
        bind(&c.locations.downloads_host, "/downloads", false),
    ];
    let report = run(&c, &id, &mounts, Records::Roots(&roots()));
    assert_eq!(report.outcome, Outcome::ReviewRequired);
    assert_eq!(report.removal_candidates, [roots()[0].clone()]);
    assert_eq!(report.additions[0].container, "/tv");
    assert!(!format!("{report:?}").contains("/OtherLibrary"));
}
#[test]
fn canonical_roots_make_repeated_preview_unchanged() {
    let c = contract(0);
    let id = identity(MediaService::Radarr);
    let mounts = [bind(&c.locations.media_host, "/data/media", false)];
    let first = run(&c, &id, &mounts, Records::Roots(&[]));
    let existing = [RootFolder {
        id: 9,
        path: format!("{}/", first.additions[0].container),
    }];
    let second = run(&c, &id, &mounts, Records::Roots(&existing));
    assert_eq!(second.outcome, Outcome::Unchanged);
    assert!(second.additions.is_empty());
    assert_eq!(second.unresolved_gates, GATES);
}
#[test]
fn frozen_jellyfin_adds_four_libraries_without_duplicate_movies() {
    let c = contract(0);
    let id = identity(MediaService::Jellyfin);
    let mounts = [bind(&c.locations.media_host, "/media", true)];
    let existing = [Library {
        name: "mOVIES".into(),
        collection_type: "MOVIES".into(),
        paths: vec!["/media/movies/".into()],
    }];
    let first = run(&c, &id, &mounts, Records::Libraries(&existing));
    assert_eq!(first.outcome, Outcome::ChangesProposed);
    assert_eq!(
        first
            .additions
            .iter()
            .map(|p| p.container.as_str())
            .collect::<Vec<_>>(),
        [
            "/media/tv",
            "/media/music",
            "/media/books",
            "/media/audiobooks"
        ]
    );
    let all: Vec<_> = first
        .expected
        .iter()
        .map(|p| Library {
            name: p.kind.name().into(),
            collection_type: p.kind.path_component().into(),
            paths: vec![p.container.clone()],
        })
        .collect();
    assert_eq!(
        run(&c, &id, &mounts, Records::Libraries(&all)).outcome,
        Outcome::Unchanged
    );
}
#[test]
fn library_paths_are_case_sensitive() {
    let c = contract(0);
    let id = identity(MediaService::Jellyfin);
    let mounts = [bind(&c.locations.media_host, "/Media", true)];
    let existing = [Library {
        name: "Movies".into(),
        collection_type: "movies".into(),
        paths: vec!["/media/movies".into()],
    }];
    let report = run(&c, &id, &mounts, Records::Libraries(&existing));
    assert_eq!(report.outcome, Outcome::Blocked);
    assert!(report.issues.contains(&Issue::ExistingLibraryConflict));
    assert!(report.additions.is_empty());
}
#[test]
fn same_name_wrong_kind_and_duplicate_library_keys_block() {
    let c = contract(0);
    let id = identity(MediaService::Jellyfin);
    let mounts = [bind(&c.locations.media_host, "/media", true)];
    let item = Library {
        name: "Movies".into(),
        collection_type: "movies".into(),
        paths: vec!["/media/movies".into()],
    };
    let duplicate = run(
        &c,
        &id,
        &mounts,
        Records::Libraries(&[item.clone(), item.clone()]),
    );
    assert!(duplicate.issues.contains(&Issue::DuplicateLibraryKey));
    assert!(duplicate.additions.is_empty());
    let wrong = Library {
        collection_type: "tv".into(),
        ..item
    };
    assert!(
        run(&c, &id, &mounts, Records::Libraries(&[wrong]))
            .issues
            .contains(&Issue::ExistingLibraryConflict)
    );
}
#[test]
fn download_libraries_and_download_destinations_are_never_seeded() {
    let c = contract(0);
    let id = identity(MediaService::Jellyfin);
    for destination in ["/downloads", "/data/downloads"] {
        let mounts = [bind(&c.locations.media_host, destination, true)];
        let report = run(&c, &id, &mounts, Records::Libraries(&[]));
        assert_eq!(report.outcome, Outcome::Blocked);
        assert!(report.issues.contains(&Issue::DownloadLibraryPath));
    }
    let mounts = [bind(&c.locations.media_host, "/media", true)];
    let bad = [Library {
        name: "Movies".into(),
        collection_type: "movies".into(),
        paths: vec!["/media/movies".into(), "/downloads/movies".into()],
    }];
    assert!(
        run(&c, &id, &mounts, Records::Libraries(&bad))
            .issues
            .contains(&Issue::ExistingDownloadLibrary)
    );
}
#[test]
fn custom_download_binding_is_used_for_review_and_prefix_siblings_survive() {
    let c = contract(0);
    let id = identity(MediaService::Sonarr);
    let mounts = [
        bind(&c.locations.media_host, "/media", false),
        bind(&c.locations.downloads_host, "/Scratch", false),
    ];
    let rows = [
        RootFolder {
            id: 1,
            path: "/Scratch/complete".into(),
        },
        RootFolder {
            id: 2,
            path: "/Scratch2/complete".into(),
        },
    ];
    let result = run(&c, &id, &mounts, Records::Roots(&rows));
    assert_eq!(result.removal_candidates, [rows[0].clone()]);
}
#[test]
fn numeric_identity_must_be_resolved_and_match_managed_identity() {
    let c = contract(0);
    let id = identity(MediaService::Lidarr);
    let mounts = [bind(&c.locations.media_host, "/media", false)];
    for declared in [
        None,
        Some(StorageMediaIdentity { uid: 0, gid: 1000 }),
        Some(StorageMediaIdentity {
            uid: 1234,
            gid: 1000,
        }),
    ] {
        let report = preview(
            100,
            &c,
            SeedContext {
                identity: &id,
                declared_media_identity: declared,
                mounts: &mounts,
            },
            None,
        )
        .unwrap();
        assert_eq!(report.outcome, Outcome::Blocked);
        assert!(report.issues.contains(&Issue::NumericIdentityMismatch));
    }
}
#[test]
fn missing_alias_shadowed_volume_and_read_only_bindings_block() {
    let c = contract(0);
    let id = identity(MediaService::Sonarr);
    let primary = bind(&c.locations.media_host, "/media", false);
    for (mounts, issue) in [
        (vec![], Issue::MissingBinding),
        (
            vec![
                primary.clone(),
                bind(&c.locations.media_host, "/alias", false),
            ],
            Issue::AmbiguousBinding,
        ),
        (
            vec![
                primary.clone(),
                ComposeMount::Volume {
                    name: "other".into(),
                    target: "/media/tv".into(),
                    read_only: false,
                },
            ],
            Issue::ShadowedBinding,
        ),
        (
            vec![
                primary.clone(),
                bind("/mnt/Foreign", "/media/tv/private", false),
            ],
            Issue::ShadowedBinding,
        ),
        (
            vec![bind(&c.locations.media_host, "/media", true)],
            Issue::ReadOnlyBinding,
        ),
    ] {
        let report = run(&c, &id, &mounts, Records::Roots(&[]));
        assert_eq!(report.outcome, Outcome::Blocked);
        assert!(report.issues.contains(&issue));
        assert!(report.additions.is_empty());
    }
}
#[test]
fn overlapping_storage_roots_are_blocked() {
    let mut c = contract(0);
    c.locations.downloads_host = format!("{}/downloads", c.locations.media_host);
    let id = identity(MediaService::Sonarr);
    let mounts = [bind(&c.locations.media_host, "/media", false)];
    assert!(
        run(&c, &id, &mounts, Records::Roots(&[]))
            .issues
            .contains(&Issue::OverlappingStorageRoots)
    );
}
#[test]
fn duplicate_root_paths_block_and_duplicate_ids_are_invalid() {
    let c = contract(0);
    let id = identity(MediaService::Sonarr);
    let mounts = [bind(&c.locations.media_host, "/media", false)];
    let rows = [
        RootFolder {
            id: 1,
            path: "/media/tv".into(),
        },
        RootFolder {
            id: 2,
            path: "/media/tv/".into(),
        },
    ];
    let report = run(&c, &id, &mounts, Records::Roots(&rows));
    assert!(report.issues.contains(&Issue::DuplicateRootPath));
    assert!(report.additions.is_empty());
    let duplicate = [rows[0].clone(), rows[0].clone()];
    assert!(
        preview(
            100,
            &c,
            context(&c, &id, &mounts),
            Some(observation(&id, Records::Roots(&duplicate)))
        )
        .is_err()
    );
}
#[test]
fn unknown_observations_never_turn_into_empty_successful_seeds() {
    let c = contract(0);
    let id = identity(MediaService::Sonarr);
    let mounts = [bind(&c.locations.media_host, "/media", false)];
    for (time, completeness, expected) in [
        (9, Completeness::Complete, SourceState::Stale),
        (101, Completeness::Complete, SourceState::Future),
        (100, Completeness::Partial, SourceState::Incomplete),
    ] {
        let o = Observation {
            sampled_at: time,
            completeness,
            ..observation(&id, Records::Roots(&[]))
        };
        let result = preview(100, &c, context(&c, &id, &mounts), Some(o)).unwrap();
        assert_eq!(result.source, expected);
        assert_eq!(result.outcome, Outcome::Unknown);
        assert!(result.additions.is_empty());
        assert_eq!(result.sampled_at, Some(time));
    }
    let none = preview(100, &c, context(&c, &id, &mounts), None).unwrap();
    assert_eq!(none.source, SourceState::Unavailable);
    assert_eq!(none.outcome, Outcome::Unknown);
}
#[test]
fn exact_stack_service_engine_project_and_files_are_bound_to_observation() {
    let c = contract(0);
    let id = identity(MediaService::Sonarr);
    let mounts = [bind(&c.locations.media_host, "/media", false)];
    for change in 0..6 {
        let mut other = id.clone();
        match change {
            0 => other.stack = "other".into(),
            1 => other.engine_id = "other".into(),
            2 => other.service = "other".into(),
            3 => other.project.name = "other".into(),
            4 => other.project.working_directory = "/opt/stacks/media".into(),
            _ => other.project.config_files.reverse(),
        }
        let result = preview(
            100,
            &c,
            context(&c, &id, &mounts),
            Some(observation(&other, Records::Roots(&[]))),
        )
        .unwrap();
        assert_eq!(result.source, SourceState::IdentityMismatch);
        assert_eq!(result.outcome, Outcome::Unknown);
        assert!(result.additions.is_empty());
    }
}
#[test]
fn record_order_does_not_change_preview() {
    let c = contract(0);
    let id = identity(MediaService::Sonarr);
    let mounts = [bind(&c.locations.media_host, "/media", false)];
    let mut rows = roots();
    let first = run(&c, &id, &mounts, Records::Roots(&rows));
    rows.reverse();
    assert_eq!(first, run(&c, &id, &mounts, Records::Roots(&rows)));
}
#[test]
fn malformed_paths_templates_and_duplicate_mount_targets_are_refused() {
    let c = contract(0);
    let id = identity(MediaService::Sonarr);
    for target in [
        "/data/../media",
        "/data//media",
        "/{{MEDIA_DIR}}",
        "/${MEDIA_DIR}",
        "/data\nmedia",
    ] {
        let mounts = [bind(&c.locations.media_host, target, false)];
        assert!(preview(100, &c, context(&c, &id, &mounts), None).is_err());
    }
    let mounts = [
        bind(&c.locations.media_host, "/media", false),
        bind(&c.locations.downloads_host, "/media", false),
    ];
    assert!(preview(100, &c, context(&c, &id, &mounts), None).is_err());
}
#[test]
fn explicit_time_and_serialization_bounds_are_enforced() {
    let c = contract(0);
    let id = identity(MediaService::Sonarr);
    let mounts = [bind(&c.locations.media_host, "/media", false)];
    assert!(preview(-1, &c, context(&c, &id, &mounts), None).is_err());
    let mut report = run(&c, &id, &mounts, Records::Roots(&[]));
    report.target.stack = "x".repeat(MAX_SUMMARY_BYTES);
    assert!(report.check_size().is_err());
}
#[test]
fn age_boundary_is_inclusive_and_retains_original_sample_time() {
    let c = contract(0);
    let id = identity(MediaService::Sonarr);
    let mounts = [bind(&c.locations.media_host, "/media", false)];
    let o = Observation {
        sampled_at: 10,
        ..observation(&id, Records::Roots(&[]))
    };
    let result = preview(100, &c, context(&c, &id, &mounts), Some(o)).unwrap();
    assert_eq!(result.source, SourceState::Fresh);
    assert_eq!(result.sampled_at, Some(10));
    assert_eq!(result.outcome, Outcome::ChangesProposed);
}
#[test]
fn custom_numeric_identity_and_literal_host_case_survive_projection() {
    let mut c = contract(0);
    c.media_identity = StorageMediaIdentity {
        uid: 2345,
        gid: 3456,
    };
    c.locations.media_host = "/mnt/storage/Media".into();
    let id = identity(MediaService::Lidarr);
    let mounts = [bind(&c.locations.media_host, "/MusicLibrary", false)];
    let report = run(&c, &id, &mounts, Records::Roots(&[]));
    assert_eq!(report.additions[0].host, "/mnt/storage/Media/music");
    assert_eq!(report.additions[0].container, "/MusicLibrary/music");
    assert_eq!(report.media_identity, c.media_identity);
}
#[test]
fn aggregate_text_record_and_mount_bounds_apply_before_summary_clones() {
    let c = contract(0);
    let id = identity(MediaService::Jellyfin);
    let mounts = [bind(&c.locations.media_host, "/media", true)];
    let mut rows: Vec<_> = (0..MAX_RECORDS)
        .map(|i| Library {
            name: format!("custom{i}"),
            collection_type: "movies".into(),
            paths: vec![format!("/custom/{i}")],
        })
        .collect();
    assert!(
        preview(
            100,
            &c,
            context(&c, &id, &mounts),
            Some(observation(&id, Records::Libraries(&rows)))
        )
        .is_ok()
    );
    rows.push(Library {
        name: "extra".into(),
        collection_type: "movies".into(),
        paths: vec![],
    });
    assert!(
        preview(
            100,
            &c,
            context(&c, &id, &mounts),
            Some(observation(&id, Records::Libraries(&rows)))
        )
        .is_err()
    );
    rows.pop();
    for row in &mut rows {
        row.paths = (0..MAX_PATHS)
            .map(|i| format!("/{}/{}/path{i}", "a".repeat(200), "b".repeat(200)))
            .collect();
    }
    assert!(
        preview(
            100,
            &c,
            context(&c, &id, &mounts),
            Some(observation(&id, Records::Libraries(&rows)))
        )
        .is_err()
    );
    let many_mounts: Vec<_> = (0..9)
        .map(|i| bind(&format!("/mnt/source{i}"), &format!("/target{i}"), false))
        .collect();
    assert!(preview(100, &c, context(&c, &id, &many_mounts), None).is_err());
}
#[test]
fn unresolved_download_root_is_not_a_literal_storage_mapping() {
    let mut c = contract(0);
    c.locations.downloads_host = "/mnt/storage/${DOWNLOADS}".into();
    // The shared storage schema permits literal braces; this media subset must
    // refuse unresolved template syntax before deriving download exclusions.
    c.validate().unwrap();
    let id = identity(MediaService::Sonarr);
    let mounts = [bind(&c.locations.media_host, "/media", false)];
    assert!(preview(100, &c, context(&c, &id, &mounts), None).is_err());
}
