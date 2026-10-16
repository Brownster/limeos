use super::*;
use crate::deployment::discovery::ProjectIdentity;

fn target(app: MediaService) -> Target {
    Target {
        identity: ServiceIdentity {
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
            service: "arr".into(),
            application: app,
        },
        container: ContainerSnapshot {
            resource: format!("container:{}", "1".repeat(64)),
            image: format!("sha256:{}", "a".repeat(64)),
            started_at: "2026-10-16T01:02:03.000000000Z".into(),
            running: true,
        },
    }
}
fn intent(app: MediaService) -> Intent {
    let kind = app_kind(app).unwrap();
    let path = match app {
        MediaService::Sonarr => "/tv",
        MediaService::Radarr => "/movies",
        _ => "/music",
    };
    Intent {
        library: LibraryPath {
            kind,
            host: format!("/srv/Media{path}"),
            container: path.into(),
        },
        import_mode: ImportMode::Move,
        completed_download_handling: true,
        naming: NamingPreset::Standard,
        quality_profile: Some("HD-1080p".into()),
    }
}
fn current(app: MediaService) -> CurrentSettings {
    CurrentSettings {
        management: Management {
            id: 11,
            copy_using_hardlinks: true,
            recycle_bin: "/Old/Trash".into(),
            use_script_import: false,
        },
        download_handling: DownloadHandling {
            id: 12,
            enabled: false,
        },
        naming: Naming {
            id: 13,
            field: SettingField::rename_field(app).unwrap(),
            rename: false,
            replace_illegal_characters: false,
        },
        profiles: vec![QualityProfile {
            id: 7,
            name: "HD-1080p".into(),
        }],
    }
}
fn observed<'a>(target: &'a Target, settings: &'a CurrentSettings) -> Observation<'a> {
    Observation {
        target,
        settings,
        sampled_at: 100,
        complete: true,
    }
}
fn run(app: MediaService) -> SettingsPreview {
    preview(
        101,
        &target(app),
        &intent(app),
        Some(observed(&target(app), &current(app))),
    )
    .unwrap()
}

