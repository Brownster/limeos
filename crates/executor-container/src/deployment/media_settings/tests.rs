use super::*;
use limeos_domain::{
    ContainerSnapshot,
    deployment::{
        discovery::ProjectIdentity,
        media::{LibraryKind, LibraryPath, ServiceIdentity},
        media_settings::{
            self as domain, CurrentSettings, ImportMode, Intent, NamingPreset, Outcome,
            ProfileAvailability, Resource, Target,
        },
    },
};
use serde_json::{Value, json};

fn fixture(app: MediaService) -> Value {
    serde_json::from_str(match app {
        MediaService::Sonarr => {
            include_str!("../../../../../tests/fixtures/deployment/media-settings/sonarr.json")
        }
        MediaService::Radarr => {
            include_str!("../../../../../tests/fixtures/deployment/media-settings/radarr.json")
        }
        _ => include_str!("../../../../../tests/fixtures/deployment/media-settings/lidarr.json"),
    })
    .unwrap()
}
fn bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}
fn target(app: MediaService) -> Target {
    Target {
        identity: ServiceIdentity {
            stack: "media".into(),
            engine_id: "engine-a".into(),
            project: ProjectIdentity {
                name: "media".into(),
                working_directory: "/opt/stacks/Media".into(),
                config_files: vec!["/opt/stacks/Media/compose.yaml".into()],
            },
            service: "arr".into(),
            application: app,
        },
        container: ContainerSnapshot {
            resource: format!("container:{}", "a".repeat(64)),
            image: format!("sha256:{}", "b".repeat(64)),
            started_at: "2026-10-16T01:02:03Z".into(),
            running: true,
        },
    }
}
fn intent(app: MediaService) -> Intent {
    let (kind, container) = match app {
        MediaService::Sonarr => (LibraryKind::Tv, "/tv"),
        MediaService::Radarr => (LibraryKind::Movies, "/movies"),
        _ => (LibraryKind::Music, "/music"),
    };
    Intent {
        library: LibraryPath {
            kind,
            host: format!("/srv/Media{container}"),
            container: container.into(),
        },
        import_mode: ImportMode::Move,
        completed_download_handling: true,
        naming: NamingPreset::Standard,
        quality_profile: Some("HD-1080p".into()),
    }
}
fn selected(app: MediaService, wire: &Value) -> CurrentSettings {
    CurrentSettings {
        management: management(&bytes(&wire["management"])).unwrap(),
        download_handling: download_handling(&bytes(&wire["download_handling"])).unwrap(),
        naming: naming(app, &bytes(&wire["naming"])).unwrap(),
        profiles: quality_profiles(&bytes(&wire["profiles"])).unwrap(),
    }
}

