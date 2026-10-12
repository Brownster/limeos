use super::*;
use crate::{ContainerStorageConsumer, ContainerStorageMount, ProtectionPathKind, StorageProfile};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn layouts() -> Vec<Value> {
    crate::pools::tests::fixtures()
}

fn inputs(index: usize) -> (StorageContract, PoolsConfig, SnapraidConfig) {
    let case = &layouts()[index];
    (
        serde_json::from_value(case["contract"].clone()).unwrap(),
        serde_json::from_value(case["pools"].clone()).unwrap(),
        serde_json::from_value(case["snapraid"].clone()).unwrap(),
    )
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

fn preview(
    index: usize,
    containers: &ContainerStorageInventory,
) -> ConfiguredStorageDependencyPreview {
    let (contract, pools, protection) = inputs(index);
    configured_storage_dependencies(&contract, &pools, &protection, containers, 100).unwrap()
}

fn count(preview: &ConfiguredStorageDependencyPreview) -> usize {
    preview.devices.iter().map(|d| d.containers.len()).sum()
}

fn device<'a>(
    preview: &'a ConfiguredStorageDependencyPreview,
    id: &str,
) -> &'a ConfiguredDeviceDependency {
    preview
        .devices
        .iter()
        .find(|d| d.expected.id == id)
        .unwrap()
}

fn summary(preview: &ConfiguredStorageDependencyPreview) -> Value {
    let mut pools: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut containers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut requirements = 0;
    for d in &preview.devices {
        for pool in &d.pools {
            pools
                .entry(pool.pool_id.clone())
                .or_default()
                .push(d.expected.id.clone());
        }
        requirements += d
            .protection
            .iter()
            .map(|p| p.requirements.len())
            .sum::<usize>();
        for c in &d.containers {
            let route = match &c.via {
                ConfiguredContainerRoute::Direct => "direct".into(),
                ConfiguredContainerRoute::Pool { pool_id } => format!("pool:{pool_id}"),
            };
            let key = format!("{}/{route}", &c.container.resource[10..11]);
            containers
                .entry(key)
                .or_default()
                .push(d.expected.id.clone());
        }
    }
    json!({
        "devices": preview.devices.iter().map(|d| &d.expected.id).collect::<Vec<_>>(),
        "pools": pools,
        "protection_requirements": requirements,
        "containers": containers,
    })
}

#[test]
fn four_frozen_layouts_match_independent_golden_relations() {
    let expected: Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/configured-dependencies/expected.json"
    ))
    .unwrap();
    for (index, case) in layouts().iter().enumerate() {
        let result = preview(index, &inventory());
        assert_eq!(summary(&result), expected[case["name"].as_str().unwrap()]);
        assert_eq!(result.media_identity, inputs(index).0.media_identity);
        for d in result.devices {
            for protection in d.protection {
                assert!(
                    protection
                        .requirements
                        .iter()
                        .all(|r| r.expected == d.expected)
                );
            }
        }
    }
}

#[test]
fn pool_paths_expand_to_every_data_member_and_never_parity() {
    let result = preview(2, &inventory());
    assert_eq!(count(&result), 14);
    assert!(device(&result, "parity-01").containers.is_empty());
    for d in result
        .devices
        .iter()
        .filter(|d| d.expected.id.starts_with("data-"))
    {
        assert_eq!(d.containers.len(), 1);
        assert_eq!(
            d.containers[0].via,
            ConfiguredContainerRoute::Pool {
                pool_id: "pool1".into()
            }
        );
        assert_eq!(d.pools[0].branches, [d.expected.mountpoint.clone()]);
    }
}

#[test]
fn direct_branch_paths_do_not_invent_consumption_of_other_pool_members() {
    let mut containers = inventory();
    containers.containers = vec![consumer('a', "/mnt/disks/data-01/Movies")];
    let result = preview(2, &containers);
    assert_eq!(count(&result), 1);
    assert_eq!(
        device(&result, "data-01").containers[0].via,
        ConfiguredContainerRoute::Direct
    );
    assert!(device(&result, "data-02").containers.is_empty());
}

