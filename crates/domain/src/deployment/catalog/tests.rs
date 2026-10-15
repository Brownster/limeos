use super::*;
use crate::{ComposeDevice, ComposeRestart, ComposeUser};

fn service(name: &str) -> CatalogService {
    CatalogService {
        base: ComposeService {
            name: name.into(),
            image: format!("example/{name}@sha256:{}", "a".repeat(64)),
            user: None,
            restart: ComposeRestart::UnlessStopped,
            ports: vec![],
            mounts: vec![],
            devices: vec![],
            privileged: false,
            host_network: false,
        },
        requires: vec![],
        depends_on: vec![],
        networks: vec![],
        configs: vec![],
        secrets: vec![],
    }
}
fn stack(name: &str) -> StackDeclaration {
    StackDeclaration {
        stack: name.into(),
        project: name.into(),
        operator: Project::default(),
        managed: Project::default(),
        orphan_policy: OrphanPolicy::Preserve,
    }
}
fn app(name: &str) -> AppTemplate {
    AppTemplate {
        app: name.into(),
        service: service(name),
        resources: Resources::default(),
    }
}
fn install(stack: &StackDeclaration, template: &AppTemplate) -> Result<CatalogPlan> {
    plan_install(
        std::slice::from_ref(stack),
        &InstallTarget::Existing {
            stack: stack.stack.clone(),
        },
        template,
        OrphanPolicy::RemoveOrphans,
    )
}
fn remove(stacks: &[StackDeclaration], name: &str, app: &str) -> Result<CatalogPlan> {
    plan_remove(
        stacks,
        &InstallationKey {
            stack: name.into(),
            app: app.into(),
        },
    )
}
fn artifact(plan: &CatalogPlan) -> Value {
    serde_json::from_slice(plan.artifact()).unwrap()
}
#[derive(Deserialize)]
struct Fixture {
    operator: Resources,
    template: Resources,
    expected_artifact_resources: Value,
}
fn fixture() -> Fixture {
    serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/deployment/catalog/resources.json"
    ))
    .unwrap()
}
fn resource_app() -> AppTemplate {
    let mut app = app("resource-app");
    app.resources = fixture().template;
    app.service.networks.push("app-net".into());
    app.service.base.mounts.push(ComposeMount::Volume {
        name: "app-data".into(),
        target: "/data".into(),
        read_only: false,
    });
    app.service.configs.push(DataGrant {
        source: "app-config".into(),
        target: Some("/app/config".into()),
    });
    app.service.secrets.push(DataGrant {
        source: "app-secret".into(),
        target: None,
    });
    app
}

#[test]
fn frozen_four_section_regression_retains_config_secret_grants_and_definitions() {
    let mut state = stack("media");
    state.operator.resources = fixture().operator;
    state.operator.services.push(service("other"));
    let plan = install(&state, &resource_app()).unwrap();
    let value = artifact(&plan);
    for section in ["networks", "volumes", "configs", "secrets"] {
        assert_eq!(
            value[section],
            fixture().expected_artifact_resources[section]
        );
    }
    assert_eq!(
        value["services"]["resource-app"]["configs"],
        json!([{"source":"app-config","target":"/app/config"}])
    );
    assert_eq!(
        value["services"]["resource-app"]["secrets"],
        json!([{"source":"app-secret"}])
    );
    assert!(value["services"].get("other").is_none());
    assert_eq!(plan.summary().operator, &state.operator);
    assert_eq!(plan.summary().added_resources.len(), 4);
    // Independent map union: operator entries survive in the effective model,
    // while the only writable artifact is the managed fragment.
    let mut effective = json!({"networks":{"user-net":{"external":true}},"volumes":{"user-data":{}},
        "configs":{"user-config":{"file":"./user.conf"}},"secrets":{"user-secret":{"external":true}}});
    for section in ["networks", "volumes", "configs", "secrets"] {
        effective[section]
            .as_object_mut()
            .unwrap()
            .extend(value[section].as_object().unwrap().clone());
        assert_eq!(effective[section].as_object().unwrap().len(), 2);
    }
    assert_eq!(plan.summary().managed_file, "limeos.override.json");
}

