use super::*;
use crate::{
    ContainerSnapshot, ContainerStorageConsumer, ContainerStorageMount, ContainerStorageSource,
    configured_dependencies::{ConfiguredContainerRoute, configured_storage_dependencies},
    shares::{SambaDependencyRoute, SambaShareConfig},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn inputs(i: usize) -> (StorageContract, PoolsConfig, SnapraidConfig) {
    let f = &crate::pools::tests::fixtures()[i];
    (
        serde_json::from_value(f["contract"].clone()).unwrap(),
        serde_json::from_value(f["pools"].clone()).unwrap(),
        serde_json::from_value(f["snapraid"].clone()).unwrap(),
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
fn consumer(path: &str) -> ContainerStorageConsumer {
    ContainerStorageConsumer {
        container: ContainerSnapshot {
            resource: format!("container:{}", "a".repeat(64)),
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
    let mut b = consumer("/mnt/downloads/Complete");
    b.container.resource = format!("container:{}", "b".repeat(64));
    ContainerStorageInventory {
        version: 1,
        engine_id: "synthetic-engine".into(),
        observed_at: 100,
        containers: vec![consumer("/mnt/storage/Movies"), b],
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
fn graph(
    i: usize,
    containers: &ContainerStorageInventory,
    shares: &SambaSharesConfig,
) -> ConfiguredStorageGraphPreview {
    let (c, p, s) = inputs(i);
    configured_storage_dependency_graph(&c, &p, &s, containers, shares, 100).unwrap()
}
fn count(g: &ConfiguredStorageGraphPreview) -> usize {
    g.devices
        .iter()
        .map(|d| d.storage.containers.len() + d.shares.len())
        .sum()
}
fn share_rows(g: &ConfiguredStorageGraphPreview) -> BTreeMap<String, Vec<String>> {
    g.devices
        .iter()
        .filter(|d| !d.shares.is_empty())
        .map(|d| {
            (
                d.storage.expected.id.clone(),
                d.shares
                    .iter()
                    .map(|s| {
                        format!(
                            "{}:{}",
                            s.share_name,
                            match &s.via {
                                SambaDependencyRoute::Direct => "direct",
                                SambaDependencyRoute::Pool { pool_id, .. } => pool_id,
                            }
                        )
                    })
                    .collect(),
            )
        })
        .collect()
}

#[test]
fn four_existing_layouts_match_independent_combined_goldens() {
    let expected: Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/storage-dependency-graph/expected.json"
    ))
    .unwrap();
    for (i, layout) in crate::pools::tests::fixtures().iter().enumerate() {
        let g = graph(i, &inventory(), &shares());
        assert_eq!(
            json!({"consumer_matches":count(&g),"shares":share_rows(&g),"unmapped":g.unmapped_shares}),
            expected[layout["name"].as_str().unwrap()]
        );
        assert_eq!(g.share_configuration, shares());
        assert_eq!(g.media_identity, inputs(i).0.media_identity);
    }
}

#[test]
fn existing_storage_rows_are_reused_without_format_or_identity_changes() {
    for i in 0..4 {
        let (c, p, s) = inputs(i);
        let containers = inventory();
        let before = configured_storage_dependencies(&c, &p, &s, &containers, 100).unwrap();
        let g =
            configured_storage_dependency_graph(&c, &p, &s, &containers, &shares(), 100).unwrap();
        assert_eq!(
            g.devices
                .iter()
                .map(|d| d.storage.clone())
                .collect::<Vec<_>>(),
            before.devices
        );
        assert_eq!(g.engine_id, before.engine_id);
        assert_eq!(g.containers_observed_at, before.containers_observed_at);
        assert_eq!(g.media_identity, before.media_identity);
    }
}

#[test]
fn disabled_stopped_read_only_consumers_and_protection_remain_visible() {
    let (c, mut p, mut s) = inputs(2);
    p.pools[0].enabled = false;
    s.enabled = false;
    let mut docker = inventory();
    docker.containers[0].container.running = false;
    docker.containers[0].mounts[0].writable = false;
    let mut smb = shares();
    smb.shares[0].enabled = false;
    smb.shares[0].read_only = true;
    let g = configured_storage_dependency_graph(&c, &p, &s, &docker, &smb, 100).unwrap();
    assert_eq!(count(&g), 28);
    assert_eq!(g.share_configuration, smb);
    for d in g
        .devices
        .iter()
        .filter(|d| d.storage.expected.id.starts_with("data-"))
    {
        assert!(!d.storage.containers[0].container.running && !d.storage.containers[0].writable);
        assert!(!d.storage.pools[0].enabled && !d.storage.protection[0].enabled);
        assert!(matches!(
            d.shares[0].via,
            SambaDependencyRoute::Pool { enabled: false, .. }
        ));
    }
}

#[test]
fn ancestor_routes_include_both_direct_and_pool_consumers_without_deduplicating_routes() {
    let docker = ContainerStorageInventory {
        containers: vec![consumer("/mnt")],
        ..inventory()
    };
    let smb = SambaSharesConfig {
        shares: vec![share("all", "/mnt")],
    };
    let g = graph(2, &docker, &smb);
    assert_eq!(count(&g), 58);
    let d = &g
        .devices
        .iter()
        .find(|d| d.storage.expected.id == "data-01")
        .unwrap();
    assert_eq!(d.shares.len(), 2);
    assert!(matches!(d.shares[0].via, SambaDependencyRoute::Direct));
    assert!(matches!(d.shares[1].via, SambaDependencyRoute::Pool { .. }));
    assert_eq!(d.storage.containers.len(), 2);
    assert!(matches!(
        d.storage.containers[0].via,
        ConfiguredContainerRoute::Direct
    ));
}

#[test]
fn ordered_branches_complete_device_metadata_and_numeric_ownership_survive() {
    let (mut c, mut p, s) = inputs(3);
    c.media_identity.uid = 1234;
    c.media_identity.gid = 4321;
    for device in &mut c.devices {
        device.serial = Some(format!("SERIAL-{}", device.id));
    }
    p.pools[0].branches = vec![
        "/mnt/disk2/Z".into(),
        "/mnt/disk1/B".into(),
        "/mnt/disk1/A".into(),
    ];
    let g = configured_storage_dependency_graph(&c, &p, &s, &inventory(), &shares(), 100).unwrap();
    assert_eq!(g.media_identity, c.media_identity);
    for d in &g.devices {
        assert!(c.devices.contains(&d.storage.expected));
    }
    let d = &g
        .devices
        .iter()
        .find(|d| d.storage.expected.id == "d1")
        .unwrap();
    assert_eq!(
        d.storage.pools[0].branches,
        ["/mnt/disk1/B", "/mnt/disk1/A"]
    );
    assert!(
        matches!(&d.shares[0].via,SambaDependencyRoute::Pool{branches,..} if branches==&["/mnt/disk1/B","/mnt/disk1/A"])
    );
    assert!(
        d.storage.protection[0]
            .requirements
            .iter()
            .all(|r| r.expected == d.storage.expected)
    );
}

#[test]
fn original_timestamp_and_named_volume_identity_are_not_renewed_or_rewritten() {
    let (c, p, s) = inputs(2);
    let mut docker = inventory();
    docker.observed_at = 98;
    docker.containers[0].mounts[0].source = ContainerStorageSource::Volume {
        path: "/mnt/storage/Movies".into(),
        name: "existing-volume".into(),
        driver: "local".into(),
    };
    let g = configured_storage_dependency_graph(&c, &p, &s, &docker, &shares(), 100).unwrap();
    assert_eq!(g.containers_observed_at, 98);
    let consumer = &g.devices[0].storage.containers[0];
    assert_eq!(consumer.source, docker.containers[0].mounts[0].source);
    assert_eq!(consumer.destination, "/data");
    assert!(configured_storage_dependency_graph(&c, &p, &s, &docker, &shares(), 104).is_err());
    assert!(configured_storage_dependency_graph(&c, &p, &s, &docker, &shares(), 97).is_err());
}

#[test]
fn share_flags_users_and_comments_are_stored_once_with_exact_name_references() {
    let mut smb = shares();
    smb.shares[0].comment = "Movies & TV".into();
    smb.shares[0].valid_users = "pi, media".into();
    smb.shares[0].guest_ok = true;
    smb.shares[0].browseable = false;
    let g = graph(2, &inventory(), &smb);
    assert_eq!(g.share_configuration, smb);
    let encoded = serde_json::to_string(&g).unwrap();
    assert_eq!(encoded.matches("Movies & TV").count(), 1);
    assert_eq!(encoded.matches("pi, media").count(), 1);
    for d in &g.devices {
        for consumer in &d.shares {
            assert_eq!(
                g.share_configuration
                    .shares
                    .iter()
                    .filter(|s| s.name == consumer.share_name)
                    .count(),
                1
            );
        }
    }
}

#[test]
fn literal_case_component_siblings_and_unmapped_paths_remain_explicit() {
    let smb = SambaSharesConfig {
        shares: vec![
            share("Case", "/mnt/Storage/Movies"),
            share("Sibling", "/mnt/storage-other"),
            share("Docs", "/srv/docs"),
        ],
    };
    let g = graph(2, &inventory(), &smb);
    assert!(g.devices.iter().all(|d| d.shares.is_empty()));
    assert_eq!(g.unmapped_shares, ["Case", "Sibling", "Docs"]);
    assert_eq!(g.share_configuration, smb);
    assert_eq!(count(&g), 14);
}

#[test]
fn enumeration_reordering_keeps_device_graph_stable_while_share_order_is_preserved() {
    let (mut c, p, s) = inputs(3);
    let mut docker = inventory();
    let mut smb = shares();
    let original = configured_storage_dependency_graph(&c, &p, &s, &docker, &smb, 100).unwrap();
    c.devices.reverse();
    docker.containers.reverse();
    assert_eq!(
        configured_storage_dependency_graph(&c, &p, &s, &docker, &smb, 100).unwrap(),
        original
    );
    smb.shares.reverse();
    let reordered = configured_storage_dependency_graph(&c, &p, &s, &docker, &smb, 100).unwrap();
    assert_eq!(reordered.devices, original.devices);
    assert_eq!(reordered.share_configuration, smb);
    assert_ne!(reordered.share_configuration, original.share_configuration);
}

#[test]
fn every_invalid_input_refuses_the_composition_without_value_leaks() {
    let (c, p, s) = inputs(2);
    let docker = inventory();
    let smb = shares();
    for mutation in 0..6 {
        let (mut c, mut p, mut s, mut docker, mut smb) =
            (c.clone(), p.clone(), s.clone(), docker.clone(), smb.clone());
        match mutation {
            0 => c.media_identity.uid = 0,
            1 => p.pools[0].branches[0] = "/mnt/missing".into(),
            2 => s.drives[0].uuid = "unknown".into(),
            3 => docker.observed_at = 94,
            4 => smb.shares[0].path = "/mnt/private\n[global]".into(),
            _ => smb.shares[0].valid_users = "password=private-value".into(),
        }
        let errors =
            configured_storage_dependency_graph(&c, &p, &s, &docker, &smb, 100).unwrap_err();
        assert!(!format!("{errors:?}").contains("private-value"));
    }
}

#[test]
fn mixed_511_512_and_513_matches_apply_one_global_limit_to_individually_valid_previews() {
    let (c, p, s) = inputs(2);
    let mut docker = ContainerStorageInventory {
        containers: vec![consumer("/mnt/storage")],
        ..inventory()
    };
    docker.containers[0].mounts = (0..35)
        .map(|i| ContainerStorageMount {
            source: ContainerStorageSource::Bind {
                path: "/mnt/storage".into(),
            },
            destination: format!("/d{i}"),
            writable: true,
        })
        .collect(); // 490
    let mut smb = SambaSharesConfig {
        shares: vec![share("pool", "/mnt/storage")],
    }; // 14 more
    for i in 0..7 {
        smb.shares
            .push(share(&format!("direct{i}"), "/mnt/disks/data-01"));
    }
    assert_eq!(
        count(&configured_storage_dependency_graph(&c, &p, &s, &docker, &smb, 100).unwrap()),
        511
    );
    smb.shares.push(share("exact", "/mnt/disks/data-01"));
    assert_eq!(
        count(&configured_storage_dependency_graph(&c, &p, &s, &docker, &smb, 100).unwrap()),
        512
    );
    smb.shares.push(share("over", "/mnt/disks/data-01"));
    assert!(smb.plan(&c, &p).is_ok());
    assert!(configured_storage_dependencies(&c, &p, &s, &docker, 100).is_ok());
    let errors = configured_storage_dependency_graph(&c, &p, &s, &docker, &smb, 100).unwrap_err();
    assert_eq!(errors[0].field, "dependencies.consumers");
    assert_eq!(
        errors[0].code,
        StoragePlanningFindingCode::ThresholdExceeded
    );
}

#[test]
fn empty_share_input_keeps_prior_rows_but_never_skips_validation_or_mutates_inputs() {
    let (c, p, s) = inputs(2);
    let docker = inventory();
    let smb = SambaSharesConfig::default();
    let before = (c.clone(), p.clone(), s.clone(), docker.clone(), smb.clone());
    let g = configured_storage_dependency_graph(&c, &p, &s, &docker, &smb, 100).unwrap();
    assert_eq!(count(&g), 14);
    assert!(g.unmapped_shares.is_empty());
    assert_eq!(
        (c.clone(), p.clone(), s.clone(), docker.clone(), smb.clone()),
        before
    );
    let mut invalid = c;
    invalid.media_identity.uid = 0;
    assert!(configured_storage_dependency_graph(&invalid, &p, &s, &docker, &smb, 100).is_err());
}