#[test]
fn disabled_configuration_stopped_containers_and_read_only_mounts_remain_visible() {
    let (contract, mut pools, mut protection) = inputs(3);
    pools.pools[0].enabled = false;
    protection.enabled = false;
    let mut containers = inventory();
    containers.containers[0].container.running = false;
    containers.containers[0].mounts[0].writable = false;
    let result =
        configured_storage_dependencies(&contract, &pools, &protection, &containers, 100).unwrap();
    let d = device(&result, "d1");
    assert!(!d.pools[0].enabled && !d.protection[0].enabled);
    assert!(!d.containers[0].container.running && !d.containers[0].writable);
    assert_eq!(
        d.containers[0].container,
        containers.containers[0].container
    );
    assert!(!device(&result, "p2").protection[0].enabled);
}

#[test]
fn ancestor_sources_keep_both_direct_and_pool_routes() {
    let mut containers = inventory();
    containers.containers = vec![consumer('a', "/mnt"), consumer('b', "/")];
    let result = preview(2, &containers);
    assert_eq!(count(&result), 58); // 15 direct + 14 pool matches for each source.
    assert_eq!(device(&result, "data-01").containers.len(), 4);
    assert_eq!(device(&result, "parity-01").containers.len(), 2);
}

#[test]
fn component_boundaries_and_case_are_literal() {
    for path in [
        "/mnt/storage-other",
        "/mnt/Storage",
        "/mnt/STORAGE",
        "/mnt/disks/data-010",
    ] {
        let mut containers = inventory();
        containers.containers = vec![consumer('a', path)];
        assert_eq!(count(&preview(2, &containers)), 0, "{path}");
    }
}

#[test]
fn same_device_branches_preserve_order_without_duplicate_pool_routes() {
    let (contract, mut pools, protection) = inputs(3);
    pools.pools[0].branches = vec!["/mnt/disk2/ZZ".into(), "/mnt/disk2/AA".into()];
    let result =
        configured_storage_dependencies(&contract, &pools, &protection, &inventory(), 100).unwrap();
    let d = device(&result, "d2");
    assert_eq!(d.pools[0].branches, pools.pools[0].branches);
    assert_eq!(d.containers.len(), 1);
    assert!(device(&result, "d1").pools.is_empty());
    assert!(device(&result, "d1").containers.is_empty());
}

#[test]
fn named_volume_identity_and_each_destination_survive_projection() {
    let mut containers = inventory();
    containers.containers.truncate(1);
    let volume = ContainerStorageSource::Volume {
        path: "/mnt/storage/Movies".into(),
        name: "Movies".into(),
        driver: "local".into(),
    };
    containers.containers[0].mounts[0].source = volume.clone();
    let mut second = containers.containers[0].mounts[0].clone();
    second.destination = "/other".into();
    second.writable = false;
    containers.containers[0].mounts.push(second);
    let result = preview(3, &containers);
    let matches = &device(&result, "d1").containers;
    assert_eq!(matches.len(), 2);
    assert!(matches.iter().all(|c| c.source == volume));
    assert_eq!(
        matches
            .iter()
            .map(|c| c.destination.as_str())
            .collect::<Vec<_>>(),
        ["/data", "/other"]
    );
    assert!(matches[0].writable && !matches[1].writable);
}

#[test]
fn generated_source_data_content_and_ordered_parity_paths_are_preserved() {
    let result = preview(3, &inventory());
    let data = &device(&result, "d2").protection[0];
    assert_eq!(
        data.requirements.iter().map(|r| r.kind).collect::<Vec<_>>(),
        [
            ProtectionPathKind::Source,
            ProtectionPathKind::Data,
            ProtectionPathKind::Content
        ]
    );
    for (id, suffix) in [("p1", "parity"), ("p2", "2-parity")] {
        let rows = &device(&result, id).protection[0].requirements;
        assert_eq!(
            rows.iter().map(|r| r.kind).collect::<Vec<_>>(),
            [
                ProtectionPathKind::Source,
                ProtectionPathKind::Content,
                ProtectionPathKind::Parity
            ]
        );
        assert_eq!(
            rows[2].path,
            format!(
                "{}/snapraid.{suffix}",
                device(&result, id).expected.mountpoint
            )
        );
    }
}

