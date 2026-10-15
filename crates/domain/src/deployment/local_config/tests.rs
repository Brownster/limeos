use super::*;
use serde_json::Value;

const TRANSMISSION: &[u8] =
    include_bytes!("../../../../../tests/fixtures/deployment/local-config/transmission.json");
const SAB: &[u8] =
    include_bytes!("../../../../../tests/fixtures/deployment/local-config/sabnzbd.ini");
fn dirs() -> DownloadDirectories {
    DownloadDirectories {
        complete: "/downloads/complete".into(),
        incomplete: Some("/downloads/incomplete".into()),
    }
}
fn draft(app: Application, source: &[u8]) -> ConfigDraft {
    plan(app, &dirs(), Some(source)).unwrap()
}
fn artifact(value: &ConfigDraft) -> &str {
    std::str::from_utf8(value.artifact().unwrap()).unwrap()
}
fn json(value: &ConfigDraft) -> Value {
    serde_json::from_slice(value.artifact().unwrap()).unwrap()
}

#[test]
fn frozen_transmission_directory_rule_preserves_other_values_and_exact_lexemes() {
    let value = draft(Application::TransmissionLegacy, TRANSMISSION);
    let output = artifact(&value);
    let parsed = json(&value);
    assert_eq!(
        value.summary().changed,
        [
            Field::CompleteDirectory,
            Field::IncompleteDirectory,
            Field::IncompleteEnabled
        ]
    );
    assert_eq!(parsed["download-dir"], "/downloads/complete");
    assert_eq!(parsed["incomplete-dir"], "/downloads/incomplete");
    assert_eq!(parsed["incomplete-dir-enabled"], true);
    assert_eq!(parsed["rpc-enabled"], true);
    assert!(output.contains("\"ratio-limit\": 1.200e+01"));
    assert!(output.contains("\"rpc-password\" : \"LOCAL_CONFIG_PRIVATE_VALUE=fixture-only\""));
    assert!(output.contains("\"path\": \"/Unrelated/Case\""));
    assert!(
        plan(Application::TransmissionLegacy, &dirs(), value.artifact())
            .unwrap()
            .artifact()
            .is_none()
    );
}
#[test]
fn frozen_sab_directory_rule_preserves_preamble_nested_servers_comments_and_secrets() {
    let value = draft(Application::Sabnzbd, SAB);
    let output = artifact(&value);
    let expected = std::str::from_utf8(SAB)
        .unwrap()
        .replace("/old/complete", "\"/downloads/complete\"")
        .replace("'/old/incomplete'", "\"/downloads/incomplete\"");
    assert_eq!(output, expected);
    assert_eq!(
        value.summary().changed,
        [Field::CompleteDirectory, Field::IncompleteDirectory]
    );
    assert!(output.contains("complete_dir = /Server/Untouched"));
    assert!(
        plan(Application::Sabnzbd, &dirs(), value.artifact())
            .unwrap()
            .artifact()
            .is_none()
    );
}
#[test]
fn missing_source_is_unavailable_instead_of_an_empty_configuration() {
    for app in [Application::TransmissionLegacy, Application::Sabnzbd] {
        assert_eq!(
            plan(app, &dirs(), None).unwrap_err(),
            ConfigError::Unavailable
        );
    }
    assert_eq!(
        plan(Application::TransmissionLegacy, &dirs(), Some(b"")).unwrap_err(),
        ConfigError::Unsupported
    );
    assert!(artifact(&draft(Application::Sabnzbd, b"")).starts_with("[misc]\n"));
}
#[test]
fn fresh_json_objects_allow_missing_fields_and_bounded_nested_unknown_values() {
    for source in [
        b"{}".as_slice(),
        b" { \n } \n",
        br#"{"other":[null,true,{"brackets":"[ ] { } , :"}]}"#,
    ] {
        let value = draft(Application::TransmissionLegacy, source);
        assert_eq!(json(&value)["incomplete-dir-enabled"], true);
        assert!(
            plan(Application::TransmissionLegacy, &dirs(), value.artifact())
                .unwrap()
                .artifact()
                .is_none()
        );
    }
}
#[test]
fn disabling_incomplete_seeding_preserves_all_existing_incomplete_settings() {
    let desired = DownloadDirectories {
        incomplete: None,
        ..dirs()
    };
    let source =
        br#"{"download-dir":"/old","incomplete-dir":"/Keep/Case","incomplete-dir-enabled":false}"#;
    let value = plan(Application::TransmissionLegacy, &desired, Some(source)).unwrap();
    assert_eq!(
        artifact(&value),
        r#"{"download-dir":"/downloads/complete","incomplete-dir":"/Keep/Case","incomplete-dir-enabled":false}"#
    );
    let source = b"[misc]\ncomplete_dir=/old\ndownload_dir=/Keep/Case\n";
    let value = plan(Application::Sabnzbd, &desired, Some(source)).unwrap();
    assert!(artifact(&value).contains("download_dir=/Keep/Case"));
    assert_eq!(value.summary().changed, [Field::CompleteDirectory]);
}
#[test]
fn unchanged_configuration_returns_no_source_artifact_and_retains_all_gates() {
    let json = br#"{"download-dir":"/downloads/complete","incomplete-dir":"/downloads/incomplete","incomplete-dir-enabled":true,"private":"fixture"}"#;
    for (app, source) in [
        (Application::TransmissionLegacy, json.as_slice()),
        (
            Application::Sabnzbd,
            b"[misc]\ncomplete_dir='/downloads/complete'\ndownload_dir=\"/downloads/incomplete\"\n",
        ),
    ] {
        let value = draft(app, source);
        assert!(value.artifact().is_none());
        assert!(value.summary().changed.is_empty());
        assert_eq!(value.summary().unresolved.len(), 6);
    }
}
#[test]
fn source_values_and_secret_artifacts_never_enter_debug_or_serializable_summary() {
    for (app, source) in [
        (Application::TransmissionLegacy, TRANSMISSION),
        (Application::Sabnzbd, SAB),
    ] {
        let value = draft(app, source);
        assert!(artifact(&value).contains("LOCAL_CONFIG_PRIVATE_VALUE=fixture-only"));
        assert!(!format!("{value:?}").contains("PRIVATE_VALUE"));
        let summary = serde_json::to_string(&value.summary()).unwrap();
        assert!(!summary.contains("PRIVATE_VALUE"));
        assert!(!summary.contains("rpc-password"));
        assert!(!summary.contains("api_key"));
    }
}
#[test]
fn json_duplicate_and_escaped_duplicate_root_keys_are_refused() {
    for source in [
        r#"{"download-dir":"/a","download-dir":"/b"}"#,
        r#"{"download-dir":"/a","download\u002ddir":"/b"}"#,
        r#"{"other":1,"other":2}"#,
    ] {
        assert_eq!(
            plan(
                Application::TransmissionLegacy,
                &dirs(),
                Some(source.as_bytes())
            )
            .unwrap_err(),
            ConfigError::Unsupported
        );
    }
    let value = draft(
        Application::TransmissionLegacy,
        br#"{"download\u002ddir":"/old"}"#,
    );
    assert!(artifact(&value).contains("download\\u002ddir"));
    assert_eq!(json(&value)["download-dir"], "/downloads/complete");
}
#[test]
fn json_modern_selected_aliases_are_explicitly_unsupported() {
    for key in ["download_dir", "incomplete_dir", "incomplete_dir_enabled"] {
        let source = format!("{{\"{key}\": \"/old\"}}");
        assert_eq!(
            plan(
                Application::TransmissionLegacy,
                &dirs(),
                Some(source.as_bytes())
            )
            .unwrap_err(),
            ConfigError::Unsupported
        );
    }
}
#[test]
fn json_non_objects_wrong_selected_types_trailing_garbage_and_invalid_utf8_refuse() {
    for source in [
        b"[]".as_slice(),
        b"null",
        b"{\"download-dir\":null}",
        b"{\"incomplete-dir-enabled\":1}",
        b"{\"download-dir\":[]} ",
        b"{} trailing",
        b"{\"private\":\"\xff\"}",
        b"{\"download-dir\":\"a\\nb\"}",
    ] {
        assert_eq!(
            plan(Application::TransmissionLegacy, &dirs(), Some(source)).unwrap_err(),
            ConfigError::Unsupported
        );
    }
}
#[test]
fn input_and_projected_output_field_limits_keep_drafts_readable_on_rerun() {
    let object = |count| {
        format!(
            "{{{}}}",
            (0..count)
                .map(|i| format!("\"field{i}\":null"))
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    let source = object(MAX_FIELDS - 3);
    let value = draft(Application::TransmissionLegacy, source.as_bytes());
    assert!(
        plan(Application::TransmissionLegacy, &dirs(), value.artifact())
            .unwrap()
            .artifact()
            .is_none()
    );
    for count in [MAX_FIELDS - 2, MAX_FIELDS, MAX_FIELDS + 1] {
        assert_eq!(
            plan(
                Application::TransmissionLegacy,
                &dirs(),
                Some(object(count).as_bytes())
            )
            .unwrap_err(),
            ConfigError::Limit
        );
    }
}
#[test]
fn json_depth_bound_counts_nested_objects_arrays_without_counting_string_brackets() {
    for depth in [MAX_DEPTH - 1, MAX_DEPTH] {
        let source = format!(
            "{{\"private\":{}0{}}}",
            "[".repeat(depth),
            "]".repeat(depth)
        );
        let result = plan(
            Application::TransmissionLegacy,
            &dirs(),
            Some(source.as_bytes()),
        );
        if depth < MAX_DEPTH {
            assert!(result.is_ok());
        } else {
            assert_eq!(result.unwrap_err(), ConfigError::Limit);
        }
    }
}
#[test]
fn complete_and_incomplete_paths_are_literal_bounded_distinct_and_nonoverlapping() {
    for complete in [
        "relative",
        "/",
        "/a//b",
        "/a/../b",
        "/a/./b",
        "/a/",
        "/a/${DIR}",
        "/a\\b",
        "/a\"b",
        "/a\nb",
        "/ a/b",
        "/a/ b",
        "/a\u{2028}b",
    ] {
        let desired = DownloadDirectories {
            complete: complete.into(),
            incomplete: None,
        };
        assert_eq!(
            plan(Application::Sabnzbd, &desired, Some(b"")).unwrap_err(),
            ConfigError::InvalidInput
        );
    }
    for incomplete in [
        "/downloads/complete",
        "/downloads/complete/child",
        "/downloads",
    ] {
        let mut desired = dirs();
        desired.incomplete = Some(incomplete.into());
        assert_eq!(
            plan(Application::Sabnzbd, &desired, Some(b"")).unwrap_err(),
            ConfigError::InvalidInput
        );
    }
    let desired = DownloadDirectories {
        complete: "/Data/Épisodes #1,TV;='[a]'".into(),
        incomplete: Some("/Data/Working".into()),
    };
    for app in [Application::TransmissionLegacy, Application::Sabnzbd] {
        let source = if app == Application::Sabnzbd {
            b"".as_slice()
        } else {
            b"{}"
        };
        let value = plan(app, &desired, Some(source)).unwrap();
        assert!(
            plan(app, &desired, value.artifact())
                .unwrap()
                .artifact()
                .is_none()
        );
    }
    let desired = DownloadDirectories {
        complete: format!("/{}", "é".repeat(256)),
        incomplete: None,
    };
    assert_eq!(
        plan(Application::Sabnzbd, &desired, Some(b"")).unwrap_err(),
        ConfigError::InvalidInput
    );
}
#[test]
fn input_and_output_byte_caps_are_explicit_without_truncation() {
    for app in [Application::TransmissionLegacy, Application::Sabnzbd] {
        assert_eq!(
            plan(app, &dirs(), Some(&vec![b' '; MAX_INPUT_BYTES + 1])).unwrap_err(),
            ConfigError::Limit
        );
    }
    let mut source = br#"{"download-dir":"/downloads/complete"}"#.to_vec();
    source.resize(MAX_INPUT_BYTES, b' ');
    let desired = DownloadDirectories {
        incomplete: None,
        ..dirs()
    };
    assert!(
        plan(Application::TransmissionLegacy, &desired, Some(&source))
            .unwrap()
            .artifact()
            .is_none()
    );
    assert_eq!(
        plan(Application::TransmissionLegacy, &dirs(), Some(&source)).unwrap_err(),
        ConfigError::Limit
    );
}
#[test]
fn crlf_and_utf8_unrelated_bytes_survive_selected_replacements_and_insertions() {
    for (app, source) in [
        (Application::TransmissionLegacy, TRANSMISSION),
        (Application::Sabnzbd, SAB),
    ] {
        let source = std::str::from_utf8(source).unwrap().replace('\n', "\r\n");
        let value = draft(app, source.as_bytes());
        for (i, byte) in value.artifact().unwrap().iter().enumerate() {
            if *byte == b'\n' {
                assert!(i > 0 && value.artifact().unwrap()[i - 1] == b'\r');
            }
        }
        assert!(
            plan(app, &dirs(), value.artifact())
                .unwrap()
                .artifact()
                .is_none()
        );
    }
}
#[test]
fn sab_missing_keys_are_inserted_before_nested_sections() {
    let source = b"[misc]\napi_key = fixture-private\n[[nested]]\ncomplete_dir = /Nested/Keep\n[servers]\n[[news]]\npassword=fixture-private\n";
    let value = draft(Application::Sabnzbd, source);
    let output = artifact(&value);
    assert!(
        output
            .find("complete_dir = \"/downloads/complete\"")
            .unwrap()
            < output.find("[[nested]]").unwrap()
    );
    assert!(output.contains("complete_dir = /Nested/Keep"));
    assert!(
        plan(Application::Sabnzbd, &dirs(), value.artifact())
            .unwrap()
            .artifact()
            .is_none()
    );
}
#[test]
fn sab_missing_misc_section_is_appended_as_top_level_with_newline_and_bom_preserved() {
    for source in [
        "__version__ = 19\n[servers]\n[[news]]\npassword=fixture-private",
        "\u{feff}",
        "# Header without newline",
    ] {
        let value = draft(Application::Sabnzbd, source.as_bytes());
        let output = artifact(&value);
        assert!(output.starts_with(source));
        assert!(output.contains("[misc]\ncomplete_dir"));
        assert!(
            plan(Application::Sabnzbd, &dirs(), value.artifact())
                .unwrap()
                .artifact()
                .is_none()
        );
    }
}
#[test]
fn sab_duplicate_sections_keys_and_case_aliases_are_refused() {
    for source in [
        "[misc]\n[misc]\n",
        "[MISC]\n",
        "[misc]\ncomplete_dir=/a\ncomplete_dir=/b\n",
        "[misc]\nComplete_Dir=/a\n",
        "[misc]\nother=1\nother=2\n",
        "[misc]\n'complete_dir'=/a\n",
    ] {
        assert_eq!(
            plan(Application::Sabnzbd, &dirs(), Some(source.as_bytes())).unwrap_err(),
            ConfigError::Unsupported
        );
    }
}
#[test]
fn sab_quoted_values_inline_comments_and_empty_existing_value_remain_idempotent() {
    for source in [
        "[misc]\ncomplete_dir='/old' # keep\ndownload_dir=\"/old\" # keep too\n",
        "[misc]\ncomplete_dir = # empty\ndownload_dir=\n",
    ] {
        let value = draft(Application::Sabnzbd, source.as_bytes());
        assert!(artifact(&value).contains("#"));
        assert!(
            plan(Application::Sabnzbd, &dirs(), value.artifact())
                .unwrap()
                .artifact()
                .is_none()
        );
    }
}
#[test]
fn sab_multiline_quoted_sections_lists_bad_headers_and_unparseable_selected_values_refuse() {
    for source in [
        "x = '''\n[misc]\ncomplete_dir = /fake\n'''\n",
        "[\"misc\"]\n",
        "[[misc]]\n",
        "[misc]]\n",
        "[misc] garbage\n",
        "[misc]\ncomplete_dir=/a,/b\n",
        "[misc]\ncomplete_dir='unterminated\n",
        "[misc]\ncomplete_dir='a' garbage\n",
        "[misc]\ncontinuation\n",
        "[misc]\ncomplete_dir=/a\rhidden\n",
    ] {
        assert_eq!(
            plan(Application::Sabnzbd, &dirs(), Some(source.as_bytes())).unwrap_err(),
            ConfigError::Unsupported
        );
    }
}
#[test]
fn sab_field_line_and_section_depth_bounds_include_generated_settings() {
    let source = format!(
        "[misc]\n{}",
        (0..MAX_FIELDS)
            .map(|i| format!("other{i}=1\n"))
            .collect::<String>()
    );
    assert_eq!(
        plan(Application::Sabnzbd, &dirs(), Some(source.as_bytes())).unwrap_err(),
        ConfigError::Limit
    );
    let source = format!(
        "[misc]\ncomplete_dir=/downloads/complete\ndownload_dir=/downloads/incomplete\n{}",
        "#\n".repeat(MAX_LINES - 3)
    );
    assert!(
        draft(Application::Sabnzbd, source.as_bytes())
            .artifact()
            .is_none()
    );
    let source = format!("#\n{}", "#\n".repeat(MAX_LINES - 1));
    assert_eq!(
        plan(Application::Sabnzbd, &dirs(), Some(source.as_bytes())).unwrap_err(),
        ConfigError::Limit
    );
    let source = format!(
        "{}x{}\n",
        "[".repeat(MAX_DEPTH + 1),
        "]".repeat(MAX_DEPTH + 1)
    );
    assert_eq!(
        plan(Application::Sabnzbd, &dirs(), Some(source.as_bytes())).unwrap_err(),
        ConfigError::Unsupported
    );
}
#[test]
fn unsupported_source_errors_are_fixed_values_without_payload_text() {
    for (app, source) in [
        (
            Application::TransmissionLegacy,
            b"PRIVATE_RAW_INVALID".as_slice(),
        ),
        (
            Application::Sabnzbd,
            b"[misc]\ncomplete_dir=\"PRIVATE_RAW_INVALID\n",
        ),
    ] {
        let error = plan(app, &dirs(), Some(source)).unwrap_err();
        assert_eq!(format!("{error:?}"), "Unsupported");
        assert_eq!(serde_json::to_string(&error).unwrap(), "\"unsupported\"");
    }
}

#[test]
fn sab_generated_scalar_keys_cannot_collide_with_direct_child_sections() {
    for source in [
        "[misc]\n[[complete_dir]]\nother=1\n",
        "[misc]\n[[server]]\nx=1\n[[download_dir]]\nx=1\n",
    ] {
        assert_eq!(
            plan(Application::Sabnzbd, &dirs(), Some(source.as_bytes())).unwrap_err(),
            ConfigError::Unsupported
        );
    }
}

#[test]
fn sab_generated_misc_section_cannot_collide_with_a_preamble_scalar() {
    for source in [
        "misc = fixture-value\n",
        "'misc' = fixture-value\n",
        "__version__=19\nmisc=fixture-value\n[servers]\n[[news]]\nx=1\n",
    ] {
        assert_eq!(
            plan(Application::Sabnzbd, &dirs(), Some(source.as_bytes())).unwrap_err(),
            ConfigError::Unsupported
        );
    }
}
