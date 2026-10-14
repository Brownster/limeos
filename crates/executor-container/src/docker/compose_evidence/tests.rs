use super::*;
use crate::docker::read_admission::{Admission, EngineReadProfile};
use serde_json::json;

fn container() -> ContainerSnapshot {
    ContainerSnapshot {
        resource: format!("container:{}", "a".repeat(64)),
        image: format!("sha256:{}", "f".repeat(64)),
        started_at: "2026-10-07T00:00:00Z".into(),
        running: false,
    }
}
fn labels() -> Value {
    json!({PROJECT:"media",SERVICE:"jellyfin",DIRECTORY:"/opt/stacks/media",
        FILES:"/opt/stacks/media/compose.yaml,/opt/stacks/media/override.yaml",ONEOFF:"False"})
}
fn parsed(labels: Value) -> Result<ComposeContainerMembership> {
    let admission = Admission::new(EngineReadProfile::CombinedV1);
    parse(
        &json!({"Config":{"Labels":labels,"Env":["TOKEN=private-value"]}}),
        &container(),
        &mut admission.scope(),
    )
}

#[test]
fn explicit_null_empty_and_unrelated_string_labels_are_non_compose() {
    for labels in [Value::Null, json!({}), json!({"secret":"private-value"})] {
        assert!(matches!(
            parsed(labels).unwrap().ownership,
            ComposeOwnership::NonCompose
        ));
    }
}
#[test]
fn partial_markers_missing_config_nonstring_and_special_members_never_become_unmanaged() {
    for key in [PROJECT, SERVICE, DIRECTORY, FILES, ONEOFF] {
        let mut value = labels();
        value.as_object_mut().unwrap().remove(key);
        assert!(parsed(value).is_err());
    }
    for value in [
        json!({"com.docker.compose.config-hash":"only-a-marker"}),
        json!({"secret":1}),
        json!([]),
        json!(1),
    ] {
        assert!(parsed(value).is_err());
    }
    for key in ["com.docker.compose.hook", "com.docker.compose.relay"] {
        let mut value = labels();
        value[key] = json!("active");
        assert!(parsed(value).is_err());
    }
    let admission = Admission::new(EngineReadProfile::CombinedV1);
    for value in [json!({}), json!({"Config":{}}), json!({"Config":null})] {
        assert!(parse(&value, &container(), &mut admission.scope()).is_err());
    }
}
#[test]
fn project_and_service_names_have_exact_literal_64_byte_grammars() {
    let mut value = labels();
    value[PROJECT] = json!("p".repeat(64));
    value[SERVICE] = json!("S".repeat(64));
    assert!(parsed(value.clone()).is_ok());
    for (key, text) in [
        (PROJECT, "p".repeat(65)),
        (SERVICE, "s".repeat(65)),
        (PROJECT, "Media".into()),
        (PROJECT, ".media".into()),
        (PROJECT, "m.edia".into()),
        (SERVICE, "_service".into()),
        (SERVICE, "private\nvalue".into()),
    ] {
        let mut bad = value.clone();
        bad[key] = json!(text);
        assert!(parsed(bad).is_err());
    }
    value[SERVICE] = json!("Media.service-1");
    assert!(parsed(value).is_ok());
}
#[test]
fn paths_accept_exact_byte_and_component_limits_and_refuse_noncanonical_names() {
    let mut value = labels();
    value[DIRECTORY] = json!(format!("/{}", "d".repeat(511)));
    value[FILES] = json!(format!("/{}", "f".repeat(511)));
    assert!(parsed(value.clone()).is_ok());
    for text in [
        "relative".into(),
        "/".into(),
        "/a/".into(),
        "/a//b".into(),
        "/a/./b".into(),
        "/a/../b".into(),
        "/private\nvalue".into(),
        "/a,b".into(),
        format!("/{}", "x".repeat(512)),
        format!("/{}", "d/".repeat(16) + "d"),
    ] {
        let mut bad = value.clone();
        bad[DIRECTORY] = json!(text);
        assert!(parsed(bad).is_err());
    }
    value[DIRECTORY] = json!(format!("/{}", "d/".repeat(15) + "d"));
    assert!(parsed(value).is_ok());
}
#[test]
fn four_ordered_distinct_files_pass_without_defaulting_trimming_or_clipping() {
    let mut value = labels();
    value[FILES] = json!("/a.yml,/b.yml,/c.yml,/d.yml");
    let result = parsed(value.clone()).unwrap();
    let ComposeOwnership::Compose { project, .. } = result.ownership else {
        panic!()
    };
    assert_eq!(
        project.config_files,
        vec!["/a.yml", "/b.yml", "/c.yml", "/d.yml"]
    );
    for files in [
        "",
        "/a.yml,",
        "/a.yml,,/b.yml",
        "/a.yml,/a.yml",
        "/a.yml,/b.yml,/c.yml,/d.yml,/e.yml",
        "/a.yml, /b.yml",
        "compose.yml",
    ] {
        value[FILES] = json!(files);
        assert!(parsed(value.clone()).is_err());
    }
    for oneoff in [json!(false), json!("false"), json!("TRUE"), json!("")] {
        let mut bad = labels();
        bad[ONEOFF] = oneoff;
        assert!(parsed(bad).is_err());
    }
}
#[test]
fn project_names_cannot_merge_different_directories_or_ordered_file_selections() {
    let first = parsed(labels()).unwrap();
    let mut different = labels();
    different[SERVICE] = json!("replica");
    different[ONEOFF] = json!("True");
    assert!(consistent(&[first.clone(), parsed(different).unwrap()]).is_ok());
    for (key, text) in [
        (DIRECTORY, "/opt/other"),
        (
            FILES,
            "/opt/stacks/media/override.yaml,/opt/stacks/media/compose.yaml",
        ),
    ] {
        let mut different = labels();
        different[key] = json!(text);
        assert!(consistent(&[first.clone(), parsed(different).unwrap()]).is_err());
    }
    let mut other = labels();
    other[PROJECT] = json!("other");
    other[DIRECTORY] = json!("/opt/other");
    assert!(consistent(&[first, parsed(other).unwrap()]).is_ok());
}
#[test]
fn full_container_identity_is_preserved_and_unknown_labels_environment_are_not_reported() {
    let mut value = labels();
    value["credential"] = json!("private-value");
    value["com.docker.compose.project.environment_file"] = json!("/private.env");
    let result = parsed(value).unwrap();
    assert_eq!(result.container, container());
    let body = serde_json::to_string(&result).unwrap();
    assert!(!body.contains("private-value"));
    assert!(!body.contains("TOKEN"));
    assert!(!body.contains("private.env"));
}
fn capture() -> Capture {
    Capture::new(EngineComposeSnapshot {
        version: 1,
        engine_id: "synthetic-engine".into(),
        containers_observed_at: 100,
        containers: vec![],
    })
}
#[test]
fn failed_or_dropped_validation_permanently_invalidates_the_capture() {
    let owner = capture();
    owner.begin().unwrap().finish(Ok(())).unwrap();
    let validation = owner.begin().unwrap();
    drop(validation);
    assert!(owner.begin().is_err());
    let owner = capture();
    assert!(owner.begin().unwrap().finish(Err(refused())).is_err());
    assert!(owner.begin().is_err());
}
#[test]
fn concurrent_refusal_cannot_be_cleared_by_the_first_successful_validation() {
    let owner = capture();
    let first = owner.begin().unwrap();
    assert!(owner.begin().is_err());
    assert!(first.finish(Ok(())).is_err());
    assert!(owner.begin().is_err());
}