#[test]
fn reordered_records_are_deterministic_without_renewing_original_age() {
    let (mut contract, mut pools, mut protection) = inputs(3);
    let mut second = pools.pools[0].clone();
    second.id = "pool2".into();
    second.name = "second".into();
    second.mount_point = "/mnt/otherpool".into();
    second.branches = vec!["/mnt/disk2/Music".into(), "/mnt/disk1/Music".into()];
    pools.pools.push(second);
    let mut containers = inventory();
    containers.containers[0].mounts.push(ContainerStorageMount {
        source: ContainerStorageSource::Bind {
            path: "/mnt/otherpool/Music".into(),
        },
        destination: "/music".into(),
        writable: false,
    });
    let original =
        configured_storage_dependencies(&contract, &pools, &protection, &containers, 100).unwrap();
    contract.devices.reverse();
    pools.pools.reverse();
    protection.drives.reverse();
    containers.containers.reverse();
    for c in &mut containers.containers {
        c.mounts.reverse();
    }
    let reordered =
        configured_storage_dependencies(&contract, &pools, &protection, &containers, 105).unwrap();
    assert_eq!(original, reordered);
    assert_eq!(reordered.containers_observed_at, 100);
    assert_eq!(reordered.engine_id, "synthetic-engine");
}

#[test]
fn invalid_contract_pool_or_protection_refuses_the_entire_preview() {
    let (contract, pools, protection) = inputs(3);
    let mut bad = contract.clone();
    bad.devices[0].filesystem_uuid = bad.devices[1].filesystem_uuid.clone();
    assert!(configured_storage_dependencies(&bad, &pools, &protection, &inventory(), 100).is_err());
    let mut bad = pools.clone();
    bad.pools[0].branches[0] = "/mnt/private-unknown-token".into();
    let errors = configured_storage_dependencies(&contract, &bad, &protection, &inventory(), 100)
        .unwrap_err();
    assert_eq!(errors[0].code, StoragePlanningFindingCode::UnknownIdentity);
    assert!(
        !serde_json::to_string(&errors)
            .unwrap()
            .contains("private-unknown-token")
    );
    for change in 0..3 {
        let mut bad = protection.clone();
        bad.enabled = false;
        match change {
            0 => bad.drives[0].uuid = "unknown".into(),
            1 => bad.drives[0].path += "/subdirectory",
            _ => bad.drives[0].role = crate::SnapraidRole::Parity,
        }
        assert!(
            configured_storage_dependencies(&contract, &pools, &bad, &inventory(), 100).is_err()
        );
    }
}

#[test]
fn disabled_recursive_pools_and_protection_pool_paths_still_refuse() {
    let (contract, pools, protection) = inputs(3);
    let mut bad = pools.clone();
    bad.pools[0].enabled = false;
    bad.pools[0].branches[0] = "/mnt/storage/Movies".into();
    assert!(
        configured_storage_dependencies(&contract, &bad, &protection, &inventory(), 100).is_err()
    );
    let mut bad = protection;
    bad.enabled = false;
    bad.drives[0].path = "/mnt/storage".into();
    let errors =
        configured_storage_dependencies(&contract, &pools, &bad, &inventory(), 100).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.code == StoragePlanningFindingCode::PoolPath)
    );
}

