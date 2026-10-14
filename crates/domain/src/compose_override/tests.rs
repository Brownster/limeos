use super::*;
use crate::{ComposeCatalog, ComposeDevice, ComposePrivilege, ComposeUser, PortProtocol};
use serde_json::{Value, json};

fn service(name: &str) -> ComposeService {
    ComposeService {
        name: name.into(),
        image: format!("example.invalid/{name}@sha256:{}", "a".repeat(64)),
        user: Some(ComposeUser {
            uid: 1000,
            gid: 1000,
        }),
        restart: ComposeRestart::UnlessStopped,
        ports: vec![],
        mounts: vec![],
        devices: vec![],
        privileged: false,
        host_network: false,
    }
}
fn project(services: Vec<ComposeService>) -> ComposeProject {
    ComposeProject { services }
}
fn port(ip: &str, published: u16, target: u16, protocol: PortProtocol) -> ComposePort {
    ComposePort {
        host_ip: ip.into(),
        published,
        target,
        protocol,
    }
}
fn bind(source: &str, target: &str, read_only: bool) -> ComposeMount {
    ComposeMount::Bind {
        source: source.into(),
        target: target.into(),
        read_only,
    }
}
fn volume(name: &str, target: &str, read_only: bool) -> ComposeMount {
    ComposeMount::Volume {
        name: name.into(),
        target: target.into(),
        read_only,
    }
}
fn device(source: &str, target: &str) -> ComposeDevice {
    ComposeDevice {
        source: source.into(),
        target: target.into(),
    }
}
fn artifact(plan: &ManagedOverridePlan) -> Value {
    serde_json::from_slice(plan.artifact()).unwrap()
}
fn conflict(base: &ComposeProject, desired: &ComposeProject) {
    assert!(matches!(
        plan_managed_override(base, base, desired),
        Err(Error(ErrorCode::Conflict))
    ));
}

