use super::*;
use limeos_domain::{
    ContainerSnapshot,
    deployment::{
        discovery::ProjectIdentity,
        download_clients::{
            self as domain, Decision, Observation, Outcome, Provider, Request, Target,
        },
        media::ServiceIdentity,
    },
};
use serde_json::json;

fn fixture(app: MediaService) -> Vec<Value> {
    serde_json::from_str(match app {
        MediaService::Sonarr => {
            include_str!("../../../../../tests/fixtures/deployment/download-clients/sonarr.json")
        }
        MediaService::Radarr => {
            include_str!("../../../../../tests/fixtures/deployment/download-clients/radarr.json")
        }
        _ => include_str!("../../../../../tests/fixtures/deployment/download-clients/lidarr.json"),
    })
    .unwrap()
}
fn bytes(value: &impl serde::Serialize) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}
fn target(app: MediaService) -> Target {
    Target {
        identity: ServiceIdentity {
            stack: "media".into(),
            engine_id: "engine-a".into(),
            service: "arr".into(),
            application: app,
            project: ProjectIdentity {
                name: "media".into(),
                working_directory: "/opt/Media".into(),
                config_files: vec!["/opt/Media/compose.yaml".into()],
            },
        },
        container: ContainerSnapshot {
            resource: format!("container:{}", "a".repeat(64)),
            image: format!("sha256:{}", "b".repeat(64)),
            started_at: "2026-10-16T01:02:03Z".into(),
            running: true,
        },
    }
}
fn preview(app: MediaService, rows: &[Client]) -> domain::Preview {
    let owner = target(app);
    let category = match app {
        MediaService::Sonarr => "sonarr",
        MediaService::Radarr => "radarr",
        _ => "lidarr",
    };
    let requests: Vec<_> = [Provider::Transmission, Provider::Sabnzbd]
        .into_iter()
        .map(|provider| Request {
            provider,
            service: provider.implementation().to_ascii_lowercase(),
            category: category.into(),
            remove_completed: true,
            remove_failed: true,
        })
        .collect();
    let services = ["arr", "transmission", "sabnzbd"].map(String::from);
    domain::preview(
        101,
        &owner,
        &requests,
        Some(Observation {
            target: &owner,
            sampled_at: 100,
            complete: true,
            declared_services: &services,
            clients: rows,
        }),
    )
    .unwrap()
}

