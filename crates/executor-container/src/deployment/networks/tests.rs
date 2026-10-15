use super::*;
use limeos_domain::deployment::{
    discovery::ProjectIdentity,
    networks::{self as model, Completeness, GroupState, LinkState, NetworkInput},
};
use serde_json::{Value, json};

const PROVIDER: &[u8] =
    include_bytes!("../../../../../tests/fixtures/deployment/networks/provider.json");
const MEMBER: &[u8] =
    include_bytes!("../../../../../tests/fixtures/deployment/networks/member.json");
const ORPHAN: &[u8] =
    include_bytes!("../../../../../tests/fixtures/deployment/networks/orphan.json");
fn id(i: usize) -> String {
    format!("{i:064x}")
}
fn compose(service: &str) -> ComposeMember {
    ComposeMember {
        project: ProjectIdentity {
            name: "media".into(),
            working_directory: "/opt/stacks/Media".into(),
            config_files: vec!["/opt/stacks/Media/compose.yaml".into()],
        },
        service: service.into(),
        one_off: false,
    }
}
fn value() -> Value {
    serde_json::from_slice(PROVIDER).unwrap()
}
fn parse(value: &Value) -> Result<Node> {
    parse_inspect(&id(1), &serde_json::to_vec(value).unwrap(), None)
}
fn report(nodes: &[Node]) -> model::NetworkInventory {
    model::observe(
        101,
        "engine-a",
        Some(NetworkInput {
            engine_id: "engine-a",
            sampled_at: 100,
            completeness: Completeness::Complete,
            nodes,
        }),
    )
    .unwrap()
}

