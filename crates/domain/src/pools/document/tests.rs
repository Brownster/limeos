use super::*;
use crate::pools::tests::{fixtures, protected};

fn config() -> PoolsConfig {
    protected().1
}
fn root() -> &'static str {
    "# operator credential=private-value\nUUID=root / ext4 defaults 0 1\nUUID=boot /boot vfat defaults 0 2\n/dev/swap none swap sw 0 0\n"
}
fn error(
    source: &[u8],
    previous: Option<&PoolsConfig>,
    desired: &PoolsConfig,
) -> StoragePlanningFindingCode {
    plan_document_edit(source, previous, desired).unwrap_err()[0].code
}
fn row(target: &str) -> String {
    format!("UUID=other {target} ext4 defaults 0 2\n")
}
fn pad(length: usize) -> String {
    let mut text = String::new();
    while text.len() < length {
        let size = (length - text.len()).min(POOL_FSTAB_MAX_LINE_BYTES);
        if size == 1 {
            text.push('\n');
        } else {
            text.push('#');
            text.extend(std::iter::repeat_n('x', size - 2));
            text.push('\n');
        }
    }
    text
}

#[test]
fn append_each_frozen_layout_preserves_operator_bytes_and_exact_rendering() {
    for case in fixtures() {
        let desired: PoolsConfig = serde_json::from_value(case["pools"].clone()).unwrap();
        let edit = plan_document_edit(root().as_bytes(), None, &desired).unwrap();
        let expected = format!("{}{}", root(), case["fstab"].as_str().unwrap());
        assert_eq!(edit.candidate(), expected.as_bytes(), "{}", case["name"]);
        assert_eq!(edit.summary().external_entries, 3);
        assert!(edit.summary().changed);
        assert!(!edit.summary().replacing_managed_block);
        assert_eq!(edit.desired(), &desired);
    }
}
#[test]
fn identical_previous_configuration_is_a_noop() {
    let desired = config();
    let source = format!("{}{}", root(), desired.render_fstab().unwrap());
    let edit = plan_document_edit(source.as_bytes(), Some(&desired), &desired).unwrap();
    assert_eq!(edit.candidate(), source.as_bytes());
    assert!(!edit.summary().changed);
    assert!(edit.summary().replacing_managed_block);
    assert_eq!(edit.summary().enabled_pools, 1);
}
#[test]
fn replace_preserves_storage_section_comments_options_and_suffix_order() {
    let previous = config();
    let mut desired = previous.clone();
    desired.pools[0].options.min_free_space = Some("100GiB".into());
    let prefix = "# BEGIN LIMEOS STORAGE V1\nUUID=disk1 /mnt/disk1 ext4 x-operator=value 0 2\n# END LIMEOS STORAGE V1\n";
    let suffix = "server:/data /mnt/Other\\040Data nfs credentials=private-value 0 0 # keep this\n";
    let source = format!("{prefix}{}{suffix}", previous.render_fstab().unwrap());
    let expected = format!("{prefix}{}{suffix}", desired.render_fstab().unwrap());
    let edit = plan_document_edit(source.as_bytes(), Some(&previous), &desired).unwrap();
    assert_eq!(edit.candidate(), expected.as_bytes());
    assert!(edit.summary().changed);
    assert_eq!(edit.summary().external_entries, 2);
}
#[test]
fn disable_and_remove_only_change_the_owned_block() {
    let previous = config();
    let source = format!(
        "{}{}# operator EOF",
        root(),
        previous.render_fstab().unwrap()
    );
    let mut disabled = previous.clone();
    disabled.pools[0].enabled = false;
    for desired in [&disabled, &PoolsConfig::default()] {
        let edit = plan_document_edit(source.as_bytes(), Some(&previous), desired).unwrap();
        let expected = format!(
            "{}{}# operator EOF",
            root(),
            desired.render_fstab().unwrap()
        );
        assert_eq!(edit.candidate(), expected.as_bytes());
        assert_eq!(edit.summary().enabled_pools, 0);
    }
}
#[test]
fn empty_marked_configuration_still_requires_a_previous_binding() {
    let empty = PoolsConfig::default();
    let source = empty.render_fstab().unwrap();
    assert_eq!(
        error(source.as_bytes(), None, &empty),
        StoragePlanningFindingCode::UnverifiedSource
    );
    assert!(plan_document_edit(source.as_bytes(), Some(&empty), &empty).is_ok());
    assert_eq!(
        error(b"", Some(&empty), &empty),
        StoragePlanningFindingCode::UnverifiedSource
    );
}
#[test]
fn external_targets_cannot_collide_with_disabled_removed_or_renamed_pools() {
    let previous = config();
    let colliding = row(&previous.pools[0].mount_point);
    let mut disabled = previous.clone();
    disabled.pools[0].enabled = false;
    assert_eq!(
        error(colliding.as_bytes(), None, &disabled),
        StoragePlanningFindingCode::Duplicate
    );
    let source = format!("{}{colliding}", previous.render_fstab().unwrap());
    assert_eq!(
        error(source.as_bytes(), Some(&previous), &PoolsConfig::default()),
        StoragePlanningFindingCode::Duplicate
    );
    let mut renamed = previous.clone();
    renamed.pools[0].mount_point = "/mnt/renamed".into();
    assert_eq!(
        error(source.as_bytes(), Some(&previous), &renamed),
        StoragePlanningFindingCode::Duplicate
    );
    let source = format!("{}{}", disabled.render_fstab().unwrap(), colliding);
    assert_eq!(
        error(source.as_bytes(), Some(&disabled), &PoolsConfig::default()),
        StoragePlanningFindingCode::Duplicate
    );
}
#[test]
fn external_targets_beneath_a_pool_block_even_when_not_equal() {
    let desired = config();
    for target in [
        "/mnt/storage",
        "/mnt/storage/Movies",
        "/mnt/storage/Movies/Sub",
    ] {
        assert_eq!(
            error(row(target).as_bytes(), None, &desired),
            StoragePlanningFindingCode::Duplicate
        );
    }
}
#[test]
fn root_parent_case_and_component_prefix_neighbours_are_preserved() {
    let desired = config();
    let source = format!(
        "{}{}{}{}",
        root(),
        row("/mnt"),
        row("/mnt/Storage"),
        row("/mnt/storage-other")
    );
    let edit = plan_document_edit(source.as_bytes(), None, &desired).unwrap();
    assert!(edit.candidate().starts_with(source.as_bytes()));
    assert_eq!(edit.summary().external_entries, 6);
}
#[test]
fn octal_encoded_letters_slashes_and_utf8_cannot_hide_collisions() {
    let desired = config();
    for target in ["/mnt/\\163torage", "/mnt\\057storage", "\\057mnt/storage"] {
        assert_eq!(
            error(row(target).as_bytes(), None, &desired),
            StoragePlanningFindingCode::Duplicate
        );
    }
    let mut unicode = desired;
    unicode.pools[0].mount_point = "/mnt/Données".into();
    assert_eq!(
        error(row("/mnt/Donn\\303\\251es").as_bytes(), None, &unicode),
        StoragePlanningFindingCode::Duplicate
    );
}
#[test]
fn escaped_foreign_space_tab_and_backslash_survive_without_recursive_decoding() {
    let desired = config();
    let source = format!(
        "{}{}{}",
        row("/mnt/Other\\040Data"),
        row("/mnt/Other\\011Data"),
        row("/mnt/\\134163torage")
    );
    let edit = plan_document_edit(source.as_bytes(), None, &desired).unwrap();
    assert!(edit.candidate().starts_with(source.as_bytes()));
}
#[test]
fn unmarked_matching_rows_and_legacy_blocks_are_never_adopted() {
    let desired = config();
    let rendered = desired.render_fstab().unwrap();
    let unmarked = rendered
        .replace(&format!("{BEGIN}\n"), "")
        .replace(&format!("{END}\n"), "");
    assert_eq!(
        error(unmarked.as_bytes(), None, &desired),
        StoragePlanningFindingCode::Duplicate
    );
    for source in [
        "# pi-health mergerfs start\n# pi-health mergerfs end\n",
        " # PI-HEALTH MERGERFS start\n",
    ] {
        assert_eq!(
            error(source.as_bytes(), None, &desired),
            StoragePlanningFindingCode::Unsupported
        );
    }
}
#[test]
fn partial_nested_duplicate_unknown_and_indented_markers_refuse() {
    let desired = config();
    let original = desired.render_fstab().unwrap();
    for source in [
        format!("{BEGIN}\n"),
        format!("{END}\n"),
        format!("{BEGIN}\n{BEGIN}\n{END}\n"),
        format!("{original}{original}"),
        original.replace("MERGERFS V1", "MERGERFS V2"),
        original.replace(BEGIN, " # BEGIN LIMEOS MERGERFS V1"),
        original.replace(BEGIN, "# begin limeos mergerfs v1"),
    ] {
        assert!(
            plan_document_edit(source.as_bytes(), Some(&desired), &desired).is_err(),
            "{source}"
        );
    }
}
#[test]
fn previous_ownership_claim_must_match_the_actual_section() {
    let previous = config();
    let original = previous.render_fstab().unwrap();
    assert_eq!(
        error(original.as_bytes(), None, &previous),
        StoragePlanningFindingCode::UnverifiedSource
    );
    for source in [
        original.replace("minfreespace=4G", "minfreespace=9G"),
        original.replace("defaults,", "ro,"),
        original.replace("# mergerfs pool:", "# operator changed:"),
        original.replace(" 0 0", " 0 0 # operator"),
    ] {
        assert_ne!(source, original);
        assert_eq!(
            error(source.as_bytes(), Some(&previous), &previous),
            StoragePlanningFindingCode::UnverifiedSource
        );
    }
}
#[test]
fn exact_byte_recheck_and_count_only_summary_hide_operator_secrets() {
    let desired = config();
    let source = format!("{}{}", root(), desired.render_fstab().unwrap());
    let edit = plan_document_edit(source.as_bytes(), Some(&desired), &desired).unwrap();
    assert!(edit.check_current(&source.clone().into_bytes()).is_ok());
    assert!(
        edit.check_current(format!("{source}# changed\n").as_bytes())
            .is_err()
    );
    for output in [
        format!("{edit:?}"),
        serde_json::to_string(&edit.summary()).unwrap(),
    ] {
        assert!(!output.contains("private-value"));
        assert!(!output.contains("UUID=root"));
        assert!(!output.contains("/mnt/storage"));
    }
}
#[test]
fn append_adds_only_the_required_line_separator() {
    let desired = config();
    for source in ["", "# EOF", "# line\n# EOF", "# line\r\n# EOF"] {
        let edit = plan_document_edit(source.as_bytes(), None, &desired).unwrap();
        let crlf = source.contains("\r\n");
        let rendered = if crlf {
            desired.render_fstab().unwrap().replace('\n', "\r\n")
        } else {
            desired.render_fstab().unwrap()
        };
        let separator = if source.is_empty() {
            ""
        } else if crlf {
            "\r\n"
        } else {
            "\n"
        };
        assert_eq!(
            edit.candidate(),
            format!("{source}{separator}{rendered}").as_bytes()
        );
    }
}
#[test]
fn crlf_owned_block_and_missing_final_newline_survive_replacement() {
    let previous = config();
    let mut desired = previous.clone();
    desired.pools[0].options.min_free_space = Some("20G".into());
    let old = previous.render_fstab().unwrap().replace('\n', "\r\n");
    let new = desired.render_fstab().unwrap().replace('\n', "\r\n");
    for ending in [true, false] {
        let source = if ending {
            old.as_str()
        } else {
            old.strip_suffix("\r\n").unwrap()
        };
        let expected = if ending {
            new.as_str()
        } else {
            new.strip_suffix("\r\n").unwrap()
        };
        assert_eq!(
            plan_document_edit(source.as_bytes(), Some(&previous), &desired)
                .unwrap()
                .candidate(),
            expected.as_bytes()
        );
    }
}
#[test]
fn mixed_external_endings_do_not_reformat_an_lf_owned_block() {
    let previous = config();
    let mut desired = previous.clone();
    desired.pools[0].enabled = false;
    let source = format!(
        "# prefix\r\n{}# suffix\r\n",
        previous.render_fstab().unwrap()
    );
    let expected = format!(
        "# prefix\r\n{}# suffix\r\n",
        desired.render_fstab().unwrap()
    );
    assert_eq!(
        plan_document_edit(source.as_bytes(), Some(&previous), &desired)
            .unwrap()
            .candidate(),
        expected.as_bytes()
    );
}
#[test]
fn bare_cr_invalid_utf8_nul_and_controls_refuse() {
    let desired = config();
    for source in [
        b"# a\rb\n".as_slice(),
        b"# a\r",
        b"# a\0\n",
        b"# a\x1b\n",
        b"\xff",
    ] {
        assert!(plan_document_edit(source, None, &desired).is_err());
    }
}
#[test]
fn physical_line_continuations_and_ambiguous_targets_refuse() {
    let desired = config();
    for source in ["# comment \\\n", "UUID=x /mnt/other ext4 defaults 0 0 \\\n"] {
        assert_eq!(
            error(source.as_bytes(), None, &desired),
            StoragePlanningFindingCode::Unsupported
        );
    }
    for target in [
        "/mnt//storage",
        "/mnt/storage/",
        "/mnt/other/../storage",
        "/mnt/storage/.",
        "relative",
        "\"/mnt/storage\"",
        "/mnt/\\x20",
        "/mnt/\\04",
        "/mnt/\\777",
        "/mnt/\\000",
        "/mnt/\\012",
    ] {
        assert_eq!(
            error(row(target).as_bytes(), None, &desired),
            StoragePlanningFindingCode::Unsupported,
            "{target}"
        );
    }
}
#[test]
fn closed_six_field_profile_keeps_opaque_options_and_inline_comments() {
    let desired = config();
    let source = "LABEL=\"Other\\040Data\"\t/mnt/Other\text4\tcredentials=private-value,x-opt=%literal\t0\t2\t# operator comment\n";
    assert!(
        plan_document_edit(source.as_bytes(), None, &desired)
            .unwrap()
            .candidate()
            .starts_with(source.as_bytes())
    );
    for source in [
        "UUID=x /mnt/other ext4\n",
        "UUID=x /mnt/other ext4 defaults\n",
        "UUID=x /mnt/other ext4 defaults 0\n",
        "UUID=x /mnt/other ext4 defaults 0 0 extra\n",
        "UUID=x /mnt/other ext4 defaults +1 0\n",
        "UUID=x /mnt/other ext4 defaults 0 invalid\n",
        "UUID=x /mnt/other ext4 defaults 4294967296 0\n",
        "; not an fstab comment\n",
    ] {
        assert_eq!(
            error(source.as_bytes(), None, &desired),
            StoragePlanningFindingCode::Unsupported
        );
    }
}
#[test]
fn total_entry_budget_includes_old_and_new_managed_rows() {
    let desired = config();
    let external: String = (0..127)
        .map(|i| row(&format!("/mnt/external{i}")))
        .collect();
    assert_eq!(
        plan_document_edit(external.as_bytes(), None, &desired)
            .unwrap()
            .summary()
            .external_entries,
        127
    );
    let external = external + &row("/mnt/external127");
    assert_eq!(
        error(external.as_bytes(), None, &desired),
        StoragePlanningFindingCode::ThresholdExceeded
    );
    let empty = PoolsConfig::default();
    assert!(plan_document_edit(external.as_bytes(), None, &empty).is_ok());
    let source = format!("{external}{}", desired.render_fstab().unwrap());
    assert_eq!(
        error(source.as_bytes(), Some(&desired), &empty),
        StoragePlanningFindingCode::ThresholdExceeded
    );
}
#[test]
fn source_and_candidate_byte_boundaries_are_distinct() {
    let empty = PoolsConfig::default();
    let rendered = empty.render_fstab().unwrap();
    let source = pad(POOL_FSTAB_MAX_BYTES - rendered.len());
    assert_eq!(
        plan_document_edit(source.as_bytes(), None, &empty)
            .unwrap()
            .candidate()
            .len(),
        POOL_FSTAB_MAX_BYTES
    );
    let larger = pad(source.len() + 1);
    assert_eq!(
        error(larger.as_bytes(), None, &empty),
        StoragePlanningFindingCode::ThresholdExceeded
    );
    let too_large = pad(POOL_FSTAB_MAX_BYTES + 1);
    assert_eq!(
        plan_document_edit(too_large.as_bytes(), None, &empty).unwrap_err()[0].field,
        "pools.fstab.bytes"
    );
}
#[test]
fn exact_full_document_noop_and_eof_owned_block_are_admitted() {
    let empty = PoolsConfig::default();
    for newline in [true, false] {
        let owned = empty.render_fstab().unwrap();
        let owned = if newline {
            owned.as_str()
        } else {
            owned.strip_suffix('\n').unwrap()
        };
        let source = format!("{}{owned}", pad(POOL_FSTAB_MAX_BYTES - owned.len()));
        let edit = plan_document_edit(source.as_bytes(), Some(&empty), &empty).unwrap();
        assert_eq!(edit.candidate().len(), POOL_FSTAB_MAX_BYTES);
        assert!(!edit.summary().changed);
    }
}
#[test]
fn physical_and_rendered_line_limits_are_bounded_before_publication() {
    let desired = config();
    let boundary = format!("#{}\n", "x".repeat(POOL_FSTAB_MAX_LINE_BYTES - 1));
    assert!(plan_document_edit(boundary.as_bytes(), None, &desired).is_ok());
    let larger = format!("#{}\n", "x".repeat(POOL_FSTAB_MAX_LINE_BYTES));
    assert_eq!(
        error(larger.as_bytes(), None, &desired),
        StoragePlanningFindingCode::ThresholdExceeded
    );
    let mut long = desired;
    long.pools[0].branches = (0..16)
        .map(|i| format!("/mnt/disk{i}/{}", "x".repeat(300)))
        .collect();
    assert!(
        long.render_fstab()
            .unwrap()
            .lines()
            .any(|line| line.len() > POOL_FSTAB_MAX_LINE_BYTES)
    );
    assert_eq!(
        error(b"", None, &long),
        StoragePlanningFindingCode::ThresholdExceeded
    );
}
#[test]
fn decoded_target_length_boundary_is_not_encoded_length() {
    let desired = config();
    let boundary = format!("/mnt/{}", "x".repeat(POOL_FSTAB_MAX_TARGET_BYTES - 5));
    assert!(plan_document_edit(row(&boundary).as_bytes(), None, &desired).is_ok());
    let encoded = boundary.replacen('/', "\\057", 1);
    assert!(plan_document_edit(row(&encoded).as_bytes(), None, &desired).is_ok());
    assert_eq!(
        error(row(&(boundary + "x")).as_bytes(), None, &desired),
        StoragePlanningFindingCode::ThresholdExceeded
    );
}
#[test]
fn malformed_previous_or_desired_configuration_is_not_a_document_ownership_escape() {
    let valid = config();
    let mut invalid = valid.clone();
    invalid.pools[0].branches[0] = "/mnt/storage/recursive".into();
    let original = valid.render_fstab().unwrap();
    assert!(plan_document_edit(b"", None, &invalid).is_err());
    assert!(plan_document_edit(original.as_bytes(), Some(&invalid), &valid).is_err());
}
#[test]
fn unrelated_operator_mergerfs_row_is_preserved_without_adoption() {
    let desired = config();
    let source = "/mnt/operator1:/mnt/operator2 /mnt/foreign fuse.mergerfs defaults,category.create=ff 0 0\n";
    let edit = plan_document_edit(source.as_bytes(), None, &desired).unwrap();
    assert!(edit.candidate().starts_with(source.as_bytes()));
    assert_eq!(edit.summary().external_entries, 1);
}

#[test]
fn configuration_shape_and_strings_refuse_before_unbounded_validation_work() {
    let valid = config();
    for kind in 0..8 {
        let mut oversized = valid.clone();
        let pool = &mut oversized.pools[0];
        match kind {
            0 => oversized.pools = vec![valid.pools[0].clone(); 33],
            1 => pool.branches = vec!["/mnt/data".into(); 33],
            2 => pool.id = "x".repeat(65),
            3 => pool.name = "x".repeat(65),
            4 => pool.mount_point = "x".repeat(513),
            5 => pool.branches[0] = "x".repeat(513),
            6 => pool.min_free_space = "1".repeat(33),
            _ => pool.options.min_free_space = Some("1".repeat(33)),
        }
        let findings = plan_document_edit(b"", None, &oversized).unwrap_err();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].field, "pools.fstab.configuration");
        assert_eq!(
            findings[0].code,
            StoragePlanningFindingCode::ThresholdExceeded
        );
        let source = valid.render_fstab().unwrap();
        assert_eq!(
            error(source.as_bytes(), Some(&oversized), &valid),
            StoragePlanningFindingCode::ThresholdExceeded
        );
    }
}
