use super::*;
const PROFILE: StorageReadProfile = StorageReadProfile::CombinedV1;

fn combined(value: &ContainerStorageInventory) -> Result<ContainerSourceEvidence> {
    PROFILE.admit_sources(value)?;
    let table = read_table_with_profile(PROFILE)?;
    let ns = host_view(&table, &read_host_table_with_profile(PROFILE)?)?;
    collect_with_profile(value, Context::new()?, table, ns, deadline(), PROFILE)
}

#[test]
fn real_sources_retain_profile_original_expiry_and_read_only_handles() {
    let (_dir, file) = fixture();
    let value = inventory(&[&file]);
    let result = combined(&value).unwrap();
    assert_eq!(result.profile, PROFILE);
    let expiry = result.expires;
    result.revalidate_inner(deadline()).unwrap();
    assert_eq!(result.expires, expiry);
    let fd = rustix::io::dup(&result.retained[0]).unwrap();
    assert!(fs::File::from(fd).read_to_end(&mut vec![]).is_err());
    fs::remove_file(&file).unwrap();
    assert!(result.revalidate_inner(deadline()).is_err());
}

#[test]
fn real_combined_source_descriptor_measurement_and_whole_count_refusal() {
    const FLAG: &str = "LIMEOS_STORAGE_SOURCE_FD_PROBE";
    if std::env::var_os(FLAG).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact", "sources::tests::admission_tests::real_combined_source_descriptor_measurement_and_whole_count_refusal", "--nocapture", "--test-threads=1"]).env(FLAG,"1").output().unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        println!("{}", String::from_utf8_lossy(&output.stdout));
        return;
    }
    let (_dir, file) = fixture();
    let value = inventory(&vec![file.as_path(); 32]);
    let count = || fs::read_dir("/proc/self/fd").unwrap().count();
    let baseline = count();
    let result = combined(&value).unwrap();
    assert_eq!(result.snapshot.sources.len(), 32);
    assert_eq!(count(), baseline + 34);
    result.revalidate_inner(deadline()).unwrap();
    assert_eq!(count(), baseline + 34);
    println!(
        "combined_source_fd_measurement sources=32 retained=34 baseline={baseline} root_authorization=private_nonroot_collection_seam"
    );
    drop(result);
    assert_eq!(count(), baseline);
    let excess = inventory(&vec![file.as_path(); 33]);
    assert!(matches!(combined(&excess), Err(Failure::Unavailable)));
    assert_eq!(count(), baseline);
}

#[test]
fn references_reserve_only_after_whole_candidate_admission() {
    let table: Vec<_> = (1..=128)
        .map(|i| row(i, 1, "/", &format!("/data/{i}")))
        .collect();
    let identity = ContainerSourceIdentity {
        device: table[0].device,
        mount_id: 1,
        inode: 1,
        kind: ContainerSourceKind::Directory,
    };
    assert_eq!(
        candidates_with_limit(&identity, "/", &table, 128)
            .unwrap()
            .len(),
        128
    );
    assert_eq!(
        candidates_with_limit(&identity, "/", &table, 127),
        Err(Failure::Unavailable)
    );
    assert_eq!(
        candidates_with_limit(&identity, "/", &table, 0),
        Err(Failure::Unavailable)
    );
    assert_eq!(candidates(&identity, "/", &table).unwrap().len(), 128);
}

#[test]
fn pending_queue_admits_exact_components_and_capacity_before_expansion() {
    let mut pending = Pending::new(PROFILE);
    pending.prepend(&vec!["x"; 256].join("/")).unwrap();
    assert_eq!(pending.values.len(), 256);
    assert!(pending.prepend("x").is_err());
    assert_eq!(pending.values.len(), 256);
    while pending.pop().is_some() {}
    assert_eq!(pending.bytes, 0);
    let part = "x".repeat(512);
    for _ in 0..32 {
        pending.prepend(&part).unwrap();
    }
    assert_eq!(pending.bytes, 16 * 1024);
    assert!(pending.prepend("x").is_err());
    assert_eq!(pending.bytes, 16 * 1024);
    while pending.pop().is_some() {}
    assert_eq!(pending.bytes, 0);
    let mut standard = Pending::new(StorageReadProfile::Standard);
    standard.prepend(&vec!["x"; 257].join("/")).unwrap();
    assert_eq!(standard.values.len(), 257);
}

#[test]
fn held_mount_comparison_keeps_original_profile_on_excess_raw_input() {
    let raw = "1 0 8:1 / / rw - ext4 x rw\n";
    let table = mounts::parse_with_profile(raw, PROFILE).unwrap();
    assert_eq!(
        table_matches_with_profile(raw, &table, &limeos_identity::digest(raw), PROFILE),
        Ok(())
    );
    let excess = format!("{raw}{}", "x".repeat(65536));
    assert_eq!(
        table_matches_with_profile(&excess, &table, &limeos_identity::digest(&excess), PROFILE),
        Err(Failure::Unavailable)
    );
}
