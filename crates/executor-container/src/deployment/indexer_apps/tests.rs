use super::*;
use limeos_domain::{
    ContainerSnapshot,
    deployment::{
        discovery::ProjectIdentity,
        indexer_apps::{self as domain, Conflict, Decision, Observation, Outcome, Request, Target},
    },
};
use serde_json::json;

fn fixture(frozen: bool) -> Vec<Value> {
    serde_json::from_str(if frozen {
        include_str!("../../../../../tests/fixtures/deployment/indexer-apps/frozen-tv-for-all.json")
    } else {
        include_str!("../../../../../tests/fixtures/deployment/indexer-apps/native.json")
    })
    .unwrap()
}
fn bytes(value: &impl serde::Serialize) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}
fn preview(apps: &[Application]) -> domain::Preview {
    let target = Target {
        stack: "media".into(),
        engine_id: "engine-a".into(),
        service: "prowlarr".into(),
        project: ProjectIdentity {
            name: "media".into(),
            working_directory: "/opt/Media".into(),
            config_files: vec!["/opt/Media/compose.yaml".into()],
        },
        container: ContainerSnapshot {
            resource: format!("container:{}", "a".repeat(64)),
            image: format!("sha256:{}", "b".repeat(64)),
            started_at: "2026-10-16T01:02:03Z".into(),
            running: true,
        },
    };
    let requests: Vec<_> = [
        (App::Sonarr, 5000),
        (App::Radarr, 2000),
        (App::Lidarr, 3000),
    ]
    .into_iter()
    .map(|(application, root)| Request {
        application,
        service: application.implementation().to_ascii_lowercase(),
        categories: vec![root, root + 30, root + 40],
        anime_categories: (application == App::Sonarr).then(|| vec![5070]),
        tags: vec![],
    })
    .collect();
    let services = ["prowlarr", "sonarr", "radarr", "lidarr"].map(String::from);
    domain::preview(
        101,
        &target,
        &requests,
        Some(Observation {
            target: &target,
            sampled_at: 100,
            complete: true,
            declared_services: &services,
            applications: apps,
        }),
    )
    .unwrap()
}
#[test]
fn native_fixture_rerun_matches_without_duplicate_additions() {
    let decoded = applications(&bytes(&fixture(false))).unwrap();
    let r = preview(&decoded);
    assert_eq!(r.outcome, Outcome::NoAdditions);
    assert!(r.additions.is_empty());
    assert!(
        r.rows
            .iter()
            .all(|r| matches!(r.decision, Decision::MatchingMetadata { .. }))
    );
    assert_eq!(decoded[1].settings.as_ref().unwrap().anime_categories, None);
    assert_eq!(preview(&[]).additions.len(), 3);
}
#[test]
fn frozen_tv_for_all_regression_blocks_movies_and_music_without_silent_updates() {
    let r = preview(&applications(&bytes(&fixture(true))).unwrap());
    assert_eq!(r.outcome, Outcome::Blocked);
    assert!(r.additions.is_empty());
    assert!(matches!(
        r.rows[0].decision,
        Decision::MatchingMetadata { .. }
    ));
    for index in [1, 2] {
        assert!(matches!(
            r.rows[index].decision,
            Decision::Blocked {
                reason: Conflict::PolicyDiffers,
                ..
            }
        ));
    }
}
#[test]
fn credentials_endpoints_and_unknown_provider_fields_stay_out_of_selected_metadata() {
    let decoded = applications(&bytes(&fixture(false))).unwrap();
    for text in [
        format!("{decoded:?}"),
        serde_json::to_string(&preview(&decoded)).unwrap(),
    ] {
        for sentinel in [
            "INDEXER_PRIVATE_VALUE",
            "PRIVATE_APPLICATION_ENDPOINT",
            "PRIVATE_CALLBACK_ENDPOINT",
            "apiKey",
            "baseUrl",
            "prowlarrUrl",
        ] {
            assert!(!text.contains(sentinel));
        }
    }
}
#[test]
fn missing_null_and_unknown_selected_metadata_remain_incomplete() {
    for field in ["syncLevel", "tags"] {
        let mut rows = fixture(false);
        rows[0].as_object_mut().unwrap().remove(field);
        let decoded = applications(&bytes(&rows)).unwrap();
        assert!(decoded[0].settings.is_none());
        assert_eq!(preview(&decoded).outcome, Outcome::Blocked);
        rows[0][field] = Value::Null;
        assert!(applications(&bytes(&rows)).unwrap()[0].settings.is_none());
    }
    for level in ["FULLSYNC", "newFutureLevel"] {
        let mut rows = fixture(false);
        rows[0]["syncLevel"] = json!(level);
        assert!(applications(&bytes(&rows)).unwrap()[0].settings.is_none());
    }
    for fields in [
        json!([]),
        json!([{"name":"syncCategories","value":null}]),
        json!([{"name":"syncCategories","value":[5000]}]),
    ] {
        let mut rows = fixture(false);
        rows[0]["fields"] = fields;
        assert!(applications(&bytes(&rows)).unwrap()[0].settings.is_none());
    }
}
#[test]
fn duplicate_consumed_fields_casing_aliases_and_null_first_values_refuse() {
    for fields in [
        json!([{"name":"syncCategories","value":null},{"name":"syncCategories","value":[5000]}]),
        json!([{"name":"SyncCategories","value":[5000]}]),
        json!([{"name":"syncCategories","value":[5000]},{"name":"animeSyncCategories","value":null},{"name":"animeSyncCategories","value":[5070]}]),
    ] {
        let mut rows = fixture(false);
        rows[0]["fields"] = fields;
        assert!(applications(&bytes(&rows)).is_err());
    }
    for raw in [
        br#"[{"id":1,"id":2,"name":"Sonarr","implementation":"Sonarr","fields":[]}]"#.as_slice(),
        br#"[{"id":1,"name":"Sonarr","implementation":"Sonarr","tags":null,"tags":[],"fields":[]}]"#,
        br#"[{"id":1,"name":"Sonarr","implementation":"Sonarr","fields":[{"name":"syncCategories","value":null,"value":[5000]}]}]"#,
    ] { assert!(applications(raw).is_err()); }
}
#[test]
fn wrong_selected_types_ids_and_duplicate_set_members_refuse() {
    for value in [
        json!(true),
        json!("5000"),
        json!([0]),
        json!([-1]),
        json!([1.5]),
        json!(["5000"]),
        json!([5000, 5000]),
    ] {
        let mut rows = fixture(false);
        rows[0]["fields"][0]["value"] = value;
        assert!(applications(&bytes(&rows)).is_err());
    }
    for field in ["id", "name", "implementation", "fields"] {
        let mut rows = fixture(false);
        rows[0].as_object_mut().unwrap().remove(field);
        assert!(applications(&bytes(&rows)).is_err());
    }
    for field in ["syncLevel", "tags"] {
        let mut rows = fixture(false);
        rows[0][field] = json!(true);
        assert!(applications(&bytes(&rows)).is_err());
    }
    let mut rows = fixture(false);
    rows[0]["tags"] = json!([1, 1]);
    assert!(applications(&bytes(&rows)).is_err());
}
#[test]
fn duplicate_ids_refuse_but_duplicate_implementations_reach_domain_ambiguity() {
    let mut rows = fixture(false);
    rows[1]["id"] = rows[0]["id"].clone();
    assert!(applications(&bytes(&rows)).is_err());
    let mut rows = fixture(false);
    let mut duplicate = rows[0].clone();
    duplicate["id"] = json!(14);
    rows.push(duplicate);
    let r = preview(&applications(&bytes(&rows)).unwrap());
    assert!(matches!(
        r.rows[0].decision,
        Decision::Blocked {
            reason: Conflict::AmbiguousImplementation,
            ..
        }
    ));
    assert!(r.additions.is_empty());
}
#[test]
fn row_field_set_and_utf8_bounds_refuse_without_truncation() {
    let base = fixture(false)[0].clone();
    let mut rows: Vec<_> = (1..=16)
        .map(|id| {
            let mut row = base.clone();
            row["id"] = json!(id);
            row
        })
        .collect();
    assert_eq!(applications(&bytes(&rows)).unwrap().len(), 16);
    let mut extra = base.clone();
    extra["id"] = json!(17);
    rows.push(extra);
    assert!(applications(&bytes(&rows)).is_err());
    let mut row = base;
    row["fields"] = json!(
        (0..64)
            .map(|i| json!({"name":format!("unused{i}"),"value":i}))
            .collect::<Vec<_>>()
    );
    assert!(applications(&bytes(&vec![row.clone()])).is_ok());
    row["fields"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"unused64","value":64}));
    assert!(applications(&bytes(&vec![row])).is_err());
    for field in ["tags", "categories"] {
        let mut rows = fixture(false);
        let ids = json!((1..=16).collect::<Vec<_>>());
        if field == "tags" {
            rows[0]["tags"] = ids;
        } else {
            rows[0]["fields"][0]["value"] = ids;
        }
        assert!(applications(&bytes(&rows)).is_ok());
        if field == "tags" {
            rows[0]["tags"].as_array_mut().unwrap().push(json!(17));
        } else {
            rows[0]["fields"][0]["value"]
                .as_array_mut()
                .unwrap()
                .push(json!(17));
        }
        assert!(applications(&bytes(&rows)).is_err());
    }
    let mut rows = fixture(false);
    rows[0]["name"] = json!("é".repeat(64));
    assert!(applications(&bytes(&rows)).is_ok());
    rows[0]["name"] = json!("é".repeat(65));
    assert!(applications(&bytes(&rows)).is_err());
}
#[test]
fn object_shapes_raw_body_ceiling_and_fixed_errors_are_enforced() {
    for value in [
        json!({"records":[]}),
        json!([[]]),
        json!([null]),
        json!(null),
    ] {
        assert!(applications(&bytes(&value)).is_err());
    }
    let mut raw = bytes(&fixture(false));
    raw.resize(32768, b' ');
    assert!(applications(&raw).is_ok());
    raw.push(b' ');
    assert!(applications(&raw).is_err());
    for raw in [b"PRIVATE_INVALID".as_slice(), b"[] trailing", b"\xff"] {
        let e = applications(raw).unwrap_err();
        assert_eq!(e.0, ErrorCode::Unavailable);
        assert!(!format!("{e:?}").contains("PRIVATE"));
    }
}
#[test]
fn unrelated_implementations_are_bounded_ordinary_metadata_not_executable_providers() {
    let rows =
        json!([{"id":7,"name":"Other instance","implementation":"OtherApplication","fields":[]}]);
    let decoded = applications(&bytes(&rows)).unwrap();
    assert!(decoded[0].settings.is_none());
    assert_eq!(preview(&decoded).additions.len(), 3);
    let mut rows = rows;
    rows[0]["name"] = json!("Sonarr");
    let r = preview(&applications(&bytes(&rows)).unwrap());
    assert!(matches!(
        r.rows[0].decision,
        Decision::Blocked {
            reason: Conflict::NameCollision,
            ..
        }
    ));
}
