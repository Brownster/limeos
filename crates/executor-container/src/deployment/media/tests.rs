use super::*;
use limeos_domain::{
    ComposeMount, StorageContract,
    deployment::{
        discovery::ProjectIdentity,
        media::{
            self as model, Completeness, MediaService, Observation, Records, SeedContext,
            ServiceIdentity,
        },
    },
};
use serde_json::{Value, json};

const ROOTS: &[u8] =
    include_bytes!("../../../../../tests/fixtures/deployment/media/rootfolders.json");
const LIBRARIES: &[u8] =
    include_bytes!("../../../../../tests/fixtures/deployment/media/libraries.json");
fn roots(value: &Value) -> Result<Vec<RootFolder>> {
    root_folders(&serde_json::to_vec(value).unwrap())
}
fn libs(value: &Value) -> Result<Vec<Library>> {
    libraries(&serde_json::to_vec(value).unwrap())
}

#[test]
fn frozen_fixture_projection_retains_only_selected_values() {
    let roots = root_folders(ROOTS).unwrap();
    let libraries = libraries(LIBRARIES).unwrap();
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0].path, "/downloads/complete/sonarr");
    assert_eq!(libraries[0].name, "Movies");
    assert_eq!(libraries[0].paths, ["/media/movies"]);
    for output in [
        format!("{roots:?}"),
        format!("{libraries:?}"),
        serde_json::to_string(&roots).unwrap(),
        serde_json::to_string(&libraries).unwrap(),
    ] {
        assert!(!output.contains("MEDIA_PRIVATE_VALUE"));
        assert!(!output.contains("apiKey"));
        assert!(!output.contains("LibraryOptions"));
    }
}
#[test]
fn frozen_jellyfin_aliases_are_supported_without_last_field_wins() {
    for (name, kind, paths) in [
        ("Name", "CollectionType", "Locations"),
        ("name", "collectionType", "locations"),
        ("name", "collection_type", "paths"),
    ] {
        let mut item = serde_json::Map::new();
        item.insert(name.into(), json!("Movies"));
        item.insert(kind.into(), json!("movies"));
        item.insert(paths.into(), json!(["/media/movies"]));
        assert_eq!(libs(&json!([item])).unwrap()[0].paths, ["/media/movies"]);
    }
    for text in [
        r#"[{"Name":"Movies","name":"Other","CollectionType":"movies","Locations":[]}]"#,
        r#"[{"Name":"Movies","CollectionType":"movies","collection_type":"tv","Locations":[]}]"#,
        r#"[{"Name":"Movies","CollectionType":"movies","Locations":[],"paths":[]}]"#,
    ] {
        assert!(libraries(text.as_bytes()).is_err());
    }
}
#[test]
fn consumed_duplicate_root_fields_are_refused() {
    for text in [
        r#"[{"id":1,"id":2,"path":"/tv"}]"#,
        r#"[{"id":1,"path":"/tv","path":"/movies"}]"#,
    ] {
        assert!(root_folders(text.as_bytes()).is_err());
    }
}
#[test]
fn records_must_be_objects_in_a_complete_array() {
    for text in [
        "null",
        "{}",
        "[null]",
        "[[1,\"/tv\"]]",
        "[]extra",
        "[1]",
        "[{}]",
    ] {
        assert!(root_folders(text.as_bytes()).is_err());
    }
    for text in [
        "null",
        "{}",
        "[null]",
        "[[\"Movies\",\"movies\",[]]]",
        "[]extra",
        "[{}]",
    ] {
        assert!(libraries(text.as_bytes()).is_err());
    }
    assert!(root_folders(b"[]").unwrap().is_empty());
    assert!(libraries(b"[]").unwrap().is_empty());
}
#[test]
fn missing_wrong_types_and_invalid_root_ids_are_refused() {
    for item in [
        json!({"id":0,"path":"/tv"}),
        json!({"id":-1,"path":"/tv"}),
        json!({"id":2147483648u32,"path":"/tv"}),
        json!({"id":"1","path":"/tv"}),
        json!({"id":1}),
        json!({"path":"/tv"}),
        json!({"id":1,"path":null}),
    ] {
        assert!(roots(&json!([item])).is_err());
    }
    for item in [
        json!({"Name":"Movies","CollectionType":"movies"}),
        json!({"Name":null,"CollectionType":"movies","Locations":[]}),
        json!({"Name":"Movies","CollectionType":1,"Locations":[]}),
        json!({"Name":"Movies","CollectionType":"movies","Locations":null}),
    ] {
        assert!(libs(&json!([item])).is_err());
    }
}
#[test]
fn bounds_precede_unbounded_record_or_path_retention() {
    let mut r: Vec<_> = (1..=MAX_RECORDS)
        .map(|id| json!({"id":id,"path":format!("/root{id}")}))
        .collect();
    assert_eq!(roots(&json!(r)).unwrap().len(), MAX_RECORDS);
    r.push(json!({"id":99,"path":"/extra"}));
    assert!(roots(&json!(r)).is_err());
    let paths: Vec<_> = (0..MAX_PATHS).map(|i| format!("/media/path{i}")).collect();
    let mut row = json!({"Name":"Movies","CollectionType":"movies","Locations":paths});
    assert_eq!(
        libs(&json!([row.clone()])).unwrap()[0].paths.len(),
        MAX_PATHS
    );
    row["Locations"]
        .as_array_mut()
        .unwrap()
        .push(json!("/extra"));
    assert!(libs(&json!([row])).is_err());
    let items: Vec<_> = (0..=MAX_RECORDS)
        .map(|i| json!({"Name":format!("Library{i}"),"CollectionType":"movies","Locations":[]}))
        .collect();
    assert!(libs(&json!(items)).is_err());
}
#[test]
fn raw_byte_limit_is_inclusive_and_applies_before_parse() {
    let mut bytes = b"[]".to_vec();
    bytes.resize(MAX_RESPONSE_BYTES, b' ');
    assert!(root_folders(&bytes).is_ok());
    assert!(libraries(&bytes).is_ok());
    bytes.push(b' ');
    assert!(root_folders(&bytes).is_err());
    assert!(libraries(&bytes).is_err());
    assert!(root_folders(&vec![b'['; MAX_RESPONSE_BYTES + 1]).is_err());
}
#[test]
fn literal_paths_keep_case_and_trailing_slash_but_refuse_traversal_and_templates() {
    let result = roots(&json!([{"id":1,"path":"/Media/TV/"}])).unwrap();
    assert_eq!(result[0].path, "/Media/TV/");
    for path in [
        "/",
        "relative",
        "/Media/../TV",
        "/Media//TV",
        "/Media\nTV",
        "/{{MEDIA}}",
        "/${MEDIA}",
        &"x".repeat(513),
    ] {
        let error = roots(&json!([{"id":1,"path":path}])).unwrap_err();
        assert_eq!(error.0, ErrorCode::Unavailable);
        assert!(!format!("{error:?}").contains(path));
    }
}
#[test]
fn library_name_type_paths_and_duplicate_path_bounds_are_explicit() {
    for item in [
        json!({"Name":"x".repeat(129),"CollectionType":"movies","Locations":[]}),
        json!({"Name":"Movies","CollectionType":"x".repeat(129),"Locations":[]}),
        json!({"Name":"Movies\nprivate","CollectionType":"movies","Locations":[]}),
        json!({"Name":"Movies","CollectionType":"movies","Locations":["/media/movies","/media/movies/"]}),
    ] {
        assert!(libs(&json!([item])).is_err());
    }
}
#[test]
fn decoded_records_feed_preview_without_granting_source_or_effect_authority() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/storage-contracts.json"
    ))
    .unwrap();
    let c: StorageContract =
        serde_json::from_value(fixture["cases"][0]["contract"].clone()).unwrap();
    let id = ServiceIdentity {
        stack: "media".into(),
        engine_id: "engine-a".into(),
        project: ProjectIdentity {
            name: "media".into(),
            working_directory: "/opt/stacks/Media".into(),
            config_files: vec!["/opt/stacks/Media/compose.yaml".into()],
        },
        service: "sonarr".into(),
        application: MediaService::Sonarr,
    };
    let mounts = [ComposeMount::Bind {
        source: format!("{}/tv", c.locations.media_host),
        target: "/tv".into(),
        read_only: false,
    }];
    let rows = root_folders(ROOTS).unwrap();
    let context = || SeedContext {
        identity: &id,
        declared_media_identity: Some(c.media_identity.clone()),
        mounts: &mounts,
    };
    let observation = |completeness| Observation {
        identity: &id,
        sampled_at: 100,
        completeness,
        records: Records::Roots(&rows),
    };
    let partial =
        model::preview(101, &c, context(), Some(observation(Completeness::Partial))).unwrap();
    assert_eq!(partial.outcome, model::Outcome::Unknown);
    assert!(partial.removal_candidates.is_empty());
    let complete = model::preview(
        101,
        &c,
        context(),
        Some(observation(Completeness::Complete)),
    )
    .unwrap();
    assert_eq!(complete.outcome, model::Outcome::ReviewRequired);
    assert_eq!(complete.unresolved_gates.len(), 6);
    assert!(
        !serde_json::to_string(&complete)
            .unwrap()
            .contains("MEDIA_PRIVATE_VALUE")
    );
}
