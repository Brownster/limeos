use super::*;
use limeos_domain::*;
use serde_json::{Value, json};

fn fixtures() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!(
        "../../../../tests/fixtures/pool-protection/layouts.json"
    ))
    .unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn capture(text: &str) -> CapturedStorageRead {
    CapturedStorageRead {
        termination: StorageReadTermination::Exited { code: 0 },
        stdout: text.as_bytes().to_vec(),
        stderr: vec![],
        stdout_truncated: false,
        stderr_truncated: false,
    }
}
const HUMAN: &str = include_str!("../../../../tests/fixtures/pool-protection/diff-human.txt");
const TAGS: &str = include_str!("../../../../tests/fixtures/pool-protection/diff-tags.txt");
#[test]
fn frozen_legacy_import_matches_typed_fixtures_and_does_not_mutate_source_bytes() {
    for case in fixtures() {
        let bytes = serde_json::to_vec(&case["legacy_pools"]).unwrap();
        let before = bytes.clone();
        let pools = import_legacy_pools(&bytes);
        assert!(pools.findings.is_empty());
        assert_eq!(bytes, before);
        let expected: PoolsConfig = serde_json::from_value(case["pools"].clone()).unwrap();
        assert_eq!(pools.configuration, Some(expected.clone()));
        let bytes = serde_json::to_vec(&case["legacy_snapraid"]).unwrap();
        let before = bytes.clone();
        let snapraid = import_legacy_snapraid(&bytes, &expected);
        assert!(snapraid.findings.is_empty());
        assert_eq!(bytes, before);
        assert_eq!(
            snapraid.configuration,
            Some(serde_json::from_value::<SnapraidConfig>(case["snapraid"].clone()).unwrap())
        );
    }
}
#[test]
fn documented_defaults_are_findings_and_explicit_empty_excludes_stay_empty() {
    let pools=import_legacy_pools(br#"{"pools":[{"id":"p","name":"p","branches":["/mnt/a","/mnt/b"],"mount_point":"/mnt/p"}]}"#);
    assert!(pools.configuration.is_some());
    assert!(
        pools
            .findings
            .iter()
            .all(|f| f.code == StoragePlanningFindingCode::DefaultApplied)
    );
    assert!(
        pools
            .findings
            .iter()
            .any(|f| f.field.ends_with("create_policy"))
    );
    let snap = import_legacy_snapraid(b"{}", &PoolsConfig::default());
    let config = snap.configuration.unwrap();
    assert_eq!(config.excludes, SNAPRAID_DEFAULT_EXCLUDES);
    assert!(!config.excludes.iter().any(|p| p == ".unionfs/"));
    let snap = import_legacy_snapraid(br#"{"excludes":[]}"#, &PoolsConfig::default());
    assert!(snap.configuration.unwrap().excludes.is_empty());
}
#[test]
fn unsupported_unknown_duplicate_and_malformed_imports_are_explicit_and_never_echo_values() {
    for bytes in [
        br#"{"pools":[],"shell":"password=private"}"#.as_slice(),
        br#"{"pools":[],"pools":[]}"#,
        br#"{"pools":null}"#,
        b"[invalid]",
        b"[]",
    ] {
        let report = import_legacy_pools(bytes);
        assert!(
            report.configuration.is_none(),
            "{}",
            String::from_utf8_lossy(bytes)
        );
        assert!(!report.findings.is_empty());
        assert!(!format!("{:?}", report.findings).contains("private"));
    }
    for options in [
        "exec=/tmp/caller",
        "category.create=nope",
        "cache.files=arbitrary",
        "dropcacheonclose=1",
        "cache.files=off,cache.files=auto-full",
        "defaults",
        "minfreespace=4G\nextra",
    ] {
        let bytes=serde_json::to_vec(&json!({"pools":[{"id":"p","name":"p","branches":["/mnt/a","/mnt/b"],"mount_point":"/mnt/p","options":options}]})).unwrap();
        assert!(
            import_legacy_pools(&bytes).configuration.is_none(),
            "{options}"
        );
    }
    assert!(
        import_legacy_pools(&vec![b' '; STORAGE_IMPORT_MAX_BYTES + 1])
            .configuration
            .is_none()
    );
}
#[test]
fn snapraid_import_rejects_missing_uuid_unsupported_settings_malformed_pool_context_and_forces() {
    let case = &fixtures()[2];
    let pools: PoolsConfig = serde_json::from_value(case["pools"].clone()).unwrap();
    for mutation in 0..6 {
        let mut json = case["legacy_snapraid"].clone();
        match mutation {
            0 => {
                json["drives"][0].as_object_mut().unwrap().remove("uuid");
            }
            1 => json["settings"]["blocksize"] = 1.into(),
            2 => json["thresholds"]["delete_threshold"] = (-1).into(),
            3 => json["schedule"]["sync_cron"] = "@reboot shell".into(),
            4 => json["force"] = true.into(),
            _ => json["settings"]["unknown"] = "private".into(),
        }
        assert!(
            import_legacy_snapraid(&serde_json::to_vec(&json).unwrap(), &pools)
                .configuration
                .is_none()
        );
    }
    let mut bad_pools = pools;
    bad_pools.pools[0].mount_point = "../bad".into();
    assert!(
        import_legacy_snapraid(
            &serde_json::to_vec(&case["legacy_snapraid"]).unwrap(),
            &bad_pools
        )
        .configuration
        .is_none()
    );
    assert!(
        import_legacy_snapraid(
            br#"{"settings":{"hashsize":8,"hashsize":16}}"#,
            &PoolsConfig::default()
        )
        .configuration
        .is_none()
    );
    for bytes in [
        br#"{"settings":[]}"#.as_slice(),
        br#"{"thresholds":[]}"#,
        br#"{"scrub":[]}"#,
        br#"{"schedule":[]}"#,
    ] {
        assert!(
            import_legacy_snapraid(bytes, &PoolsConfig::default())
                .configuration
                .is_none()
        );
    }
}
#[test]
fn diff_parses_complete_human_and_tag_outputs_to_the_same_counts() {
    let human = parse_snapraid_diff(&capture(HUMAN)).unwrap();
    let tags = parse_snapraid_diff(&capture(TAGS)).unwrap();
    assert_eq!(human, tags);
    assert_eq!(human.removed, 2);
    assert_eq!(human.updated, 3);
}
#[test]
fn diff_empty_partial_malformed_ambiguous_overflow_or_invalid_text_fails_closed() {
    for text in [
        "",
        "No error detected.",
        "51 removed\n0 updated\n",
        "summary:exit:equal\n",
        "garbage removed\n",
        "-1 removed\n",
        "0 removed\n0 removed\n",
        "18446744073709551616 removed\n",
        "0 removed\n\u{1b}[31m",
        "summary:exit:error\n",
        "summary:removed:0:extra\n",
    ] {
        assert!(parse_snapraid_diff(&capture(text)).is_err(), "{text:?}");
    }
    assert!(parse_snapraid_diff(&capture(&format!("{HUMAN}0 removed\n"))).is_err());
    assert!(parse_snapraid_diff(&capture(&TAGS.replace("exit:diff", "exit:equal"))).is_err());
    let mut invalid = capture(HUMAN);
    invalid.stdout.push(255);
    assert!(parse_snapraid_diff(&invalid).is_err());
    let mut truncated = capture(HUMAN);
    truncated.stdout_truncated = true;
    let failure = parse_snapraid_diff(&truncated).unwrap_err();
    assert_eq!(
        failure.termination,
        Some(StorageReadTermination::Exited { code: 0 })
    );
    assert_eq!(failure.reason, StorageReadFailureReason::OutputLimit);
    let mut huge = capture(HUMAN);
    huge.stderr = vec![b'x'; STORAGE_READ_MAX_BYTES];
    assert!(parse_snapraid_diff(&huge).is_err());
}
#[test]
fn mfs001_result_handling_preserves_failure_status_despite_simulated_success_text() {
    for status in [
        StorageReadTermination::Exited { code: 17 },
        StorageReadTermination::TimedOut,
        StorageReadTermination::MissingBinary,
        StorageReadTermination::Signaled { signal: 9 },
        StorageReadTermination::IoError,
        StorageReadTermination::OutputLimit,
    ] {
        let mut result = capture(HUMAN);
        result.termination = status.clone();
        result.stderr = b"real process error".to_vec();
        let error = parse_snapraid_diff(&result).unwrap_err();
        assert_eq!(error.termination, Some(status.clone()));
        assert_eq!(error.detail, "real process error");
        result.stdout = b"No error detected.\nsummary:exit:ok\n".to_vec();
        assert_eq!(
            parse_snapraid_status(&result).unwrap_err().termination,
            Some(status)
        );
    }
}
#[test]
fn failed_diff_evidence_remains_unavailable_with_real_error_status() {
    let config: SnapraidConfig = serde_json::from_value(fixtures()[2]["snapraid"].clone()).unwrap();
    let sources = ProtectionSourceEvidence {
        observed_at: 100,
        inventory: StorageInventory {
            host_root_mount_id: 1,
            topology_digest: "a".repeat(64),
            mounts_digest: "b".repeat(64),
            fstab_digest: "c".repeat(64),
            fstab_entries: vec![],
            devices: vec![],
        },
        paths: vec![],
    };
    let mut result = capture(HUMAN);
    result.termination = StorageReadTermination::TimedOut;
    let evidence = snapraid_diff_evidence(&config, &sources, 101, &result);
    assert!(matches!(
        evidence.outcome,
        SnapraidDiffOutcome::Unavailable(StorageReadFailure {
            termination: Some(StorageReadTermination::TimedOut),
            ..
        })
    ));
    assert_eq!(evidence.configuration, config);
    assert_eq!(evidence.mounts_digest, sources.inventory.mounts_digest);
}
#[test]
fn status_unknown_and_nonzero_results_never_become_healthy() {
    assert_eq!(
        parse_snapraid_status(&capture("No error detected.\n"))
            .unwrap()
            .health,
        SnapraidHealth::Healthy
    );
    assert_eq!(
        parse_snapraid_status(&capture("Sync required\n"))
            .unwrap()
            .health,
        SnapraidHealth::SyncRequired
    );
    let error = parse_snapraid_status(&capture("2 missing\nDamaged file 3\n")).unwrap();
    assert_eq!(error.health, SnapraidHealth::Errors);
    assert_eq!(error.missing_files, Some(2));
    assert_eq!(error.damaged_files, Some(3));
    for text in [
        "",
        "Status unknown",
        "All data protected",
        "summary:exit:ok",
        "bad missing\n",
        "0 missing\n0 missing\n",
    ] {
        assert!(parse_snapraid_status(&capture(text)).is_err());
    }
}
#[test]
fn logs_preserve_numeric_summary_scan_progress_and_unescape_without_path_or_argv_leaks() {
    let view = parse_snapraid_log(include_bytes!(
        "../../../../tests/fixtures/pool-protection/log-tags.txt"
    ))
    .unwrap();
    assert_eq!(view.summary["added"], 5);
    assert_eq!(view.scan_counts["add"], 2);
    assert_eq!(view.scan_counts["remove"], 1);
    assert_eq!(view.exit.as_deref(), Some("diff"));
    let progress = view.progress.unwrap();
    assert_eq!(progress.percent, 40);
    assert_eq!(progress.size_speed, 15.5);
    assert_eq!(progress.cpu, 3.2);
    assert_eq!(view.messages[0].message, "hello:world");
    assert_eq!(view.ignored_lines, 1);
    assert!(!format!("{:?}", view.messages).contains("Movies"));
    let escaped = parse_snapraid_log(br"msg:status:literal\\d colon\dline\nnext").unwrap();
    assert_eq!(escaped.messages[0].message, "literal\\d colon:line\nnext");
}
#[test]
fn logs_reject_malformed_counts_nonfinite_progress_and_resource_limits() {
    for bytes in [
        b"summary:added:bad".as_slice(),
        b"summary:added:1\nsummary:added:1",
        b"summary:exit:fake",
        b"run:pos:10:20:300:101:120:15.5:3.2:900",
        b"run:pos:10:20:300:40:120:NaN:3.2:900",
        b"run:pos:bad",
        &[255],
    ] {
        assert!(parse_snapraid_log(bytes).is_err());
    }
    assert!(parse_snapraid_log(&vec![b'x'; STORAGE_READ_MAX_BYTES + 1]).is_err());
    assert_eq!(
        parse_snapraid_log(b"summary:added:bad")
            .unwrap_err()
            .termination,
        None
    );
    assert!(parse_snapraid_log("ignored\n".repeat(STORAGE_LOG_MAX_EVENTS + 1).as_bytes()).is_err());
}
#[test]
fn sanitization_removes_controls_and_credentials_from_process_errors_and_log_messages() {
    let mut result = capture(HUMAN);
    result.termination = StorageReadTermination::Exited { code: 1 };
    result.stderr =
        b"\x1b[31mfailed\x1b[0m\npassword=secret-value\n\x1b]0;title\x07safe\n".to_vec();
    let error = parse_snapraid_diff(&result).unwrap_err();
    assert_eq!(
        error.detail,
        "failed\n[credential-bearing line redacted]\nsafe"
    );
    let log = parse_snapraid_log(br"msg:error:Bearer private-token").unwrap();
    assert_eq!(
        log.messages[0].message,
        "[credential-bearing line redacted]"
    );
    assert!(sanitize_storage_text("ééé", 5).len() <= 5);
}
#[test]
fn pool_status_distinguishes_missing_disabled_wrong_filesystem_and_invalid_capacity() {
    let config: PoolsConfig = serde_json::from_value(fixtures()[2]["pools"].clone()).unwrap();
    assert_eq!(
        pool_status(&config, &[]).unwrap()[0].health,
        PoolReadHealth::Missing
    );
    let m = PoolMountObservation {
        mountpoint: "/mnt/storage".into(),
        filesystem: "ext4".into(),
        mount_id: 100,
        total_bytes: Some(100),
        free_bytes: Some(50),
    };
    assert_eq!(
        pool_status(&config, std::slice::from_ref(&m)).unwrap()[0].health,
        PoolReadHealth::Unknown
    );
    let mut m = m;
    m.filesystem = "fuse.mergerfs".into();
    assert_eq!(
        pool_status(&config, std::slice::from_ref(&m)).unwrap()[0].free_bytes,
        Some(50)
    );
    m.free_bytes = Some(101);
    assert_eq!(
        pool_status(&config, std::slice::from_ref(&m)).unwrap()[0].free_bytes,
        None
    );
    assert_eq!(
        pool_status(&config, &[m.clone(), m]).unwrap()[0].health,
        PoolReadHealth::Unknown
    );
    let mut config = config;
    config.pools[0].enabled = false;
    assert_eq!(
        pool_status(&config, &[]).unwrap()[0].health,
        PoolReadHealth::Disabled
    );
}
