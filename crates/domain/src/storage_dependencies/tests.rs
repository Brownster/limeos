use super::*;
use serde_json::{Value, json};

fn consumer(id: char, source: &str, running: bool) -> ContainerStorageConsumer {
    ContainerStorageConsumer {
        container: ContainerSnapshot {
            resource: format!("container:{}", id.to_string().repeat(64)),
            image: format!("sha256:{}", "f".repeat(64)),
            started_at: "2026-10-06T00:00:00Z".into(),
            running,
        },
        mounts: vec![ContainerStorageMount {
            source: ContainerStorageSource::Bind {
                path: source.into(),
            },
            destination: "/data".into(),
            writable: true,
        }],
    }
}
fn inventory() -> ContainerStorageInventory {
    ContainerStorageInventory {
        version: 1,
        engine_id: "fixture-engine".into(),
        observed_at: 100,
        containers: vec![consumer('a', "/mnt/storage/Media", true)],
    }
}
fn contract() -> StorageContract {
    let fixtures: Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/storage-contracts.json"
    ))
    .unwrap();
    serde_json::from_value(fixtures["cases"][0]["contract"].clone()).unwrap()
}
#[test]
fn declared_overlap_includes_stopped_ancestors_but_preserves_components_and_case() {
    let mut value = inventory();
    value.containers.extend([
        consumer('b', "/mnt", false),
        consumer('c', "/mnt/storage-other", true),
        consumer('d', "/mnt/Storage", true),
        consumer('e', "/", false),
    ]);
    let dependencies = declared_container_storage_dependencies(&contract(), &value, 100).unwrap();
    let storage = dependencies
        .iter()
        .find(|d| d.mountpoint == "/mnt/storage")
        .unwrap();
    assert_eq!(
        storage.containers,
        ['a', 'b', 'e'].map(|c| format!("container:{}", c.to_string().repeat(64)))
    );
    let duplicate_mount = value.containers[0].mounts[0].clone();
    value.containers[0].mounts.push(duplicate_mount);
    assert!(declared_container_storage_dependencies(&contract(), &value, 100).is_err());
}
#[test]
fn source_paths_reject_traversal_noncanonical_and_control_components() {
    for source in [
        "",
        "relative",
        "/mnt//disk",
        "/mnt/disk/",
        "/mnt/../disk",
        "/mnt/./disk",
        "/mnt/disk\n",
        "/mnt/disk\0",
    ] {
        let mut value = inventory();
        value.containers[0].mounts[0].source = ContainerStorageSource::Bind {
            path: source.into(),
        };
        assert!(value.validate(100).is_err(), "{source:?}");
    }
    for source in [
        "/",
        "/mnt/My Disk",
        "/mnt/storage/Media",
        "/mnt/storage/media",
    ] {
        assert!(container_storage_path(source));
    }
}
#[test]
fn collection_age_identity_and_unknown_fields_fail_closed() {
    let value = inventory();
    assert!(value.validate(105).is_ok());
    assert_eq!(value.validate(106).unwrap_err().0, ErrorCode::Conflict);
    assert_eq!(value.validate(99).unwrap_err().0, ErrorCode::Conflict);
    for (field, bad) in [
        ("version", json!(2)),
        ("observed_at", json!(-1)),
        ("engine_id", json!("")),
        ("engine_id", json!("secret\nvalue")),
    ] {
        let mut raw = serde_json::to_value(&value).unwrap();
        raw[field] = bad;
        assert!(
            serde_json::from_value::<ContainerStorageInventory>(raw)
                .unwrap()
                .validate(100)
                .is_err()
        );
    }
    let mut raw = serde_json::to_value(&value).unwrap();
    raw["complete_physical_dependencies"] = json!(true);
    assert!(serde_json::from_value::<ContainerStorageInventory>(raw).is_err());
    let mut duplicate = value.clone();
    duplicate.containers.push(duplicate.containers[0].clone());
    assert!(duplicate.validate(100).is_err());
}
#[test]
fn named_volumes_preserve_driver_and_require_complete_identity() {
    let mut value = inventory();
    value.containers[0].mounts[0].source = ContainerStorageSource::Volume {
        path: "/var/lib/docker/volumes/data/_data".into(),
        name: "data".into(),
        driver: "local".into(),
    };
    assert!(value.validate(100).is_ok());
    let encoded = serde_json::to_value(&value).unwrap();
    for (field, bad) in [
        ("driver", json!("")),
        ("name", json!("../data")),
        ("path", json!("/var/lib/docker/../secrets")),
    ] {
        let mut raw = encoded.clone();
        raw["containers"][0]["mounts"][0]["source"][field] = bad;
        assert!(
            serde_json::from_value::<ContainerStorageInventory>(raw)
                .unwrap()
                .validate(100)
                .is_err()
        );
    }
}
#[test]
fn aggregate_mount_and_container_limits_do_not_admit_partial_evidence() {
    let mut value = inventory();
    let mounts: Vec<_> = (0..64)
        .map(|i| ContainerStorageMount {
            destination: format!("/data/{i}"),
            ..value.containers[0].mounts[0].clone()
        })
        .collect();
    value.containers = (0..4)
        .map(|i| ContainerStorageConsumer {
            container: ContainerSnapshot {
                resource: format!("container:{i:064x}"),
                ..value.containers[0].container.clone()
            },
            mounts: mounts.clone(),
        })
        .collect();
    assert!(value.validate(100).is_ok());
    value.containers.push(consumer('e', "/mnt/other", false));
    assert!(value.validate(100).is_err());
    value.containers = (0..65)
        .map(|i| ContainerStorageConsumer {
            container: ContainerSnapshot {
                resource: format!("container:{i:064x}"),
                ..value.containers[0].container.clone()
            },
            mounts: vec![],
        })
        .collect();
    assert!(value.validate(100).is_err());
}