#[test]
fn frozen_sonarr_intent_uses_correct_fields_and_distinct_config_resources() {
    let result = run(MediaService::Sonarr);
    assert_eq!(result.outcome, Outcome::ChangesProposed);
    assert_eq!(result.recycle_bin, "/tv/.recycle");
    assert_eq!(result.changes.len(), 5);
    assert_eq!(
        result.changes[0],
        SettingChange {
            resource: Resource::MediaManagement,
            resource_id: 11,
            field: SettingField::CopyUsingHardlinks,
            before: SettingValue::Boolean(true),
            after: SettingValue::Boolean(false)
        }
    );
    assert_eq!(result.changes[2].resource, Resource::DownloadClientConfig);
    assert_eq!(result.changes[2].resource_id, 12);
    assert_eq!(
        result.changes[2].field.wire_name(),
        "enableCompletedDownloadHandling"
    );
    assert!(matches!(
        result.quality_profile,
        ProfileAvailability::Present { id: 7, .. }
    ));
    assert_eq!(result.unresolved.len(), 6);
}
#[test]
fn each_application_has_its_own_rename_field_and_api_version() {
    for (app, field, root) in [
        (MediaService::Sonarr, "renameEpisodes", "/api/v3"),
        (MediaService::Radarr, "renameMovies", "/api/v3"),
        (MediaService::Lidarr, "renameTracks", "/api/v1"),
    ] {
        let result = run(app);
        assert_eq!(result.changes[3].field.wire_name(), field);
        assert_eq!(result.changes[3].resource_id, 13);
        for resource in [
            Resource::MediaManagement,
            Resource::DownloadClientConfig,
            Resource::Naming,
            Resource::QualityProfiles,
        ] {
            assert!(resource.read_path(app).unwrap().starts_with(root));
        }
    }
    assert_eq!(
        Resource::DownloadClientConfig
            .read_path(MediaService::Sonarr)
            .unwrap(),
        "/api/v3/config/downloadclient"
    );
}
#[test]
fn correctly_observed_defaults_are_a_no_op_with_unresolved_gates() {
    for app in [
        MediaService::Sonarr,
        MediaService::Radarr,
        MediaService::Lidarr,
    ] {
        let mut settings = current(app);
        settings.management.copy_using_hardlinks = false;
        settings.management.recycle_bin = format!("{}/.recycle", intent(app).library.container);
        settings.download_handling.enabled = true;
        settings.naming.rename = true;
        settings.naming.replace_illegal_characters = true;
        let result = preview(
            101,
            &target(app),
            &intent(app),
            Some(observed(&target(app), &settings)),
        )
        .unwrap();
        assert_eq!(result.outcome, Outcome::Unchanged);
        assert!(result.changes.is_empty());
        assert_eq!(result.unresolved, GATES);
    }
}
#[test]
fn hardlink_and_keep_original_choices_remain_explicit_and_physically_unverified() {
    let app = MediaService::Sonarr;
    let mut request = intent(app);
    request.import_mode = ImportMode::Hardlink;
    request.naming = NamingPreset::KeepOriginal;
    let result = preview(
        101,
        &target(app),
        &request,
        Some(observed(&target(app), &current(app))),
    )
    .unwrap();
    assert!(!result.changes.iter().any(|c| matches!(
        c.field,
        SettingField::CopyUsingHardlinks | SettingField::RenameEpisodes
    )));
    assert!(
        result
            .changes
            .iter()
            .any(|c| c.field == SettingField::ReplaceIllegalCharacters)
    );
    assert!(
        result
            .unresolved
            .contains(&SettingsGate::VerifiedLibraryMountsOwnershipAndHardlinks)
    );
}
#[test]
fn unrequested_profile_is_distinct_from_unknown_or_missing() {
    let app = MediaService::Sonarr;
    let mut request = intent(app);
    request.quality_profile = None;
    let mut settings = current(app);
    settings.profiles.clear();
    let result = preview(
        101,
        &target(app),
        &request,
        Some(observed(&target(app), &settings)),
    )
    .unwrap();
    assert_eq!(result.quality_profile, ProfileAvailability::NotRequested);
    assert_eq!(result.changes.len(), 5);
}
#[test]
fn missing_or_ambiguous_requested_profiles_block_all_changes_without_default_assignment() {
    let app = MediaService::Lidarr;
    let mut settings = current(app);
    settings.profiles[0].name = "Lossless".into();
    let result = preview(
        101,
        &target(app),
        &intent(app),
        Some(observed(&target(app), &settings)),
    )
    .unwrap();
    assert!(matches!(
        result.quality_profile,
        ProfileAvailability::Missing { .. }
    ));
    assert_eq!(result.outcome, Outcome::Blocked);
    assert!(result.changes.is_empty());
    settings.profiles = vec![
        QualityProfile {
            id: 7,
            name: "HD-1080p".into(),
        },
        QualityProfile {
            id: 8,
            name: "HD-1080p".into(),
        },
    ];
    let result = preview(
        101,
        &target(app),
        &intent(app),
        Some(observed(&target(app), &settings)),
    )
    .unwrap();
    assert!(matches!(
        result.quality_profile,
        ProfileAvailability::Ambiguous { .. }
    ));
    assert!(result.changes.is_empty());
}
#[test]
fn profile_lookup_preserves_frozen_exact_name_case() {
    let app = MediaService::Sonarr;
    let mut settings = current(app);
    settings.profiles[0].name = "hd-1080p".into();
    let result = preview(
        101,
        &target(app),
        &intent(app),
        Some(observed(&target(app), &settings)),
    )
    .unwrap();
    assert!(matches!(
        result.quality_profile,
        ProfileAvailability::Missing { .. }
    ));
}
#[test]
fn custom_script_import_blocks_without_proposing_to_disable_operator_configuration() {
    let app = MediaService::Radarr;
    let mut settings = current(app);
    settings.management.use_script_import = true;
    let result = preview(
        101,
        &target(app),
        &intent(app),
        Some(observed(&target(app), &settings)),
    )
    .unwrap();
    assert_eq!(result.outcome, Outcome::Blocked);
    assert_eq!(result.issues, [SettingsIssue::ScriptImportEnabled]);
    assert!(result.changes.is_empty());
}
#[test]
fn unknown_sources_never_turn_into_default_settings_or_empty_profile_success() {
    let app = MediaService::Sonarr;
    let owner = target(app);
    let request = intent(app);
    let settings = current(app);
    let result = preview(101, &owner, &request, None).unwrap();
    assert_eq!(result.source, SourceState::Unavailable);
    assert_eq!(result.quality_profile, ProfileAvailability::Unknown);
    assert!(result.changes.is_empty());
    let mut sample = observed(&owner, &settings);
    sample.complete = false;
    let result = preview(101, &owner, &request, Some(sample)).unwrap();
    assert_eq!(result.source, SourceState::Incomplete);
    assert_eq!(result.outcome, Outcome::Unknown);
    assert!(result.changes.is_empty());
    let mut owner = owner;
    owner.container.running = false;
    assert_eq!(
        preview(101, &owner, &request, Some(observed(&owner, &settings)))
            .unwrap()
            .source,
        SourceState::Stopped
    );
}
#[test]
fn age_boundary_is_inclusive_and_original_timestamp_is_retained() {
    let app = MediaService::Sonarr;
    let owner = target(app);
    let request = intent(app);
    let settings = current(app);
    assert_eq!(
        preview(190, &owner, &request, Some(observed(&owner, &settings)))
            .unwrap()
            .source,
        SourceState::Fresh
    );
    let result = preview(191, &owner, &request, Some(observed(&owner, &settings))).unwrap();
    assert_eq!(result.source, SourceState::Stale);
    assert_eq!(result.sampled_at, Some(100));
    assert!(result.changes.is_empty());
    assert_eq!(
        preview(99, &owner, &request, Some(observed(&owner, &settings)))
            .unwrap()
            .source,
        SourceState::Future
    );
}
#[test]
fn exact_target_includes_stack_engine_project_order_service_and_container_instance() {
    let app = MediaService::Sonarr;
    let owner = target(app);
    let request = intent(app);
    let settings = current(app);
    for field in 0..9 {
        let mut other = owner.clone();
        match field {
            0 => other.identity.stack = "other".into(),
            1 => other.identity.engine_id = "other".into(),
            2 => other.identity.service = "other".into(),
            3 => other.identity.project.name = "other".into(),
            4 => other.identity.project.working_directory = "/Other/Case".into(),
            5 => other.identity.project.config_files.reverse(),
            6 => other.container.image = format!("sha256:{}", "b".repeat(64)),
            7 => other.container.started_at = "2026-10-16T02:02:03Z".into(),
            _ => other.container.resource = format!("container:{}", "2".repeat(64)),
        }
        let result = preview(101, &owner, &request, Some(observed(&other, &settings))).unwrap();
        assert_eq!(result.source, SourceState::IdentityMismatch);
        assert!(result.changes.is_empty());
    }
}
#[test]
fn application_drift_and_wrong_naming_field_cannot_relabel_a_sample() {
    let owner = target(MediaService::Sonarr);
    let other = target(MediaService::Radarr);
    let result = preview(
        101,
        &owner,
        &intent(MediaService::Sonarr),
        Some(observed(&other, &current(MediaService::Radarr))),
    )
    .unwrap();
    assert_eq!(result.source, SourceState::IdentityMismatch);
    assert_eq!(
        preview(
            101,
            &owner,
            &intent(MediaService::Sonarr),
            Some(observed(&owner, &current(MediaService::Radarr)))
        )
        .unwrap_err(),
        SettingsError::InvalidInput
    );
}
#[test]
fn recycle_bin_is_derived_beneath_literal_case_sensitive_library_paths() {
    let app = MediaService::Sonarr;
    let mut request = intent(app);
    request.library.container = "/Media/TV Shows".into();
    let result = preview(
        101,
        &target(app),
        &request,
        Some(observed(&target(app), &current(app))),
    )
    .unwrap();
    assert_eq!(result.recycle_bin, "/Media/TV Shows/.recycle");
    for container in [
        "/downloads",
        "/downloads/sub",
        "/data/downloads",
        "/data/downloads/sub",
        "/tv/../downloads",
        "/tv/${ROOT}",
        "/tv/",
    ] {
        request.library.container = container.into();
        assert_eq!(
            preview(101, &target(app), &request, None).unwrap_err(),
            SettingsError::InvalidInput
        );
    }
    request.library.container = "/downloads2".into();
    assert!(preview(101, &target(app), &request, None).is_ok());
}
#[test]
fn library_kind_and_recycle_path_bound_are_admitted_before_projection() {
    let app = MediaService::Sonarr;
    let mut request = intent(app);
    request.library.kind = LibraryKind::Movies;
    assert_eq!(
        preview(101, &target(app), &request, None).unwrap_err(),
        SettingsError::InvalidInput
    );
    request.library.kind = LibraryKind::Tv;
    request.library.container = format!("/{}/{}", "a".repeat(250), "b".repeat(254));
    assert_eq!(
        preview(101, &target(app), &request, None).unwrap_err(),
        SettingsError::InvalidInput
    );
}
#[test]
fn invalid_profiles_ids_text_and_counts_are_not_silently_normalized() {
    let app = MediaService::Sonarr;
    for field in 0..6 {
        let mut settings = current(app);
        match field {
            0 => settings.management.id = 0,
            1 => settings.naming.id = u32::MAX,
            2 => settings.profiles.push(settings.profiles[0].clone()),
            3 => settings.profiles[0].name = "a\nb".into(),
            4 => settings.management.recycle_bin = "x".repeat(513),
            _ => settings.profiles[0].name = "é".repeat(65),
        };
        assert_eq!(
            preview(
                101,
                &target(app),
                &intent(app),
                Some(observed(&target(app), &settings))
            )
            .unwrap_err(),
            SettingsError::InvalidInput
        );
    }
    let mut settings = current(app);
    settings.profiles = (1..=17)
        .map(|id| QualityProfile {
            id,
            name: format!("Profile {id}"),
        })
        .collect();
    assert_eq!(
        preview(
            101,
            &target(app),
            &intent(app),
            Some(observed(&target(app), &settings))
        )
        .unwrap_err(),
        SettingsError::Limit
    );
}
#[test]
fn unsupported_application_invalid_clock_or_identity_refuse() {
    let app = MediaService::Sonarr;
    assert_eq!(
        preview(-1, &target(app), &intent(app), None).unwrap_err(),
        SettingsError::InvalidInput
    );
    let mut owner = target(app);
    owner.identity.application = MediaService::Jellyfin;
    assert_eq!(
        preview(101, &owner, &intent(app), None).unwrap_err(),
        SettingsError::Unsupported
    );
    let mut owner = target(app);
    owner.container.resource = "container:short".into();
    assert_eq!(
        preview(101, &owner, &intent(app), None).unwrap_err(),
        SettingsError::InvalidInput
    );
}
#[test]
fn profile_order_does_not_change_diffs_or_availability() {
    let app = MediaService::Sonarr;
    let mut settings = current(app);
    settings.profiles.push(QualityProfile {
        id: 8,
        name: "Other".into(),
    });
    let first = preview(
        101,
        &target(app),
        &intent(app),
        Some(observed(&target(app), &settings)),
    )
    .unwrap();
    settings.profiles.reverse();
    let second = preview(
        101,
        &target(app),
        &intent(app),
        Some(observed(&target(app), &settings)),
    )
    .unwrap();
    assert_eq!(first, second);
}
#[test]
fn streaming_serialization_bound_applies_to_public_preview_values() {
    let mut result = run(MediaService::Sonarr);
    assert!(result.check_size().is_ok());
    result.recycle_bin = "x".repeat(MAX_REPORT_BYTES + 1);
    assert_eq!(result.check_size().unwrap_err(), SettingsError::Limit);
}
