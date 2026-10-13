use super::*;
use limeos_domain::{
    ContainerSnapshot, ContainerStorageConsumer, ContainerStorageInventory, ContainerStorageMount,
    ContainerStorageSource, shares::SambaShareConfig,
    storage_dependency_graph::configured_storage_dependency_graph,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};
use tempfile::TempDir;

fn limits() -> ConfigurationLimits {
    ConfigurationLimits {
        max_file_bytes: MAX_FILE_BYTES,
        max_total_bytes: MAX_TOTAL_BYTES,
        max_retained_fds: MAX_FDS,
        max_report_bytes: MAX_REPORT_BYTES,
    }
}
fn cases() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!(
        "../../../../tests/fixtures/pool-protection/layouts.json"
    ))
    .unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn shares() -> SambaSharesConfig {
    SambaSharesConfig {
        shares: [
            ("media", "/mnt/storage/Movies"),
            ("downloads", "/mnt/downloads/Complete"),
            ("docs", "/srv/docs"),
        ]
        .into_iter()
        .map(|(name, path)| SambaShareConfig {
            name: name.into(),
            path: path.into(),
            comment: String::new(),
            read_only: false,
            guest_ok: false,
            browseable: true,
            valid_users: String::new(),
            enabled: true,
        })
        .collect(),
    }
}
fn inventory() -> ContainerStorageInventory {
    ContainerStorageInventory {
        version: 1,
        engine_id: "synthetic-engine".into(),
        observed_at: 100,
        containers: [
            ("a", "/mnt/storage/Movies"),
            ("b", "/mnt/downloads/Complete"),
        ]
        .into_iter()
        .map(|(id, path)| ContainerStorageConsumer {
            container: ContainerSnapshot {
                resource: format!("container:{}", id.repeat(64)),
                image: format!("sha256:{}", "f".repeat(64)),
                started_at: "2026-10-06T00:00:00Z".into(),
                running: true,
            },
            mounts: vec![ContainerStorageMount {
                source: ContainerStorageSource::Bind { path: path.into() },
                destination: "/data".into(),
                writable: true,
            }],
        })
        .collect(),
    }
}
struct Fixture {
    tree: TempDir,
    names: [PathBuf; 4],
}
impl Fixture {
    fn new() -> Self {
        Self::from_case(&cases()[2])
    }
    fn from_case(case: &Value) -> Self {
        let tree = tempfile::tempdir().unwrap();
        fs::create_dir(tree.path().join("settings")).unwrap();
        let names = ["contract", "pools", "protection", "shares"]
            .map(|s| PathBuf::from(format!("/settings/{s}.json")));
        let f = Self { tree, names };
        for (i, value) in [
            case["contract"].clone(),
            case["pools"].clone(),
            case["snapraid"].clone(),
            serde_json::to_value(shares()).unwrap(),
        ]
        .iter()
        .enumerate()
        {
            f.write(i, &serde_json::to_vec(value).unwrap());
        }
        f
    }
    fn paths(&self) -> ConfigurationPaths<'_> {
        ConfigurationPaths {
            contract: &self.names[0],
            pools: &self.names[1],
            protection: &self.names[2],
            shares: &self.names[3],
        }
    }
    fn path(&self, i: usize) -> PathBuf {
        self.tree
            .path()
            .join(self.names[i].strip_prefix("/").unwrap())
    }
    fn write(&self, i: usize, bytes: &[u8]) {
        fs::write(self.path(i), bytes).unwrap();
        fs::set_permissions(self.path(i), fs::Permissions::from_mode(0o600)).unwrap();
    }
    fn root(&self) -> OwnedFd {
        rustix::fs::open(
            self.tree.path(),
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap()
    }
    fn collect(&self) -> Result<StorageConfigurationEvidence> {
        self.collect_with(limits(), &|| false)
    }
    fn collect_with(
        &self,
        limits: ConfigurationLimits,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<StorageConfigurationEvidence> {
        let paths = self.paths();
        let parts = admit(&paths, limits)?;
        // Only the private root/authentication seam differs. File metadata,
        // traversal, positional reads and comparison all use production code.
        collect(
            self.root(),
            &paths,
            parts,
            CollectionBudget {
                owner: rustix::process::geteuid().as_raw(),
                limits,
                at: 100,
                expires: Instant::now() + LIFETIME,
                deadline: Instant::now() + WORK,
            },
            cancelled,
        )
    }
    fn revalidate(
        &self,
        owner: &StorageConfigurationEvidence,
        at: i64,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<()> {
        owner.checked(|| owner.validate_from(&self.root(), at, Instant::now() + WORK, cancelled))
    }
}
fn refusal<T>(result: Result<T>, expected: Failure) {
    assert!(matches!(result, Err(e) if e == expected));
}

#[test]
fn four_frozen_layouts_feed_the_existing_graph_goldens() {
    let expected: Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/storage-dependency-graph/expected.json"
    ))
    .unwrap();
    for case in cases() {
        let f = Fixture::from_case(&case);
        let owner = f.collect().unwrap();
        let c = owner.configuration();
        assert_eq!(serde_json::to_value(&c.contract).unwrap(), case["contract"]);
        assert_eq!(serde_json::to_value(&c.pools).unwrap(), case["pools"]);
        assert_eq!(
            serde_json::to_value(&c.protection).unwrap(),
            case["snapraid"]
        );
        assert_eq!(c.shares, shares());
        let g = configured_storage_dependency_graph(
            &c.contract,
            &c.pools,
            &c.protection,
            &inventory(),
            &c.shares,
            100,
        )
        .unwrap();
        let rows: BTreeMap<String, Vec<String>> = g
            .devices
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
                                    limeos_domain::shares::SambaDependencyRoute::Direct => "direct",
                                    limeos_domain::shares::SambaDependencyRoute::Pool {
                                        pool_id,
                                        ..
                                    } => pool_id,
                                }
                            )
                        })
                        .collect(),
                )
            })
            .collect();
        let count: usize = g
            .devices
            .iter()
            .map(|d| d.storage.containers.len() + d.shares.len())
            .sum();
        assert_eq!(
            json!({"consumer_matches":count,"shares":rows,"unmapped":g.unmapped_shares}),
            expected[case["name"].as_str().unwrap()]
        );
        assert_eq!(owner.retained_descriptor_count(), 9);
        assert_eq!(owner.snapshot.observed_at, 100);
        assert_eq!(
            owner
                .snapshot
                .files
                .iter()
                .map(|o| o.role)
                .collect::<Vec<_>>(),
            ROLES
        );
        f.revalidate(&owner, 101, &|| false).unwrap();
    }
}

