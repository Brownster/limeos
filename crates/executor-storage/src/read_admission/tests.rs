use super::*;
use crate::{
    mounts,
    topology::{DeviceNumber, Node},
};
use limeos_domain::{
    ContainerSnapshot, ContainerStorageConsumer, ContainerStorageMount, ContainerStorageSource,
};
use std::collections::{BTreeMap, BTreeSet};

const PROFILE: StorageReadProfile = StorageReadProfile::CombinedV1;
fn inventory(count: usize, mounts: usize) -> ContainerStorageInventory {
    ContainerStorageInventory {
        version: 1,
        engine_id: "fixture".into(),
        observed_at: 100,
        containers: (0..count)
            .map(|i| ContainerStorageConsumer {
                container: ContainerSnapshot {
                    resource: format!("container:{i:064x}"),
                    image: format!("sha256:{i:064x}"),
                    running: i < 8,
                    started_at: "2026-10-13T00:00:00Z".into(),
                },
                mounts: (0..mounts)
                    .map(|j| ContainerStorageMount {
                        source: ContainerStorageSource::Bind {
                            path: "/data".into(),
                        },
                        destination: format!("/data/{j}"),
                        writable: true,
                    })
                    .collect(),
            })
            .collect(),
    }
}
fn topology(count: usize) -> Topology {
    Topology(
        (0..count)
            .map(|i| {
                (
                    DeviceNumber {
                        major: 8,
                        minor: i as u32,
                    },
                    Node {
                        name: format!("sd{i}"),
                        parents: BTreeSet::new(),
                        serial: None,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>(),
    )
}
fn rows(count: usize) -> String {
    (1..=count)
        .map(|i| format!("{i} 0 8:1 / / rw - ext4 x rw\n"))
        .collect()
}

#[test]
fn complete_membership_counts_stopped_mounts_and_mountless_running_consumers() {
    assert_eq!(PROFILE.admit_sources(&inventory(16, 2)), Ok(32));
    assert_eq!(
        PROFILE.admit_sources(&inventory(16, 3)),
        Err(Failure::Unavailable)
    );
    assert_eq!(
        PROFILE.admit_sources(&inventory(17, 0)),
        Err(Failure::Unavailable)
    );
    let mut d = inventory(9, 0);
    d.containers[8].container.running = true;
    assert_eq!(PROFILE.admit_sources(&d), Err(Failure::Unavailable));
    assert_eq!(StorageReadProfile::Standard.admit_sources(&d), Ok(0));
    assert_eq!(
        StorageReadProfile::Standard.admit_sources(&inventory(65, 0)),
        Err(Failure::InvalidPlan)
    );
}

#[test]
fn probe_preflight_counts_all_nonvirtual_nodes_without_opening_devices() {
    let mut t = topology(48);
    for (i, node) in t.0.values_mut().enumerate().skip(40) {
        node.name = format!("loop{i}");
    }
    assert_eq!(PROFILE.admit_probes(&t), Ok(40));
    t.0.values_mut().last().unwrap().name = "sda1".into();
    assert_eq!(PROFILE.admit_probes(&t), Err(Failure::Unavailable));
    assert_eq!(StorageReadProfile::Standard.admit_probes(&t), Ok(41));
    // Synthetic 15 parent/partition pairs plus three boot nodes. No host layout
    // or block device was inspected by this admission fixture.
    assert_eq!(PROFILE.admit_probes(&topology(33)), Ok(33));
}

#[test]
fn host_table_rows_and_raw_bytes_accept_exact_then_refuse_excess() {
    let table = mounts::parse_with_profile(&rows(512), PROFILE).unwrap();
    assert_eq!(table.len(), 512);
    let payload = table.capacity() * size_of::<mounts::Mount>()
        + table
            .iter()
            .map(|m| m.root.capacity() + m.path.capacity() + m.filesystem.capacity())
            .sum::<usize>();
    assert!(payload <= 128 * 1024);
    assert!(mounts::parse_with_profile(&rows(513), PROFILE).is_err());
    let base = "1 0 8:1 / / rw - ext4  rw\n";
    let exact = base.replace(
        "ext4  ",
        &format!("ext4 {} ", "x".repeat(65536 - base.len())),
    );
    assert_eq!(exact.len(), 65536);
    assert!(mounts::parse_with_profile(&exact, PROFILE).is_ok());
    assert!(mounts::parse_with_profile(&exact.replace("ext4 x", "ext4 xx"), PROFILE).is_err());
    assert!(mounts::parse_with_profile(rows(512).trim_end(), PROFILE).is_err());
}

#[test]
fn decoded_fields_and_fixed_tokens_refuse_whole_mount_tables() {
    let raw = format!(
        "1 0 8:1 / /{} rw - {} x rw\n",
        "\\040".repeat(511),
        "x".repeat(64)
    );
    assert_eq!(
        mounts::parse_with_profile(&raw, PROFILE).unwrap()[0]
            .path
            .len(),
        512
    );
    for raw in [
        raw.replace(" rw - ", "\\040 rw - "),
        raw.replace(" x rw", "x x rw"),
        "1 0 8:1 / / ro - ext4 x invalid\n".into(),
        format!("1 0 8:1 / / rw {} - ext4 x rw\n", vec!["a"; 33].join(" ")),
        "1 0 8:1 / / rw - ext4 x rw extra\n".into(),
    ] {
        assert!(mounts::parse_with_profile(&raw, PROFILE).is_err());
    }
    assert!(
        mounts::parse_with_profile(
            &format!("1 0 8:1 / / rw {} - ext4 x rw\n", vec!["a"; 32].join(" ")),
            PROFILE
        )
        .is_ok()
    );
    assert!(mounts::fixed_fields::<6>(&"a ".repeat(20000), false).is_err());
}

#[test]
fn field_decoding_counts_bytes_and_checks_escapes_before_copying() {
    assert_eq!(
        decode_path(&format!("/{}x", "é".repeat(255)), 512)
            .unwrap()
            .len(),
        512
    );
    assert_eq!(
        decode_path(&format!("/{}", "é".repeat(256)), 512),
        Err(Failure::Unavailable)
    );
    for raw in ["/bad\\041", "/bad\\"] {
        assert_eq!(decode_path(raw, 512), Err(Failure::Unavailable));
    }
    assert_eq!(copy_text("abcd", 3), Err(Failure::Unavailable));
}

#[test]
fn bounded_json_counts_escaped_output_without_building_an_oversized_body() {
    assert_eq!(json_limit(&"abc", 5), Ok(()));
    assert_eq!(json_limit(&"abc", 4), Err(Failure::Unavailable));
    assert_eq!(json_limit(&"\n", 4), Ok(()));
    assert_eq!(json_limit(&"\n", 3), Err(Failure::Unavailable));
    let value = inventory(16, 2);
    assert_eq!(
        PROFILE.digest(&value).unwrap(),
        StorageReadProfile::Standard.digest(&value).unwrap()
    );
}

#[test]
fn bounded_file_reader_checks_sentinel_before_extending_the_output() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("table");
    std::fs::write(&file, vec![b'a'; 65536]).unwrap();
    assert_eq!(PROFILE.read(&file, 65536).unwrap().len(), 65536);
    std::fs::write(&file, vec![b'a'; 65537]).unwrap();
    assert_eq!(PROFILE.read(&file, 65536), Err(Failure::Unavailable));
}

#[test]
fn standard_mount_parser_preserves_legacy_large_tables_and_decoded_fields() {
    assert!(mounts::parse_with_profile(&rows(513), StorageReadProfile::Standard).is_ok());
    let raw = format!("1 0 8:1 / /{} rw - ext4 x rw\n", "x".repeat(512));
    assert!(mounts::parse(&raw).is_ok());
    assert!(mounts::parse_with_profile(&raw, PROFILE).is_err());
}

#[tokio::test]
async fn public_dependency_admission_rejects_excess_membership_before_host_io() {
    let d = inventory(17, 0);
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/storage-contracts.json"
    ))
    .unwrap();
    let contract: limeos_domain::StorageContract =
        serde_json::from_value(fixtures["cases"][0]["contract"].clone()).unwrap();
    contract.validate().unwrap();
    assert!(matches!(
        crate::inspect_container_dependencies_with_profile(&contract, &d, PROFILE).await,
        Err(Failure::Unavailable)
    ));
    assert!(matches!(
        crate::inspect_container_sources_with_profile(&d, PROFILE),
        Err(Failure::Unavailable)
    ));
}
