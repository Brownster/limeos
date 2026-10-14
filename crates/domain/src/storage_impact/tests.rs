use super::*;
use crate::{
    ContainerStorageConsumer, ContainerStorageMount, ContainerStorageSource, StorageFilesystem,
    StorageRole, shares::SambaShareConfig,
};
use serde_json::{Value, json};

fn inputs(i: usize) -> (StorageContract, PoolsConfig, SnapraidConfig) {
    let fixture = &crate::pools::tests::fixtures()[i];
    (
        serde_json::from_value(fixture["contract"].clone()).unwrap(),
        serde_json::from_value(fixture["pools"].clone()).unwrap(),
        serde_json::from_value(fixture["snapraid"].clone()).unwrap(),
    )
}
fn share(name: &str, path: &str) -> SambaShareConfig {
    SambaShareConfig {
        name: name.into(),
        path: path.into(),
        comment: String::new(),
        read_only: false,
        guest_ok: false,
        browseable: true,
        valid_users: String::new(),
        enabled: true,
    }
}
fn shares() -> SambaSharesConfig {
    SambaSharesConfig {
        shares: vec![
            share("media", "/mnt/storage/Movies"),
            share("downloads", "/mnt/downloads/Complete"),
            share("docs", "/srv/docs"),
        ],
    }
}
fn consumer(id: char, path: &str) -> ContainerStorageConsumer {
    ContainerStorageConsumer {
        container: ContainerSnapshot {
            resource: format!("container:{}", id.to_string().repeat(64)),
            image: format!("sha256:{}", "f".repeat(64)),
            started_at: "2026-10-06T00:00:00Z".into(),
            running: true,
        },
        mounts: vec![ContainerStorageMount {
            source: ContainerStorageSource::Bind { path: path.into() },
            destination: "/data".into(),
            writable: true,
        }],
    }
}
fn inventory() -> ContainerStorageInventory {
    ContainerStorageInventory {
        version: 1,
        engine_id: "synthetic-engine".into(),
        observed_at: 100,
        containers: vec![
            consumer('a', "/mnt/storage/Movies"),
            consumer('b', "/mnt/downloads/Complete"),
        ],
    }
}
fn target(contract: &StorageContract, id: &str) -> StorageDevice {
    contract
        .devices
        .iter()
        .find(|d| d.id == id)
        .unwrap()
        .clone()
}
fn preview(i: usize, id: &str) -> ConfiguredStorageImpactPreview {
    let (c, p, s) = inputs(i);
    configured_storage_impact(&c, &p, &s, &inventory(), &shares(), &[target(&c, id)], 100).unwrap()
}
fn short(resource: &str) -> &str {
    &resource["container:".len().."container:".len() + 1]
}
fn labels(p: &ConfiguredStorageImpactPreview) -> Vec<String> {
    use ConfiguredStorageImpactResource::*;
    p.resource_candidates
        .iter()
        .map(|r| match r {
            Device { device_id } => format!("device:{device_id}"),
            Mount { mountpoint } => format!("mount:{mountpoint}"),
            Pool { pool_id } => format!("pool:{pool_id}"),
            Snapraid => "snapraid".into(),
            Share { share_name } => format!("share:{share_name}"),
            Container { resource } => format!("container:{}", short(resource)),
        })
        .collect()
}
fn code<T: std::fmt::Debug>(
    result: StoragePlanningResult<T>,
    field: &str,
    expected: StoragePlanningFindingCode,
) {
    let errors = result.unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].field, field);
    assert_eq!(errors[0].code, expected);
}