#[test]
fn native_fixtures_port_frozen_intent_without_duplicate_additions_on_rerun() {
    for app in [
        MediaService::Sonarr,
        MediaService::Radarr,
        MediaService::Lidarr,
    ] {
        let rows = clients(app, &bytes(&fixture(app))).unwrap();
        let result = preview(app, &rows);
        assert_eq!(result.outcome, Outcome::NoAdditions);
        assert!(result.additions.is_empty());
        assert!(
            result
                .rows
                .iter()
                .all(|r| matches!(r.decision, Decision::Present { .. }))
        );
        let result = preview(app, &[]);
        assert_eq!(result.additions.len(), 2);
        assert_eq!(
            result.additions[0].category_field,
            CategoryField::for_app(app).unwrap()
        );
    }
}
#[test]
fn generic_category_does_not_substitute_and_other_application_fields_remain_unknown() {
    let mut rows = fixture(MediaService::Sonarr);
    rows[0]["fields"] = json!([{"name":"category","value":"sonarr"}]);
    rows[0]["category"] = json!("sonarr");
    let decoded = clients(MediaService::Sonarr, &bytes(&rows)).unwrap();
    assert_eq!(decoded[0].category, Category::Unknown);
    assert_eq!(
        preview(MediaService::Sonarr, &decoded).outcome,
        Outcome::Blocked
    );
    let decoded = clients(MediaService::Radarr, &bytes(&fixture(MediaService::Sonarr))).unwrap();
    assert!(decoded.iter().all(|c| c.category == Category::Unknown));
    assert_eq!(
        preview(MediaService::Radarr, &decoded).outcome,
        Outcome::Blocked
    );
}
#[test]
fn connection_credentials_and_unknown_fields_never_enter_selected_metadata_or_preview() {
    let rows = clients(MediaService::Sonarr, &bytes(&fixture(MediaService::Sonarr))).unwrap();
    let debug = format!("{rows:?}");
    let report = serde_json::to_string(&preview(MediaService::Sonarr, &rows)).unwrap();
    for text in [debug, report] {
        assert!(!text.contains("REGISTRATION_PRIVATE_VALUE"));
        assert!(!text.contains("PRIVATE_ENDPOINT"));
        assert!(!text.contains("ignored-generic"));
    }
}
#[test]
fn missing_or_null_native_category_is_unknown_not_an_empty_string_or_default() {
    for fields in [
        json!([]),
        json!([{"name":"tvCategory"}]),
        json!([{"name":"tvCategory","value":null}]),
    ] {
        let mut rows = fixture(MediaService::Sonarr);
        rows[0]["fields"] = fields;
        let decoded = clients(MediaService::Sonarr, &bytes(&rows)).unwrap();
        assert_eq!(decoded[0].category, Category::Unknown);
        assert!(preview(MediaService::Sonarr, &decoded).additions.is_empty());
    }
    let mut rows = fixture(MediaService::Sonarr);
    rows[0]["fields"] = json!([{"name":"tvCategory","value":""}]);
    assert_eq!(
        clients(MediaService::Sonarr, &bytes(&rows)).unwrap()[0].category,
        Category::Known("".into())
    );
}
#[test]
fn consumed_field_duplicates_aliases_and_wrong_types_are_refused() {
    for fields in [
        json!([{"name":"tvCategory","value":null},{"name":"tvCategory","value":"sonarr"}]),
        json!([{"name":"tvCategory","value":"sonarr"},{"name":"TVCategory","value":"sonarr"}]),
        json!([{"name":"tvCategory","value":1}]),
        json!([{"name":"tvCategory","value":true}]),
        json!([{"name":"tvCategory","value":["sonarr"]}]),
        json!([{"name":"tvCategory","value":{}}]),
    ] {
        let mut rows = fixture(MediaService::Sonarr);
        rows[0]["fields"] = fields;
        assert!(clients(MediaService::Sonarr, &bytes(&rows)).is_err());
    }
    let raw = br#"[{"id":1,"id":2,"name":"T","implementation":"Transmission","enable":true,"removeCompletedDownloads":true,"removeFailedDownloads":true,"fields":[]}]"#;
    assert!(clients(MediaService::Sonarr, raw).is_err());
    let raw = br#"[{"id":1,"name":"T","implementation":"Transmission","enable":true,"removeCompletedDownloads":true,"removeFailedDownloads":true,"fields":[{"name":"tvCategory","value":null,"value":"sonarr"}]}]"#;
    assert!(clients(MediaService::Sonarr, raw).is_err());
}
#[test]
fn malformed_selected_metadata_and_missing_policy_booleans_refuse() {
    for field in [
        "id",
        "name",
        "implementation",
        "enable",
        "removeCompletedDownloads",
        "removeFailedDownloads",
        "fields",
    ] {
        let mut rows = fixture(MediaService::Sonarr);
        rows[0].as_object_mut().unwrap().remove(field);
        assert!(clients(MediaService::Sonarr, &bytes(&rows)).is_err());
        let mut rows = fixture(MediaService::Sonarr);
        rows[0][field] = Value::Null;
        assert!(clients(MediaService::Sonarr, &bytes(&rows)).is_err());
    }
    for id in [json!(0), json!(-1), json!(u32::MAX), json!(1.5), json!("1")] {
        let mut rows = fixture(MediaService::Sonarr);
        rows[0]["id"] = id;
        assert!(clients(MediaService::Sonarr, &bytes(&rows)).is_err());
    }
}
#[test]
fn object_only_lists_raw_body_limit_and_fixed_errors_are_enforced() {
    for value in [
        json!({"records":[]}),
        json!([[]]),
        json!([null]),
        json!(null),
    ] {
        assert!(clients(MediaService::Sonarr, &bytes(&value)).is_err());
    }
    let mut raw = bytes(&fixture(MediaService::Sonarr));
    raw.resize(32768, b' ');
    assert!(clients(MediaService::Sonarr, &raw).is_ok());
    raw.push(b' ');
    assert!(clients(MediaService::Sonarr, &raw).is_err());
    for raw in [b"PRIVATE_INVALID".as_slice(), b"[] trailing", b"\xff"] {
        let error = clients(MediaService::Sonarr, raw).unwrap_err();
        assert_eq!(error.0, ErrorCode::Unavailable);
        assert!(!format!("{error:?}").contains("PRIVATE"));
    }
    assert!(clients(MediaService::Jellyfin, b"[]").is_err());
}
#[test]
fn registration_field_and_text_limits_are_admitted_without_truncation() {
    let base = fixture(MediaService::Sonarr)[0].clone();
    let mut rows: Vec<_> = (1..=16)
        .map(|id| {
            let mut row = base.clone();
            row["id"] = json!(id);
            row
        })
        .collect();
    assert_eq!(
        clients(MediaService::Sonarr, &bytes(&rows)).unwrap().len(),
        16
    );
    let mut extra = base.clone();
    extra["id"] = json!(17);
    rows.push(extra);
    assert!(clients(MediaService::Sonarr, &bytes(&rows)).is_err());
    let mut row = base;
    row["fields"] = json!(
        (0..64)
            .map(|i| json!({"name":format!("unused{i}"),"value":i}))
            .collect::<Vec<_>>()
    );
    assert!(clients(MediaService::Sonarr, &bytes(&vec![row.clone()])).is_ok());
    row["fields"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"unused64","value":64}));
    assert!(clients(MediaService::Sonarr, &bytes(&vec![row])).is_err());
    let mut rows = fixture(MediaService::Sonarr);
    rows[0]["name"] = json!("é".repeat(64));
    assert!(clients(MediaService::Sonarr, &bytes(&rows)).is_ok());
    rows[0]["name"] = json!("é".repeat(65));
    assert!(clients(MediaService::Sonarr, &bytes(&rows)).is_err());
}
#[test]
fn duplicate_ids_refuse_but_duplicate_native_matches_reach_domain_conflict() {
    let mut rows = fixture(MediaService::Sonarr);
    rows[1]["id"] = rows[0]["id"].clone();
    assert!(clients(MediaService::Sonarr, &bytes(&rows)).is_err());
    let mut rows = fixture(MediaService::Sonarr);
    let mut copy = rows[0].clone();
    copy["id"] = json!(13);
    rows.push(copy);
    let result = preview(
        MediaService::Sonarr,
        &clients(MediaService::Sonarr, &bytes(&rows)).unwrap(),
    );
    assert_eq!(result.outcome, Outcome::Blocked);
    assert!(matches!(
        result.rows[0].decision,
        Decision::Blocked {
            reason: domain::Conflict::DuplicateMatch,
            ..
        }
    ));
}