// Test-side oracle applies map/scalar, unique-resource and append rules to JSON.
// It does not call the compiler's representability or delta selection helpers.
fn merge(base: &mut Value, overlay: &Value, field: &str) {
    match (base, overlay) {
        (Value::Object(a), Value::Object(b)) => {
            for (key, value) in b {
                if let Some(existing) = a.get_mut(key) {
                    merge(existing, value, key);
                } else {
                    a.insert(key.clone(), value.clone());
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            for value in b {
                let key_matches = |old: &Value| match field {
                    "volumes" => old["target"] == value["target"],
                    "ports" => ["host_ip", "target", "published", "protocol"]
                        .iter()
                        .all(|key| old[key] == value[key]),
                    _ => false,
                };
                if let Some(old) = a.iter_mut().find(|old| key_matches(old)) {
                    merge(old, value, field);
                } else {
                    a.push(value.clone());
                }
            }
        }
        (a, b) => *a = b.clone(),
    }
}
fn wire_services(project: &ComposeProject) -> Value {
    let mut services = serde_json::Map::new();
    for s in &project.services {
        let mut value = json!({"image":s.image,"restart":s.restart,"privileged":s.privileged});
        if let Some(user) = s.user {
            value["user"] = json!(format!("{}:{}", user.uid, user.gid));
        }
        if s.host_network {
            value["network_mode"] = json!("host");
        }
        if !s.ports.is_empty() {
            value["ports"] = Value::Array(
                s.ports
                    .iter()
                    .map(|p| {
                        json!({
                            "host_ip":p.host_ip,"published":p.published.to_string(),
                            "target":p.target,"protocol":p.protocol
                        })
                    })
                    .collect(),
            );
        }
        if !s.mounts.is_empty() {
            value["volumes"] = Value::Array(s.mounts.iter().map(|m| match m {
                ComposeMount::Volume {name,target,read_only} =>
                    json!({"type":"volume","source":name,"target":target,"read_only":read_only}),
                ComposeMount::Bind {source,target,read_only} =>
                    json!({"type":"bind","source":source,"target":target,"read_only":read_only,
                           "bind":{"create_host_path":false}}),
            }).collect());
        }
        if !s.devices.is_empty() {
            value["devices"] = json!(
                s.devices
                    .iter()
                    .map(|d| format!("{}:{}:rwm", d.source, d.target))
                    .collect::<Vec<_>>()
            );
        }
        services.insert(s.name.clone(), value);
    }
    Value::Object(services)
}
fn sorted_sequences(value: &mut Value) {
    match value {
        Value::Array(a) => {
            for item in a.iter_mut() {
                sorted_sequences(item);
            }
            a.sort_by_key(Value::to_string);
        }
        Value::Object(o) => {
            for item in o.values_mut() {
                sorted_sequences(item);
            }
        }
        _ => {}
    }
}
fn assert_combined(plan: &ManagedOverridePlan) {
    let summary = plan.summary();
    let mut actual = wire_services(summary.operator);
    merge(&mut actual, &artifact(plan)["services"], "services");
    let mut expected = wire_services(summary.desired);
    sorted_sequences(&mut actual);
    sorted_sequences(&mut expected);
    assert_eq!(actual, expected);
}

#[test]
fn byte_exact_new_service_preserves_numeric_ownership_and_long_syntax() {
    let mut s = service("worker");
    s.user = Some(ComposeUser {
        uid: 12345,
        gid: 23456,
    });
    s.restart = ComposeRestart::No;
    s.ports = vec![port("::1", 8443, 443, PortProtocol::Tcp)];
    s.mounts = vec![
        volume("cache", "/cache", true),
        bind("/srv/media", "/media", false),
    ];
    s.devices = vec![device("/dev/video0", "/dev/video1")];
    let plan =
        plan_managed_override(&project(vec![]), &project(vec![]), &project(vec![s])).unwrap();
    assert_eq!(
        plan.artifact(),
        include_bytes!("../../../../tests/fixtures/compose-override/new-service.override.json")
            .strip_suffix(b"\n")
            .unwrap()
    );
    assert_combined(&plan);
}

#[test]
fn merge_delta_changes_mount_sources_and_modes_without_duplicating_inherited_rows() {
    let mut s = service("api");
    s.ports = vec![port("0.0.0.0", 8080, 80, PortProtocol::Tcp)];
    s.mounts = vec![
        volume("cache", "/cache", false),
        bind("/srv/media", "/media", true),
    ];
    s.devices = vec![device("/dev/video0", "/dev/video0")];
    let base = project(vec![s.clone()]);
    s.image = format!("example.invalid/api@sha256:{}", "b".repeat(64));
    s.user = Some(ComposeUser {
        uid: 0,
        gid: u32::MAX,
    });
    s.privileged = true;
    s.ports.push(port("127.0.0.1", 8081, 81, PortProtocol::Udp));
    s.mounts = vec![
        volume("cache-new", "/cache", true),
        bind("/srv/new-media", "/media", false),
    ];
    s.devices.push(device("/dev/video1", "/dev/video1"));
    let plan = plan_managed_override(&base, &base, &project(vec![s])).unwrap();
    assert_eq!(
        plan.artifact(),
        include_bytes!("../../../../tests/fixtures/compose-override/merge-delta.override.json")
            .strip_suffix(b"\n")
            .unwrap()
    );
    assert_combined(&plan);
    assert_eq!(
        artifact(&plan)["services"]["api"]["ports"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        artifact(&plan)["services"]["api"]["devices"],
        json!(["/dev/video1:/dev/video1:rwm"])
    );
}

#[test]
fn removing_one_managed_service_preserves_unrelated_operator_and_managed_services() {
    let base = project(vec![service("operator")]);
    let current = project(vec![
        service("other"),
        service("retired"),
        service("operator"),
    ]);
    let desired = project(vec![service("other"), service("operator")]);
    let unchanged = serde_json::to_vec(&base).unwrap();
    let plan = plan_managed_override(&base, &current, &desired).unwrap();
    assert_eq!(plan.summary().shutdown_required, ["retired"]);
    assert_eq!(plan.summary().impact.services.len(), 1);
    assert!(artifact(&plan)["services"].get("operator").is_none());
    assert!(artifact(&plan)["services"].get("retired").is_none());
    assert!(artifact(&plan)["services"].get("other").is_some());
    assert_eq!(serde_json::to_vec(&base).unwrap(), unchanged);
    assert_combined(&plan);
}

#[test]
fn empty_desired_keeps_shutdown_names_and_all_execution_gates() {
    let empty = project(vec![]);
    let plan = plan_managed_override(&empty, &project(vec![service("last")]), &empty).unwrap();
    assert_eq!(plan.artifact(), br#"{"services":{}}"#);
    assert_eq!(plan.summary().shutdown_required, ["last"]);
    assert_eq!(plan.summary().unresolved, GATES);
    assert_eq!(plan.summary().managed_file, COMPOSE_MANAGED_FILE);
    let summary = serde_json::to_value(plan.summary()).unwrap();
    assert!(summary.get("approval").is_none());
    assert!(summary.get("receipt").is_none());
    assert!(summary.get("artifact").is_none());
    assert_eq!(summary["artifact_bytes"], plan.artifact().len());
}

#[test]
fn operator_service_removal_refuses_even_when_other_services_survive() {
    let base = project(vec![service("api"), service("worker")]);
    conflict(&base, &project(vec![service("worker")]));
    conflict(&base, &project(vec![]));
}

#[test]
fn inherited_port_removal_and_remapping_refuse_instead_of_silently_appending() {
    let mut s = service("api");
    s.ports.push(port("127.0.0.1", 8080, 80, PortProtocol::Tcp));
    let base = project(vec![s.clone()]);
    for replacement in [
        None,
        Some(port("127.0.0.1", 8081, 80, PortProtocol::Tcp)),
        Some(port("127.0.0.2", 8080, 80, PortProtocol::Tcp)),
        Some(port("127.0.0.1", 8080, 81, PortProtocol::Tcp)),
        Some(port("127.0.0.1", 8080, 80, PortProtocol::Udp)),
    ] {
        s.ports = replacement.into_iter().collect();
        conflict(&base, &project(vec![s.clone()]));
    }
}

#[test]
fn inherited_mount_removal_and_kind_switch_refuse_without_reset_tags() {
    let mut s = service("api");
    s.mounts.push(bind("/srv/media", "/media", false));
    let base = project(vec![s.clone()]);
    s.mounts.clear();
    conflict(&base, &project(vec![s.clone()]));
    s.mounts.push(volume("media", "/media", false));
    conflict(&base, &project(vec![s]));
    let base = project(vec![ComposeService {
        mounts: vec![volume("media", "/media", false)],
        ..service("api")
    }]);
    let desired = project(vec![ComposeService {
        mounts: vec![bind("/srv/media", "/media", false)],
        ..service("api")
    }]);
    conflict(&base, &desired);
}

#[test]
fn inherited_device_removal_or_source_and_target_remapping_refuses() {
    let mut s = service("api");
    s.devices.push(device("/dev/video0", "/dev/video0"));
    let base = project(vec![s.clone()]);
    for devices in [
        vec![],
        vec![device("/dev/video1", "/dev/video0")],
        vec![device("/dev/video0", "/dev/video1")],
    ] {
        s.devices = devices;
        conflict(&base, &project(vec![s.clone()]));
    }
}

#[test]
fn inherited_user_or_host_network_clearing_refuses_but_explicit_privilege_clearing_works() {
    let mut s = service("api");
    let base = project(vec![s.clone()]);
    s.user = None;
    conflict(&base, &project(vec![s.clone()]));
    s.user = Some(ComposeUser { uid: 0, gid: 0 });
    s.host_network = true;
    s.privileged = true;
    let elevated = project(vec![s.clone()]);
    s.host_network = false;
    conflict(&elevated, &project(vec![s.clone()]));
    s.host_network = true;
    s.privileged = false;
    s.restart = ComposeRestart::No;
    let plan = plan_managed_override(&elevated, &elevated, &project(vec![s])).unwrap();
    assert_eq!(artifact(&plan)["services"]["api"]["privileged"], false);
    assert_combined(&plan);
}

#[test]
fn replacing_managed_file_can_clear_managed_ports_devices_user_and_host_network() {
    let base = project(vec![ComposeService {
        user: None,
        ..service("api")
    }]);
    let mut s = base.services[0].clone();
    s.user = Some(ComposeUser { uid: 42, gid: 43 });
    s.ports.push(port("0.0.0.0", 8080, 80, PortProtocol::Tcp));
    s.devices.push(device("/dev/video0", "/dev/video0"));
    let plan = plan_managed_override(&base, &project(vec![s]), &base).unwrap();
    assert_eq!(plan.artifact(), br#"{"services":{}}"#);
    let current = project(vec![ComposeService {
        host_network: true,
        ..base.services[0].clone()
    }]);
    assert!(plan_managed_override(&base, &current, &base).is_ok());
    assert_combined(&plan);
}

#[test]
fn existing_p03_catalog_retains_full_impact_and_elevation_without_reusing_approval() {
    let catalog: ComposeCatalog = serde_json::from_slice(include_bytes!(
        "../../../../tests/fixtures/compose-catalog.json"
    ))
    .unwrap();
    let stack = &catalog.stacks[0];
    let base = project(vec![stack.current.services[0].clone()]);
    let standard = &stack
        .templates
        .iter()
        .find(|t| t.id == "standard")
        .unwrap()
        .project;
    let plan = plan_managed_override(&base, &stack.current, standard).unwrap();
    assert_eq!(plan.summary().shutdown_required, ["retired"]);
    assert_eq!(plan.summary().impact.services.len(), 3);
    assert_combined(&plan);
    let elevated = &stack
        .templates
        .iter()
        .find(|t| t.id == "elevated")
        .unwrap()
        .project;
    let plan = plan_managed_override(&project(vec![]), &stack.current, elevated).unwrap();
    assert_eq!(
        plan.summary().impact.elevated,
        [
            ComposePrivilege::Privileged,
            ComposePrivilege::HostMount,
            ComposePrivilege::Device,
            ComposePrivilege::DockerSocket,
            ComposePrivilege::HostNetwork
        ]
    );
    assert_eq!(
        artifact(&plan)["services"]["engine"]["network_mode"],
        "host"
    );
    assert_eq!(plan.summary().unresolved.len(), 8);
    assert_combined(&plan);
}

#[test]
fn invalid_or_unrepresentable_current_input_refuses_before_proposing_desired_changes() {
    let base = project(vec![service("api")]);
    assert!(matches!(
        plan_managed_override(&base, &project(vec![]), &base),
        Err(Error(ErrorCode::Conflict))
    ));
    let mut invalid = base.clone();
    invalid.services[0].image = "untrusted:latest".into();
    assert!(matches!(
        plan_managed_override(&base, &invalid, &base),
        Err(Error(ErrorCode::InvalidInput))
    ));
}

#[test]
fn deterministic_order_and_ip_normalization_preserve_borrowed_inputs() {
    let mut a = service("alpha");
    a.ports = vec![
        port("0:0:0:0:0:0:0:1", 8081, 81, PortProtocol::Tcp),
        port("127.0.0.1", 8080, 80, PortProtocol::Udp),
    ];
    a.mounts = vec![bind("/srv/z", "/z", true), volume("cache", "/a", false)];
    a.devices = vec![
        device("/dev/video1", "/dev/z"),
        device("/dev/video0", "/dev/a"),
    ];
    let original = project(vec![service("beta"), a]);
    let saved = original.clone();
    let empty = project(vec![]);
    let one = plan_managed_override(&empty, &empty, &original).unwrap();
    let mut reordered = original.clone();
    reordered.services.reverse();
    for s in &mut reordered.services {
        s.ports.reverse();
        s.mounts.reverse();
        s.devices.reverse();
    }
    let two = plan_managed_override(&empty, &empty, &reordered).unwrap();
    assert_eq!(one.artifact(), two.artifact());
    assert_eq!(one.summary().desired, two.summary().desired);
    assert_eq!(original, saved);
    assert_eq!(one.summary().desired.services[0].ports[1].host_ip, "::1");
    assert_combined(&one);
}

#[test]
fn all_borrowed_collection_and_string_limits_refuse_before_clone() {
    let good = project(vec![service("api")]);
    let mut excess = vec![project(
        (0..17).map(|n| service(&format!("s{n}"))).collect(),
    )];
    for field in ["name", "image", "ip", "bind", "target", "volume", "device"] {
        let mut s = service("api");
        match field {
            "name" => s.name = "x".repeat(49),
            "image" => s.image = "x".repeat(265),
            "ip" => s
                .ports
                .push(port(&"x".repeat(46), 8080, 80, PortProtocol::Tcp)),
            "bind" => s.mounts.push(bind(&"x".repeat(257), "/media", false)),
            "target" => s.mounts.push(volume("media", &"x".repeat(257), false)),
            "volume" => s.mounts.push(volume(&"x".repeat(49), "/media", false)),
            "device" => s.devices.push(device(&"x".repeat(257), "/dev/video0")),
            _ => unreachable!(),
        }
        excess.push(project(vec![s]));
    }
    for category in ["ports", "mounts", "devices"] {
        let mut s = service("api");
        for n in 0..9 {
            match category {
                "ports" => s
                    .ports
                    .push(port("127.0.0.1", 8000 + n, 80, PortProtocol::Tcp)),
                "mounts" => s.mounts.push(volume("media", &format!("/m{n}"), false)),
                "devices" => s.devices.push(device("/dev/video0", &format!("/dev/v{n}"))),
                _ => unreachable!(),
            }
        }
        excess.push(project(vec![s]));
    }
    for invalid in excess {
        assert!(preflight(&invalid).is_err());
        for inputs in [
            (&invalid, &good, &good),
            (&good, &invalid, &good),
            (&good, &good, &invalid),
        ] {
            assert!(matches!(
                plan_managed_override(inputs.0, inputs.1, inputs.2),
                Err(Error(ErrorCode::InvalidInput))
            ));
        }
    }
}

#[test]
fn maximum_supported_counts_deduplicate_named_volumes_and_refuse_artifact_excess() {
    let empty = project(vec![]);
    let desired = project(
        (0..16)
            .map(|n| {
                let mut s = service(&format!("s{n}"));
                s.mounts = (0..8)
                    .map(|m| volume("shared", &format!("/m{m}"), false))
                    .collect();
                s
            })
            .collect(),
    );
    let plan = plan_managed_override(&empty, &empty, &desired).unwrap();
    assert_eq!(artifact(&plan)["volumes"], json!({"shared":{}}));
    assert_combined(&plan);
    let distinct = project(
        (0..16)
            .map(|n| {
                let mut s = service(&format!("s{n}"));
                s.mounts = (0..8)
                    .map(|m| volume(&format!("s{n}m{m}"), &format!("/m{m}"), false))
                    .collect();
                s
            })
            .collect(),
    );
    let maximum = plan_managed_override(&empty, &empty, &distinct).unwrap();
    assert_eq!(
        artifact(&maximum)["volumes"].as_object().unwrap().len(),
        128
    );
    assert_combined(&maximum);
    let mut huge = desired;
    for s in &mut huge.services {
        s.mounts = (0..8)
            .map(|n| {
                bind(
                    &format!("/{}", "s".repeat(255)),
                    &format!("/m{n}{}", "t".repeat(252)),
                    false,
                )
            })
            .collect();
    }
    let mut valid = huge.clone();
    valid.normalize().unwrap();
    assert!(matches!(
        plan_managed_override(&empty, &empty, &huge),
        Err(Error(ErrorCode::InvalidInput))
    ));
}

#[test]
fn capped_writers_accept_exact_limits_and_never_extend_on_excess() {
    let mut artifact = ArtifactWriter(vec![0; ARTIFACT_BYTES - 1]);
    artifact.write_all(&[1]).unwrap();
    assert_eq!(artifact.0.len(), ARTIFACT_BYTES);
    assert!(artifact.write_all(&[2]).is_err());
    assert_eq!(artifact.0.len(), ARTIFACT_BYTES);
    let mut summary = CountWriter(SUMMARY_BYTES - 1);
    summary.write_all(&[1]).unwrap();
    assert!(summary.write_all(&[2]).is_err());
    assert_eq!(summary.0, SUMMARY_BYTES);
}

#[test]
fn unchanged_projects_have_empty_override_and_no_shutdown_or_impact() {
    let base = project(vec![service("api")]);
    let plan = plan_managed_override(&base, &base, &base).unwrap();
    assert_eq!(plan.artifact(), br#"{"services":{}}"#);
    assert!(plan.summary().impact.services.is_empty());
    assert!(plan.summary().shutdown_required.is_empty());
    assert_combined(&plan);
}

#[test]
fn invalid_desired_paths_images_ports_and_mount_device_collisions_refuse() {
    let empty = project(vec![]);
    let mut bad_image = service("api");
    bad_image.image = "${IMAGE}".into();
    let mut bad_path = service("api");
    bad_path.mounts.push(bind("/srv/../media", "/media", false));
    let mut bad_ports = service("api");
    bad_ports.ports = vec![
        port("0.0.0.0", 8080, 80, PortProtocol::Tcp),
        port("127.0.0.1", 8080, 81, PortProtocol::Tcp),
    ];
    let mut bad_target = service("api");
    bad_target
        .mounts
        .push(bind("/srv/media", "/dev/video0", false));
    bad_target
        .devices
        .push(device("/dev/video0", "/dev/video0"));
    for bad in [bad_image, bad_path, bad_ports, bad_target] {
        assert!(matches!(
            plan_managed_override(&empty, &empty, &project(vec![bad])),
            Err(Error(ErrorCode::InvalidInput))
        ));
    }
}