#[test]
fn frozen_synthetic_inspect_fixtures_keep_resolved_and_hinted_members_separate() {
    let nodes = [
        parse_inspect(&id(1), PROVIDER, Some(compose("vpn"))).unwrap(),
        parse_inspect(&id(2), MEMBER, Some(compose("sonarr"))).unwrap(),
        parse_inspect(&id(3), ORPHAN, Some(compose("transmission"))).unwrap(),
    ];
    let report = report(&nodes);
    assert_eq!(report.groups[0].state, GroupState::OrphanHints);
    assert_eq!(report.groups[0].member_ids, [id(2)]);
    assert_eq!(report.groups[0].hinted_orphan_ids, [id(3)]);
    assert_eq!(report.members[1].state, LinkState::MissingProvider);
    assert_eq!(report.members[1].provider_id, None);
    assert_eq!(report.sampled_at, Some(100));
}
#[test]
fn environment_health_log_and_unknown_label_values_do_not_escape() {
    let node = parse_inspect(&id(1), PROVIDER, None).unwrap();
    assert_eq!(node.health, Health::Healthy);
    assert!(!format!("{node:?}").contains("NETWORK_PRIVATE_VALUE"));
    let report = report(&[node]);
    for text in [
        serde_json::to_string(&report).unwrap(),
        format!("{report:?}"),
    ] {
        assert!(!text.contains("NETWORK_PRIVATE_VALUE"));
        assert!(!text.contains("operator.private"));
    }
}
#[test]
fn identity_mismatch_and_invalid_expected_ids_fail_closed() {
    assert_eq!(
        parse_inspect(&id(2), PROVIDER, None).unwrap_err().0,
        ErrorCode::Conflict
    );
    for expected in ["1".into(), "A".repeat(64), format!("container:{}", id(1))] {
        assert!(parse_inspect(&expected, PROVIDER, None).is_err());
    }
}
#[test]
fn duplicate_consumed_fields_are_refused_at_each_level() {
    let text = std::str::from_utf8(PROVIDER).unwrap();
    for modified in [
        text.replacen("\"Id\":", "\"Id\":\"ignored\",\"Id\":", 1),
        text.replacen("\"Name\":", "\"Name\":\"/other\",\"Name\":", 1),
        text.replacen("\"HostConfig\":", "\"HostConfig\":{},\"HostConfig\":", 1),
        text.replacen(
            "\"NetworkMode\":",
            "\"NetworkMode\":\"host\",\"NetworkMode\":",
            1,
        ),
        text.replacen("\"State\":", "\"State\":{},\"State\":", 1),
        text.replacen("\"Running\":", "\"Running\":false,\"Running\":", 1),
        text.replacen("\"Paused\":", "\"Paused\":false,\"Paused\":", 1),
        text.replacen("\"Health\":", "\"Health\":null,\"Health\":", 1),
        text.replacen("\"Status\":", "\"Status\":\"starting\",\"Status\":", 1),
        text.replacen("\"Config\":", "\"Config\":{},\"Config\":", 1),
        text.replacen("\"Labels\":", "\"Labels\":null,\"Labels\":", 1),
    ] {
        assert!(parse_inspect(&id(1), modified.as_bytes(), None).is_err());
    }
    let text = std::str::from_utf8(MEMBER).unwrap().replacen(
        "\"com.docker.compose.depends_on\":",
        "\"com.docker.compose.depends_on\":null,\"com.docker.compose.depends_on\":",
        1,
    );
    assert!(parse_inspect(&id(2), text.as_bytes(), None).is_err());
}
#[test]
fn missing_or_wrongly_typed_required_metadata_is_unavailable() {
    for field in ["Id", "Name", "HostConfig", "State", "Config"] {
        let mut input = value();
        input.as_object_mut().unwrap().remove(field);
        assert!(parse(&input).is_err());
        let mut input = value();
        input[field] = Value::Null;
        assert!(parse(&input).is_err());
    }
    for field in ["Running", "Paused", "Restarting", "Dead"] {
        let mut input = value();
        input["State"].as_object_mut().unwrap().remove(field);
        assert!(parse(&input).is_err());
        let mut input = value();
        input["State"][field] = json!("false");
        assert!(parse(&input).is_err());
    }
    for field in ["NetworkMode", "Status"] {
        let mut input = value();
        let parent = if field == "NetworkMode" {
            &mut input["HostConfig"]
        } else {
            &mut input["State"]["Health"]
        };
        parent[field] = Value::Null;
        assert!(parse(&input).is_err());
    }
}
#[test]
fn paused_restarting_dead_and_stopped_providers_are_not_available() {
    for field in ["Paused", "Restarting", "Dead"] {
        let mut input = value();
        input["State"][field] = json!(true);
        let node = parse(&input).unwrap();
        assert!(!node.running);
        let member = parse_inspect(&id(2), MEMBER, None).unwrap();
        assert_eq!(
            report(&[node, member]).groups[0].state,
            GroupState::ProviderStopped
        );
    }
    let mut input = value();
    input["State"]["Running"] = json!(false);
    assert!(!parse(&input).unwrap().running);
}
#[test]
fn health_absent_and_unknown_are_distinct() {
    let mut input = value();
    input["State"].as_object_mut().unwrap().remove("Health");
    assert_eq!(parse(&input).unwrap().health, Health::NotConfigured);
    input["State"]["Health"] = json!({"Status":"unexpected-private-health-value"});
    let node = parse(&input).unwrap();
    assert_eq!(node.health, Health::Unknown);
    assert!(!format!("{node:?}").contains("unexpected-private-health-value"));
}
#[test]
fn configured_health_without_result_stays_unknown() {
    let mut input = value();
    input["State"].as_object_mut().unwrap().remove("Health");
    input["Config"]["Healthcheck"] = json!({"Test": ["CMD-SHELL", "private-health-command"]});
    let node = parse(&input).unwrap();
    assert_eq!(node.health, Health::Unknown);
    assert!(!format!("{node:?}").contains("private-health-command"));
    let member = parse_inspect(&id(2), MEMBER, None).unwrap();
    assert_eq!(
        report(&[node, member]).groups[0].state,
        GroupState::HealthUnknown
    );
}
#[test]
fn healthcheck_object_is_selective_but_malformed_shape_is_refused() {
    for test in [json!(["NONE"]), json!(["CMD", "NETWORK_PRIVATE_VALUE"])] {
        let mut input = value();
        input["State"].as_object_mut().unwrap().remove("Health");
        input["Config"]["Healthcheck"] = json!({"Test":test});
        assert_eq!(parse(&input).unwrap().health, Health::Unknown);
    }
    for bad in [json!("private"), json!([]), json!(true)] {
        let mut input = value();
        input["Config"]["Healthcheck"] = bad;
        assert!(parse(&input).is_err());
    }
}
#[test]
fn arrays_cannot_substitute_for_engine_objects_even_when_struct_fields_match() {
    let input = value();
    let root_array = json!([
        input["Id"],
        input["Name"],
        input["HostConfig"],
        input["State"],
        input["Config"]
    ]);
    assert!(parse(&root_array).is_err());
    for path in ["HostConfig", "State", "Config"] {
        let mut input = value();
        input[path] = json!([]);
        assert!(parse(&input).is_err());
    }
    for path in ["Healthcheck", "Labels"] {
        let mut input = value();
        input["Config"][path] = json!([]);
        assert!(parse(&input).is_err());
    }
    let mut input = value();
    input["State"]["Health"] = json!(["healthy"]);
    assert!(parse(&input).is_err());
    input = value();
    input["HostConfig"] = json!(["bridge"]);
    assert!(parse(&input).is_err());
}
#[test]
fn valid_single_frozen_dependency_labels_supply_hints_only() {
    for condition in [
        "service_started",
        "service_healthy",
        "service_completed_successfully",
    ] {
        for restart in ["true", "false"] {
            assert_eq!(
                single_dependency(&format!("vpn:{condition}:{restart}")),
                Some("vpn".into())
            );
        }
    }
    let node = parse_inspect(&id(3), ORPHAN, None).unwrap();
    assert_eq!(node.dependency_hint.as_deref(), Some("vpn"));
    assert_eq!(report(&[node]).members[0].hinted_provider_id, None);
}
#[test]
fn multiple_or_malformed_dependencies_never_choose_the_first() {
    for label in [
        "vpn:service_started:true,db:service_started:false",
        "vpn",
        "vpn:service_started",
        "vpn:service_started:yes",
        "vpn:other:true",
        "vpn:service_started:true:extra",
        " vpn:service_started:true",
        "bad\nservice:service_started:true",
        "vpn:service_started:true,",
    ] {
        let mut input = value();
        input["Config"]["Labels"]["com.docker.compose.depends_on"] = json!(label);
        assert_eq!(parse(&input).unwrap().dependency_hint, None);
    }
}
#[test]
fn dependency_label_type_and_size_limits_are_refused() {
    for bad in [
        json!(["vpn"]),
        json!({}),
        json!(1),
        json!("x".repeat(MAX_DEPENDENCY_LABEL_BYTES + 1)),
    ] {
        let mut input = value();
        input["Config"]["Labels"]["com.docker.compose.depends_on"] = bad;
        assert!(parse(&input).is_err());
    }
}
#[test]
fn malformed_names_and_network_modes_are_refused_without_error_echo() {
    for field in ["Name", "NetworkMode"] {
        for bad in [
            "",
            "bad/private-value",
            "bad\nprivate-value",
            &"x".repeat(257),
        ] {
            let mut input = value();
            if field == "Name" {
                input["Name"] = json!(bad);
            } else {
                input["HostConfig"]["NetworkMode"] = json!(bad);
            }
            let error = parse(&input).unwrap_err();
            assert_eq!(error.0, ErrorCode::Unavailable);
            assert!(!format!("{error:?}").contains("private-value"));
        }
    }
    let mut input = value();
    input["Name"] = json!("//vpn");
    assert!(parse(&input).is_err());
}
#[test]
fn service_reference_is_diagnostic_and_uses_supplied_b2_identity() {
    let mut member: Value = serde_json::from_slice(MEMBER).unwrap();
    member["HostConfig"]["NetworkMode"] = json!("service:vpn");
    let bytes = serde_json::to_vec(&member).unwrap();
    let provider = parse_inspect(&id(1), PROVIDER, Some(compose("vpn"))).unwrap();
    let member = parse_inspect(&id(2), &bytes, Some(compose("sonarr"))).unwrap();
    assert_eq!(
        report(&[provider, member]).members[0].state,
        LinkState::Resolved
    );
}
#[test]
fn raw_byte_limit_is_inclusive_and_precedes_parse() {
    let mut bytes = PROVIDER.to_vec();
    bytes.resize(MAX_INSPECT_BYTES, b' ');
    assert!(parse_inspect(&id(1), &bytes, None).is_ok());
    bytes.push(b' ');
    assert!(parse_inspect(&id(1), &bytes, None).is_err());
    assert!(parse_inspect(&id(1), &vec![b'['; MAX_INSPECT_BYTES + 1], None).is_err());
}
#[test]
fn malformed_json_and_trailing_data_are_refused() {
    for bytes in [b"{}garbage".as_slice(), b"[]", b"null", b"{", b"\xff"] {
        assert!(parse_inspect(&id(1), bytes, None).is_err());
    }
}
#[test]
fn deep_valid_ignored_payload_is_skipped_without_retention() {
    let mut text = std::str::from_utf8(PROVIDER)
        .unwrap()
        .trim_end()
        .trim_end_matches('}')
        .to_string();
    text.push_str(&format!(
        ",\"ignored\":{}0{}}}",
        "[".repeat(150),
        "]".repeat(150)
    ));
    // serde_json skips IgnoredAny iteratively. Valid ignored nesting does not
    // consume the typed decoder's recursion budget; the raw byte cap still holds.
    let node = parse_inspect(&id(1), text.as_bytes(), None).unwrap();
    assert_eq!(node.id, id(1));
    assert!(!format!("{node:?}").contains("ignored"));
    text.pop();
    text.pop();
    assert!(parse_inspect(&id(1), text.as_bytes(), None).is_err());
}
#[test]
fn cohort_completeness_and_origin_are_not_supplied_by_parse_success() {
    let node = parse_inspect(&id(1), PROVIDER, None).unwrap();
    let report = model::observe(
        101,
        "engine-a",
        Some(NetworkInput {
            engine_id: "engine-a",
            sampled_at: 100,
            completeness: Completeness::Partial,
            nodes: &[node],
        }),
    )
    .unwrap();
    assert_eq!(report.source, model::SourceState::Incomplete);
    assert_eq!(report.observed_containers, None);
    assert!(report.groups.is_empty());
}