#[test]
fn equal_definitions_reuse_operator_resources_without_adopting_them() {
    let mut state = stack("media");
    state.operator.resources = fixture().template;
    let plan = install(&state, &resource_app()).unwrap();
    let value = artifact(&plan);
    for section in ["networks", "volumes", "configs", "secrets"] {
        assert!(value.get(section).is_none());
    }
    assert_eq!(plan.summary().reused_resources.len(), 4);
    assert!(plan.summary().added_resources.is_empty());
    assert_eq!(
        plan.summary().desired_managed.resources,
        Resources::default()
    );
}

#[test]
fn conflicting_resources_in_each_section_refuse_without_replacement() {
    for section in [
        ResourceSection::Networks,
        ResourceSection::Volumes,
        ResourceSection::Configs,
        ResourceSection::Secrets,
    ] {
        let mut state = stack("media");
        state.operator.resources = fixture().template;
        match section {
            ResourceSection::Networks => {
                state
                    .operator
                    .resources
                    .networks
                    .insert("app-net".into(), NetworkDefinition::External { name: None });
            }
            ResourceSection::Volumes => {
                state
                    .operator
                    .resources
                    .volumes
                    .insert("app-data".into(), VolumeDefinition::External { name: None });
            }
            ResourceSection::Configs => {
                state
                    .operator
                    .resources
                    .configs
                    .insert("app-config".into(), DataDefinition::External { name: None });
            }
            ResourceSection::Secrets => {
                state
                    .operator
                    .resources
                    .secrets
                    .insert("app-secret".into(), DataDefinition::External { name: None });
            }
        }
        let error = install(&state, &resource_app()).unwrap_err();
        assert_eq!(error.code, CatalogErrorCode::ResourceConflict);
        assert_eq!(error.resource.unwrap().section, section);
    }
}

