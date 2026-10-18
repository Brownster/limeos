use super::*;
use crate::pools::tests::{fixtures, protected};
use crate::protection::tests::evidence;

#[test]
fn frozen_layouts_retain_rendering_order_complete_sources_and_original_age() {
    let mut accepted = 0;
    for case in fixtures() {
        let config: SnapraidConfig = serde_json::from_value(case["snapraid"].clone()).unwrap();
        let pools: PoolsConfig = serde_json::from_value(case["pools"].clone()).unwrap();
        let contract: StorageContract = serde_json::from_value(case["contract"].clone()).unwrap();
        let sources = evidence(&config, &pools, &contract);
        let result = plan_snapraid_scrub(
            &config,
            &pools,
            &contract,
            &sources,
            Default::default(),
            102,
        );
        if !config.enabled {
            assert!(result.is_err());
            continue;
        }
        let preview = result.unwrap();
        accepted += 1;
        assert_eq!(preview.configuration.configuration, case["snapraid_text"]);
        assert_eq!(preview.configuration.desired, config);
        assert_eq!(preview.expected_sources, sources);
        assert_eq!(preview.expected_sources.observed_at, 100);
        assert_eq!(
            preview.configuration.media_identity,
            contract.media_identity
        );
        let json = serde_json::to_vec(&preview).unwrap();
        assert_eq!(
            serde_json::from_slice::<SnapraidScrubPreview>(&json).unwrap(),
            preview
        );
    }
    assert_eq!(accepted, 2);
}

#[test]
fn manual_options_preserve_omission_zero_and_supported_numeric_boundaries() {
    let (contract, pools, config) = protected();
    let sources = evidence(&config, &pools, &contract);
    for percent in [None, Some(0), Some(8), Some(12), Some(100)] {
        for age_days in [None, Some(0), Some(10), Some(u16::MAX)] {
            let request = SnapraidScrubRequest { percent, age_days };
            let preview =
                plan_snapraid_scrub(&config, &pools, &contract, &sources, request, 102).unwrap();
            assert_eq!(preview.request, request);
            assert_eq!(
                serde_json::from_value::<SnapraidScrubRequest>(
                    serde_json::to_value(request).unwrap()
                )
                .unwrap(),
                request
            );
        }
    }
    assert_eq!(
        serde_json::from_str::<SnapraidScrubRequest>("{}").unwrap(),
        SnapraidScrubRequest::default()
    );
}

#[test]
fn configured_and_manual_percent_limits_and_defaults_remain_distinct() {
    let (contract, pools, mut config) = protected();
    config.scrub.percent = 37;
    config.scrub.age_days = 23;
    let sources = evidence(&config, &pools, &contract);
    let default = plan_snapraid_scrub(
        &config,
        &pools,
        &contract,
        &sources,
        Default::default(),
        102,
    )
    .unwrap();
    assert_eq!(default.request.percent, None);
    assert_eq!(default.request.age_days, None);
    let zero = SnapraidScrubRequest {
        percent: Some(0),
        age_days: Some(0),
    };
    assert_eq!(
        plan_snapraid_scrub(&config, &pools, &contract, &sources, zero, 102)
            .unwrap()
            .request,
        zero
    );
    config.scrub.percent = 0;
    assert!(config.validate(&pools).is_err());
    assert!(plan_snapraid_scrub(&config, &pools, &contract, &sources, zero, 102).is_err());
}

#[test]
fn manual_numeric_schema_refuses_overflow_fraction_strings_and_free_options() {
    for json in [
        r#"{"percent":-1}"#,
        r#"{"percent":256}"#,
        r#"{"percent":8.5}"#,
        r#"{"percent":"full"}"#,
        r#"{"percent":"bad"}"#,
        r#"{"percent":"new"}"#,
        r#"{"age_days":-1}"#,
        r#"{"age_days":65536}"#,
        r#"{"age_days":1.5}"#,
        r#"{"age_days":"10"}"#,
        r#"{"force":true}"#,
        r#"{"conf_path":"/etc/foreign"}"#,
        r#"{"args":["--force-empty"]}"#,
        r#"{"log_target":">/tmp/file"}"#,
    ] {
        assert!(
            serde_json::from_str::<SnapraidScrubRequest>(json).is_err(),
            "{json}"
        );
    }
    for percent in 101..=u8::MAX {
        let request = SnapraidScrubRequest {
            percent: Some(percent),
            age_days: None,
        };
        assert_eq!(request.validate().unwrap_err()[0].field, "request.percent");
    }
}

#[test]
fn explicit_manual_request_does_not_enable_or_depend_on_recurring_policy() {
    let (contract, pools, mut config) = protected();
    config.scrub.enabled = false;
    config.schedule.scrub_enabled = false;
    config.schedule.sync_enabled = false;
    let sources = evidence(&config, &pools, &contract);
    let preview = plan_snapraid_scrub(
        &config,
        &pools,
        &contract,
        &sources,
        Default::default(),
        102,
    )
    .unwrap();
    assert_eq!(preview.configuration.desired, config);
    config.enabled = false;
    assert_eq!(
        plan_snapraid_scrub(
            &config,
            &pools,
            &contract,
            &sources,
            Default::default(),
            102
        )
        .unwrap_err()[0]
            .field,
        "enabled"
    );
}