#[test]
fn missing_file_in_each_role_refuses_the_whole_set() {
    for i in 0..4 {
        let f = Fixture::new();
        fs::remove_file(f.path(i)).unwrap();
        refusal(f.collect(), Failure::NotReady);
    }
}
#[test]
fn closed_json_rejects_unknown_duplicate_malformed_and_trailing_values() {
    for bytes in [
        b"{}".as_slice(),
        b"{\"pools\":[],\"unknown\":1}",
        b"{\"pools\":[],\"pools\":[]}",
        b"{\"pools\":[]} {}",
        b"\xff",
        b"",
    ] {
        let f = Fixture::new();
        f.write(1, bytes);
        refusal(f.collect(), Failure::InvalidPlan);
    }
}
#[test]
fn valid_empty_pools_are_explicit_and_cross_file_mismatches_refuse() {
    let f = Fixture::from_case(&cases()[0]);
    f.write(1, br#"{"pools":[]}"#);
    f.collect().unwrap();
    let f = Fixture::new();
    let mut pools = cases()[2]["pools"].clone();
    pools["pools"][0]["branches"][0] = json!("/mnt/unconfigured/Movies");
    f.write(1, &serde_json::to_vec(&pools).unwrap());
    refusal(f.collect(), Failure::InvalidPlan);
}
#[test]
fn invalid_contract_and_invalid_share_relationships_refuse() {
    let f = Fixture::new();
    let mut value: Value = serde_json::from_slice(&fs::read(f.path(0)).unwrap()).unwrap();
    value["media_identity"]["uid"] = json!(0);
    f.write(0, &serde_json::to_vec(&value).unwrap());
    refusal(f.collect(), Failure::InvalidPlan);
    let f = Fixture::new();
    let mut smb = shares();
    smb.shares[0].name = "global".into();
    f.write(3, &serde_json::to_vec(&smb).unwrap());
    refusal(f.collect(), Failure::InvalidPlan);
}
#[test]
fn disabled_configuration_and_order_survive_collection() {
    let f = Fixture::new();
    let mut case = cases()[2].clone();
    case["pools"]["pools"][0]["enabled"] = json!(false);
    case["snapraid"]["enabled"] = json!(false);
    let mut smb = shares();
    smb.shares[0].enabled = false;
    smb.shares[0].read_only = true;
    f.write(1, &serde_json::to_vec(&case["pools"]).unwrap());
    f.write(2, &serde_json::to_vec(&case["snapraid"]).unwrap());
    f.write(3, &serde_json::to_vec(&smb).unwrap());
    let c = f.collect().unwrap();
    assert!(!c.configuration.pools.pools[0].enabled && !c.configuration.protection.enabled);
    assert_eq!(
        serde_json::to_value(&c.configuration.protection).unwrap(),
        case["snapraid"]
    );
    assert_eq!(c.configuration.shares, smb);
}
#[test]
fn leaf_symlinks_hardlinks_directories_and_fifos_refuse_without_reading() {
    for kind in 0..4 {
        let f = Fixture::new();
        let target = f.path(1);
        fs::remove_file(&target).unwrap();
        match kind {
            0 => symlink(f.path(0), &target).unwrap(),
            1 => fs::hard_link(f.path(0), &target).unwrap(),
            2 => fs::create_dir(&target).unwrap(),
            _ => rustix::fs::mknodat(
                rustix::fs::CWD,
                &target,
                rustix::fs::FileType::Fifo,
                Mode::from_raw_mode(0o600),
                0,
            )
            .unwrap(),
        }
        refusal(f.collect(), Failure::UnsafePath);
    }
}
#[test]
fn unprotected_leaf_or_ancestor_permissions_refuse() {
    for leaf in [true, false] {
        let f = Fixture::new();
        let target = if leaf {
            f.path(1)
        } else {
            f.tree.path().join("settings")
        };
        fs::set_permissions(
            target,
            fs::Permissions::from_mode(if leaf { 0o620 } else { 0o770 }),
        )
        .unwrap();
        refusal(f.collect(), Failure::UnsafePath);
    }
}
#[test]
fn wrong_owner_is_refused_by_the_same_metadata_policy() {
    let f = Fixture::new();
    let uid = rustix::process::geteuid().as_raw();
    refusal(
        directory(&f.root(), uid.wrapping_add(1)),
        Failure::UnsafePath,
    );
    let fd = rustix::fs::open(f.path(0), OFlags::RDONLY, Mode::empty()).unwrap();
    refusal(
        file_identity(&fd, uid.wrapping_add(1), MAX_FILE_BYTES),
        Failure::UnsafePath,
    );
}
#[test]
fn ancestor_symlink_refuses() {
    let f = Fixture::new();
    fs::rename(f.tree.path().join("settings"), f.tree.path().join("moved")).unwrap();
    symlink("moved", f.tree.path().join("settings")).unwrap();
    refusal(f.collect(), Failure::UnsafePath);
}
#[test]
fn replacement_disappearance_and_permission_changes_poison_the_owner() {
    for kind in 0..3 {
        let f = Fixture::new();
        let owner = f.collect().unwrap();
        match kind {
            0 => {
                fs::rename(f.path(1), f.tree.path().join("old")).unwrap();
                f.write(1, br#"{"pools":[]}"#);
            }
            1 => fs::remove_file(f.path(1)).unwrap(),
            _ => fs::set_permissions(f.path(1), fs::Permissions::from_mode(0o620)).unwrap(),
        }
        assert!(f.revalidate(&owner, 100, &|| false).is_err());
        assert!(owner.state.get() == ValidationState::Poisoned);
        refusal(f.revalidate(&owner, 100, &|| false), Failure::Conflict);
    }
}
#[test]
fn same_byte_rewrite_refuses_instead_of_minting_a_new_identity() {
    let f = Fixture::new();
    let owner = f.collect().unwrap();
    let bytes = fs::read(f.path(1)).unwrap();
    f.write(1, &bytes);
    assert_eq!(fs::read(f.path(1)).unwrap(), owner.files[1].bytes);
    refusal(f.revalidate(&owner, 100, &|| false), Failure::Conflict);
}
#[test]
fn ancestor_replacement_and_mode_change_are_detected() {
    for replace in [true, false] {
        let f = Fixture::new();
        let owner = f.collect().unwrap();
        let settings = f.tree.path().join("settings");
        if replace {
            fs::rename(&settings, f.tree.path().join("old")).unwrap();
            fs::create_dir(&settings).unwrap();
            // Move the original single-link files back: file identities alone
            // would match; the retained directory identity must still refuse.
            for i in 0..4 {
                fs::rename(
                    f.tree
                        .path()
                        .join("old")
                        .join(f.path(i).file_name().unwrap()),
                    f.path(i),
                )
                .unwrap();
            }
        } else {
            fs::set_permissions(&settings, fs::Permissions::from_mode(0o700)).unwrap();
        }
        refusal(f.revalidate(&owner, 100, &|| false), Failure::Conflict);
    }
}
#[test]
fn unrelated_directory_entries_do_not_invalidate_configuration() {
    let f = Fixture::new();
    let owner = f.collect().unwrap();
    fs::write(f.tree.path().join("settings/unrelated"), b"unrelated").unwrap();
    f.revalidate(&owner, 100, &|| false).unwrap();
}
#[test]
fn changed_content_and_appended_whitespace_refuse() {
    for edit in [true, false] {
        let f = Fixture::new();
        let owner = f.collect().unwrap();
        let mut bytes = fs::read(f.path(1)).unwrap();
        if edit {
            bytes[0] = b'[';
        } else {
            bytes.push(b' ');
        }
        f.write(1, &bytes);
        refusal(f.revalidate(&owner, 100, &|| false), Failure::Conflict);
    }
}
#[test]
fn exact_file_and_total_byte_limits_pass_and_one_extra_byte_refuses() {
    let f = Fixture::new();
    let lengths: Vec<usize> = (0..4).map(|i| fs::read(f.path(i)).unwrap().len()).collect();
    let exact = ConfigurationLimits {
        max_file_bytes: *lengths.iter().max().unwrap() as u32,
        max_total_bytes: lengths.iter().sum::<usize>() as u32,
        ..limits()
    };
    f.collect_with(exact, &|| false).unwrap();
    refusal(
        f.collect_with(
            ConfigurationLimits {
                max_file_bytes: exact.max_file_bytes - 1,
                ..exact
            },
            &|| false,
        ),
        Failure::Unavailable,
    );
    refusal(
        f.collect_with(
            ConfigurationLimits {
                max_total_bytes: exact.max_total_bytes - 1,
                ..exact
            },
            &|| false,
        ),
        Failure::Unavailable,
    );
}
#[test]
fn exact_descriptor_and_report_limits_pass_and_one_less_refuses() {
    let f = Fixture::new();
    let owner = f.collect().unwrap();
    let mut exact = ConfigurationLimits {
        max_retained_fds: 9,
        ..limits()
    };
    // Changing a decimal limit changes the report length; find its stable exact
    // serialized size without assuming a machine-specific statx identity size.
    for _ in 0..4 {
        let candidate = f.collect_with(exact, &|| false).unwrap();
        exact.max_report_bytes = serde_json::to_vec(candidate.snapshot()).unwrap().len() as u32;
    }
    f.collect_with(exact, &|| false).unwrap();
    refusal(
        f.collect_with(
            ConfigurationLimits {
                max_retained_fds: 8,
                ..limits()
            },
            &|| false,
        ),
        Failure::Unavailable,
    );
    refusal(
        f.collect_with(
            ConfigurationLimits {
                max_report_bytes: exact.max_report_bytes - 1,
                ..exact
            },
            &|| false,
        ),
        Failure::Unavailable,
    );
    assert!(serde_json::to_vec(owner.snapshot()).unwrap().len() < MAX_REPORT_BYTES as usize);
}
#[test]
fn maximum_path_depth_charges_each_retained_ancestor_before_opening() {
    let mut f = Fixture::new();
    let prefix = "dir/".repeat(15);
    let names = ["contract", "pools", "protection", "shares"]
        .map(|name| PathBuf::from(format!("/{prefix}{name}.json")));
    fs::create_dir_all(f.tree.path().join(&prefix)).unwrap();
    for (i, name) in names.iter().enumerate() {
        fs::rename(
            f.path(i),
            f.tree.path().join(name.strip_prefix("/").unwrap()),
        )
        .unwrap();
    }
    f.names = names;
    let owner = f.collect().unwrap();
    assert_eq!(owner.retained_descriptor_count(), MAX_FDS as usize);
    f.revalidate(&owner, 100, &|| false).unwrap();
    refusal(
        f.collect_with(
            ConfigurationLimits {
                max_retained_fds: MAX_FDS - 1,
                ..limits()
            },
            &|| false,
        ),
        Failure::Unavailable,
    );
}
#[test]
fn absolute_file_and_total_caps_refuse_oversized_bytes_before_parsing() {
    let f = Fixture::new();
    let mut bytes = fs::read(f.path(0)).unwrap();
    bytes.resize(MAX_FILE_BYTES as usize, b' ');
    f.write(0, &bytes);
    f.collect().unwrap();
    bytes.push(b' ');
    f.write(0, &bytes);
    refusal(f.collect(), Failure::Unavailable);
    bytes.pop();
    f.write(0, &bytes);
    let mut protection = fs::read(f.path(2)).unwrap();
    protection.resize(MAX_FILE_BYTES as usize, b' ');
    f.write(2, &protection);
    refusal(f.collect(), Failure::Unavailable);
}
#[test]
fn invalid_limits_and_noncanonical_duplicate_or_deep_paths_refuse_preflight() {
    let f = Fixture::new();
    for bad in [
        "relative",
        "/",
        "/settings/../pools",
        "/settings//pools",
        "/settings/./pools",
        "/settings/pools/",
        "/settings/pools\n",
    ] {
        let mut paths = f.paths();
        paths.pools = Path::new(bad);
        refusal(admit(&paths, limits()), Failure::InvalidPlan);
    }
    let duplicate = ConfigurationPaths {
        pools: &f.names[0],
        ..f.paths()
    };
    refusal(admit(&duplicate, limits()), Failure::AmbiguousIdentity);
    let deep = format!("/{}pools", "dir/".repeat(16));
    let deep = ConfigurationPaths {
        pools: Path::new(&deep),
        ..f.paths()
    };
    refusal(admit(&deep, limits()), Failure::InvalidPlan);
    for bad in [
        ConfigurationLimits {
            max_file_bytes: 0,
            ..limits()
        },
        ConfigurationLimits {
            max_file_bytes: MAX_FILE_BYTES + 1,
            ..limits()
        },
        ConfigurationLimits {
            max_total_bytes: 0,
            ..limits()
        },
        ConfigurationLimits {
            max_total_bytes: MAX_TOTAL_BYTES + 1,
            ..limits()
        },
        ConfigurationLimits {
            max_retained_fds: MAX_FDS + 1,
            ..limits()
        },
        ConfigurationLimits {
            max_report_bytes: MAX_REPORT_BYTES + 1,
            ..limits()
        },
    ] {
        refusal(admit(&f.paths(), bad), Failure::InvalidPlan);
    }
}
#[test]
fn cancellation_expiry_backward_time_and_retry_do_not_renew_the_lease() {
    let f = Fixture::new();
    refusal(f.collect_with(limits(), &|| true), Failure::Conflict);
    for kind in 0..4 {
        let mut owner = f.collect().unwrap();
        let original_expiry = owner.expires;
        f.revalidate(&owner, 101, &|| false).unwrap();
        assert_eq!(owner.expires, original_expiry);
        let (at, cancel) = match kind {
            0 => (99, false),
            1 => (105, false),
            2 => (100, true),
            _ => {
                owner.expires = Instant::now() - Duration::from_secs(1);
                (100, false)
            }
        };
        refusal(
            f.revalidate(&owner, at, &|| cancel),
            if cancel {
                Failure::Conflict
            } else {
                Failure::TimedOut
            },
        );
        owner.expires = Instant::now() + LIFETIME;
        refusal(f.revalidate(&owner, 100, &|| false), Failure::Conflict);
    }
}
#[test]
fn failed_reentrant_validation_cannot_be_cleared_by_an_outer_success() {
    let f = Fixture::new();
    let owner = f.collect().unwrap();
    refusal(
        owner.checked(|| {
            refusal(owner.checked(|| Ok(())), Failure::Conflict);
            Ok(())
        }),
        Failure::Conflict,
    );
    refusal(f.revalidate(&owner, 100, &|| false), Failure::Conflict);
}
#[test]
fn report_binds_raw_bytes_without_copying_configuration_into_metadata() {
    let f = Fixture::new();
    let owner = f.collect().unwrap();
    for (i, observation) in owner.snapshot.files.iter().enumerate() {
        assert_eq!(
            observation.bytes_digest,
            limeos_identity::digest(std::str::from_utf8(&fs::read(f.path(i)).unwrap()).unwrap())
        );
    }
    let mut bytes = fs::read(f.path(1)).unwrap();
    bytes.push(b' ');
    f.write(1, &bytes);
    let fresh = f.collect().unwrap();
    assert_eq!(
        owner.snapshot.configuration_digest,
        fresh.snapshot.configuration_digest
    );
    assert_ne!(
        owner.snapshot.files[1].bytes_digest,
        fresh.snapshot.files[1].bytes_digest
    );
    let report = serde_json::to_value(owner.snapshot()).unwrap();
    assert!(report.get("configuration").is_none());
    assert!(report["files"][0].get("bytes").is_none());
}
#[test]
fn public_collection_refuses_without_root_or_live_host_access() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    // Canonical nonexistent paths pass admission; public authentication refuses
    // before proc or configuration reads. No workstation storage is contacted.
    let paths = ConfigurationPaths {
        contract: Path::new("/limeos-test-contract"),
        pools: Path::new("/limeos-test-pools"),
        protection: Path::new("/limeos-test-protection"),
        shares: Path::new("/limeos-test-shares"),
    };
    refusal(
        inspect_storage_configuration(paths, limits(), &|| false),
        Failure::Unavailable,
    );
}
