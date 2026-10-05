use super::*;
use serde_json::{Value, json};

fn golden() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!(
        "../../../../tests/fixtures/storage-contracts.json"
    ))
    .unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}

fn parse(value: Value) -> Result<StorageContract> {
    let contract: StorageContract =
        serde_json::from_value(value).map_err(|_| Error(ErrorCode::InvalidInput))?;
    contract.validate()?;
    Ok(contract)
}

#[test]
fn frozen_contracts_round_trip_without_path_or_ownership_changes() {
    for case in golden() {
        let original = case["contract"].clone();
        let contract = parse(original.clone()).unwrap();
        assert_eq!(serde_json::to_value(&contract).unwrap(), original);
        assert_eq!(
            contract.media_identity,
            StorageMediaIdentity {
                uid: 1000,
                gid: 1000
            }
        );
        let plan = contract.mount_wait_plan(30).unwrap();
        assert_eq!(
            plan.devices
                .iter()
                .map(|d| d.mountpoint.clone())
                .collect::<Vec<_>>(),
            serde_json::from_value::<Vec<String>>(case["wait_mountpoints"].clone()).unwrap()
        );
        assert_eq!(plan.timeout_seconds, 30);
    }
}

#[test]
fn waits_require_device_roots_and_a_bounded_deadline() {
    let cases = golden();
    let single = parse(cases[0]["contract"].clone()).unwrap();
    assert_eq!(
        single.mount_wait_plan(1).unwrap().devices[0].mountpoint,
        "/mnt/storage"
    );
    assert!(single.mount_wait_plan(0).is_err());
    assert!(single.mount_wait_plan(121).is_err());
    assert!(single.mount_wait_plan(u16::MAX).is_err());
    assert_eq!(single.mount_wait_plan(120).unwrap().timeout_seconds, 120);
    let pool = parse(cases[2]["contract"].clone()).unwrap();
    let waits = pool.mount_wait_plan(30).unwrap();
    assert_eq!(waits.devices.len(), 15);
    assert!(!waits.devices.iter().any(|d| d.mountpoint == "/mnt/storage"
        || d.mountpoint == pool.locations.media_host
        || d.mountpoint == pool.locations.downloads_host));
    let mut unsafe_pool = pool.clone();
    unsafe_pool.devices[0].mountpoint = "/mnt/storage".into();
    assert!(unsafe_pool.mount_wait_plan(30).is_err());
    unsafe_pool.devices[0].mountpoint = "/mnt/storage/data".into();
    assert!(unsafe_pool.mount_wait_plan(30).is_err());
    unsafe_pool.devices[0].mountpoint = "/mnt/storage-extra".into();
    assert!(unsafe_pool.mount_wait_plan(30).is_ok());
}