#[test]
fn four_frozen_layouts_match_independent_selection_and_resource_goldens() {
    let expected: Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/storage-impact/expected.json"
    ))
    .unwrap();
    for (i, f) in crate::pools::tests::fixtures().iter().enumerate() {
        let e = &expected[f["name"].as_str().unwrap()];
        let id = e["target"].as_str().unwrap();
        let p = preview(i, id);
        assert_eq!(
            json!({"target":id,"selected":p.selected_devices,"related":p.related_devices,
            "running":p.running_containers.iter().map(|c|short(&c.resource)).collect::<Vec<_>>(),
            "resources":labels(&p)}),
            *e
        );
        assert_eq!(p.evidence_needed.len(), 8);
    }
}
#[test]
fn full_checked_graph_and_original_age_remain_unchanged() {
    for i in 0..4 {
        let (c, p, s) = inputs(i);
        let docker = inventory();
        let smb = shares();
        let expected = configured_storage_dependency_graph(&c, &p, &s, &docker, &smb, 105).unwrap();
        let result =
            configured_storage_impact(&c, &p, &s, &docker, &smb, &[c.devices[0].clone()], 105)
                .unwrap();
        assert_eq!(result.graph, expected);
        assert_eq!(result.graph.containers_observed_at, 100);
        assert_eq!(result.graph.media_identity, c.media_identity);
        assert_eq!(result.graph.share_configuration, smb);
    }
}
#[test]
fn every_expected_device_field_must_match_the_current_valid_contract() {
    let (c, p, s) = inputs(0);
    let selected = c.devices[0].clone();
    for change in 0..5 {
        let mut changed = selected.clone();
        match change {
            0 => changed.filesystem_uuid = "private-uuid".into(),
            1 => changed.filesystem = StorageFilesystem::Xfs,
            2 => changed.role = StorageRole::Parity,
            3 => changed.mountpoint = "/mnt/private-mount".into(),
            _ => changed.serial = Some("private-serial".into()),
        }
        code(
            configured_storage_impact(&c, &p, &s, &inventory(), &shares(), &[changed], 100),
            "impact.selection",
            StoragePlanningFindingCode::StaleEvidence,
        );
    }
}
#[test]
fn missing_duplicate_unknown_and_amplified_selections_refuse_without_echoing_values() {
    let (c, p, s) = inputs(0);
    for selection in [vec![], vec![c.devices[0].clone(); 33]] {
        code(
            configured_storage_impact(&c, &p, &s, &inventory(), &shares(), &selection, 100),
            "impact.selection",
            StoragePlanningFindingCode::InvalidConfiguration,
        );
    }
    code(
        configured_storage_impact(
            &c,
            &p,
            &s,
            &inventory(),
            &shares(),
            &[c.devices[0].clone(), c.devices[0].clone()],
            100,
        ),
        "impact.selection",
        StoragePlanningFindingCode::Duplicate,
    );
    let mut unknown = c.devices[0].clone();
    unknown.id = "private-value".into();
    let errors = configured_storage_impact(&c, &p, &s, &inventory(), &shares(), &[unknown], 100)
        .unwrap_err();
    assert_eq!(errors[0].code, StoragePlanningFindingCode::UnknownIdentity);
    assert!(!format!("{errors:?}").contains("private-value"));
}
#[test]
fn stopped_disabled_and_read_only_dependents_still_enter_resource_candidates() {
    let (c, mut p, mut s) = inputs(2);
    p.pools[0].enabled = false;
    s.enabled = false;
    let mut docker = inventory();
    docker.containers[0].container.running = false;
    docker.containers[0].mounts[0].writable = false;
    let mut smb = shares();
    smb.shares[0].enabled = false;
    smb.shares[0].read_only = true;
    let result =
        configured_storage_impact(&c, &p, &s, &docker, &smb, &[c.devices[0].clone()], 100).unwrap();
    let values = labels(&result);
    for wanted in ["pool:pool1", "snapraid", "share:media", "container:a"] {
        assert!(values.iter().any(|r| r == wanted));
    }
    assert!(result.running_containers.is_empty());
    assert!(!result.graph.devices[0].storage.pools[0].enabled);
    assert!(!result.graph.devices[0].storage.protection[0].enabled);
    assert!(!result.graph.devices[0].storage.containers[0].writable);
    assert!(result.graph.share_configuration.shares[0].read_only);
}
#[test]
fn related_member_devices_do_not_import_their_direct_consumers_or_other_pools() {
    let (c, mut p, s) = inputs(3);
    let mut second = p.pools[0].clone();
    second.id = "pool2".into();
    second.name = "other".into();
    second.mount_point = "/mnt/otherpool".into();
    second.branches = vec!["/mnt/disk2/Other".into(), "/mnt/disk2/More".into()];
    p.pools.push(second);
    assert!(p.plan(&c).is_ok());
    let mut docker = inventory();
    docker.containers[1] = consumer('b', "/mnt/disk2/Other");
    docker.containers.push(consumer('c', "/mnt/otherpool"));
    let mut smb = shares();
    smb.shares.push(share("other", "/mnt/disk2/Other"));
    let selected = target(&c, "d1");
    let result =
        configured_storage_impact(&c, &p, &s, &docker, &smb, &[selected.clone()], 100).unwrap();
    assert_eq!(result.related_devices, vec!["d1", "d2", "p1", "p2"]);
    let values = labels(&result);
    assert!(values.contains(&"container:a".into()));
    for unwanted in [
        "container:b",
        "container:c",
        "share:other",
        "pool:pool2",
        "mount:/mnt/otherpool",
    ] {
        assert!(!values.iter().any(|r| r == unwanted));
    }
    let both = configured_storage_impact(
        &c,
        &p,
        &s,
        &docker,
        &smb,
        &[selected, target(&c, "d2")],
        100,
    )
    .unwrap();
    for wanted in ["container:b", "container:c", "share:other", "pool:pool2"] {
        assert!(labels(&both).iter().any(|r| r == wanted));
    }
}
#[test]
fn parity_loss_relates_the_whole_protection_configuration_without_selecting_pool_members() {
    let result = preview(3, "p2");
    assert_eq!(result.selected_devices, vec!["p2"]);
    assert_eq!(result.related_devices, vec!["d1", "d2", "p1", "p2"]);
    assert!(result.running_containers.is_empty());
    assert_eq!(
        labels(&result)
            .iter()
            .filter(|r| r.as_str() == "snapraid")
            .count(),
        1
    );
    assert!(
        !result
            .resource_candidates
            .iter()
            .any(|r| matches!(r, ConfiguredStorageImpactResource::Pool { .. }))
    );
    assert!(
        result
            .graph
            .devices
            .iter()
            .all(|d| !d.storage.protection.is_empty())
    );
}
#[test]
fn literal_components_case_ancestors_and_named_volumes_preserve_full_identity() {
    let (c, p, s) = inputs(0);
    let mut volume = consumer('a', "/mnt/storage/Movies");
    volume.mounts[0].source = ContainerStorageSource::Volume {
        path: "/mnt/storage/Movies".into(),
        name: "named-volume".into(),
        driver: "local".into(),
    };
    let mut docker = inventory();
    docker.containers = vec![
        volume.clone(),
        consumer('b', "/mnt/storage-other"),
        consumer('c', "/mnt/Storage/Movies"),
        consumer('d', "/mnt"),
        consumer('e', "/"),
    ];
    let result =
        configured_storage_impact(&c, &p, &s, &docker, &shares(), &[c.devices[0].clone()], 100)
            .unwrap();
    assert_eq!(
        result.running_containers,
        vec![
            volume.container,
            docker.containers[3].container.clone(),
            docker.containers[4].container.clone()
        ]
    );
    let row = &result.graph.devices[0].storage.containers[0];
    assert_eq!(row.source, volume.mounts[0].source);
    assert_eq!(row.destination, "/data");
    assert_eq!(row.container.image, format!("sha256:{}", "f".repeat(64)));
}
#[test]
fn target_route_and_input_enumeration_order_do_not_duplicate_candidates_or_running_identities() {
    let (mut c, p, s) = inputs(3);
    let mut docker = inventory();
    let smb = shares();
    docker.containers[0].mounts.push(ContainerStorageMount {
        source: ContainerStorageSource::Bind {
            path: "/mnt".into(),
        },
        destination: "/ancestor".into(),
        writable: false,
    });
    let targets = [target(&c, "d1"), target(&c, "d2")];
    let first = configured_storage_impact(&c, &p, &s, &docker, &smb, &targets, 100).unwrap();
    c.devices.reverse();
    docker.containers.reverse();
    let other = configured_storage_impact(
        &c,
        &p,
        &s,
        &docker,
        &smb,
        &[targets[1].clone(), targets[0].clone()],
        100,
    )
    .unwrap();
    assert_eq!(first, other);
    assert_eq!(first.running_containers.len(), 1);
    assert_eq!(
        labels(&first)
            .iter()
            .filter(|r| r.as_str() == "container:a")
            .count(),
        1
    );
}
#[test]
fn all_unused_configuration_and_original_container_age_are_validated() {
    let (c, p, s) = inputs(1);
    let selected = [target(&c, "downloads")];
    for at in [99, 106] {
        code(
            configured_storage_impact(&c, &p, &s, &inventory(), &shares(), &selected, at),
            "containers",
            StoragePlanningFindingCode::StaleEvidence,
        );
    }
    let mut invalid = c.clone();
    invalid.devices[2].serial = Some("private-value\n".into());
    let mut invalid_shares = shares();
    invalid_shares.shares[2].comment = "private-value\n".into();
    for errors in [
        configured_storage_impact(&invalid, &p, &s, &inventory(), &shares(), &selected, 100)
            .unwrap_err(),
        configured_storage_impact(&c, &p, &s, &inventory(), &invalid_shares, &selected, 100)
            .unwrap_err(),
    ] {
        assert!(!format!("{errors:?}").contains("private-value"));
    }
    let mut invalid_docker = inventory();
    invalid_docker.containers[0].mounts[0].source = ContainerStorageSource::Bind {
        path: "/private-value/../bad".into(),
    };
    assert!(
        configured_storage_impact(&c, &p, &s, &invalid_docker, &shares(), &selected, 100).is_err()
    );
}
#[test]
fn global_512_route_limit_is_not_bypassed_by_selecting_a_single_device() {
    let (c, p, s) = inputs(2);
    let selected = [target(&c, "data-01")];
    let mut docker = inventory();
    docker.containers = vec![consumer('a', "/mnt/storage")];
    docker.containers[0].mounts = (0..35)
        .map(|i| ContainerStorageMount {
            source: ContainerStorageSource::Bind {
                path: "/mnt/storage".into(),
            },
            destination: format!("/d{i}"),
            writable: true,
        })
        .collect();
    let mut smb = SambaSharesConfig {
        shares: (0..22)
            .map(|i| share(&format!("s{i}"), "/mnt/disks/data-01"))
            .collect(),
    };
    assert!(configured_storage_impact(&c, &p, &s, &docker, &smb, &selected, 100).is_ok());
    smb.shares.push(share("over", "/mnt/disks/data-01"));
    code(
        configured_storage_impact(&c, &p, &s, &docker, &smb, &selected, 100),
        "dependencies.consumers",
        StoragePlanningFindingCode::ThresholdExceeded,
    );
}
#[test]
fn exact_128_resources_pass_and_a_129th_distinct_subject_refuses_the_whole_preview() {
    let (c, p, s) = inputs(2);
    let selected = [target(&c, "data-01")];
    let smb = SambaSharesConfig {
        shares: (0..64)
            .map(|i| share(&format!("s{i}"), "/mnt/disks/data-01"))
            .collect(),
    };
    let make = |i: usize| {
        let mut value = consumer('a', "/mnt/disks/data-01");
        value.container.resource = format!("container:{i:064x}");
        value
    };
    let mut docker = inventory();
    docker.containers = (0..31).map(make).collect();
    let result = configured_storage_impact(&c, &p, &s, &docker, &smb, &selected, 100).unwrap();
    assert_eq!(result.resource_candidates.len(), 128);
    assert_eq!(result.running_containers.len(), 31);
    docker.containers.push(make(31));
    code(
        configured_storage_impact(&c, &p, &s, &docker, &smb, &selected, 100),
        "impact.resources",
        StoragePlanningFindingCode::ThresholdExceeded,
    );
}
#[test]
fn valid_bounded_inputs_can_exceed_the_report_byte_limit_without_returning_partial_rows() {
    let (c, p, s) = inputs(2);
    let selected = [target(&c, "data-01")];
    let mut docker = inventory();
    docker.containers = vec![consumer('a', "/mnt/storage")];
    docker.containers[0].mounts = (0..35)
        .map(|i| ContainerStorageMount {
            source: ContainerStorageSource::Bind {
                path: format!("/mnt/storage/{}", "x".repeat(480)),
            },
            destination: format!("/{}{i}", "d".repeat(480)),
            writable: true,
        })
        .collect();
    let smb = SambaSharesConfig::default();
    let graph = configured_storage_dependency_graph(&c, &p, &s, &docker, &smb, 100).unwrap();
    assert!(serde_json::to_vec(&graph).unwrap().len() > MAX_STORAGE_IMPACT_BYTES);
    code(
        configured_storage_impact(&c, &p, &s, &docker, &smb, &selected, 100),
        "impact.preview",
        StoragePlanningFindingCode::ThresholdExceeded,
    );
}
#[test]
fn empty_declared_dependents_keep_unmapped_shares_and_every_missing_evidence_boundary() {
    let (c, p, s) = inputs(1);
    let mut docker = inventory();
    docker.containers.clear();
    let smb = SambaSharesConfig {
        shares: vec![share("docs", "/srv/docs")],
    };
    let result =
        configured_storage_impact(&c, &p, &s, &docker, &smb, &[target(&c, "backup")], 100).unwrap();
    assert_eq!(labels(&result), vec!["device:backup", "mount:/mnt/backup"]);
    assert_eq!(result.graph.unmapped_shares, vec!["docs"]);
    assert!(result.running_containers.is_empty());
    assert_eq!(
        result.evidence_needed,
        vec![
            StorageImpactEvidenceNeeded::RetainedIdentityAndFreshness,
            StorageImpactEvidenceNeeded::CompletePhysicalConsumers,
            StorageImpactEvidenceNeeded::ActualPoolState,
            StorageImpactEvidenceNeeded::ActualProtectionState,
            StorageImpactEvidenceNeeded::ActualShareState,
            StorageImpactEvidenceNeeded::StackOwnership,
            StorageImpactEvidenceNeeded::CompleteClaimsAndApproval,
            StorageImpactEvidenceNeeded::EffectTimeValidation
        ]
    );
}
#[test]
fn planning_never_mutates_the_original_configurations_or_selection() {
    let (c, p, s) = inputs(3);
    let docker = inventory();
    let smb = shares();
    let selection = c.devices.clone();
    let before = (
        c.clone(),
        p.clone(),
        s.clone(),
        docker.clone(),
        smb.clone(),
        selection.clone(),
    );
    assert!(configured_storage_impact(&c, &p, &s, &docker, &smb, &selection, 100).is_ok());
    assert_eq!((c, p, s, docker, smb, selection), before);
}
