use super::*;
fn catalog() -> ComposeCatalog {
    serde_json::from_str(include_str!(
        "../../../../tests/fixtures/compose-catalog.json"
    ))
    .unwrap()
}
#[test]
fn impact_describes_added_removed_changed_services_and_preserves_numeric_owners() {
    let mut catalog = catalog();
    catalog.normalize().unwrap();
    let stack = &catalog.stacks[0];
    let template = stack.templates.iter().find(|t| t.id == "standard").unwrap();
    let diff = ComposeImpact::between(&stack.current, &template.project);
    assert_eq!(
        diff.services
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["api", "retired", "worker"]
    );
    assert!(diff.services[0].before.is_some() && diff.services[0].after.is_some());
    assert!(diff.services[1].after.is_none() && diff.services[2].before.is_none());
    assert_eq!(
        diff.services[0].after.as_ref().unwrap().user,
        Some(ComposeUser {
            uid: 1000,
            gid: 1000
        })
    );
    assert!(diff.elevated.is_empty());
    assert!(
        ComposeImpact::between(&template.project, &template.project)
            .services
            .is_empty()
    );
}
#[test]
fn every_supported_elevation_is_derived_from_manifest_not_caller_claims() {
    let mut catalog = catalog();
    catalog.normalize().unwrap();
    let project = &catalog.stacks[0].templates[0].project;
    assert_eq!(
        project.elevated(),
        [
            ComposePrivilege::Privileged,
            ComposePrivilege::HostMount,
            ComposePrivilege::Device,
            ComposePrivilege::DockerSocket,
            ComposePrivilege::HostNetwork
        ]
    );
    let mut project = project.clone();
    project.services[0].privileged = false;
    project.services[0].host_network = false;
    project.services[0].devices.clear();
    project.services[0].mounts[0] = ComposeMount::Bind {
        source: "/mnt/media".into(),
        target: "/data".into(),
        read_only: true,
    };
    assert_eq!(project.elevated(), [ComposePrivilege::HostMount]);
}
#[test]
fn normalization_is_stable_across_catalog_and_service_order() {
    let mut first = catalog();
    let mut second = first.clone();
    second.stacks[0].templates.reverse();
    second.stacks[0].current.services.reverse();
    second.stacks[0].templates[0].project.services.reverse();
    first.normalize().unwrap();
    second.normalize().unwrap();
    assert_eq!(first, second);
}
#[test]
fn unpinned_images_interpolation_traversal_duplicate_services_and_ambiguous_files_fail() {
    for bad in [
        "example.invalid/app:latest",
        "${IMAGE}",
        "user:password@example.invalid/app",
        "../app@sha256:bad",
    ] {
        let mut c = catalog();
        c.stacks[0].current.services[0].image = bad.into();
        assert!(c.normalize().is_err(), "{bad}");
    }
    for bad in [
        "relative/path",
        "/mnt/../secret",
        "/mnt//data",
        "/mnt/${DATA}",
        "/mnt/./data",
        "/mnt/data\n",
    ] {
        let mut c = catalog();
        c.stacks[0].templates[0].project.services[0].mounts = vec![ComposeMount::Bind {
            source: bad.into(),
            target: "/data".into(),
            read_only: true,
        }];
        assert!(c.normalize().is_err(), "{bad}");
    }
    let mut c = catalog();
    let duplicate = c.stacks[0].current.services[0].clone();
    c.stacks[0].current.services.push(duplicate);
    assert!(c.normalize().is_err());
    let mut c = catalog();
    c.stacks[0].operator_files.push(ComposeFile {
        name: ComposeFileName::DockerComposeYml,
        sha256: "2".repeat(64),
    });
    assert!(c.normalize().is_err());
}
#[test]
fn wildcard_port_conflicts_and_overlapping_mount_targets_fail() {
    let port = ComposePort {
        host_ip: "127.0.0.1".into(),
        published: 8080,
        target: 80,
        protocol: PortProtocol::Tcp,
    };
    let mut c = catalog();
    c.stacks[0].current.services[0].ports = vec![port.clone()];
    c.stacks[0].current.services[1].ports = vec![ComposePort {
        host_ip: "0.0.0.0".into(),
        ..port
    }];
    assert!(c.normalize().is_err());
    let mut c = catalog();
    c.stacks[0].templates[0].project.services[0].mounts = vec![
        ComposeMount::Volume {
            name: "config".into(),
            target: "/config".into(),
            read_only: false,
        },
        ComposeMount::Bind {
            source: "/mnt/data".into(),
            target: "/config".into(),
            read_only: true,
        },
    ];
    assert!(c.normalize().is_err());
}
#[test]
fn unsupported_privilege_fields_arbitrary_yaml_and_secret_environment_are_rejected() {
    for extra in [
        "cap_add",
        "security_opt",
        "environment",
        "command",
        "compose_yaml",
    ] {
        let mut value = serde_json::to_value(catalog()).unwrap();
        value["stacks"][0]["templates"][0]["project"]["services"][0][extra] =
            serde_json::json!(["arbitrary"]);
        assert!(
            serde_json::from_value::<ComposeCatalog>(value).is_err(),
            "{extra}"
        );
    }
    assert!(
        serde_json::from_str::<ComposeSelection>(
            r#"{"stack":"media","template":"standard","principal":"admin"}"#
        )
        .is_err()
    );
}
