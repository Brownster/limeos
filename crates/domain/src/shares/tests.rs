use super::*;
use crate::{
    StorageFilesystem, StorageProfile, StorageRole,
    pools::tests::{fixtures, protected},
};
use serde_json::Value;

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
fn config(path: &str) -> SambaSharesConfig {
    SambaSharesConfig {
        shares: vec![share("media", path)],
    }
}
fn routes(preview: &SambaSharesPreview) -> Vec<Vec<String>> {
    preview
        .dependencies
        .iter()
        .map(|dependency| {
            dependency
                .requirements
                .iter()
                .map(|requirement| {
                    let route = match &requirement.via {
                        SambaDependencyRoute::Direct => "direct",
                        SambaDependencyRoute::Pool { pool_id, .. } => pool_id,
                    };
                    format!("{}:{route}", requirement.expected.id)
                })
                .collect()
        })
        .collect()
}

#[test]
fn all_four_existing_layouts_match_hand_written_dependency_goldens() {
    let expected: Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/samba/layout-dependencies.json"
    ))
    .unwrap();
    let shares = SambaSharesConfig {
        shares: vec![
            share("media", "/mnt/storage/Movies"),
            share("downloads", "/mnt/downloads/Complete"),
            share("docs", "/srv/docs"),
        ],
    };
    for case in fixtures() {
        let contract: StorageContract = serde_json::from_value(case["contract"].clone()).unwrap();
        let pools = serde_json::from_value(case["pools"].clone()).unwrap();
        let preview = shares.plan(&contract, &pools).unwrap();
        assert_eq!(
            serde_json::to_value(routes(&preview)).unwrap(),
            expected[case["name"].as_str().unwrap()]
        );
        assert_eq!(preview.media_identity, contract.media_identity);
        assert_eq!(preview.desired, shares);
        assert!(preview.unmapped_shares.contains(&"docs".into()));
        for dependency in &preview.dependencies {
            for requirement in &dependency.requirements {
                assert!(contract.devices.contains(&requirement.expected));
            }
        }
    }
}

#[test]
fn section_preview_keeps_share_order_literal_spaces_case_and_all_flags() {
    let mut a = share("Z.Media", "/srv/Media Library/Movies");
    a.comment = "Movies & TV".into();
    a.read_only = true;
    a.guest_ok = true;
    a.browseable = false;
    a.valid_users = "pi, media".into();
    let b = share("a_docs", "/srv/docs");
    let mut disabled = share("disabled", "/srv/disabled");
    disabled.enabled = false;
    let shares = SambaSharesConfig {
        shares: vec![a, disabled, b],
    };
    let before = shares.clone();
    assert_eq!(
        shares.render_sections().unwrap(),
        "# BEGIN LIMEOS SAMBA V1\n[Z.Media]\n   comment = Movies & TV\n   path = /srv/Media Library/Movies\n   read only = yes\n   guest ok = yes\n   browseable = no\n   valid users = pi, media\n\n[a_docs]\n   path = /srv/docs\n   read only = no\n   guest ok = no\n   browseable = yes\n\n# END LIMEOS SAMBA V1\n"
    );
    assert_eq!(shares, before);
    assert_eq!(
        shares,
        serde_json::from_slice(&serde_json::to_vec(&shares).unwrap()).unwrap()
    );
}

#[test]
fn disabled_and_read_only_shares_keep_disabled_pool_requirements() {
    let (contract, mut pools, _) = protected();
    pools.pools[0].enabled = false;
    let mut shares = config("/mnt/storage/Movies");
    shares.shares[0].enabled = false;
    shares.shares[0].read_only = true;
    let preview = shares.plan(&contract, &pools).unwrap();
    assert_eq!(preview.dependencies[0].requirements.len(), 14);
    assert!(
        preview.dependencies[0]
            .requirements
            .iter()
            .all(|r| { matches!(r.via, SambaDependencyRoute::Pool { enabled: false, .. }) })
    );
    assert_eq!(preview.desired, shares);
    assert!(!preview.managed_sections.contains("[media]"));
}