#[test]
fn current_managed_definitions_and_unused_resources_are_retained() {
    let mut state = stack("media");
    state.managed.resources = fixture().template;
    state.managed.services.push(service("old"));
    let installed = install(&state, &app("new")).unwrap();
    for section in ["networks", "volumes", "configs", "secrets"] {
        assert_eq!(
            artifact(&installed)[section],
            fixture().expected_artifact_resources[section]
        );
    }
    let removed = remove(&[state], "media", "old").unwrap();
    assert!(
        artifact(&removed)["services"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    for section in ["networks", "volumes", "configs", "secrets"] {
        assert_eq!(
            artifact(&removed)[section],
            fixture().expected_artifact_resources[section]
        );
    }
}

#[test]
fn same_app_in_two_stacks_requires_exact_target_and_removes_only_one_service() {
    let mut a = stack("a");
    a.managed.services = vec![service("web"), service("db")];
    let mut b = stack("b");
    b.managed.services = vec![service("web")];
    let plan = remove(&[a.clone(), b.clone()], "a", "web").unwrap();
    assert_eq!(
        plan.summary().installation,
        &InstallationKey {
            stack: "a".into(),
            app: "web".into()
        }
    );
    assert_eq!(plan.summary().shutdown_required, &["web".to_owned()]);
    assert_eq!(plan.summary().base_service_impact.services.len(), 1);
    assert!(artifact(&plan)["services"].get("web").is_none());
    assert!(artifact(&plan)["services"].get("db").is_some());
    assert_eq!(
        remove(&[a, b], "missing", "web").unwrap_err().code,
        CatalogErrorCode::NotFound
    );
}

#[test]
fn dependencies_block_removal_for_both_catalog_and_compose_requirements() {
    for catalog in [false, true] {
        let mut state = stack("media");
        let mut child = service("child");
        if catalog {
            child.requires.push("db".into());
        } else {
            child.depends_on.push("db".into());
        }
        state.managed.services = vec![service("db"), child];
        let failure = remove(&[state], "media", "db").unwrap_err();
        assert_eq!(failure.code, CatalogErrorCode::DependentServices);
        assert_eq!(failure.services, vec!["child"]);
    }
}

#[test]
fn catalog_requires_does_not_silently_add_compose_startup_order() {
    let mut state = stack("media");
    state.managed.services.push(service("db"));
    let mut template = app("web");
    template.service.requires.push("db".into());
    let plan = install(&state, &template).unwrap();
    assert!(
        artifact(&plan)["services"]["web"]
            .get("depends_on")
            .is_none()
    );
    template.service.depends_on.push("db".into());
    assert_eq!(
        artifact(&install(&state, &template).unwrap())["services"]["web"]["depends_on"],
        json!(["db"])
    );
    assert_eq!(
        install(&stack("other"), &template).unwrap_err().code,
        CatalogErrorCode::MissingDependencies
    );
}

#[test]
fn inherited_operator_services_are_never_removed_or_shadowed() {
    let mut state = stack("media");
    state.operator.services.push(service("web"));
    assert_eq!(
        remove(std::slice::from_ref(&state), "media", "web")
            .unwrap_err()
            .code,
        CatalogErrorCode::InheritedService
    );
    assert_eq!(
        install(&state, &app("web")).unwrap_err().code,
        CatalogErrorCode::AlreadyExists
    );
    state.managed.services.push(service("web"));
    assert_eq!(
        install(&state, &app("new")).unwrap_err().code,
        CatalogErrorCode::InheritedService
    );
}

#[test]
fn explicit_new_stack_and_orphan_policy_are_not_inferred() {
    let template = app("web");
    assert_eq!(
        plan_install(
            &[],
            &InstallTarget::Existing {
                stack: "media".into()
            },
            &template,
            OrphanPolicy::Preserve
        )
        .unwrap_err()
        .code,
        CatalogErrorCode::NotFound
    );
    let plan = plan_install(
        &[],
        &InstallTarget::New {
            stack: "media".into(),
        },
        &template,
        OrphanPolicy::RemoveOrphans,
    )
    .unwrap();
    assert_eq!(plan.summary().action, CatalogAction::InstallNew);
    assert_eq!(plan.summary().orphan_policy, OrphanPolicy::RemoveOrphans);
    assert!(
        plan.summary()
            .catalog_unresolved
            .contains(&CatalogGate::PreparedTargetAndAbsence)
    );
    assert_eq!(
        plan_install(
            &[stack("media")],
            &InstallTarget::New {
                stack: "media".into()
            },
            &template,
            OrphanPolicy::Preserve
        )
        .unwrap_err()
        .code,
        CatalogErrorCode::AlreadyExists
    );
    assert_eq!(
        install(&stack("media"), &template)
            .unwrap()
            .summary()
            .orphan_policy,
        OrphanPolicy::Preserve
    );
}

#[test]
fn numeric_user_and_case_sensitive_paths_survive_the_artifact() {
    let mut template = resource_app();
    template.service.base.user = Some(ComposeUser {
        uid: 1234,
        gid: 5678,
    });
    template.service.base.mounts.push(ComposeMount::Bind {
        source: "/srv/Media/Shows".into(),
        target: "/data/Shows".into(),
        read_only: false,
    });
    let value = artifact(&install(&stack("media"), &template).unwrap());
    assert_eq!(value["services"]["resource-app"]["user"], "1234:5678");
    assert_eq!(
        value["services"]["resource-app"]["volumes"][1]["source"],
        "/srv/Media/Shows"
    );
    assert_eq!(
        value["secrets"]["app-secret"]["file"],
        "/srv/Resource-App/Key.secret"
    );
}

#[test]
fn unknown_refs_placeholders_and_unresolved_resource_paths_refuse() {
    let mut template = resource_app();
    template.resources.configs.clear();
    assert_eq!(
        install(&stack("media"), &template).unwrap_err().code,
        CatalogErrorCode::InvalidReference
    );
    for path in [
        "{{CONFIG_DIR}}/key",
        "$HOME/key",
        "../secret",
        "/etc/../secret",
        "./a//key",
        "/a\0key",
    ] {
        let mut template = resource_app();
        template.resources.secrets.insert(
            "app-secret".into(),
            DataDefinition::File { file: path.into() },
        );
        assert_eq!(
            install(&stack("media"), &template).unwrap_err().code,
            CatalogErrorCode::InvalidInput,
            "{path}"
        );
    }
}

#[test]
fn unsupported_fields_cannot_be_discarded_from_typed_resources() {
    for value in [
        json!({"kind":"file","file":"./config","content":"DO_NOT_READ"}),
        json!({"kind":"environment","environment":"TOKEN"}),
    ] {
        assert!(serde_json::from_value::<DataDefinition>(value).is_err());
    }
    assert!(
        serde_json::from_value::<NetworkDefinition>(
            json!({"kind":"bridge","name":null,"internal":false,"driver_opts":{"unknown":"x"}})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<DataGrant>(
            json!({"source":"config","target":"/config","uid":"1234"})
        )
        .is_err()
    );
}

#[test]
fn config_secret_and_mount_target_collisions_are_rejected() {
    let mut template = resource_app();
    template.service.secrets[0].target = Some("/app/config".into());
    assert_eq!(
        install(&stack("media"), &template).unwrap_err().code,
        CatalogErrorCode::InvalidInput
    );
    template.service.secrets[0].target = Some("/data".into());
    assert_eq!(
        install(&stack("media"), &template).unwrap_err().code,
        CatalogErrorCode::InvalidInput
    );
    template.service.secrets[0].target = Some("/dev/key".into());
    template.service.base.devices.push(ComposeDevice {
        source: "/dev/example".into(),
        target: "/dev/key".into(),
    });
    assert_eq!(
        install(&stack("media"), &template).unwrap_err().code,
        CatalogErrorCode::InvalidInput
    );
}

#[test]
fn aggregate_selected_text_limit_rejects_individually_bounded_fields() {
    let mut state = stack("media");
    state.managed.services = (0..MAX_SERVICES - 1)
        .map(|i| {
            let mut value = service(&format!("app-{i}"));
            for j in 0..MAX_ENTRIES {
                value.base.mounts.push(ComposeMount::Bind {
                    source: format!("/Source/{i}/{j}/{}", "a".repeat(240)),
                    target: format!("/Target/{j}/{}", "b".repeat(240)),
                    read_only: true,
                });
            }
            value
        })
        .collect();
    let mut template = app("web");
    for j in 0..MAX_ENTRIES {
        template.service.base.devices.push(ComposeDevice {
            source: format!("/dev/source/{j}/{}", "c".repeat(240)),
            target: format!("/dev/target/{j}/{}", "d".repeat(240)),
        });
    }
    // No individual field or collection exceeds its own bound. Admission must
    // account for both selected sources and the incoming template together.
    assert_eq!(
        install(&state, &template).unwrap_err().code,
        CatalogErrorCode::Limit
    );
}

#[test]
fn json_escaping_cannot_expand_artifact_past_its_byte_bound() {
    let mut state = stack("media");
    for j in 0..MAX_ENTRIES {
        state
            .operator
            .resources
            .configs
            .insert(format!("cfg-{j}"), DataDefinition::External { name: None });
    }
    let grants = (0..MAX_ENTRIES)
        .map(|j| DataGrant {
            source: format!("cfg-{j}"),
            target: Some(format!(
                "/Config{j}/{}/{}",
                "\"".repeat(240),
                "\"".repeat(240)
            )),
        })
        .collect::<Vec<_>>();
    state.managed.services = (0..8)
        .map(|i| {
            let mut value = service(&format!("app-{i}"));
            value.configs = grants.clone();
            value
        })
        .collect();
    let mut template = app("web");
    template.service.configs = grants;
    // Legal literal Linux path bytes fit input admission, but JSON escaping
    // more than doubles this part of the artifact. Output has its own bound.
    admit(&state, Some(&template)).unwrap();
    assert_eq!(
        install(&state, &template).unwrap_err().code,
        CatalogErrorCode::Limit
    );
}

#[test]
fn relative_resource_references_and_external_names_are_preserved() {
    let mut template = resource_app();
    template.resources.configs.insert(
        "app-config".into(),
        DataDefinition::File {
            file: "./Config/App.conf".into(),
        },
    );
    template.resources.secrets.insert(
        "app-secret".into(),
        DataDefinition::External {
            name: Some("EXISTING-Secret".into()),
        },
    );
    let value = artifact(&install(&stack("media"), &template).unwrap());
    assert_eq!(value["configs"]["app-config"]["file"], "./Config/App.conf");
    assert_eq!(
        value["secrets"]["app-secret"],
        json!({"external":true,"name":"EXISTING-Secret"})
    );
}

#[test]
fn cycles_and_duplicate_metadata_names_refuse() {
    let mut template = app("web");
    template.service.requires.push("web".into());
    assert_eq!(
        install(&stack("media"), &template).unwrap_err().code,
        CatalogErrorCode::CyclicDependencies
    );
    template.service.requires = vec!["db".into(), "db".into()];
    assert_eq!(
        install(&stack("media"), &template).unwrap_err().code,
        CatalogErrorCode::InvalidInput
    );
}

#[test]
fn selected_service_and_resource_limits_fail_before_proposal_clones() {
    let mut state = stack("media");
    state.managed.services = (0..MAX_SERVICES)
        .map(|i| service(&format!("app-{i}")))
        .collect();
    assert_eq!(
        install(&state, &app("new")).unwrap_err().code,
        CatalogErrorCode::Limit
    );
    let mut template = app("web");
    template.service.networks = (0..=MAX_ENTRIES).map(|i| format!("net-{i}")).collect();
    assert_eq!(
        install(&stack("media"), &template).unwrap_err().code,
        CatalogErrorCode::Limit
    );
    template = app("web");
    template.resources.configs = (0..=MAX_RESOURCES)
        .map(|i| {
            (
                format!("cfg-{i}"),
                DataDefinition::File {
                    file: "./key".into(),
                },
            )
        })
        .collect();
    assert_eq!(
        install(&stack("media"), &template).unwrap_err().code,
        CatalogErrorCode::Limit
    );
}

#[test]
fn per_source_resource_limits_allow_distinct_operator_and_managed_resources() {
    let mut state = stack("media");
    for i in 0..MAX_RESOURCES {
        state.operator.resources.configs.insert(
            format!("op-{i}"),
            DataDefinition::File {
                file: "./operator.conf".into(),
            },
        );
        state.managed.resources.configs.insert(
            format!("own-{i}"),
            DataDefinition::File {
                file: "./managed.conf".into(),
            },
        );
    }
    let plan = install(&state, &app("web")).unwrap();
    assert_eq!(
        artifact(&plan)["configs"].as_object().unwrap().len(),
        MAX_RESOURCES
    );
}

#[test]
fn artifact_is_deterministic_and_debug_does_not_dump_payloads() {
    let state = stack("media");
    let template = resource_app();
    let a = install(&state, &template).unwrap();
    let b = install(&state, &template).unwrap();
    assert_eq!(a.artifact(), b.artifact());
    assert_eq!(
        serde_json::to_value(a.summary()).unwrap(),
        serde_json::to_value(b.summary()).unwrap()
    );
    assert!(!format!("{a:?}").contains("Key.secret"));
    assert_eq!(a.summary().compose_unresolved.len(), 8);
    assert!(
        a.summary()
            .catalog_unresolved
            .contains(&CatalogGate::ConfigSecretFileOwners)
    );
    assert!(
        a.summary()
            .catalog_unresolved
            .contains(&CatalogGate::VerifiedSelectedServiceShutdown)
    );
}
