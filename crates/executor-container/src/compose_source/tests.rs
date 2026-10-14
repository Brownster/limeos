use super::*;
use serde_json::{Value, json};

const OPERATOR: &[u8] = include_bytes!("../../../../tests/fixtures/compose-source/operator.json");
const DELTA: &[u8] =
    include_bytes!("../../../../tests/fixtures/compose-override/merge-delta.override.json");
const NEW: &[u8] =
    include_bytes!("../../../../tests/fixtures/compose-override/new-service.override.json");
fn minimal() -> Value {
    json!({"services":{"api":{"image":format!("example.invalid/api@sha256:{}", "a".repeat(64))}}})
}
fn source(value: &Value) -> Result<ComposeJsonSources> {
    admit_json_sources(&serde_json::to_vec(value).unwrap(), None)
}
fn refused(value: Value) {
    assert!(source(&value).is_err(), "{value}");
}
fn minimal_service() -> Value {
    minimal()["services"]["api"].clone()
}
fn port(n: u16) -> Value {
    json!({"host_ip":"127.0.0.1","published":n.to_string(),"target":80,"protocol":"tcp"})
}
fn mount(n: usize) -> Value {
    json!({"type":"bind","source":format!("/srv/m{n}"),"target":format!("/m{n}"),
           "read_only":false,"bind":{"create_host_path":false}})
}

#[test]
fn actual_compiler_artifacts_and_operator_bytes_admit_and_combine_exactly() {
    let source = admit_json_sources(OPERATOR, Some(DELTA)).unwrap();
    let service = &source.summary().current.services[0];
    assert_eq!(
        service.user,
        Some(ComposeUser {
            uid: 0,
            gid: u32::MAX
        })
    );
    assert_eq!(service.ports.len(), 2);
    assert_eq!(service.devices.len(), 2);
    assert_eq!(
        service.mounts,
        [
            ComposeMount::Volume {
                name: "cache-new".into(),
                target: "/cache".into(),
                read_only: true
            },
            ComposeMount::Bind {
                source: "/srv/new-media".into(),
                target: "/media".into(),
                read_only: false
            }
        ]
    );
    assert!(service.privileged);
    assert_eq!(source.summary().declared_volumes, ["cache", "cache-new"]);
    assert_eq!(
        source.plan(source.summary().current).unwrap().artifact(),
        DELTA.strip_suffix(b"\n").unwrap()
    );
    let empty = br#"{"services":{}}"#;
    let new = admit_json_sources(empty, Some(NEW)).unwrap();
    assert_eq!(new.summary().current.services[0].name, "worker");
    assert_eq!(
        new.plan(new.summary().current).unwrap().artifact(),
        NEW.strip_suffix(b"\n").unwrap()
    );
}

#[test]
fn minimal_defaults_and_missing_scalars_inherit_while_explicit_false_overrides() {
    let defaults_source = source(&minimal()).unwrap();
    let defaults = &defaults_source.summary().operator.services[0];
    assert_eq!(defaults.user, None);
    assert_eq!(defaults.restart, ComposeRestart::No);
    assert!(!defaults.host_network && !defaults.privileged);
    let mut base = minimal_service();
    base["user"] = json!("42:43");
    base["restart"] = json!("always");
    base["privileged"] = json!(true);
    let operator = serde_json::to_vec(&json!({"services":{"api":base}})).unwrap();
    let mut managed = minimal();
    managed["services"]["api"]["privileged"] = json!(false);
    let managed = serde_json::to_vec(&managed).unwrap();
    let joined = admit_json_sources(&operator, Some(&managed)).unwrap();
    let current = &joined.summary().current.services[0];
    assert_eq!(current.user, Some(ComposeUser { uid: 42, gid: 43 }));
    assert_eq!(current.restart, ComposeRestart::Always);
    assert!(!current.privileged);
}