#[test]
fn primary_schema_fixtures_and_frozen_defaults_project_correct_resource_changes() {
    for app in [
        MediaService::Sonarr,
        MediaService::Radarr,
        MediaService::Lidarr,
    ] {
        let wire = fixture(app);
        let current = selected(app, &wire);
        let owner = target(app);
        let mut request = intent(app);
        let result = domain::preview(
            101,
            &owner,
            &request,
            Some(domain::Observation {
                target: &owner,
                sampled_at: 100,
                complete: true,
                settings: &current,
            }),
        )
        .unwrap();
        if app == MediaService::Lidarr {
            assert_eq!(result.outcome, Outcome::Blocked);
            assert!(matches!(
                result.quality_profile,
                ProfileAvailability::Missing { .. }
            ));
            request.quality_profile = Some("Lossless".into());
        }
        let result = domain::preview(
            101,
            &owner,
            &request,
            Some(domain::Observation {
                target: &owner,
                sampled_at: 100,
                complete: true,
                settings: &current,
            }),
        )
        .unwrap();
        assert_eq!(result.changes.len(), 5);
        assert_eq!(result.changes[2].resource, Resource::DownloadClientConfig);
        assert_eq!(
            result.changes[2].resource_id,
            wire["download_handling"]["id"].as_u64().unwrap() as u32
        );
    }
}
#[test]
fn legacy_bogus_flags_do_not_supply_the_current_hardlink_or_download_handling_value() {
    let wire = fixture(MediaService::Sonarr);
    let settings = selected(MediaService::Sonarr, &wire);
    assert!(settings.management.copy_using_hardlinks);
    assert!(!settings.download_handling.enabled);
    assert!(
        management(br#"{"id":1,"useHardlinks":false,"recycleBin":"","useScriptImport":false}"#)
            .is_err()
    );
    assert!(download_handling(br#"{"id":1,"copyUsingHardlinks":true,"recycleBin":""}"#).is_err());
}
#[test]
fn unknown_credentials_commands_and_custom_naming_formats_are_not_retained() {
    let app = MediaService::Sonarr;
    let settings = selected(app, &fixture(app));
    let owner = target(app);
    let result = domain::preview(
        101,
        &owner,
        &intent(app),
        Some(domain::Observation {
            target: &owner,
            sampled_at: 100,
            complete: true,
            settings: &settings,
        }),
    )
    .unwrap();
    for text in [
        format!("{settings:?}"),
        format!("{result:?}"),
        serde_json::to_string(&result).unwrap(),
    ] {
        assert!(!text.contains("SETTINGS_PRIVATE_VALUE"));
        assert!(!text.contains("UNSELECTED_SCRIPT"));
        assert!(!text.contains("UNSELECTED_NAMING"));
    }
}
#[test]
fn required_selected_fields_and_script_import_status_never_default_from_missing_or_null() {
    for key in ["id", "copyUsingHardlinks", "recycleBin", "useScriptImport"] {
        let mut value = fixture(MediaService::Sonarr)["management"].clone();
        value.as_object_mut().unwrap().remove(key);
        assert!(management(&bytes(&value)).is_err());
        value[key] = Value::Null;
        assert!(management(&bytes(&value)).is_err());
    }
    for value in [
        json!({"id":1}),
        json!({"id":1,"enableCompletedDownloadHandling":null}),
        json!({"id":1,"enableCompletedDownloadHandling":1}),
    ] {
        assert!(download_handling(&bytes(&value)).is_err());
    }
}
#[test]
fn naming_must_have_the_expected_application_field_with_a_boolean_value() {
    for app in [
        MediaService::Sonarr,
        MediaService::Radarr,
        MediaService::Lidarr,
    ] {
        for other in [
            MediaService::Sonarr,
            MediaService::Radarr,
            MediaService::Lidarr,
        ] {
            let wire = fixture(other);
            assert_eq!(naming(app, &bytes(&wire["naming"])).is_ok(), app == other);
        }
    }
    assert!(
        naming(
            MediaService::Jellyfin,
            &bytes(&fixture(MediaService::Sonarr)["naming"])
        )
        .is_err()
    );
    for v in [
        json!({"id":1,"renameEpisodes":null,"replaceIllegalCharacters":true}),
        json!({"id":1,"renameEpisodes":"true","replaceIllegalCharacters":true}),
    ] {
        assert!(naming(MediaService::Sonarr, &bytes(&v)).is_err());
    }
}
#[test]
fn consumed_duplicate_fields_including_null_first_rename_flags_are_refused() {
    assert!(
        management(
            br#"{"id":1,"id":2,"copyUsingHardlinks":true,"recycleBin":"","useScriptImport":false}"#
        )
        .is_err()
    );
    assert!(download_handling(br#"{"id":1,"enableCompletedDownloadHandling":false,"enableCompletedDownloadHandling":true}"#).is_err());
    assert!(naming(MediaService::Sonarr, br#"{"id":1,"renameEpisodes":null,"renameEpisodes":true,"replaceIllegalCharacters":true}"#).is_err());
    assert!(naming(MediaService::Sonarr, br#"{"id":1,"renameEpisodes":false,"renameEpisodes":true,"replaceIllegalCharacters":true}"#).is_err());
}
#[test]
fn consumed_resources_are_objects_and_profile_lists_contain_only_objects() {
    for value in [
        json!([]),
        json!([1, true, "", false]),
        json!(null),
        json!("string"),
    ] {
        let raw = bytes(&value);
        assert!(management(&raw).is_err());
        assert!(download_handling(&raw).is_err());
        assert!(naming(MediaService::Sonarr, &raw).is_err());
    }
    assert!(quality_profiles(br#"[[1,"HD-1080p"]]"#).is_err());
    assert!(quality_profiles(br#"{"records":[]}"#).is_err());
}
#[test]
fn profile_count_ids_and_name_byte_bounds_are_enforced_without_truncation() {
    let rows: Vec<_> = (1..=16)
        .map(|id| json!({"id":id,"name":format!("Profile {id}")}))
        .collect();
    assert_eq!(quality_profiles(&bytes(&json!(rows))).unwrap().len(), 16);
    let mut rows = rows;
    rows.push(json!({"id":17,"name":"Too many"}));
    assert!(quality_profiles(&bytes(&json!(rows))).is_err());
    for rows in [
        json!([{"id":0,"name":"Name"}]),
        json!([{"id":1,"name":"A"},{"id":1,"name":"B"}]),
        json!([{"id":1,"name":""}]),
        json!([{"id":1,"name":"é".repeat(65)}]),
    ] {
        assert!(quality_profiles(&bytes(&rows)).is_err());
    }
    assert!(quality_profiles(&bytes(&json!([{"id":1,"name":"é".repeat(64)}]))).is_ok());
    assert_eq!(
        quality_profiles(br#"[{"id":1,"name":"Same"},{"id":2,"name":"Same"}]"#)
            .unwrap()
            .len(),
        2
    ); // Domain reports ambiguity, not an arbitrary first row.
}
#[test]
fn raw_body_ceiling_applies_before_parsing_and_selected_text_has_its_own_limits() {
    let mut raw = bytes(&fixture(MediaService::Sonarr)["management"]);
    raw.resize(32768, b' ');
    assert!(management(&raw).is_ok());
    raw.push(b' ');
    assert!(management(&raw).is_err());
    let raw = vec![b' '; 32769];
    assert!(download_handling(&raw).is_err());
    assert!(naming(MediaService::Sonarr, &raw).is_err());
    assert!(quality_profiles(&raw).is_err());
    let mut value = fixture(MediaService::Sonarr)["management"].clone();
    value["recycleBin"] = json!("x".repeat(513));
    assert!(management(&bytes(&value)).is_err());
}
#[test]
fn malformed_data_and_trailing_payloads_return_fixed_errors_without_source_text() {
    for raw in [
        b"PRIVATE_INVALID".as_slice(),
        b"{\"id\":1} garbage",
        b"\xff",
    ] {
        let error = management(raw).unwrap_err();
        assert_eq!(error.0, ErrorCode::Unavailable);
        assert!(!format!("{error:?}").contains("PRIVATE"));
    }
}