#[test]
fn ancestor_and_direct_paths_are_conservative_without_prefix_or_case_aliases() {
    let (contract, pools, _) = protected();
    let ancestor = config("/mnt").plan(&contract, &pools).unwrap();
    assert_eq!(ancestor.dependencies[0].requirements.len(), 29);
    let direct = config("/mnt/disks/data-01/Movies")
        .plan(&contract, &pools)
        .unwrap();
    assert_eq!(routes(&direct), vec![vec!["data-01:direct".to_string()]]);
    for path in [
        "/mnt/storage-other/Movies",
        "/mnt/Storage/Movies",
        "/mnt/disks/data-010",
        "/srv/docs",
    ] {
        let unmapped = config(path).plan(&contract, &pools).unwrap();
        assert!(unmapped.dependencies[0].requirements.is_empty());
        assert_eq!(unmapped.unmapped_shares, ["media"]);
    }
    let parity = config("/mnt/parity/parity-01")
        .plan(&contract, &pools)
        .unwrap();
    assert_eq!(
        parity.dependencies[0].requirements[0].expected.role,
        StorageRole::Parity
    );
}

#[test]
fn multiple_branches_on_one_device_coalesce_but_keep_branch_order() {
    let (contract, mut pools, _) = protected();
    pools.pools[0].branches = vec![
        "/mnt/disks/data-02/Z".into(),
        "/mnt/disks/data-01/B".into(),
        "/mnt/disks/data-01/A".into(),
    ];
    let preview = config("/mnt/storage").plan(&contract, &pools).unwrap();
    assert_eq!(
        routes(&preview),
        vec![vec![
            "data-01:pool1".to_string(),
            "data-02:pool1".to_string()
        ]]
    );
    assert!(matches!(&preview.dependencies[0].requirements[0].via,
        SambaDependencyRoute::Pool { branches, .. }
        if branches == &["/mnt/disks/data-01/B", "/mnt/disks/data-01/A"]));
    let mut reordered = contract;
    reordered.devices.reverse();
    assert_eq!(
        config("/mnt/storage").plan(&reordered, &pools).unwrap(),
        preview
    );
}

#[test]
fn every_pool_is_propagated_with_its_own_id_and_branch_mapping() {
    let (contract, mut pools, _) = protected();
    pools.pools[0].branches.truncate(2);
    let mut other = pools.pools[0].clone();
    other.id = "another".into();
    other.name = "another".into();
    other.mount_point = "/mnt/other".into();
    other.branches = vec!["/mnt/disks/data-03".into(), "/mnt/disks/data-04".into()];
    pools.pools.push(other);
    let preview = config("/mnt").plan(&contract, &pools).unwrap();
    assert_eq!(preview.dependencies[0].requirements.len(), 19);
    let rows = routes(&preview);
    assert!(rows[0].contains(&"data-03:another".into()));
    assert!(!rows[0].contains(&"data-03:pool1".into()));
    pools.pools.reverse();
    assert_eq!(config("/mnt").plan(&contract, &pools).unwrap(), preview);
}

#[test]
fn empty_shares_still_validate_storage_and_pool_configuration() {
    let (mut contract, mut pools, _) = protected();
    assert!(
        SambaSharesConfig::default()
            .plan(&contract, &pools)
            .unwrap()
            .dependencies
            .is_empty()
    );
    contract.media_identity.uid = 0;
    assert!(
        SambaSharesConfig::default()
            .plan(&contract, &pools)
            .is_err()
    );
    let (contract, _, _) = protected();
    pools.pools[0].branches[0] = "/mnt/missing".into();
    assert!(
        SambaSharesConfig::default()
            .plan(&contract, &pools)
            .is_err()
    );
    pools.pools[0].branches[0] = contract.devices.last().unwrap().mountpoint.clone();
    assert!(
        SambaSharesConfig::default()
            .plan(&contract, &pools)
            .is_err()
    );
    pools.pools[0].enabled = false;
    pools.pools[0].branches[0] = "/mnt/storage/recursive".into();
    assert!(
        SambaSharesConfig::default()
            .plan(&contract, &pools)
            .is_err()
    );
}

