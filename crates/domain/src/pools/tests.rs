use super::*;
use crate::{SnapraidConfig, StorageContract};
use serde_json::Value;

pub(crate) fn fixtures() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!(
        "../../../../tests/fixtures/pool-protection/layouts.json"
    ))
    .unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
pub(crate) fn protected() -> (StorageContract, PoolsConfig, SnapraidConfig) {
    let case = &fixtures()[2];
    (
        serde_json::from_value(case["contract"].clone()).unwrap(),
        serde_json::from_value(case["pools"].clone()).unwrap(),
        serde_json::from_value(case["snapraid"].clone()).unwrap(),
    )
}
#[test]
fn frozen_golden_layouts_round_trip_and_preserve_ownership_and_branch_order() {
    for case in fixtures() {
        let config: PoolsConfig = serde_json::from_value(case["pools"].clone()).unwrap();
        let contract: StorageContract = serde_json::from_value(case["contract"].clone()).unwrap();
        let preview = config.plan(&contract).unwrap();
        assert_eq!(preview.media_identity, contract.media_identity);
        assert_eq!(
            preview.managed_fstab,
            case["fstab"].as_str().unwrap(),
            "{}",
            case["name"]
        );
        let encoded = serde_json::to_vec(&preview).unwrap();
        assert_eq!(
            preview,
            serde_json::from_slice::<PoolPreview>(&encoded).unwrap()
        );
        for (pool, dependency) in config.pools.iter().zip(preview.dependencies) {
            assert_eq!(pool.id, dependency.pool_id);
            assert_eq!(pool.branches, preview.desired.pools[0].branches);
            assert_eq!(
                dependency.devices[0].mountpoint,
                contract.devices[0].mountpoint
            );
        }
    }
}
#[test]
fn all_frozen_policies_and_presets_render_with_typed_overrides() {
    let (_, mut config, _) = protected();
    for policy in [
        PoolCreatePolicy::Epmfs,
        PoolCreatePolicy::Eplfs,
        PoolCreatePolicy::Eplus,
        PoolCreatePolicy::Mfs,
        PoolCreatePolicy::Lfs,
        PoolCreatePolicy::Lus,
        PoolCreatePolicy::Rand,
        PoolCreatePolicy::Pfrd,
        PoolCreatePolicy::Ff,
    ] {
        for preset in [
            PoolPreset::Linux66Plus,
            PoolPreset::Linux65Mmap,
            PoolPreset::Linux65NoMmap,
        ] {
            config.pools[0].create_policy = policy;
            config.pools[0].preset = preset;
            let text = config.render_fstab().unwrap();
            assert!(text.contains(&format!("category.create={}", policy.as_str())));
            assert!(text.contains(if preset == PoolPreset::Linux65Mmap {
                "cache.files=auto-full"
            } else {
                "cache.files=off"
            }));
            assert_eq!(text, config.render_fstab().unwrap());
        }
    }
}
#[test]
fn reject_ambiguous_overlapping_or_recursive_pools_including_disabled_pools() {
    let (_, config, _) = protected();
    let mut duplicate = config.clone();
    duplicate.pools.push(duplicate.pools[0].clone());
    assert!(duplicate.validate().is_err());
    let mut recursive = config.clone();
    recursive.pools[0].enabled = false;
    recursive.pools[0].branches[0] = "/mnt/storage/Movies".into();
    assert!(
        recursive
            .validate()
            .unwrap_err()
            .iter()
            .any(|f| f.code == StoragePlanningFindingCode::PoolPath)
    );
    let mut nested = config.clone();
    nested.pools[0].branches[1] = format!("{}/child", nested.pools[0].branches[0]);
    assert!(nested.validate().is_err());
    let mut other = config.pools[0].clone();
    other.id = "second".into();
    other.name = "second".into();
    other.mount_point = "/mnt/storage/subpool".into();
    other.branches = vec!["/mnt/other1".into(), "/mnt/other2".into()];
    nested.pools = vec![config.pools[0].clone(), other];
    assert!(nested.validate().is_err());
}
#[test]
fn reject_noncanonical_paths_and_render_delimiters_without_normalizing_case() {
    let (_, config, _) = protected();
    for path in [
        "/mnt/d/../other",
        "/mnt//d",
        "/mnt/d/",
        "/mnt/d\nprehash",
        "/mnt/d:other",
        "/mnt/d,other",
        "/mnt/d\\other",
        "/mnt/d*",
        "/mnt/d space",
        "/home/d",
        "/mnt/d#comment",
    ] {
        let mut invalid = config.clone();
        invalid.pools[0].branches[0] = path.into();
        assert!(invalid.validate().is_err(), "{path}");
    }
    let mut literal = config;
    literal.pools[0].branches[0] += "/Movies";
    assert!(literal.render_fstab().unwrap().contains("/Movies"));
    assert!(!literal.render_fstab().unwrap().contains("/movies"));
}
#[test]
fn pool_identity_binding_rejects_unknown_branch_and_parity_role() {
    let (mut contract, config, _) = protected();
    contract.devices[0].mountpoint = "/mnt/replaced".into();
    assert!(config.plan(&contract).is_err());
    let (contract, mut config, _) = protected();
    config.pools[0].branches[0] = contract.devices.last().unwrap().mountpoint.clone();
    assert!(config.plan(&contract).is_err());
}
#[test]
fn minimum_space_uses_bounded_frozen_grammar_and_unknown_settings_are_closed() {
    for valid in ["4G", "1GiB", "1kib", "0", "999B", "3Ti", "01m"] {
        assert!(size(valid), "{valid}");
    }
    for invalid in [
        "",
        "4Z",
        "-1G",
        "1.5G",
        "1 G",
        "18446744073709551616G",
        "1G,noexec",
    ] {
        assert!(!size(invalid), "{invalid}");
    }
    let (_, config, _) = protected();
    let mut json = serde_json::to_value(config).unwrap();
    json["shell"] = "bad".into();
    assert!(serde_json::from_value::<PoolsConfig>(json).is_err());
}