#[test]
fn every_path_role_refuses_missing_unsafe_or_incorrect_object_types() {
    let (contract, pools, config) = protected();
    let sources = evidence(&config, &pools, &contract);
    for index in 0..sources.paths.len() {
        for state in [
            ProtectionPathState::Unavailable,
            ProtectionPathState::Unsafe,
            ProtectionPathState::AbsentFile,
        ] {
            let mut bad = sources.clone();
            bad.paths[index].state = state;
            assert!(
                plan_snapraid_scrub(&config, &pools, &contract, &bad, Default::default(), 102)
                    .is_err(),
                "{index}: {state:?}"
            );
        }
        let mut bad = sources.clone();
        bad.paths[index].state = if bad.paths[index].state == ProtectionPathState::Directory {
            ProtectionPathState::RegularFile
        } else {
            ProtectionPathState::Directory
        };
        assert!(
            plan_snapraid_scrub(&config, &pools, &contract, &bad, Default::default(), 102).is_err()
        );
    }
}

#[test]
fn sync_may_create_absent_files_while_scrub_refuses_each_absent_input() {
    let (contract, pools, config) = protected();
    let sources = evidence(&config, &pools, &contract);
    let requirements = config.plan(&pools, &contract).unwrap().requirements;
    for (index, requirement) in requirements.iter().enumerate() {
        if !matches!(
            requirement.kind,
            ProtectionPathKind::Content | ProtectionPathKind::Parity
        ) {
            continue;
        }
        let mut absent = sources.clone();
        absent.paths[index].state = ProtectionPathState::AbsentFile;
        let diff = SnapraidDiffEvidence {
            observed_at: 101,
            configuration: config.clone(),
            topology_digest: absent.inventory.topology_digest.clone(),
            mounts_digest: absent.inventory.mounts_digest.clone(),
            outcome: SnapraidDiffOutcome::Parsed(SnapraidDiff {
                added: 0,
                removed: 0,
                updated: 0,
                moved: 0,
                copied: 0,
                restored: 0,
            }),
        };
        assert!(plan_snapraid_sync(&config, &pools, &contract, &absent, &diff, 102).is_ok());
        assert_eq!(
            plan_snapraid_scrub(&config, &pools, &contract, &absent, Default::default(), 102)
                .unwrap_err()[0]
                .field,
            format!("sources.paths[{index}]")
        );
    }
}

#[test]
fn incomplete_extra_reordered_and_rebound_path_evidence_refuses() {
    let (contract, pools, config) = protected();
    let sources = evidence(&config, &pools, &contract);
    // Source/data attestations for one root are identical. Alter distinct paths
    // for reordering/duplication to exercise an actual binding change.
    assert_eq!(sources.paths[0], sources.paths[1]);
    let distinct = sources
        .paths
        .iter()
        .position(|path| path.path != sources.paths[0].path)
        .unwrap();
    for change in 0..7 {
        let mut bad = sources.clone();
        match change {
            0 => {
                bad.paths.pop();
            }
            1 => bad.paths.push(bad.paths[0].clone()),
            2 => bad.paths.swap(0, distinct),
            3 => bad.paths[0].path.push_str("-other"),
            4 => bad.paths[0].device.minor += 100,
            5 => bad.paths[0].mount_id += 100,
            _ => bad.paths[distinct] = bad.paths[0].clone(),
        }
        assert_ne!(bad, sources, "fixture mutation {change} must change facts");
        assert!(
            plan_snapraid_scrub(&config, &pools, &contract, &bad, Default::default(), 102).is_err(),
            "{change}"
        );
    }
}

#[test]
fn scrub_never_inherits_sync_threshold_admission_or_synthesizes_a_diff() {
    let (contract, pools, mut config) = protected();
    config.thresholds.delete_threshold = 0;
    config.thresholds.update_threshold = 0;
    let sources = evidence(&config, &pools, &contract);
    let diff = SnapraidDiffEvidence {
        observed_at: 101,
        configuration: config.clone(),
        topology_digest: sources.inventory.topology_digest.clone(),
        mounts_digest: sources.inventory.mounts_digest.clone(),
        outcome: SnapraidDiffOutcome::Parsed(SnapraidDiff {
            added: 0,
            removed: 1,
            updated: 1,
            moved: 0,
            copied: 0,
            restored: 0,
        }),
    };
    assert!(
        plan_snapraid_sync(&config, &pools, &contract, &sources, &diff, 102)
            .unwrap_err()
            .iter()
            .all(|finding| finding.code == StoragePlanningFindingCode::ThresholdExceeded)
    );
    let scrub = plan_snapraid_scrub(
        &config,
        &pools,
        &contract,
        &sources,
        Default::default(),
        102,
    )
    .unwrap();
    assert_eq!(scrub.configuration.desired.thresholds, config.thresholds);
    assert_eq!(scrub.expected_sources, sources);
}