#[test]
fn unrelated_operator_services_survive_and_managed_removal_retains_shutdown_and_gates() {
    let mut operator = minimal();
    operator["services"]["other"] = minimal_service();
    let operator = serde_json::to_vec(&operator).unwrap();
    let source = admit_json_sources(&operator, Some(NEW)).unwrap();
    let plan = source.plan(source.summary().operator).unwrap();
    assert_eq!(plan.summary().shutdown_required, ["worker"]);
    assert_eq!(plan.summary().desired.services.len(), 2);
    assert_eq!(plan.summary().unresolved.len(), 8);
    assert_eq!(plan.artifact(), br#"{"services":{}}"#);
}

#[test]
fn raw_fingerprints_distinguish_whitespace_but_checked_projects_and_plans_are_stable() {
    let one = admit_json_sources(OPERATOR, Some(DELTA)).unwrap();
    let mut spaced = b" \n".to_vec();
    spaced.extend_from_slice(OPERATOR);
    let two = admit_json_sources(&spaced, Some(DELTA)).unwrap();
    assert_ne!(
        one.summary().operator_fingerprint.sha256,
        two.summary().operator_fingerprint.sha256
    );
    assert_eq!(one.summary().operator, two.summary().operator);
    assert_eq!(one.summary().operator_fingerprint.bytes, OPERATOR.len());
    assert_eq!(
        one.summary().managed_fingerprint.unwrap().sha256,
        hex::encode(Sha256::digest(DELTA))
    );
    assert_eq!(
        one.plan(one.summary().current).unwrap().artifact(),
        two.plan(two.summary().current).unwrap().artifact()
    );
    let report = serde_json::to_value(one.summary()).unwrap();
    for key in ["body", "approval", "receipt", "lease", "root", "namespace"] {
        assert!(report.get(key).is_none());
    }
}

#[test]
fn unknown_fields_refuse_at_root_service_port_mount_bind_and_volume_definition() {
    for key in [
        "name", "version", "include", "networks", "x-extra", "secrets", "configs",
    ] {
        let mut value = minimal();
        value[key] = json!({});
        refused(value);
    }
    for key in [
        "environment",
        "env_file",
        "command",
        "entrypoint",
        "build",
        "extends",
        "cap_add",
        "profiles",
        "networks",
        "labels",
        "volumes_from",
    ] {
        let mut value = minimal();
        value["services"]["api"][key] = json!([]);
        refused(value);
    }
    let mut value = minimal();
    value["services"]["api"]["ports"] = json!([port(8080)]);
    value["services"]["api"]["ports"][0]["mode"] = json!("host");
    refused(value);
    for key in ["consistency", "volume", "tmpfs"] {
        let mut value = minimal();
        value["services"]["api"]["volumes"] = json!([mount(0)]);
        value["services"]["api"]["volumes"][0][key] = json!({});
        refused(value);
    }
    let mut value = minimal();
    value["services"]["api"]["volumes"] = json!([mount(0)]);
    value["services"]["api"]["volumes"][0]["bind"]["propagation"] = json!("rshared");
    refused(value);
    for key in ["driver", "external", "name", "driver_opts", "labels"] {
        let mut value = minimal();
        value["volumes"] = json!({"unused":{key:"x"}});
        refused(value);
    }
}

#[test]
fn duplicate_and_escaped_equivalent_fields_names_and_empty_definition_keys_refuse() {
    let service = serde_json::to_string(&minimal_service()).unwrap();
    for raw in [
        format!(r#"{{"services":{{"api":{service}}},"services":{{}}}}"#),
        format!(r#"{{"services":{{"api":{service},"\u0061pi":{service}}}}}"#),
        format!(r#"{{"services":{{"api":{service}}},"volumes":{{"cache":{{}},"\u0063ache":{{}}}}}}"#),
        r#"{"services":{"api":{"image":"x","\u0069mage":"y"}}}"#.into(),
        r#"{"services":{"api":{"image":"x","ports":[{"host_ip":"x","host_ip":"y","published":"80","target":80,"protocol":"tcp"}]}}}"#.into(),
        r#"{"services":{"api":{"image":"x","volumes":[{"type":"bind","source":"/srv/a","target":"/a","read_only":false,"bind":{"create_host_path":false,"create_host_path":true}}]}}}"#.into(),
    ] {
        let error = serde_json::from_str::<Document>(&raw).err().unwrap();
        assert!(error.to_string().contains("duplicate"), "{error}");
        assert!(admit_json_sources(raw.as_bytes(), None).is_err());
    }
}

#[test]
fn explicit_null_wrong_types_and_missing_required_values_never_become_defaults() {
    for key in [
        "image",
        "user",
        "restart",
        "privileged",
        "network_mode",
        "ports",
        "volumes",
        "devices",
    ] {
        let mut value = minimal();
        value["services"]["api"][key] = Value::Null;
        refused(value);
    }
    for value in [
        json!({}),
        json!({"services":null}),
        json!({"services":[]}),
        json!({"services":{"api":{}}}),
        json!({"services":{},"volumes":null}),
        json!({"services":{},"volumes":{"cache":null}}),
    ] {
        refused(value);
    }
    let mut value = minimal();
    value["services"]["api"]["ports"] = json!(["8080:80"]);
    refused(value);
    let mut value = minimal();
    value["services"]["api"]["ports"] = json!([{"published":8080,"target":80,"protocol":"tcp"}]);
    refused(value);
    let mut value = minimal();
    value["services"]["api"]["volumes"] = json!(["/srv/media:/media"]);
    refused(value);
}

#[test]
fn canonical_numeric_owners_and_published_ports_refuse_names_signs_zeros_ranges_and_overflow() {
    for user in [
        "root",
        "1000",
        "01000:1000",
        "+1:2",
        "-1:2",
        "1:02",
        "4294967296:1",
        "1:2:3",
        "${UID}:1000",
    ] {
        let mut value = minimal();
        value["services"]["api"]["user"] = json!(user);
        refused(value);
    }
    for published in ["0", "00080", "+80", "80-90", "65536", "${PORT}"] {
        let mut value = minimal();
        let mut p = port(8080);
        p["published"] = json!(published);
        value["services"]["api"]["ports"] = json!([p]);
        refused(value);
    }
    let mut value = minimal();
    value["services"]["api"]["user"] = json!("4294967295:0");
    assert_eq!(
        source(&value).unwrap().summary().current.services[0].user,
        Some(ComposeUser {
            uid: u32::MAX,
            gid: 0
        })
    );
}

#[test]
fn binds_require_explicit_no_creation_and_literal_sources_targets_and_modes() {
    for change in [
        "missing",
        "true",
        "null",
        "relative",
        "dotdot",
        "interpolation",
        "missing_read_only",
        "volume_bind",
    ] {
        let mut m = mount(0);
        match change {
            "missing" => {
                m.as_object_mut().unwrap().remove("bind");
            }
            "true" => m["bind"]["create_host_path"] = json!(true),
            "null" => m["bind"] = Value::Null,
            "relative" => m["source"] = json!("./media"),
            "dotdot" => m["target"] = json!("/media/../root"),
            "interpolation" => m["source"] = json!("${MEDIA}"),
            "missing_read_only" => {
                m.as_object_mut().unwrap().remove("read_only");
            }
            "volume_bind" => {
                m["type"] = json!("volume");
                m["source"] = json!("cache");
            }
            _ => unreachable!(),
        }
        let mut value = minimal();
        value["services"]["api"]["volumes"] = json!([m]);
        refused(value);
    }
}

#[test]
fn device_permissions_and_host_modes_are_closed_and_collisions_refuse() {
    for d in [
        "/dev/video0",
        "/dev/video0:/dev/video0",
        "/dev/video0:/dev/video0:rw",
        "/dev/video0:/dev/video0:rwm:extra",
        "/srv/file:/dev/video0:rwm",
    ] {
        let mut value = minimal();
        value["services"]["api"]["devices"] = json!([d]);
        refused(value);
    }
    for mode in [
        "bridge",
        "none",
        "service:api",
        "container:other",
        "${NETWORK}",
    ] {
        let mut value = minimal();
        value["services"]["api"]["network_mode"] = json!(mode);
        refused(value);
    }
    let mut value = minimal();
    value["services"]["api"]["network_mode"] = json!("host");
    value["services"]["api"]["ports"] = json!([port(8080)]);
    refused(value);
    let mut value = minimal();
    let mut m = mount(0);
    m["target"] = json!("/dev/video0");
    value["services"]["api"]["volumes"] = json!([m]);
    value["services"]["api"]["devices"] = json!(["/dev/video0:/dev/video0:rwm"]);
    refused(value);
}

#[test]
fn referenced_volumes_must_be_declared_and_invalid_unused_definitions_refuse() {
    let mut value = minimal();
    value["services"]["api"]["volumes"] =
        json!([{"type":"volume","source":"cache","target":"/cache","read_only":false}]);
    refused(value.clone());
    value["volumes"] = json!({"cache":{}});
    assert!(source(&value).is_ok());
    value["volumes"]["Unused"] = json!({});
    refused(value);
    let mut managed: Value = serde_json::from_slice(DELTA).unwrap();
    managed["volumes"]["unused"] = json!({"external":true});
    assert!(admit_json_sources(OPERATOR, Some(&serde_json::to_vec(&managed).unwrap())).is_err());
}

#[test]
fn maps_arrays_and_strings_refuse_excess_before_decoding_the_extra_value() {
    let service = serde_json::to_string(&minimal_service()).unwrap();
    let sixteen = (0..16)
        .map(|n| format!("\"s{n}\":{service}"))
        .collect::<Vec<_>>()
        .join(",");
    let error = serde_json::from_str::<Document>(&format!(
        "{{\"services\":{{{sixteen},\"extra\":false}}}}"
    ));
    assert!(
        error
            .err()
            .unwrap()
            .to_string()
            .contains("Compose map limit")
    );
    let volumes = (0..128)
        .map(|n| format!("\"v{n}\":{{}}"))
        .collect::<Vec<_>>()
        .join(",");
    let error = serde_json::from_str::<Document>(&format!(
        "{{\"services\":{{}},\"volumes\":{{{volumes},\"extra\":false}}}}"
    ));
    assert!(
        error
            .err()
            .unwrap()
            .to_string()
            .contains("Compose map limit")
    );
    let mut value = minimal();
    value["services"]["api"]["ports"] = json!(
        (0..8)
            .map(|n| port(8000 + n))
            .chain([json!(false)])
            .collect::<Vec<_>>()
    );
    let error = serde_json::from_value::<Document>(value);
    assert!(
        error
            .err()
            .unwrap()
            .to_string()
            .contains("Compose array limit")
    );
    for category in ["volumes", "devices"] {
        let mut value = minimal();
        let mut entries: Vec<Value> = (0..8)
            .map(|n| {
                if category == "volumes" {
                    mount(n)
                } else {
                    json!(format!("/dev/v{n}:/dev/v{n}:rwm"))
                }
            })
            .collect();
        entries.push(json!(false));
        value["services"]["api"][category] = json!(entries);
        assert!(
            serde_json::from_value::<Document>(value)
                .err()
                .unwrap()
                .to_string()
                .contains("Compose array limit")
        );
    }
    for (key, length) in [("image", 265), ("user", 22)] {
        let mut value = minimal();
        value["services"]["api"][key] = json!("x".repeat(length));
        assert!(
            serde_json::from_value::<Document>(value)
                .err()
                .unwrap()
                .to_string()
                .contains("Compose string limit")
        );
    }
    let mut value = minimal();
    value["services"]["x".repeat(49)] = minimal_service();
    refused(value);
    let mut value = minimal();
    let mut p = port(8080);
    p["host_ip"] = json!("x".repeat(46));
    value["services"]["api"]["ports"] = json!([p]);
    assert!(
        serde_json::from_value::<Document>(value)
            .err()
            .unwrap()
            .to_string()
            .contains("Compose string limit")
    );
    for field in ["source", "target"] {
        let mut value = minimal();
        let mut m = mount(0);
        m[field] = json!("x".repeat(257));
        value["services"]["api"]["volumes"] = json!([m]);
        assert!(
            serde_json::from_value::<Document>(value)
                .err()
                .unwrap()
                .to_string()
                .contains("Compose string limit")
        );
    }
    let mut value = minimal();
    value["services"]["api"]["devices"] = json!(["x".repeat(518)]);
    assert!(
        serde_json::from_value::<Document>(value)
            .err()
            .unwrap()
            .to_string()
            .contains("Compose string limit")
    );
}

#[test]
fn raw_byte_limits_trailing_documents_invalid_utf8_and_yaml_are_explicit_refusals() {
    let mut exact = br#"{"services":{}}"#.to_vec();
    exact.resize(SOURCE_BYTES, b' ');
    assert!(admit_json_sources(&exact, None).is_ok());
    exact.push(b' ');
    assert!(admit_json_sources(&exact, None).is_err());
    assert!(admit_json_sources(OPERATOR, Some(&exact)).is_err());
    for raw in [
        &b""[..],
        &b"{\"services\":{}} {}"[..],
        &b"{\"services\":{}}\xff"[..],
        &b"services:\n  api: &base\n    image: example.invalid/api\n"[..],
    ] {
        assert!(admit_json_sources(raw, None).is_err());
    }
}

#[test]
fn combined_service_port_mount_device_and_volume_limits_refuse_without_truncation() {
    let mut base = minimal();
    for n in 1..16 {
        base["services"][format!("s{n}")] = minimal_service();
    }
    let base = serde_json::to_vec(&base).unwrap();
    assert!(admit_json_sources(&base, None).is_ok());
    assert!(admit_json_sources(&base, Some(NEW)).is_err());
    for category in ["ports", "volumes", "devices"] {
        let mut base = minimal();
        let mut managed = minimal();
        match category {
            "ports" => {
                base["services"]["api"][category] =
                    json!((0..8).map(|n| port(8000 + n)).collect::<Vec<_>>());
                managed["services"]["api"][category] = json!([port(9000)]);
            }
            "volumes" => {
                base["services"]["api"][category] = json!((0..8).map(mount).collect::<Vec<_>>());
                managed["services"]["api"][category] = json!([mount(9)]);
            }
            "devices" => {
                base["services"]["api"][category] = json!(
                    (0..8)
                        .map(|n| format!("/dev/v{n}:/dev/v{n}:rwm"))
                        .collect::<Vec<_>>()
                );
                managed["services"]["api"][category] = json!(["/dev/v9:/dev/v9:rwm"]);
            }
            _ => unreachable!(),
        }
        assert!(
            admit_json_sources(
                &serde_json::to_vec(&base).unwrap(),
                Some(&serde_json::to_vec(&managed).unwrap())
            )
            .is_err()
        );
    }
    let mut base = minimal();
    base["volumes"] = json!({});
    for n in 0..128 {
        base["volumes"][format!("v{n}")] = json!({});
    }
    let mut managed = minimal();
    managed["volumes"] = json!({"extra":{}});
    assert!(
        admit_json_sources(
            &serde_json::to_vec(&base).unwrap(),
            Some(&serde_json::to_vec(&managed).unwrap())
        )
        .is_err()
    );
}

#[test]
fn overlapping_devices_mount_kind_switches_and_combined_port_conflicts_refuse() {
    let mut managed = minimal();
    managed["services"]["api"]["devices"] = json!(["/dev/video0:/dev/video0:rwm"]);
    assert!(matches!(
        admit_json_sources(OPERATOR, Some(&serde_json::to_vec(&managed).unwrap())),
        Err(Error(ErrorCode::Conflict))
    ));
    managed["services"]["api"]
        .as_object_mut()
        .unwrap()
        .remove("devices");
    let mut m = mount(0);
    m["target"] = json!("/cache");
    managed["services"]["api"]["volumes"] = json!([m]);
    assert!(matches!(
        admit_json_sources(OPERATOR, Some(&serde_json::to_vec(&managed).unwrap())),
        Err(Error(ErrorCode::Conflict))
    ));
    let mut managed = minimal();
    managed["services"]["api"]["ports"] = json!([port(8080)]);
    assert!(admit_json_sources(OPERATOR, Some(&serde_json::to_vec(&managed).unwrap())).is_err());
}

#[test]
fn invalid_overridden_operator_fields_and_unrepresentable_desired_changes_refuse() {
    let mut invalid = minimal();
    invalid["services"]["api"]["image"] = json!("mutable:latest");
    assert!(admit_json_sources(&serde_json::to_vec(&invalid).unwrap(), Some(DELTA)).is_err());
    let source = admit_json_sources(OPERATOR, Some(DELTA)).unwrap();
    assert!(source.plan(&ComposeProject::default()).is_err());
    let mut desired = source.summary().current.clone();
    desired.services[0].ports.clear();
    assert!(source.plan(&desired).is_err());
}

#[test]
fn source_summary_count_accepts_exact_limit_and_refuses_without_increment() {
    let mut count = Count(SUMMARY_BYTES - 1);
    count.write_all(&[0]).unwrap();
    assert!(count.write_all(&[0]).is_err());
    assert_eq!(count.0, SUMMARY_BYTES);
}

#[test]
fn shared_native_conformance_cases_match_checked_projects_and_compiler_artifacts() {
    let fixtures: Value = serde_json::from_slice(include_bytes!(
        "../../../../tests/fixtures/compose-source/conformance-cases.json"
    ))
    .unwrap();
    for case in fixtures["cases"].as_array().unwrap() {
        let operator = serde_json::to_vec(&case["operator"]).unwrap();
        let managed = (!case["current_managed"].is_null())
            .then(|| serde_json::to_vec(&case["current_managed"]).unwrap());
        let source = admit_json_sources(&operator, managed.as_deref()).unwrap();
        let desired: ComposeProject = serde_json::from_value(case["desired"].clone()).unwrap();
        let plan = source.plan(&desired).unwrap();
        let artifact: Value = serde_json::from_slice(plan.artifact()).unwrap();
        assert_eq!(artifact, case["artifact"], "{}", case["id"]);
    }
}

#[test]
fn full_unique_resource_sets_allow_key_replacement_without_growing() {
    let mut base = minimal();
    let mut managed = minimal();
    base["services"]["api"]["ports"] = json!((0..8).map(|n| port(8000 + n)).collect::<Vec<_>>());
    managed["services"]["api"]["ports"] = json!([port(8000)]);
    let owner = admit_json_sources(
        &serde_json::to_vec(&base).unwrap(),
        Some(&serde_json::to_vec(&managed).unwrap()),
    )
    .unwrap();
    assert_eq!(owner.summary().current.services[0].ports.len(), 8);
    assert!(owner.plan(owner.summary().current).is_ok());
    let mut base = minimal();
    let mut managed = minimal();
    base["services"]["api"]["volumes"] = json!((0..8).map(mount).collect::<Vec<_>>());
    let mut replacement = mount(0);
    replacement["source"] = json!("/srv/replaced");
    managed["services"]["api"]["volumes"] = json!([replacement]);
    let owner = admit_json_sources(
        &serde_json::to_vec(&base).unwrap(),
        Some(&serde_json::to_vec(&managed).unwrap()),
    )
    .unwrap();
    assert_eq!(owner.summary().current.services[0].mounts.len(), 8);
    assert!(owner.plan(owner.summary().current).is_ok());
    let mut base = minimal();
    base["volumes"] = json!({});
    for n in 0..128 {
        base["volumes"][format!("v{n}")] = json!({});
    }
    let mut managed = minimal();
    managed["volumes"] = json!({"v0":{}});
    let owner = admit_json_sources(
        &serde_json::to_vec(&base).unwrap(),
        Some(&serde_json::to_vec(&managed).unwrap()),
    )
    .unwrap();
    assert_eq!(owner.summary().declared_volumes.len(), 128);
}
