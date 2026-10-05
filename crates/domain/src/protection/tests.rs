use super::*;
use crate::pools::tests::{fixtures, protected};
use crate::{StorageObservedDevice, StorageObservedMount};

pub(crate) fn evidence(
    config: &SnapraidConfig,
    pools: &PoolsConfig,
    contract: &StorageContract,
) -> ProtectionSourceEvidence {
    let preview = config.plan(pools, contract).unwrap();
    let devices: Vec<_> = contract
        .devices
        .iter()
        .enumerate()
        .map(|(i, d)| StorageObservedDevice {
            device: StorageBlockDevice {
                major: 8,
                minor: i as u32 + 1,
            },
            filesystem_uuid: d.filesystem_uuid.clone(),
            filesystem: "ext4".into(),
            serial: d.serial.clone(),
            boot_backing: false,
            in_use_as_swap: false,
            mounts: vec![StorageObservedMount {
                mountpoint: d.mountpoint.clone(),
                mount_id: i as u64 + 10,
                filesystem_root: "/".into(),
                filesystem: "ext4".into(),
                writable: true,
            }],
        })
        .collect();
    let paths = preview
        .requirements
        .iter()
        .map(|r| {
            let d = devices
                .iter()
                .find(|d| d.filesystem_uuid == r.expected.filesystem_uuid)
                .unwrap();
            ProtectionPathObservation {
                path: r.path.clone(),
                state: match r.kind {
                    ProtectionPathKind::Data | ProtectionPathKind::Source => {
                        ProtectionPathState::Directory
                    }
                    _ => ProtectionPathState::RegularFile,
                },
                device: d.device,
                mount_id: d.mounts[0].mount_id,
            }
        })
        .collect();
    ProtectionSourceEvidence {
        observed_at: 100,
        inventory: StorageInventory {
            host_root_mount_id: 1,
            topology_digest: "a".repeat(64),
            mounts_digest: "b".repeat(64),
            fstab_digest: "c".repeat(64),
            fstab_entries: vec![],
            devices,
        },
        paths,
    }
}
fn diff(config: &SnapraidConfig, sources: &ProtectionSourceEvidence) -> SnapraidDiffEvidence {
    SnapraidDiffEvidence {
        observed_at: 101,
        configuration: config.clone(),
        topology_digest: sources.inventory.topology_digest.clone(),
        mounts_digest: sources.inventory.mounts_digest.clone(),
        outcome: SnapraidDiffOutcome::Parsed(SnapraidDiff {
            added: 5,
            removed: 2,
            updated: 3,
            moved: 0,
            copied: 0,
            restored: 0,
        }),
    }
}
#[test]
fn snapraid_frozen_golden_layouts_round_trip_with_sorted_parity_and_ordered_data_content() {
    for case in fixtures() {
        let config: SnapraidConfig = serde_json::from_value(case["snapraid"].clone()).unwrap();
        let pools: PoolsConfig = serde_json::from_value(case["pools"].clone()).unwrap();
        let contract: StorageContract = serde_json::from_value(case["contract"].clone()).unwrap();
        let preview = config.plan(&pools, &contract).unwrap();
        assert_eq!(
            preview.configuration,
            case["snapraid_text"].as_str().unwrap()
        );
        assert_eq!(preview.configuration, config.render(&pools).unwrap());
        assert_eq!(preview.media_identity, contract.media_identity);
        assert_eq!(
            preview,
            serde_json::from_slice::<SnapraidPreview>(&serde_json::to_vec(&preview).unwrap())
                .unwrap()
        );
        assert!(!preview.configuration.contains(".unionfs/"));
    }
}
#[test]
fn sra002_all_roles_and_generated_files_reject_pool_roots_and_descendants_even_disabled() {
    let (_, mut pools, config) = protected();
    pools.pools[0].enabled = false;
    for index in [0, config.drives.len() - 1] {
        for path in ["/mnt/storage", "/mnt/storage/Movies", "/mnt/storage2"] {
            let mut config = config.clone();
            config.drives[index].path = path.into();
            let result = config.validate(&pools);
            if path == "/mnt/storage2" {
                assert!(result.is_ok());
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .iter()
                        .any(|f| f.code == StoragePlanningFindingCode::PoolPath)
                );
            }
        }
    }
    // A virtual root matching a generated content/parity leaf is forbidden too.
    for suffix in ["snapraid.content", "snapraid.parity"] {
        let mut pools = pools.clone();
        pools.pools[0].mount_point = format!("/mnt/parity/parity-01/{suffix}");
        assert!(
            config
                .validate(&pools)
                .unwrap_err()
                .iter()
                .any(|f| f.code == StoragePlanningFindingCode::PoolPath)
        );
    }
}
#[test]
fn sra001_accept_only_complete_fresh_mounted_evidence_for_every_path() {
    let (contract, pools, config) = protected();
    let sources = evidence(&config, &pools, &contract);
    let diff = diff(&config, &sources);
    assert!(plan_snapraid_sync(&config, &pools, &contract, &sources, &diff, 102).is_ok());
    for i in 0..sources.paths.len() {
        let mut incomplete = sources.clone();
        incomplete.paths[i].state = ProtectionPathState::Unavailable;
        assert!(
            plan_snapraid_sync(&config, &pools, &contract, &incomplete, &diff, 102).is_err(),
            "path {i}"
        );
        let mut replaced = sources.clone();
        replaced.paths[i].mount_id += 100;
        assert!(plan_snapraid_sync(&config, &pools, &contract, &replaced, &diff, 102).is_err());
        let mut unsafe_path = sources.clone();
        unsafe_path.paths[i].state = ProtectionPathState::Unsafe;
        assert!(plan_snapraid_sync(&config, &pools, &contract, &unsafe_path, &diff, 102).is_err());
    }
}
#[test]
fn sra001_bare_directories_wrong_uuid_unknown_identity_and_duplicate_uuid_block_sync() {
    let (contract, pools, config) = protected();
    let sources = evidence(&config, &pools, &contract);
    let diff = diff(&config, &sources);
    for mutation in 0..4 {
        let mut bad = sources.clone();
        match mutation {
            0 => bad.inventory.devices[0].mounts.clear(),
            1 => bad.inventory.devices[0].filesystem_uuid = "replaced".into(),
            2 => {
                bad.inventory.devices.remove(0);
            }
            _ => {
                let mut same = bad.inventory.devices[0].clone();
                same.device.minor = 100;
                same.mounts.clear();
                bad.inventory.devices.push(same);
            }
        };
        assert!(plan_snapraid_sync(&config, &pools, &contract, &bad, &diff, 102).is_err());
    }
}
#[test]
fn sra001_boot_swap_readonly_bind_mount_and_serial_mismatch_block_sync() {
    let (contract, pools, config) = protected();
    let sources = evidence(&config, &pools, &contract);
    let diff = diff(&config, &sources);
    for mutation in 0..7 {
        let mut bad = sources.clone();
        let d = &mut bad.inventory.devices[0];
        match mutation {
            0 => d.boot_backing = true,
            1 => d.in_use_as_swap = true,
            2 => d.mounts[0].writable = false,
            3 => d.mounts[0].filesystem_root = "/subdir".into(),
            4 => d.serial = Some("swapped".into()),
            5 => d.filesystem = "xfs".into(),
            _ => d.mounts[0].mount_id = 1,
        }
        assert!(plan_snapraid_sync(&config, &pools, &contract, &bad, &diff, 102).is_err());
    }
}
#[test]
fn unqualified_multidevice_filesystem_identity_remains_closed() {
    let (mut contract, pools, config) = protected();
    contract.devices[0].filesystem = crate::StorageFilesystem::Btrfs;
    let mut sources = evidence(&config, &pools, &contract);
    sources.inventory.devices[0].filesystem = "btrfs".into();
    sources.inventory.devices[0].mounts[0].filesystem = "btrfs".into();
    let diff = diff(&config, &sources);
    assert_eq!(
        plan_snapraid_sync(&config, &pools, &contract, &sources, &diff, 102).unwrap_err()[0].code,
        StoragePlanningFindingCode::Unsupported
    );
}
#[test]
fn nested_and_ambiguous_mounts_block_sync_and_absent_files_still_require_verified_parent_mounts() {
    let (contract, pools, config) = protected();
    let mut sources = evidence(&config, &pools, &contract);
    let diff = diff(&config, &sources);
    for (r, p) in config
        .plan(&pools, &contract)
        .unwrap()
        .requirements
        .iter()
        .zip(&mut sources.paths)
    {
        if matches!(
            r.kind,
            ProtectionPathKind::Content | ProtectionPathKind::Parity
        ) {
            p.state = ProtectionPathState::AbsentFile;
        }
    }
    assert!(plan_snapraid_sync(&config, &pools, &contract, &sources, &diff, 102).is_ok());
    let mut hidden = sources.clone();
    let mut m = hidden.inventory.devices[0].mounts[0].clone();
    m.mountpoint += "/snapraid.content";
    m.mount_id = 1000;
    hidden.inventory.devices[1].mounts.push(m);
    assert!(plan_snapraid_sync(&config, &pools, &contract, &hidden, &diff, 102).is_err());
    let mut ambiguous = sources.clone();
    let duplicate = ambiguous.inventory.devices[0].mounts[0].clone();
    ambiguous.inventory.devices[1].mounts.push(duplicate);
    assert!(plan_snapraid_sync(&config, &pools, &contract, &ambiguous, &diff, 102).is_err());
}
#[test]
fn stale_future_or_rebound_source_and_diff_evidence_block_sync() {
    let (contract, pools, config) = protected();
    let sources = evidence(&config, &pools, &contract);
    let good = diff(&config, &sources);
    for now in [99, 106, i64::MAX] {
        assert!(plan_snapraid_sync(&config, &pools, &contract, &sources, &good, now).is_err());
    }
    for mutation in 0..5 {
        let mut bad = good.clone();
        match mutation {
            0 => bad.observed_at = 96,
            1 => bad.observed_at = 103,
            2 => bad.configuration.excludes.push("*.bak".into()),
            3 => bad.mounts_digest = "d".repeat(64),
            _ => bad.topology_digest = "e".repeat(64),
        }
        assert!(plan_snapraid_sync(&config, &pools, &contract, &sources, &bad, 102).is_err());
    }
    // Refresh source descriptors after diff completion; unchanged digests bind it.
    let mut refreshed = sources.clone();
    refreshed.observed_at = 102;
    assert!(plan_snapraid_sync(&config, &pools, &contract, &refreshed, &good, 103).is_ok());
}
#[test]
fn threshold_boundaries_pass_and_excesses_require_a_separate_operation() {
    let (contract, pools, config) = protected();
    let sources = evidence(&config, &pools, &contract);
    let mut diff = diff(&config, &sources);
    let SnapraidDiffOutcome::Parsed(counts) = &mut diff.outcome else {
        panic!()
    };
    counts.removed = 50;
    counts.updated = 500;
    assert!(plan_snapraid_sync(&config, &pools, &contract, &sources, &diff, 102).is_ok());
    for (removed, updated) in [(51, 500), (50, 501), (u64::MAX, u64::MAX)] {
        let SnapraidDiffOutcome::Parsed(counts) = &mut diff.outcome else {
            panic!()
        };
        counts.removed = removed;
        counts.updated = updated;
        assert!(
            plan_snapraid_sync(&config, &pools, &contract, &sources, &diff, 102)
                .unwrap_err()
                .iter()
                .all(|f| f.code == StoragePlanningFindingCode::ThresholdExceeded)
        );
    }
    let mut json = serde_json::to_value(config).unwrap();
    json["force"] = true.into();
    assert!(serde_json::from_value::<SnapraidConfig>(json).is_err());
}
#[test]
fn failed_or_missing_diff_cannot_be_overridden_by_zero_counts_or_log_success() {
    let (contract, pools, config) = protected();
    let sources = evidence(&config, &pools, &contract);
    let mut diff = diff(&config, &sources);
    for termination in [
        StorageReadTermination::Exited { code: 1 },
        StorageReadTermination::MissingBinary,
        StorageReadTermination::TimedOut,
        StorageReadTermination::OutputLimit,
        StorageReadTermination::IoError,
        StorageReadTermination::Signaled { signal: 9 },
    ] {
        diff.outcome = SnapraidDiffOutcome::Unavailable(StorageReadFailure {
            termination: Some(termination),
            reason: StorageReadFailureReason::ProcessFailure,
            detail: "summary:exit:ok".into(),
        });
        assert_eq!(
            plan_snapraid_sync(&config, &pools, &contract, &sources, &diff, 102).unwrap_err()[0]
                .code,
            StoragePlanningFindingCode::DiffUnavailable
        );
    }
}
#[test]
fn configuration_rejects_parity_gaps_duplicates_missing_content_uuid_and_injection() {
    let (_, pools, config) = protected();
    for mutation in 0..6 {
        let mut bad = config.clone();
        match mutation {
            0 => bad.drives.last_mut().unwrap().parity_level = 2,
            1 => bad.drives.push(bad.drives.last().unwrap().clone()),
            2 => bad.drives.iter_mut().for_each(|d| d.content = false),
            3 => bad.drives[0].uuid.clear(),
            4 => bad.excludes.push("*.tmp\ndata injected /mnt/x".into()),
            _ => bad.drives[0].path += "/../storage",
        }
        assert!(bad.validate(&pools).is_err());
    }
}