#[test]
fn nested_device_or_pool_roots_refuse_ambiguous_lexical_ownership() {
    let (mut contract, pools, _) = protected();
    contract.devices[1].mountpoint = "/mnt/disks/data-01/nested".into();
    assert!(
        config("/mnt")
            .plan(&contract, &PoolsConfig::default())
            .is_err()
    );
    let (mut contract, _, _) = protected();
    contract.devices.last_mut().unwrap().mountpoint = "/mnt/storage/parity".into();
    assert!(config("/mnt/storage").plan(&contract, &pools).is_err());
}

#[test]
fn names_are_bounded_reserved_and_case_collisions_refuse_even_when_disabled() {
    for name in [
        "", ".", "..", "bad name", "global", "GLOBAL", "homes", "printers", "print$", "ipc$",
        "[share]", "x\npath",
    ] {
        let mut shares = config("/srv/docs");
        shares.shares[0].name = name.into();
        assert!(shares.render_sections().is_err(), "{name:?}");
    }
    let mut shares = config("/srv/docs");
    shares.shares[0].name = "a".repeat(64);
    assert!(shares.validate().is_ok());
    shares.shares[0].name.push('a');
    assert!(shares.validate().is_err());
    shares = config("/srv/docs");
    let mut duplicate = share("MEDIA", "/srv/other");
    duplicate.enabled = false;
    shares.shares.push(duplicate);
    assert_eq!(
        shares.validate().unwrap_err()[0].code,
        StoragePlanningFindingCode::Duplicate
    );
}

#[test]
fn unsafe_paths_cannot_inject_sections_or_expand_substitutions() {
    for path in [
        "/",
        "srv/docs",
        " /srv/docs",
        "/srv/docs ",
        "/srv//docs",
        "/srv/docs/",
        "/srv/../docs",
        "/srv/./docs",
        "/srv/a\nb",
        "/srv/a\rb",
        "/srv/a\tb",
        "/srv/%u",
        "/srv/a\\",
        "/srv/[global]",
        "/srv/a#b",
        "/srv/a;b",
        "/srv/a=b",
        "/srv/\"docs\"",
        "/srv/'docs'",
        "/srv/\u{00a0}docs",
    ] {
        assert!(config(path).render_sections().is_err(), "{path:?}");
    }
    let path = format!("/{}", "a".repeat(511));
    assert!(config(&path).validate().is_ok());
    assert!(config(&(path + "a")).validate().is_err());
    assert!(config("/srv/Médiathèque/My Movies").validate().is_ok());
}

#[test]
fn comments_and_local_user_expressions_are_checked_before_rendering() {
    for comment in [
        "comment\n[global]",
        "comment\rpath = /",
        "comment\\",
        "user %u",
        "comment#",
        "comment;",
        " comment ",
    ] {
        let mut shares = config("/srv/docs");
        shares.shares[0].comment = comment.into();
        assert!(shares.render_sections().is_err(), "{comment:?}");
    }
    let mut shares = config("/srv/docs");
    shares.shares[0].comment = "é".repeat(120);
    assert!(shares.validate().is_ok());
    shares.shares[0].comment.push('x');
    assert!(shares.validate().is_err());
    for users in [
        "@media",
        "+media",
        "DOMAIN\\pi",
        "%S",
        "pi media",
        "pi,,media",
        "pi,",
        ",pi",
        "pi,PI",
        "pi,\nroot",
        "pi,\troot",
        "pi,\u{00a0}root",
        " pi ",
        "0user",
    ] {
        let mut shares = config("/srv/docs");
        shares.shares[0].valid_users = users.into();
        assert!(shares.render_sections().is_err(), "{users:?}");
    }
    shares.shares[0].comment.clear();
    shares.shares[0].valid_users = (0..32)
        .map(|i| format!("user{i}"))
        .collect::<Vec<_>>()
        .join(",");
    assert!(shares.validate().is_ok());
    shares.shares[0].valid_users += ",user32";
    assert!(shares.validate().is_err());
    shares.shares[0].valid_users = "a".repeat(32);
    assert!(shares.validate().is_ok());
    shares.shares[0].valid_users.push('a');
    assert!(shares.validate().is_err());
    shares.shares[0].valid_users = format!("pi,{}root", " ".repeat(2041));
    assert_eq!(shares.shares[0].valid_users.len(), 2048);
    assert!(shares.validate().is_ok());
    shares.shares[0].valid_users.insert(3, ' ');
    assert!(shares.validate().is_err());
}