#[test]
fn original_five_second_deadline_refuses_future_expired_and_invalid_times() {
    let (contract, pools, config) = protected();
    let sources = evidence(&config, &pools, &contract);
    assert!(
        plan_snapraid_scrub(
            &config,
            &pools,
            &contract,
            &sources,
            Default::default(),
            105
        )
        .is_ok()
    );
    for now in [99, 106, i64::MIN, i64::MAX] {
        assert_eq!(
            plan_snapraid_scrub(
                &config,
                &pools,
                &contract,
                &sources,
                Default::default(),
                now
            )
            .unwrap_err()[0]
                .code,
            StoragePlanningFindingCode::StaleEvidence
        );
    }
    let mut bad = sources.clone();
    bad.observed_at = 0;
    assert!(plan_snapraid_scrub(&config, &pools, &contract, &bad, Default::default(), 1).is_err());
}

#[test]
fn each_device_refuses_bare_replaced_duplicate_boot_swap_readonly_bind_or_wrong_mount() {
    let (contract, pools, config) = protected();
    let sources = evidence(&config, &pools, &contract);
    for index in 0..sources.inventory.devices.len() {
        for change in 0..10 {
            let mut bad = sources.clone();
            let device = &mut bad.inventory.devices[index];
            match change {
                0 => device.mounts.clear(),
                1 => device.filesystem_uuid.push_str("-replaced"),
                2 => device.boot_backing = true,
                3 => device.in_use_as_swap = true,
                4 => device.mounts[0].writable = false,
                5 => device.mounts[0].filesystem_root = "/bind".into(),
                6 => device.mounts[0].mount_id = 1,
                7 => device.serial = Some("replaced-serial".into()),
                8 => device.filesystem = "xfs".into(),
                _ => {
                    let mut duplicate = device.clone();
                    duplicate.device.minor += 100;
                    duplicate.mounts.clear();
                    bad.inventory.devices.push(duplicate);
                }
            }
            assert!(
                plan_snapraid_scrub(&config, &pools, &contract, &bad, Default::default(), 102)
                    .is_err(),
                "{index}: {change}"
            );
        }
    }
}

#[test]
fn hidden_leaf_nested_data_and_ambiguous_mounts_refuse() {
    let (contract, pools, config) = protected();
    let sources = evidence(&config, &pools, &contract);
    for suffix in ["", "/snapraid.content", "/nested"] {
        let mut bad = sources.clone();
        let mut mount = bad.inventory.devices[0].mounts[0].clone();
        mount.mountpoint.push_str(suffix);
        mount.mount_id = 1000;
        bad.inventory.devices[1].mounts.push(mount);
        assert!(
            plan_snapraid_scrub(&config, &pools, &contract, &bad, Default::default(), 102).is_err()
        );
    }
}

#[test]
fn qualified_filesystems_reuse_the_current_protection_support_boundary() {
    for (filesystem, label) in [
        (crate::StorageFilesystem::Ext2, "ext2"),
        (crate::StorageFilesystem::Ext3, "ext3"),
        (crate::StorageFilesystem::Ext4, "ext4"),
        (crate::StorageFilesystem::Xfs, "xfs"),
        (crate::StorageFilesystem::Btrfs, "btrfs"),
    ] {
        let (mut contract, pools, config) = protected();
        contract.devices[0].filesystem = filesystem;
        let mut sources = evidence(&config, &pools, &contract);
        sources.inventory.devices[0].filesystem = label.into();
        sources.inventory.devices[0].mounts[0].filesystem = label.into();
        let result = plan_snapraid_scrub(
            &config,
            &pools,
            &contract,
            &sources,
            Default::default(),
            102,
        );
        if label == "btrfs" {
            assert_eq!(
                result.unwrap_err()[0].code,
                StoragePlanningFindingCode::Unsupported
            );
        } else {
            assert!(result.is_ok(), "{label}");
        }
    }
}

#[test]
fn all_drive_roles_and_generated_leaves_refuse_pool_roots_even_when_disabled() {
    let (contract, pools, config) = protected();
    for index in 0..config.drives.len() {
        for path in ["/mnt/storage", "/mnt/storage/child"] {
            let mut bad = config.clone();
            bad.drives[index].path = path.into();
            assert!(
                plan_snapraid_scrub(
                    &bad,
                    &pools,
                    &contract,
                    &evidence(&config, &pools, &contract),
                    Default::default(),
                    102
                )
                .unwrap_err()
                .iter()
                .any(|finding| finding.code == StoragePlanningFindingCode::PoolPath)
            );
        }
    }
    for suffix in ["snapraid.content", "snapraid.parity"] {
        let mut bad = pools.clone();
        bad.pools[0].enabled = false;
        bad.pools[0].mount_point = format!("/mnt/parity/parity-01/{suffix}");
        assert!(
            plan_snapraid_scrub(
                &config,
                &bad,
                &contract,
                &evidence(&config, &pools, &contract),
                Default::default(),
                102
            )
            .unwrap_err()
            .iter()
            .any(|finding| finding.code == StoragePlanningFindingCode::PoolPath)
        );
    }
}
