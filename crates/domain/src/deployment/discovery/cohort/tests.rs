use super::*;
use serde_json::{Value, json};

fn fixture() -> Value {
    json!({"version":1,"catalog_current":true,"engine_current":true,"catalog_valid_for_millis":90000,"engine_valid_for_millis":90000,"catalog":{"engine_id":"engine","sampled_at":100,"stacks":[{
        "stack":"media","project":"media","working_directory":"/opt/stacks/Media",
        "config_files":["/opt/stacks/Media/compose.yaml"],"services":["web"],"files":{"state":"not_inspected"}
    }]},"engine":{"engine_id":"engine","sampled_at":99,"members":[{
        "kind":"compose","container":{"resource":format!("container:{:064x}",1),"image":format!("sha256:{}","a".repeat(64)),"started_at":"2026-10-20T00:00:00Z","running":true},
        "project":{"name":"media","working_directory":"/opt/stacks/Media","config_files":["/opt/stacks/Media/compose.yaml"]},"service":"web","one_off":false
    }],"truncated":0,"omitted":[]}})
}
fn decode(value: Value) -> Result<DiscoveryCohort> {
    DiscoveryCohort::decode(&serde_json::to_vec(&value).unwrap())
}

#[test]
fn round_trip_preserves_source_clocks_and_failed_source_withholds_counts() {
    let cohort = decode(fixture()).unwrap();
    let bytes = serde_json::to_vec(&cohort).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), fixture());
    let fresh = cohort.project(101).unwrap();
    assert_eq!(fresh.stacks[0].runtime, RuntimeStatus::Running);
    assert_eq!(fresh.truncated_containers, Some(0));
    // A report from before the truncation count decodes as complete; a
    // truncated one carries its count to the fresh view only.
    let mut older = fixture();
    older["engine"].as_object_mut().unwrap().remove("truncated");
    older["engine"].as_object_mut().unwrap().remove("omitted");
    assert_eq!(decode(older).unwrap().engine().unwrap().truncated, 0);
    let mut over = fixture();
    over["engine"]["truncated"] = json!(3);
    let over = decode(over).unwrap();
    assert_eq!(over.project(101).unwrap().truncated_containers, Some(3));
    assert_eq!(
        over.project_sources(101, true, false)
            .unwrap()
            .truncated_containers,
        None
    );
    for (catalog, engine) in [(false, true), (true, false), (false, false)] {
        let view = cohort.project_sources(101, catalog, engine).unwrap();
        assert_eq!(view.catalog.sampled_at, Some(100));
        assert_eq!(view.engine.sampled_at, Some(99));
        assert_eq!(view.stacks[0].runtime, RuntimeStatus::Unknown);
        assert_eq!(view.stacks[0].counts, None);
        assert_eq!(view.engine_matches_catalog, None);
    }
    assert_eq!(
        cohort.project(198).unwrap().engine.state,
        SourceState::Stale
    );
    assert_eq!(
        cohort.project(98).unwrap().catalog.state,
        SourceState::Stale
    );
}

#[test]
fn an_omitted_member_marks_its_stack_and_only_while_the_engine_is_fresh() {
    let mut value = fixture();
    value["engine"]["omitted"] = json!([
        {"resource":format!("container:{:064x}",2),"project":"media","reason":"changing"},
        {"resource":format!("container:{:064x}",3),"project":null,"reason":"unsupported"}
    ]);
    let cohort = decode(value).unwrap();
    let view = cohort.project(101).unwrap();
    assert_eq!(view.omitted_containers.len(), 2);
    assert_eq!(view.omitted_containers[0].reason, OmissionReason::Changing);
    assert_eq!(view.stacks[0].issues, [StackIssue::OmittedMembers]);
    let stale = cohort.project_sources(101, true, false).unwrap();
    assert!(stale.omitted_containers.is_empty());
    assert!(stale.stacks[0].issues.is_empty());
    // A left-out container cannot also be a member, repeat, or name a bad project.
    for bad in [
        json!({"resource":format!("container:{:064x}",1),"project":"media","reason":"changing"}),
        json!({"resource":"container:nothex","project":null,"reason":"changing"}),
        json!({"resource":format!("container:{:064x}",2),"project":"Bad Name","reason":"changing"}),
        json!({"resource":format!("container:{:064x}",2),"project":null,"reason":"gone"}),
    ] {
        let mut value = fixture();
        value["engine"]["omitted"] = json!([bad]);
        assert!(decode(value).is_err());
    }
    let mut repeated = fixture();
    let one = json!({"resource":format!("container:{:064x}",2),"project":null,"reason":"changing"});
    repeated["engine"]["omitted"] = json!([one.clone(), one]);
    assert!(decode(repeated).is_err());
}

