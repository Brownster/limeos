use super::*;
use serde_json::json;

const SAMPLE: &[u8] = include_bytes!("../../../../tests/fixtures/samba/sample.json");
const IMPORTED: &[u8] = include_bytes!("../../../../tests/fixtures/samba/imported.json");

#[test]
fn frozen_sample_and_imported_share_fixtures_match_section_goldens() {
    for (bytes, sections, defaults) in [
        (
            SAMPLE,
            include_str!("../../../../tests/fixtures/samba/sample.sections"),
            3,
        ),
        (
            IMPORTED,
            include_str!("../../../../tests/fixtures/samba/imported.sections"),
            0,
        ),
    ] {
        let before = bytes.to_vec();
        let report = import_legacy_samba(bytes);
        let config = report.configuration.unwrap();
        assert_eq!(config.render_sections().unwrap(), sections);
        assert_eq!(report.findings.len(), defaults);
        assert!(
            report
                .findings
                .iter()
                .all(|f| f.code == StoragePlanningFindingCode::DefaultApplied)
        );
        assert_eq!(bytes, before);
        if defaults == 0 {
            assert_eq!(
                serde_json::to_value(&config).unwrap(),
                serde_json::from_slice::<Value>(bytes).unwrap()
            );
            assert!(!config.shares[1].enabled);
            assert!(config.shares[1].read_only);
        }
    }
}

