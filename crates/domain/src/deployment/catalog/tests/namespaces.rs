use super::*;
use crate::{ComposePort, PortProtocol};

fn sharing(name: &str, provider: &str) -> CatalogService {
    let mut child = service(name);
    child.network_mode = Some(NetworkMode::Service {
        service: provider.into(),
    });
    child
}
fn binding(service: &str, provider: &str) -> NamespaceBinding {
    NamespaceBinding {
        service: service.into(),
        provider: provider.into(),
    }
}
fn port() -> ComposePort {
    ComposePort {
        host_ip: "127.0.0.1".into(),
        target: 9091,
        published: 9091,
        protocol: PortProtocol::Tcp,
    }
}
#[derive(Deserialize)]
struct NamespaceFixture {
    provider: String,
    ports: Vec<ComposePort>,
    members: Vec<Member>,
    expected_namespace: String,
    expected_dependency: Value,
}
#[derive(Deserialize)]
struct Member {
    name: String,
    network_mode: NetworkMode,
    depends_on: Vec<Dependency>,
    host_path: String,
    container_path: String,
}

#[test]
fn frozen_media_namespace_and_healthy_dependency_fixture_survives_rendering() {
    let fixture: NamespaceFixture = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/deployment/catalog/namespaces.json"
    )))
    .unwrap();
    let mut state = stack("media");
    let mut provider = service(&fixture.provider);
    provider.base.ports = fixture.ports;
    state.managed.services.push(provider);
    for member in fixture.members {
        let mut template = app(&member.name);
        template.service.network_mode = Some(member.network_mode);
        template.service.requires = vec![fixture.provider.clone()];
        template.service.depends_on = member.depends_on;
        template.service.base.user = Some(ComposeUser {
            uid: 1001,
            gid: 2002,
        });
        template.service.base.mounts.push(ComposeMount::Bind {
            source: member.host_path.clone(),
            target: member.container_path.clone(),
            read_only: false,
        });
        let plan = install(&state, &template).unwrap();
        let doc = artifact(&plan);
        let row = &doc["services"][&member.name];
        assert_eq!(row["network_mode"], fixture.expected_namespace);
        assert_eq!(row["depends_on"], fixture.expected_dependency);
        assert!(row.get("ports").is_none());
        assert!(row.get("networks").is_none());
        assert_eq!(row["user"], "1001:2002");
        assert_eq!(row["volumes"][0]["source"], member.host_path);
        assert_eq!(row["volumes"][0]["target"], member.container_path);
        assert_eq!(
            doc["services"][&fixture.provider]["ports"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        state.managed = plan.summary().desired_managed.clone();
    }
    assert_eq!(state.managed.services.len(), 3);
}
#[test]
fn namespace_without_explicit_dependency_still_requires_its_provider() {
    let mut template = app("client");
    template.service = sharing("client", "vpn");
    assert_eq!(
        install(&stack("media"), &template).unwrap_err().services,
        ["vpn"]
    );
    let mut state = stack("media");
    state.managed.services.push(service("vpn"));
    let plan = install(&state, &template).unwrap();
    assert_eq!(
        artifact(&plan)["services"]["client"]["network_mode"],
        "service:vpn"
    );
    assert!(
        artifact(&plan)["services"]["client"]
            .get("depends_on")
            .is_none()
    );
    assert_eq!(plan.summary().namespace_desired, [binding("client", "vpn")]);
}
#[test]
fn namespace_provider_is_resolved_in_the_selected_stack_only() {
    let mut other = stack("other");
    other.managed.services.push(service("vpn"));
    let mut template = app("client");
    template.service = sharing("client", "vpn");
    let error = plan_install(
        &[stack("media"), other],
        &InstallTarget::Existing {
            stack: "media".into(),
        },
        &template,
        OrphanPolicy::Preserve,
    )
    .unwrap_err();
    assert_eq!(error.code, CatalogErrorCode::MissingDependencies);
}
#[test]
fn inherited_provider_remains_in_operator_source_while_managed_client_references_it() {
    let mut state = stack("media");
    let mut vpn = service("vpn");
    vpn.base.ports.push(port());
    state.operator.services.push(vpn);
    let mut template = app("client");
    template.service = sharing("client", "vpn");
    let plan = install(&state, &template).unwrap();
    assert!(artifact(&plan)["services"].get("vpn").is_none());
    assert_eq!(plan.summary().operator, &state.operator);
    assert_eq!(plan.summary().namespace_desired, [binding("client", "vpn")]);
}
#[test]
fn managed_and_operator_consumers_both_block_provider_removal_without_depends_on() {
    for inherited in [false, true] {
        let mut state = stack("media");
        state.managed.services.push(service("vpn"));
        if inherited {
            state.operator.services.push(sharing("client", "vpn"));
        } else {
            state.managed.services.push(sharing("client", "vpn"));
        }
        let error = remove(&[state], "media", "vpn").unwrap_err();
        assert_eq!(error.code, CatalogErrorCode::DependentServices);
        assert_eq!(error.services, ["client"]);
    }
}
#[test]
fn removing_a_namespace_consumer_preserves_provider_ports_and_peer_namespace() {
    let mut state = stack("media");
    let mut vpn = service("vpn");
    vpn.base.ports.push(port());
    state.managed.services = vec![vpn, sharing("a", "vpn"), sharing("b", "vpn")];
    let plan = remove(&[state], "media", "a").unwrap();
    assert_eq!(plan.summary().shutdown_required, ["a"]);
    assert_eq!(
        plan.summary().namespace_before,
        [binding("a", "vpn"), binding("b", "vpn")]
    );
    assert_eq!(plan.summary().namespace_desired, [binding("b", "vpn")]);
    let doc = artifact(&plan);
    assert!(doc["services"].get("a").is_none());
    assert_eq!(doc["services"]["vpn"]["ports"][0]["published"], "9091");
    assert_eq!(doc["services"]["b"]["network_mode"], "service:vpn");
}
#[test]
fn own_ports_networks_or_host_mode_are_never_combined_with_service_namespace() {
    for conflict in 0..3 {
        let mut state = stack("media");
        state.managed.services.push(service("vpn"));
        let mut template = app("client");
        template.service = sharing("client", "vpn");
        match conflict {
            0 => template.service.base.ports.push(port()),
            1 => template.service.networks.push("default".into()),
            _ => template.service.base.host_network = true,
        }
        assert_eq!(
            install(&state, &template).unwrap_err().code,
            CatalogErrorCode::InvalidInput
        );
    }
}
#[test]
fn provider_ports_are_never_synthesized_from_the_consumer() {
    let mut state = stack("media");
    state.managed.services.push(service("vpn"));
    let mut template = app("client");
    template.service = sharing("client", "vpn");
    let doc = artifact(&install(&state, &template).unwrap());
    assert!(doc["services"]["vpn"].get("ports").is_none());
    assert!(doc["services"]["client"].get("ports").is_none());
}
#[test]
fn self_reference_and_mixed_catalog_startup_namespace_cycles_are_refused() {
    for kind in 0..3 {
        let mut state = stack("media");
        let mut vpn = service("vpn");
        match kind {
            0 => {
                vpn.network_mode = Some(NetworkMode::Service {
                    service: "client".into(),
                })
            }
            1 => vpn.depends_on.push("client".into()),
            _ => vpn.requires.push("client".into()),
        }
        // Validate the whole cycle in one supplied declaration before removing an unrelated service.
        state.managed.services = vec![vpn, sharing("client", "vpn"), service("unused")];
        assert_eq!(
            remove(&[state], "media", "unused").unwrap_err().code,
            CatalogErrorCode::CyclicDependencies
        );
    }
    let mut template = app("self");
    template.service = sharing("self", "self");
    assert_eq!(
        install(&stack("media"), &template).unwrap_err().code,
        CatalogErrorCode::CyclicDependencies
    );
}
#[test]
fn maximum_acyclic_namespace_chain_is_bounded_and_preserves_immediate_edges() {
    let mut state = stack("media");
    state.managed.services.push(service("s0"));
    for i in 1..15 {
        state
            .managed
            .services
            .push(sharing(&format!("s{i}"), &format!("s{}", i - 1)));
    }
    let mut template = app("s15");
    template.service = sharing("s15", "s14");
    let plan = install(&state, &template).unwrap();
    assert_eq!(plan.summary().namespace_desired.len(), 15);
    assert_eq!(
        artifact(&plan)["services"]["s15"]["network_mode"],
        "service:s14"
    );
    state.managed = plan.summary().desired_managed.clone();
    assert_eq!(
        install(&state, &app("too-many")).unwrap_err().code,
        CatalogErrorCode::Limit
    );
}
#[test]
fn host_namespace_providers_and_chains_to_them_remain_unsupported() {
    for chain in [false, true] {
        let mut state = stack("media");
        let mut host = service("host");
        host.base.host_network = true;
        state.managed.services.push(host);
        if chain {
            state.managed.services.push(sharing("middle", "host"));
        }
        let mut template = app("client");
        template.service = sharing("client", if chain { "middle" } else { "host" });
        assert_eq!(
            install(&state, &template).unwrap_err().code,
            CatalogErrorCode::Unsupported
        );
    }
}
#[test]
fn all_startup_conditions_restart_and_required_flags_render_without_weakening() {
    for condition in [
        DependencyCondition::ServiceStarted,
        DependencyCondition::ServiceHealthy,
        DependencyCondition::ServiceCompletedSuccessfully,
    ] {
        for restart in [false, true] {
            for required in [false, true] {
                let mut state = stack("media");
                state.managed.services.push(service("provider"));
                let mut template = app("client");
                template.service.depends_on.push(Dependency {
                    service: "provider".into(),
                    condition,
                    restart,
                    required,
                });
                let row = artifact(&install(&state, &template).unwrap())["services"]["client"]["depends_on"]["provider"].clone();
                assert_eq!(
                    row,
                    json!({"condition": condition, "restart": restart, "required": required})
                );
            }
        }
    }
}
#[test]
fn duplicate_dependencies_invalid_names_and_missing_optional_providers_are_refused() {
    let mut state = stack("media");
    state.managed.services.push(service("provider"));
    let mut template = app("client");
    template.service.depends_on = vec!["provider".into(), "provider".into()];
    assert_eq!(
        install(&state, &template).unwrap_err().code,
        CatalogErrorCode::InvalidInput
    );
    template.service.depends_on = vec![Dependency {
        service: "absent".into(),
        condition: DependencyCondition::ServiceStarted,
        restart: false,
        required: false,
    }];
    assert_eq!(
        install(&state, &template).unwrap_err().code,
        CatalogErrorCode::MissingDependencies
    );
    for name in ["", "service:vpn", "${VPN}", "other/vpn", "bad\nname"] {
        template.service.depends_on.clear();
        template.service.network_mode = Some(NetworkMode::Service {
            service: name.into(),
        });
        assert_eq!(
            install(&state, &template).unwrap_err().code,
            CatalogErrorCode::InvalidInput
        );
    }
}
#[test]
fn namespace_and_condition_models_refuse_unrepresented_or_malformed_fields() {
    for value in [
        json!("service:vpn"),
        json!({"kind":"container", "service":"vpn"}),
        json!({"kind":"service", "service":"vpn", "fallback":"bridge"}),
    ] {
        assert!(serde_json::from_value::<NetworkMode>(value).is_err());
    }
    for value in [
        json!({"service":"vpn","condition":"healthy","restart":false,"required":true}),
        json!({"service":"vpn","condition":"service_healthy","restart":"false","required":true}),
        json!({"service":"vpn","condition":"service_healthy","restart":false,"required":true,"timeout":10}),
    ] {
        assert!(serde_json::from_value::<Dependency>(value).is_err());
    }
}
#[test]
fn expanded_review_has_version_two_complete_models_and_unresolved_namespace_gate() {
    let mut state = stack("media");
    state.managed.services.push(service("vpn"));
    let mut template = app("client");
    template.service = sharing("client", "vpn");
    template.service.depends_on.push(Dependency {
        service: "vpn".into(),
        condition: DependencyCondition::ServiceHealthy,
        restart: true,
        required: true,
    });
    let plan = install(&state, &template).unwrap();
    let summary = plan.summary();
    assert_eq!(summary.version, 2);
    assert!(summary.namespace_before.is_empty());
    assert_eq!(
        summary.desired_managed.services[0].network_mode,
        template.service.network_mode
    );
    assert_eq!(
        summary.desired_managed.services[0].depends_on,
        template.service.depends_on
    );
    assert_eq!(summary.compose_unresolved.len(), 8);
    assert_eq!(summary.catalog_unresolved.len(), 8);
    assert!(
        summary
            .catalog_unresolved
            .contains(&CatalogGate::VerifiedNamespaceProviderAndStartup)
    );
    // Base Compose impact alone has no namespace/condition representation.
    assert!(
        !serde_json::to_string(summary.base_service_impact)
            .unwrap()
            .contains("service_healthy")
    );
}
#[test]
fn namespace_edges_and_long_dependency_rendering_are_deterministic() {
    let mut state = stack("media");
    state.managed.services = vec![service("b"), service("a")];
    let mut template = app("client");
    template.service = sharing("client", "a");
    template.service.depends_on = vec!["b".into(), "a".into()];
    let first = install(&state, &template).unwrap();
    template.service.depends_on.reverse();
    state.managed.services.reverse();
    let second = install(&state, &template).unwrap();
    assert_eq!(first.artifact(), second.artifact());
    assert_eq!(
        first.summary().namespace_desired,
        second.summary().namespace_desired
    );
}

#[test]
fn completed_startup_condition_cannot_require_a_namespace_provider_to_have_exited() {
    for chain in [false, true] {
        let mut state = stack("media");
        state.managed.services.push(service("vpn"));
        if chain {
            state.managed.services.push(sharing("middle", "vpn"));
        }
        let mut template = app("client");
        template.service = sharing("client", if chain { "middle" } else { "vpn" });
        template.service.depends_on.push(Dependency {
            service: "vpn".into(),
            condition: DependencyCondition::ServiceCompletedSuccessfully,
            restart: false,
            required: true,
        });
        assert_eq!(
            install(&state, &template).unwrap_err().code,
            CatalogErrorCode::Unsupported
        );
    }
    let mut state = stack("media");
    state.managed.services = vec![service("vpn"), service("setup")];
    let mut template = app("client");
    template.service = sharing("client", "vpn");
    template.service.depends_on.push(Dependency {
        service: "setup".into(),
        condition: DependencyCondition::ServiceCompletedSuccessfully,
        restart: false,
        required: true,
    });
    assert!(install(&state, &template).is_ok());
}