#[test]
fn share_count_and_expanded_requirement_limits_refuse_whole_requests() {
    let (mut contract, _, _) = protected();
    // Sixteen declared devices, no pool: 32 ancestor shares reach exactly 512.
    let mut extra = contract.devices[0].clone();
    extra.id = "extra".into();
    extra.filesystem_uuid = "uuid-extra".into();
    extra.mountpoint = "/mnt/extra".into();
    extra.filesystem = StorageFilesystem::Ext4;
    contract.devices.push(extra);
    let mut shares = SambaSharesConfig {
        shares: (0..32).map(|i| share(&format!("s{i}"), "/mnt")).collect(),
    };
    let preview = shares.plan(&contract, &PoolsConfig::default()).unwrap();
    assert_eq!(
        preview
            .dependencies
            .iter()
            .map(|d| d.requirements.len())
            .sum::<usize>(),
        512
    );
    shares.shares[31].path = "/mnt/disks".into(); // 31*16 + 14 = 510
    shares.shares.push(share("last", "/mnt/extra"));
    assert_eq!(
        shares
            .plan(&contract, &PoolsConfig::default())
            .unwrap()
            .dependencies
            .iter()
            .map(|d| d.requirements.len())
            .sum::<usize>(),
        511
    );
    shares.shares.push(share("another", "/mnt/extra"));
    assert!(shares.plan(&contract, &PoolsConfig::default()).is_ok());
    shares.shares.push(share("over", "/mnt/extra"));
    assert_eq!(
        shares.plan(&contract, &PoolsConfig::default()).unwrap_err()[0].code,
        StoragePlanningFindingCode::ThresholdExceeded
    );
    let mut shares = SambaSharesConfig {
        shares: (0..64)
            .map(|i| share(&format!("s{i}"), "/srv/docs"))
            .collect(),
    };
    assert!(shares.validate().is_ok());
    shares.shares.push(share("over", "/srv/docs"));
    assert!(shares.render_sections().is_err());
    assert_eq!(contract.profile, StorageProfile::ProtectedPool);
}

#[test]
fn pool_expansion_counts_towards_the_same_whole_request_limit() {
    let (contract, pools, _) = protected();
    let mut shares = SambaSharesConfig {
        shares: (0..36)
            .map(|i| share(&format!("s{i}"), "/mnt/storage"))
            .collect(),
    };
    assert_eq!(
        shares
            .plan(&contract, &pools)
            .unwrap()
            .dependencies
            .iter()
            .map(|d| d.requirements.len())
            .sum::<usize>(),
        504
    );
    shares.shares.push(share("over", "/mnt/storage"));
    assert!(shares.plan(&contract, &pools).is_err());
}

#[test]
fn findings_do_not_echo_paths_users_comments_or_section_names() {
    for field in ["name", "path", "comment", "valid_users"] {
        let mut shares = config("/srv/docs");
        let secret = "password=private-value\n[global]";
        match field {
            "name" => shares.shares[0].name = secret.into(),
            "path" => shares.shares[0].path = secret.into(),
            "comment" => shares.shares[0].comment = secret.into(),
            _ => shares.shares[0].valid_users = secret.into(),
        }
        let errors = shares.validate().unwrap_err();
        assert_eq!(errors[0].field, format!("samba.shares[0].{field}"));
        assert!(!format!("{errors:?}").contains("private-value"));
    }
}