#[test]
fn each_frozen_default_is_reported_and_explicit_values_do_not_default() {
    let report = import_legacy_samba(br#"{"shares":[{"name":"media","path":"/srv/docs"}]}"#);
    let fields: Vec<_> = report.findings.iter().map(|f| f.field.as_str()).collect();
    assert_eq!(
        fields,
        [
            "samba.shares[0].comment",
            "samba.shares[0].read_only",
            "samba.shares[0].guest_ok",
            "samba.shares[0].browseable",
            "samba.shares[0].valid_users",
            "samba.shares[0].enabled"
        ]
    );
    let share = &report.configuration.unwrap().shares[0];
    assert!(!share.read_only && !share.guest_ok && share.browseable && share.enabled);
    assert!(share.comment.is_empty() && share.valid_users.is_empty());
    let empty = import_legacy_samba(b"{}");
    assert_eq!(empty.configuration, Some(SambaSharesConfig::default()));
    assert_eq!(
        empty.findings,
        [issue(
            "samba.shares",
            StoragePlanningFindingCode::DefaultApplied
        )]
    );
    assert!(import_legacy_samba(br#"{"shares":[]}"#).findings.is_empty());
    let mut explicit = serde_json::from_slice::<Value>(IMPORTED).unwrap();
    explicit["shares"][0]["browseable"] = false.into();
    explicit["shares"][0]["enabled"] = false.into();
    let report = import_legacy_samba(&serde_json::to_vec(&explicit).unwrap());
    assert!(report.findings.is_empty());
    assert!(!report.configuration.unwrap().shares[0].browseable);
}

#[test]
fn renderer_trimming_is_preserved_but_names_and_paths_are_never_repaired() {
    let mut value = serde_json::from_slice::<Value>(IMPORTED).unwrap();
    value["shares"][0]["comment"] = "  Movies & TV  ".into();
    value["shares"][0]["valid_users"] = "  pi,media  ".into();
    let report = import_legacy_samba(&serde_json::to_vec(&value).unwrap());
    let config = report.configuration.unwrap();
    assert_eq!(config.shares[0].comment, "Movies & TV");
    assert_eq!(config.shares[0].valid_users, "pi,media");
    for field in ["name", "path"] {
        let mut value = value.clone();
        value["shares"][0][field] =
            format!(" {} ", value["shares"][0][field].as_str().unwrap()).into();
        assert!(
            import_legacy_samba(&serde_json::to_vec(&value).unwrap())
                .configuration
                .is_none()
        );
    }
}

#[test]
fn malformed_unknown_duplicate_and_sequence_shaped_inputs_refuse_without_values() {
    for bytes in [
        b"".as_slice(),
        b"[]",
        b"[[]]",
        b"null",
        b"[invalid]",
        b"{}{}",
        br#"{"shares":null}"#,
        br#"{"shares":{}}"#,
        br#"{"shares":[null]}"#,
        br#"{"shares":[["media","/srv/docs","",false,false,true,"",true]]}"#,
        br#"{"shares":[],"shares":[]}"#,
        br#"{"shares":[{"name":"a","name":"b","path":"/srv/docs"}]}"#,
        br#"{"shares":[{"name":"a","path":"/srv/a","path":"/srv/b"}]}"#,
        br#"{"shares":[],"password":"private-value"}"#,
        br#"{"shares":[{"name":"a","path":"/srv/docs","force_user":"private-value"}]}"#,
        br#"{"shares":[{"name":"a"}]}"#,
        br#"{"shares":[{"name":"a","path":"/srv/docs","enabled":"false"}]}"#,
        br#"{"shares":[{"name":42,"path":"/srv/docs"}]}"#,
        br#"{"shares":[{"name":"a","path":"/srv/docs","comment":null}]}"#,
        &[255],
    ] {
        let report = import_legacy_samba(bytes);
        assert!(report.configuration.is_none(), "{bytes:?}");
        assert!(!report.findings.is_empty());
        assert!(!format!("{:?}", report.findings).contains("private-value"));
    }
    assert_eq!(
        import_legacy_samba(br#"{"include":"/tmp/private-value"}"#).findings[0].code,
        StoragePlanningFindingCode::Unsupported
    );
}

#[test]
fn every_boolean_requires_a_json_boolean_without_truthiness_or_null_defaults() {
    for field in ["read_only", "guest_ok", "browseable", "enabled"] {
        for value in [
            json!("yes"),
            json!("false"),
            json!(1),
            json!(null),
            json!([]),
        ] {
            let mut config = serde_json::from_slice::<Value>(IMPORTED).unwrap();
            config["shares"][0][field] = value;
            assert!(
                import_legacy_samba(&serde_json::to_vec(&config).unwrap())
                    .configuration
                    .is_none(),
                "{field}"
            );
        }
    }
}

#[test]
fn invalid_disabled_share_blocks_whole_import_without_partial_configuration() {
    let mut config = serde_json::from_slice::<Value>(IMPORTED).unwrap();
    config["shares"][1]["path"] = "/srv/docs\n[global]".into();
    let report = import_legacy_samba(&serde_json::to_vec(&config).unwrap());
    assert!(report.configuration.is_none());
    assert_eq!(report.findings[0].field, "samba.shares[1].path");
    config["shares"][1]["path"] = "/srv/docs".into();
    config["shares"][1]["name"] = "MEDIA".into();
    assert!(
        import_legacy_samba(&serde_json::to_vec(&config).unwrap())
            .configuration
            .is_none()
    );
}

#[test]
fn byte_and_record_bounds_accept_exact_limits_and_refuse_over_limit() {
    let mut exact = br#"{"shares":[]}"#.to_vec();
    exact.resize(STORAGE_IMPORT_MAX_BYTES, b' ');
    assert!(import_legacy_samba(&exact).configuration.is_some());
    exact.push(b' ');
    assert_eq!(
        import_legacy_samba(&exact).findings[0].code,
        StoragePlanningFindingCode::ThresholdExceeded
    );
    let mut shares: Vec<_> = (0..64)
        .map(|i| json!({"name":format!("s{i}"),"path":"/srv/docs"}))
        .collect();
    let report = import_legacy_samba(&serde_json::to_vec(&json!({"shares":shares})).unwrap());
    assert_eq!(report.configuration.unwrap().shares.len(), 64);
    assert_eq!(report.findings.len(), 384);
    shares.push(json!({"name":"over","path":"/srv/docs"}));
    assert!(
        import_legacy_samba(&serde_json::to_vec(&json!({"shares":shares})).unwrap())
            .configuration
            .is_none()
    );
}

#[test]
fn unsupported_user_groups_domains_substitutions_and_embedded_lines_refuse() {
    for users in [
        "@media",
        "DOMAIN\\pi",
        "%U",
        "pi media",
        "pi,\nroot",
        "pi,\rroot",
        "pi,\u{00a0}root",
        "pi,pi",
        "pi,,media",
    ] {
        let mut config = serde_json::from_slice::<Value>(IMPORTED).unwrap();
        config["shares"][0]["valid_users"] = users.into();
        let report = import_legacy_samba(&serde_json::to_vec(&config).unwrap());
        assert!(report.configuration.is_none(), "{users:?}");
        assert_eq!(
            report.findings[0].code,
            StoragePlanningFindingCode::Unsupported
        );
    }
}
