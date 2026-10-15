use super::*;
use serde_json::{Value, json};

fn id(index: usize) -> String {
    format!("{index:064x}")
}
fn project() -> ProjectIdentity {
    ProjectIdentity {
        name: "media".into(),
        working_directory: "/opt/stacks/Media".into(),
        config_files: vec![
            "/opt/stacks/Media/compose.yaml".into(),
            "/opt/stacks/Media/limeos.override.json".into(),
        ],
    }
}
fn node(index: usize, name: &str, mode: &str) -> Node {
    Node {
        id: id(index),
        name: name.into(),
        running: true,
        health: Health::NotConfigured,
        network_mode: Some(mode.into()),
        dependency_hint: None,
        compose: Some(ComposeMember {
            project: project(),
            service: name.into(),
            one_off: false,
        }),
    }
}
fn read(nodes: &[Node]) -> NetworkInventory {
    observe(
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
fn pair() -> Vec<Node> {
    vec![
        node(1, "vpn", "bridge"),
        node(2, "sonarr", &format!("container:{}", id(1))),
    ]
}

#[test]
fn frozen_provider_member_orphan_regression_keeps_hint_separate() {
    let mut nodes = pair();
    nodes[0].health = Health::Healthy;
    let mut orphan = node(3, "transmission", &format!("container:{}", id(99)));
    orphan.running = false;
    orphan.dependency_hint = Some("vpn".into());
    nodes.push(orphan);
    let report = read(&nodes);
    assert_eq!(report.groups.len(), 1);
    let group = &report.groups[0];
    assert_eq!(group.provider.id, id(1));
    assert_eq!(group.member_ids, [id(2)]);
    assert_eq!(group.hinted_orphan_ids, [id(3)]);
    assert_eq!(group.state, GroupState::OrphanHints);
    let orphan = &report.members[1];
    assert_eq!(orphan.state, LinkState::MissingProvider);
    assert_eq!(orphan.provider_id, None);
    assert_eq!(orphan.hinted_provider_id, Some(id(1)));
    assert!(!orphan.container.running);
}
#[test]
fn full_id_and_exact_case_sensitive_name_resolve() {
    for target in [id(1), "vpn".into()] {
        let mut nodes = pair();
        nodes[1].network_mode = Some(format!("container:{target}"));
        assert_eq!(read(&nodes).members[0].state, LinkState::Resolved);
    }
    let mut nodes = pair();
    nodes[1].network_mode = Some("container:VPN".into());
    assert_eq!(read(&nodes).members[0].state, LinkState::MissingProvider);
}
#[test]
fn short_container_id_does_not_expand_or_count_as_missing() {
    let mut nodes = pair();
    for length in [1, 6, 11, 12, 63] {
        nodes[1].network_mode = Some(format!("container:{}", &id(1)[..length]));
        let report = read(&nodes);
        assert_eq!(
            report.members[0].state,
            LinkState::UnsupportedContainerPrefix
        );
        assert!(report.groups.is_empty());
    }
}
#[test]
fn conflicting_full_id_and_name_match_is_ambiguous() {
    let mut nodes = pair();
    nodes.push(node(3, &id(1), "bridge"));
    let report = read(&nodes);
    assert_eq!(report.members[0].state, LinkState::AmbiguousProvider);
    assert!(report.groups.is_empty());
}
#[test]
fn compose_service_uses_project_identity_instead_of_container_name() {
    let mut nodes = pair();
    nodes[0].name = "media-vpn-1".into();
    nodes[1].network_mode = Some("service:vpn".into());
    assert_eq!(read(&nodes).members[0].provider_id, Some(id(1)));
    for change in 0..4 {
        let mut changed = nodes.clone();
        let p = &mut changed[0].compose.as_mut().unwrap().project;
        match change {
            0 => p.name = "other".into(),
            1 => p.working_directory = "/opt/stacks/media".into(),
            2 => p.config_files.reverse(),
            _ => p.config_files[0] = "/opt/stacks/Media/Compose.yaml".into(),
        }
        assert_eq!(read(&changed).members[0].state, LinkState::MissingProvider);
    }
}
#[test]
fn standalone_service_reference_cannot_use_a_global_name() {
    let mut nodes = pair();
    nodes[1].network_mode = Some("service:vpn".into());
    nodes[1].compose = None;
    assert_eq!(read(&nodes).members[0].state, LinkState::MissingProject);
}
#[test]
fn replicas_are_ambiguous_even_when_only_one_is_running() {
    let mut nodes = pair();
    nodes[1].network_mode = Some("service:vpn".into());
    let mut replica = node(3, "vpn-2", "bridge");
    replica.compose.as_mut().unwrap().service = "vpn".into();
    replica.running = false;
    nodes.push(replica);
    assert_eq!(read(&nodes).members[0].state, LinkState::AmbiguousProvider);
}
#[test]
fn one_offs_never_satisfy_service_provider_but_remain_visible_members() {
    let mut nodes = pair();
    nodes[1].network_mode = Some("service:vpn".into());
    nodes[0].compose.as_mut().unwrap().one_off = true;
    assert_eq!(read(&nodes).members[0].state, LinkState::MissingProvider);
    nodes[0].compose.as_mut().unwrap().one_off = false;
    nodes[1].compose.as_mut().unwrap().one_off = true;
    nodes[1].running = false;
    let report = read(&nodes);
    assert_eq!(report.members[0].state, LinkState::Resolved);
    assert!(!report.members[0].container.running);
    assert!(
        report.members[0]
            .container
            .compose
            .as_ref()
            .unwrap()
            .one_off
    );
}
#[test]
fn hint_does_not_repair_dead_id_or_cross_project_boundaries() {
    let mut nodes = pair();
    nodes[1].network_mode = Some(format!("container:{}", id(99)));
    nodes[1].dependency_hint = Some("vpn".into());
    nodes[0].compose.as_mut().unwrap().project.name = "other".into();
    let report = read(&nodes);
    assert_eq!(report.members[0].provider_id, None);
    assert_eq!(report.members[0].hinted_provider_id, None);
    assert_eq!(report.members[0].dependency_hint.as_deref(), Some("vpn"));
    assert!(report.groups.is_empty());
}
#[test]
fn hints_do_not_guess_between_replicas() {
    let mut nodes = pair();
    nodes[1].network_mode = Some(format!("container:{}", id(99)));
    nodes[1].dependency_hint = Some("vpn".into());
    let mut other = node(3, "vpn-2", "bridge");
    other.compose.as_mut().unwrap().service = "vpn".into();
    nodes.push(other);
    assert_eq!(read(&nodes).members[0].hinted_provider_id, None);
}
#[test]
fn self_cycles_and_chains_are_never_observed_running() {
    let mut nodes = pair();
    nodes[0].network_mode = Some("container:vpn".into());
    let report = read(&nodes);
    assert_eq!(report.members[0].state, LinkState::SelfReference);
    assert_eq!(report.members[1].state, LinkState::CyclicProvider);
    assert!(
        report
            .groups
            .iter()
            .all(|g| g.state == GroupState::UnsupportedTopology)
    );
    nodes[0].network_mode = Some("container:sonarr".into());
    assert!(
        read(&nodes)
            .members
            .iter()
            .all(|m| m.state == LinkState::CyclicProvider)
    );
    nodes.push(node(3, "root", "bridge"));
    nodes[0].network_mode = Some("container:root".into());
    let report = read(&nodes);
    assert_eq!(report.members[1].state, LinkState::ChainedProvider);
    assert_eq!(report.groups[0].state, GroupState::UnsupportedTopology);
}
#[test]
fn missing_tail_in_a_chain_cannot_be_healthy() {
    let mut nodes = pair();
    nodes[0].network_mode = Some("container:missing".into());
    let report = read(&nodes);
    assert_eq!(report.members[0].state, LinkState::MissingProvider);
    assert_eq!(report.members[1].state, LinkState::ChainedProvider);
    assert_eq!(report.groups[0].state, GroupState::UnsupportedTopology);
}
#[test]
fn frozen_health_fixture_maps_provider_conditions_without_vpn_claims() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/deployment/networks/health.json"
    ))
    .unwrap();
    for case in fixture.as_array().unwrap() {
        let mut nodes = pair();
        nodes[0].health = match case["health"].as_str().unwrap() {
            "healthy" => Health::Healthy,
            "unhealthy" => Health::Unhealthy,
            "starting" => Health::Starting,
            "unknown" => Health::Unknown,
            _ => Health::NotConfigured,
        };
        nodes[0].running = case["running"].as_bool().unwrap();
        let report = read(&nodes);
        assert_eq!(
            serde_json::to_value(report.groups[0].state).unwrap(),
            case["group_state"]
        );
        let text = serde_json::to_string(&report).unwrap();
        assert!(!text.contains("vpn_leak"));
        assert!(!text.contains("public_ip"));
    }
}
#[test]
fn missing_mode_is_unknown_instead_of_independent_or_healthy() {
    let mut nodes = pair();
    nodes[0].network_mode = None;
    let report = read(&nodes);
    assert_eq!(report.groups[0].state, GroupState::ProviderNetworkUnknown);
    assert_eq!(report.unknown_mode_containers, Some(1));
    assert_eq!(report.independent_containers, Some(0));
}
#[test]
fn independent_network_modes_are_counted_without_inventing_shared_groups() {
    let nodes: Vec<_> = ["host", "bridge", "none", "default", "Media_network"]
        .iter()
        .enumerate()
        .map(|(i, mode)| node(i, &format!("n{i}"), mode))
        .collect();
    let report = read(&nodes);
    assert_eq!(report.independent_containers, Some(5));
    assert!(report.groups.is_empty());
    assert!(report.members.is_empty());
}
#[test]
fn unavailable_partial_stale_future_and_wrong_engine_never_report_zero() {
    let missing = observe(100, "engine-a", None).unwrap();
    assert_eq!(missing.source, SourceState::Unavailable);
    assert_eq!(missing.observed_containers, None);
    for (engine, time, completeness, expected) in [
        ("engine-a", 9, Completeness::Complete, SourceState::Stale),
        ("engine-a", 101, Completeness::Complete, SourceState::Future),
        (
            "other",
            100,
            Completeness::Complete,
            SourceState::EngineMismatch,
        ),
        (
            "engine-a",
            100,
            Completeness::Partial,
            SourceState::Incomplete,
        ),
    ] {
        let nodes = pair();
        let report = observe(
            100,
            "engine-a",
            Some(NetworkInput {
                engine_id: engine,
                sampled_at: time,
                completeness,
                nodes: &nodes,
            }),
        )
        .unwrap();
        assert_eq!(report.source, expected);
        assert_eq!(report.sampled_at, Some(time));
        assert_eq!(report.observed_containers, None);
        assert_eq!(report.independent_containers, None);
        assert!(report.members.is_empty());
        assert!(report.groups.is_empty());
    }
}
#[test]
fn fresh_empty_and_inclusive_age_boundary_preserve_original_time() {
    let report = observe(
        100,
        "engine-a",
        Some(NetworkInput {
            engine_id: "engine-a",
            sampled_at: 10,
            completeness: Completeness::Complete,
            nodes: &[],
        }),
    )
    .unwrap();
    assert_eq!(report.source, SourceState::Fresh);
    assert_eq!(report.sampled_at, Some(10));
    assert_eq!(report.observed_containers, Some(0));
}
#[test]
fn output_is_deterministic_and_keeps_all_stopped_members() {
    let mut nodes = pair();
    nodes.push(node(3, "radarr", "container:vpn"));
    nodes[2].running = false;
    let first = serde_json::to_value(read(&nodes)).unwrap();
    nodes.reverse();
    assert_eq!(first, serde_json::to_value(read(&nodes)).unwrap());
    assert_eq!(first["groups"][0]["member_ids"], json!([id(2), id(3)]));
}
#[test]
fn duplicate_ids_names_and_invalid_text_are_refused() {
    for change in 0..7 {
        let mut nodes = pair();
        match change {
            0 => nodes[1].id = nodes[0].id.clone(),
            1 => nodes[1].name = nodes[0].name.clone(),
            2 => nodes[1].id = "a".repeat(63),
            3 => nodes[1].name = "bad\nname".into(),
            4 => nodes[1].network_mode = Some("container:".into()),
            5 => nodes[1].network_mode = Some("service:vpn:extra".into()),
            _ => nodes[1].dependency_hint = Some("bad,hint".into()),
        }
        assert!(
            observe(
                100,
                "engine-a",
                Some(NetworkInput {
                    engine_id: "engine-a",
                    sampled_at: 100,
                    completeness: Completeness::Complete,
                    nodes: &nodes
                })
            )
            .is_err()
        );
    }
}
#[test]
fn project_paths_and_ordered_files_are_admitted_before_projection() {
    for change in 0..5 {
        let mut n = node(1, "vpn", "bridge");
        let p = &mut n.compose.as_mut().unwrap().project;
        match change {
            0 => p.config_files.clear(),
            1 => p.config_files.push(p.config_files[0].clone()),
            2 => p.working_directory = "/opt/../stacks".into(),
            3 => p.name = "Media".into(),
            _ => p.config_files[0] = "/opt//Media/compose.yaml".into(),
        }
        assert!(n.validate().is_err());
    }
}
#[test]
fn input_member_and_aggregate_text_budgets_are_enforced() {
    let mut nodes: Vec<_> = (0..MAX_MEMBERS)
        .map(|i| node(i, &format!("node{i}"), "bridge"))
        .collect();
    assert_eq!(read(&nodes).observed_containers, Some(16));
    nodes.push(node(99, "extra", "bridge"));
    assert!(
        observe(
            100,
            "engine-a",
            Some(NetworkInput {
                engine_id: "engine-a",
                sampled_at: 100,
                completeness: Completeness::Complete,
                nodes: &nodes
            })
        )
        .is_err()
    );
    nodes.pop();
    for node in &mut nodes {
        node.compose.as_mut().unwrap().project.config_files = (0..4)
            .map(|i| format!("/{}/{}/file{i}", "a".repeat(240), "b".repeat(240)))
            .collect();
    }
    assert!(
        observe(
            100,
            "engine-a",
            Some(NetworkInput {
                engine_id: "engine-a",
                sampled_at: 100,
                completeness: Completeness::Complete,
                nodes: &nodes
            })
        )
        .is_err()
    );
}
#[test]
fn invalid_times_and_engine_identity_are_refused() {
    assert!(observe(-1, "engine-a", None).is_err());
    assert!(observe(0, "", None).is_err());
    assert!(
        observe(
            0,
            "engine-a",
            Some(NetworkInput {
                engine_id: "engine-a",
                sampled_at: MAX_TIME + 1,
                completeness: Completeness::Complete,
                nodes: &[]
            })
        )
        .is_err()
    );
}
#[test]
fn report_serialization_is_stream_bounded() {
    let mut report = read(&pair());
    report.groups[0].provider.name = "x".repeat(MAX_REPORT_BYTES);
    assert!(report.check_size().is_err());
}