#[test]
fn executor_wait_input_is_validated_without_trusting_its_generator() {
    let original = parse(golden()[0]["contract"].clone())
        .unwrap()
        .mount_wait_plan(3)
        .unwrap();
    for (field, bad) in [
        ("id", json!("ROOT")),
        ("id", json!("../disk")),
        ("filesystem_uuid", json!("$(command)")),
        ("filesystem_uuid", json!("")),
        ("mountpoint", json!("/mnt/data/../boot")),
        ("mountpoint", json!("/boot")),
        ("serial", json!(" padded ")),
    ] {
        let mut value = serde_json::to_value(&original).unwrap();
        value["devices"][0][field] = bad;
        assert!(
            serde_json::from_value::<StorageMountWaitPlan>(value)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    let mut duplicate = original.clone();
    duplicate.devices.push(duplicate.devices[0].clone());
    assert!(duplicate.validate().is_err());
    assert!(
        StorageMountWaitPlan {
            devices: Vec::new(),
            ..original.clone()
        }
        .validate()
        .is_err()
    );
    assert!(
        StorageMountWaitPlan {
            timeout_seconds: 121,
            ..original
        }
        .validate()
        .is_err()
    );
}

#[test]
fn ownership_and_fixed_container_paths_cannot_change_implicitly() {
    let original = golden()[0]["contract"].clone();
    for bad in [
        json!(0),
        json!(-1),
        json!(true),
        json!(1.5),
        json!(2147483648u32),
    ] {
        for field in ["uid", "gid"] {
            let mut value = original.clone();
            value["media_identity"][field] = bad.clone();
            assert!(parse(value).is_err());
        }
    }
    for field in ["media_container", "downloads_container", "config_container"] {
        let mut value = original.clone();
        value["locations"][field] = json!("/replacement");
        assert!(parse(value).is_err());
    }
}

#[test]
fn closed_schema_rejects_version_changes_and_kernel_path_authority() {
    let original = golden()[0]["contract"].clone();
    for field in ["schema_version", "profile"] {
        let mut value = original.clone();
        value[field] = json!("unsupported");
        assert!(parse(value).is_err());
    }
    let mut value = original.clone();
    value["devices"][0]["kernel_name"] = json!("/dev/sda1");
    assert!(parse(value).is_err());
    let mut value = original.clone();
    value["locations"]["mount_options"] = json!("exec,suid");
    assert!(parse(value).is_err());
    let mut value = original;
    value["media_identity"]["username"] = json!("root");
    assert!(parse(value).is_err());
}

#[test]
fn duplicate_and_ambiguous_assignments_fail_closed() {
    let original = golden()[1]["contract"].clone();
    for field in ["id", "filesystem_uuid", "mountpoint"] {
        let mut value = original.clone();
        value["devices"][1][field] = value["devices"][0][field].clone();
        assert!(parse(value).is_err());
    }
    let mut value = original.clone();
    value["devices"][1]["role"] = json!("data");
    assert!(parse(value).is_err());
    let mut value = original.clone();
    let mut second_backup = value["devices"][2].clone();
    second_backup["id"] = json!("backup-b");
    second_backup["filesystem_uuid"] = json!("uuid-backup-b");
    second_backup["mountpoint"] = json!("/mnt/backup-b");
    value["devices"].as_array_mut().unwrap().push(second_backup);
    assert!(parse(value).is_err());
    let mut value = original;
    value["devices"] = json!([]);
    assert!(parse(value).is_err());
    let mut pool = parse(golden()[2]["contract"].clone()).unwrap();
    pool.devices.pop();
    assert!(pool.validate().is_err());
    let mut crowded = parse(golden()[2]["contract"].clone()).unwrap();
    while crowded.devices.len() <= STORAGE_MAX_DEVICES {
        let index = crowded.devices.len();
        crowded.devices.push(StorageDevice {
            id: format!("data-{index}"),
            role: StorageRole::Data,
            filesystem_uuid: format!("uuid-{index}"),
            filesystem: StorageFilesystem::Ext4,
            mountpoint: format!("/mnt/disks/extra-{index}"),
            serial: None,
        });
    }
    assert!(crowded.validate().is_err());
}

#[test]
fn canonical_paths_use_components_and_preserve_literal_case() {
    let original = golden()[0]["contract"].clone();
    for bad in [
        "/",
        "relative",
        "/mnt/storage/../etc",
        "/mnt/storage//media",
        "/mnt/storage/./media",
        "/mnt/storage/media/",
        "/mnt/storage-extra/media",
        "/mnt/storage/media\n",
        "/mnt/storage/\0media",
    ] {
        let mut value = original.clone();
        value["locations"]["media_host"] = json!(bad);
        assert!(parse(value).is_err());
    }
    let mut value = original.clone();
    value["locations"]["media_host"] = json!("/mnt/storage/TV and Movies");
    assert_eq!(
        parse(value).unwrap().locations.media_host,
        "/mnt/storage/TV and Movies"
    );
    let mut value = original;
    value["locations"]["media_host"] = json!(format!("/mnt/storage/{}", "x".repeat(513)));
    assert!(parse(value).is_err());
}

#[test]
fn serial_uuid_and_filesystem_rules_preserve_the_frozen_supported_set() {
    let original = golden()[0]["contract"].clone();
    for fs in [
        "btrfs", "exfat", "ext2", "ext3", "ext4", "ntfs", "vfat", "xfs",
    ] {
        let mut value = original.clone();
        value["devices"][0]["filesystem"] = json!(fs);
        assert!(parse(value).is_ok());
    }
    for (field, bad) in [
        ("serial", " leading"),
        ("serial", "line\nfeed"),
        ("filesystem_uuid", "-not-a-uuid"),
        ("filesystem_uuid", "uuid;command"),
        ("id", "UPPER"),
        ("filesystem", "zfs"),
    ] {
        let mut value = original.clone();
        value["devices"][0][field] = json!(bad);
        assert!(parse(value).is_err());
    }
    let mut value = original;
    value["devices"][0]["serial"] = json!("DAS-3-BAY-5");
    assert_eq!(
        parse(value).unwrap().devices[0].serial.as_deref(),
        Some("DAS-3-BAY-5")
    );
}

#[test]
fn role_specific_locations_cannot_escape_their_assigned_filesystem() {
    let mut separate = golden()[1]["contract"].clone();
    separate["locations"]["downloads_host"] = json!("/mnt/storage/downloads");
    assert!(parse(separate).is_err());
    let mut backup = golden()[1]["contract"].clone();
    backup["locations"]["backup_host"] = json!("/mnt/backup-other/limeos");
    assert!(parse(backup).is_err());
    let mut pool = golden()[2]["contract"].clone();
    pool["locations"]["media_host"] = json!("/mnt/disks/data-01/media");
    assert!(parse(pool).is_err());
}