#[test]
fn missing_catalog_never_proves_an_unregistered_project_or_empty_registry() {
    let mut value = fixture();
    value["catalog"] = Value::Null;
    value["catalog_current"] = json!(false);
    value["catalog_valid_for_millis"] = json!(0);
    let view = decode(value).unwrap().project(101).unwrap();
    assert!(view.stacks.is_empty());
    assert_eq!(view.catalog.state, SourceState::Unavailable);
    assert_eq!(
        view.other_projects[0].relation,
        RegistrationRelation::CatalogUnknown
    );
}

#[test]
fn refuses_nested_cardinality_text_and_cross_source_identity_defects() {
    for pointer in [
        "/catalog/stacks/0/services",
        "/catalog/stacks/0/config_files",
        "/engine/members/0/project/config_files",
        "/catalog/stacks",
    ] {
        let mut value = fixture();
        let rows = value.pointer_mut(pointer).unwrap().as_array_mut().unwrap();
        let row = rows[0].clone();
        for _ in 0..17 {
            rows.push(row.clone());
        }
        assert!(decode(value).is_err(), "{pointer}");
    }
    for (pointer, size) in [
        ("/catalog/engine_id", 129),
        ("/catalog/stacks/0/stack", 65),
        ("/engine/members/0/project/working_directory", 513),
        ("/engine/members/0/container/started_at", 65),
    ] {
        let mut value = fixture();
        *value.pointer_mut(pointer).unwrap() = json!("x".repeat(size));
        assert!(decode(value).is_err(), "{pointer}");
    }
    for pointer in ["/catalog/sampled_at", "/engine/sampled_at"] {
        let mut value = fixture();
        *value.pointer_mut(pointer).unwrap() = json!(-1);
        assert!(decode(value).is_err());
    }
    let mut value = fixture();
    value["catalog"]["stacks"][0]["services"] = json!(["web", "web"]);
    assert!(decode(value).is_err());
    let mut value = fixture();
    value["version"] = json!(2);
    assert!(decode(value).is_err());
    assert!(DiscoveryCohort::decode(&vec![b' '; MAX_REPORT_BYTES + 1]).is_err());
}

#[test]
fn variants_reject_extraneous_null_duplicate_and_authority_fields() {
    for (field, value) in [
        ("principal", json!("admin")),
        ("approval", json!(true)),
        ("version", json!(2)),
    ] {
        let mut wire = fixture();
        wire[field] = value;
        assert!(decode(wire).is_err());
    }
    for (kind, field, value) in [
        ("standalone", "project", Value::Null),
        ("standalone", "one_off", json!(false)),
        ("compose", "service", Value::Null),
    ] {
        let mut wire = fixture();
        wire["engine"]["members"][0]["kind"] = json!(kind);
        wire["engine"]["members"][0][field] = value;
        assert!(decode(wire).is_err());
    }
    let text = serde_json::to_string(&fixture()).unwrap().replacen(
        "\"sampled_at\":100",
        "\"sampled_at\":100,\"sampled_at\":100",
        1,
    );
    assert!(DiscoveryCohort::decode(text.as_bytes()).is_err());
}

#[test]
fn excess_nested_value_is_refused_before_traversal_in_any_member_field_order() {
    // A malformed excess value would yield a syntax error if traversed. Check
    // the bound's error even with the discriminator following the large field.
    let text = r#"{"version":1,"engine":{"engine_id":"e","sampled_at":0,"members":[{"project":{"name":"m","working_directory":"/m","config_files":["/a","/b","/c","/d",[broken]},"kind":"compose"}]}}"#;
    let error = serde_json::from_str::<DiscoveryCohort>(text)
        .unwrap_err()
        .to_string();
    assert!(error.contains("stack array limit"), "{error}");
}