#[test]
fn stale_future_invalid_and_ambiguous_docker_declarations_refuse() {
    let (contract, pools, protection) = inputs(3);
    for at in [99, 106] {
        let errors =
            configured_storage_dependencies(&contract, &pools, &protection, &inventory(), at)
                .unwrap_err();
        assert_eq!(
            errors,
            vec![finding(
                "containers",
                StoragePlanningFindingCode::StaleEvidence
            )]
        );
    }
    for change in 0..4 {
        let mut bad = inventory();
        match change {
            0 => bad.engine_id = "private-secret\n".into(),
            1 => bad.containers.push(bad.containers[0].clone()),
            2 => {
                let duplicate = bad.containers[0].mounts[0].clone();
                bad.containers[0].mounts.push(duplicate);
            }
            _ => {
                bad.containers[0].mounts[0].source = ContainerStorageSource::Bind {
                    path: "/mnt/../secret".into(),
                }
            }
        }
        let errors =
            configured_storage_dependencies(&contract, &pools, &protection, &bad, 100).unwrap_err();
        assert_eq!(
            errors,
            vec![finding(
                "containers",
                StoragePlanningFindingCode::InvalidConfiguration
            )]
        );
    }
}

fn maximum_inventory() -> ContainerStorageInventory {
    let mut value = inventory();
    value.containers = ['a', 'b', 'c', 'd']
        .into_iter()
        .map(|id| {
            let mut c = consumer(id, "/");
            c.mounts = (0..64)
                .map(|i| ContainerStorageMount {
                    source: ContainerStorageSource::Bind { path: "/".into() },
                    destination: format!("/data/{i}"),
                    writable: true,
                })
                .collect();
            c
        })
        .collect();
    value
}

#[test]
fn expansion_accepts_511_and_512_but_never_returns_a_clipped_result() {
    let (mut contract, pools, protection) = inputs(0);
    contract.devices.push(inputs(1).0.devices[2].clone());
    let mut containers = maximum_inventory();
    let result =
        configured_storage_dependencies(&contract, &pools, &protection, &containers, 100).unwrap();
    assert_eq!(count(&result), MAX_CONFIGURED_CONTAINER_MATCHES);
    containers.containers[0].mounts[0].source = ContainerStorageSource::Bind {
        path: "/mnt/storage".into(),
    };
    assert_eq!(
        count(
            &configured_storage_dependencies(&contract, &pools, &protection, &containers, 100)
                .unwrap()
        ),
        511
    );
    contract.devices.push(inputs(1).0.devices[1].clone());
    contract.profile = StorageProfile::SeparateDownloads;
    contract.locations.downloads_host = "/mnt/downloads/Complete".into();
    let errors = configured_storage_dependencies(&contract, &pools, &protection, &containers, 100)
        .unwrap_err();
    assert_eq!(
        errors,
        vec![finding(
            "dependencies.containers",
            StoragePlanningFindingCode::ThresholdExceeded
        )]
    );
}

#[test]
fn pool_amplification_counts_every_route_across_the_whole_request() {
    let (contract, pools, protection) = inputs(2);
    let mut containers = inventory();
    containers.containers.truncate(1);
    containers.containers[0].mounts = (0..17)
        .map(|i| ContainerStorageMount {
            source: ContainerStorageSource::Bind { path: "/".into() },
            destination: format!("/data/{i}"),
            writable: false,
        })
        .collect();
    let result =
        configured_storage_dependencies(&contract, &pools, &protection, &containers, 100).unwrap();
    assert_eq!(count(&result), 493);
    let mut extra = containers.containers[0].mounts[0].clone();
    extra.destination = "/extra".into();
    containers.containers[0].mounts.push(extra);
    let errors = configured_storage_dependencies(&contract, &pools, &protection, &containers, 100)
        .unwrap_err();
    assert_eq!(
        errors[0].code,
        StoragePlanningFindingCode::ThresholdExceeded
    );
}

#[test]
fn inputs_remain_unchanged_and_numeric_media_ownership_is_preserved() {
    let (mut contract, pools, protection) = inputs(3);
    contract.media_identity.uid = 1101;
    contract.media_identity.gid = 1102;
    let containers = inventory();
    let before = (
        contract.clone(),
        pools.clone(),
        protection.clone(),
        containers.clone(),
    );
    let result =
        configured_storage_dependencies(&contract, &pools, &protection, &containers, 100).unwrap();
    assert_eq!(result.media_identity, contract.media_identity);
    assert_eq!(before, (contract, pools, protection, containers));
}
