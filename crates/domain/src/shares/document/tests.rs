use super::*;
use crate::shares::SambaShareConfig;

fn imported() -> SambaSharesConfig {
    serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/samba/imported.json"
    ))
    .unwrap()
}
fn config(name: &str) -> SambaSharesConfig {
    SambaSharesConfig {
        shares: vec![SambaShareConfig {
            name: name.into(),
            path: "/srv/Media Library".into(),
            comment: "Movies & TV".into(),
            read_only: false,
            guest_ok: false,
            browseable: true,
            valid_users: "pi, media".into(),
            enabled: true,
        }],
    }
}
fn changed() -> SambaSharesConfig {
    let mut config = imported();
    config.shares[0].path = "/srv/Media Library/Épisodes".into();
    config.shares[0].read_only = true;
    config
}
fn padded(size: usize) -> String {
    let mut text = String::new();
    while text.len() < size {
        let count = (size - text.len()).min(128);
        if count == 1 {
            text.push('\n');
        } else {
            text.push('#');
            text.extend(std::iter::repeat_n('x', count - 2));
            text.push('\n');
        }
    }
    text
}

#[test]
fn frozen_imported_sections_append_without_changing_operator_bytes() {
    let desired = imported();
    let golden = include_str!("../../../../../tests/fixtures/samba/imported.sections");
    assert_eq!(desired.render_sections().unwrap(), golden);
    let source = "# Operator comments: café\n[global]\n workgroup = EXAMPLE\n include = /operator/unresolved.conf\n[archive]\n path = /srv/Archives Privées\n";
    let plan = plan_document_edit(source.as_bytes(), None, &desired).unwrap();
    assert_eq!(plan.candidate(), format!("{source}{golden}").as_bytes());
    assert_eq!(plan.desired(), &desired);
    assert_eq!(plan.summary().enabled_shares, 1);
    assert!(!plan.summary().replacing_managed_block);
}

#[test]
fn replacement_changes_only_the_exact_owned_span() {
    let previous = imported();
    let desired = changed();
    let prefix = "# Keep whitespace\r\n[global]\r\n  workgroup = EXAMPLE\r\n[operator]\r\n path = /srv/Outside Case\r\n";
    let suffix = "# Keep suffix\n[printer$]\n printable = yes\n";
    let source = format!("{prefix}{}{suffix}", previous.render_sections().unwrap());
    let plan = plan_document_edit(source.as_bytes(), Some(&previous), &desired).unwrap();
    assert_eq!(
        plan.candidate(),
        format!("{prefix}{}{suffix}", desired.render_sections().unwrap()).as_bytes()
    );
    assert!(plan.summary().changed);
    assert!(plan.summary().replacing_managed_block);
}

#[test]
fn no_op_and_updates_preserve_owned_line_endings_and_final_newline() {
    let previous = imported();
    let desired = changed();
    for crlf in [false, true] {
        for final_newline in [false, true] {
            let source = rendered(&previous, crlf, final_newline).unwrap();
            let unchanged =
                plan_document_edit(source.as_bytes(), Some(&previous), &previous).unwrap();
            assert_eq!(unchanged.candidate(), source.as_bytes());
            assert!(!unchanged.summary().changed);
            let plan = plan_document_edit(source.as_bytes(), Some(&previous), &desired).unwrap();
            assert_eq!(
                plan.candidate(),
                rendered(&desired, crlf, final_newline).unwrap().as_bytes()
            );
        }
    }
}

#[test]
fn removal_and_disabled_configuration_remove_only_owned_shares() {
    let previous = imported();
    let prefix = "[global]\n workgroup = EXAMPLE\n";
    let suffix = "[operator]\n path = /srv/docs\n";
    let source = format!("{prefix}{}{suffix}", previous.render_sections().unwrap());
    let mut disabled = previous.clone();
    for share in &mut disabled.shares {
        share.enabled = false;
    }
    for desired in [SambaSharesConfig::default(), disabled] {
        let plan = plan_document_edit(source.as_bytes(), Some(&previous), &desired).unwrap();
        assert_eq!(
            plan.candidate(),
            format!("{prefix}# BEGIN LIMEOS SAMBA V1\n# END LIMEOS SAMBA V1\n{suffix}").as_bytes()
        );
        assert_eq!(plan.summary().enabled_shares, 0);
    }
}

#[test]
fn append_uses_last_line_style_and_adds_only_the_required_separator() {
    let desired = config("media");
    for (source, separator, crlf) in [
        ("[global]\n workgroup = EXAMPLE", "\n", false),
        ("[global]\r\n workgroup = EXAMPLE", "\r\n", true),
        ("[global]\r\n", "", true),
        ("", "", false),
    ] {
        let plan = plan_document_edit(source.as_bytes(), None, &desired).unwrap();
        assert_eq!(
            plan.candidate(),
            format!(
                "{source}{separator}{}",
                rendered(&desired, crlf, true).unwrap()
            )
            .as_bytes()
        );
        assert!(plan.candidate().starts_with(source.as_bytes()));
    }
}

#[test]
fn exact_content_binding_refuses_any_new_source_bytes() {
    let desired = imported();
    let source = b"[global]\n";
    let plan = plan_document_edit(source, None, &desired).unwrap();
    plan.check_current(source).unwrap();
    for current in [
        b"[GLOBAL]\n".as_slice(),
        b"[global]\n# changed\n",
        b"[global]",
        b"",
    ] {
        assert_eq!(
            plan.check_current(current).unwrap_err()[0].field,
            "samba.document.current"
        );
    }
}

#[test]
fn markers_require_a_matching_previous_configuration_and_never_adopt_legacy_sections() {
    let desired = imported();
    let owned = desired.render_sections().unwrap();
    assert!(plan_document_edit(owned.as_bytes(), None, &desired).is_err());
    assert!(plan_document_edit(b"[global]\n", Some(&desired), &desired).is_err());
    let unmarked = b"# Generated by Pi-Health\n[media]\n path = /mnt/media\n";
    assert_eq!(
        plan_document_edit(unmarked, None, &desired).unwrap_err()[0].code,
        StoragePlanningFindingCode::Duplicate
    );
    assert!(plan_document_edit(unmarked, Some(&desired), &desired).is_err());
}

#[test]
fn duplicate_nested_reversed_unfinished_and_unknown_markers_refuse() {
    let desired = imported();
    let valid = desired.render_sections().unwrap();
    let invalid = [
        format!("{valid}{valid}"),
        format!("{BEGIN}\n{valid}{END}\n"),
        format!("{END}\n{BEGIN}\n"),
        format!("{BEGIN}\n"),
        format!("{END}\n"),
        valid.replace("V1", "V2"),
        valid.replace(BEGIN, " # BEGIN LIMEOS SAMBA V1"),
        valid.replace(END, "# end limeos samba v1"),
        valid.replace(END, "# END LIMEOS SAMBA V1 extra"),
    ];
    for source in invalid {
        assert!(
            plan_document_edit(source.as_bytes(), Some(&desired), &desired).is_err(),
            "{source:?}"
        );
    }
}

#[test]
fn manual_edits_inside_a_previously_managed_block_are_preserved_by_refusal() {
    let previous = imported();
    let desired = changed();
    let canonical = previous.render_sections().unwrap();
    for source in [
        canonical.replace("guest ok = yes", "guest ok = no"),
        canonical.replace("[media]", "# operator note\n[media]"),
        canonical.replace("/mnt/media", "/mnt/other"),
        canonical.replace("[media]", "[Media]"),
    ] {
        let before = source.clone();
        assert_eq!(
            plan_document_edit(source.as_bytes(), Some(&previous), &desired).unwrap_err()[0].field,
            "samba.document.managed_block"
        );
        assert_eq!(source, before);
    }
}

#[test]
fn external_case_and_whitespace_collisions_refuse_enabled_and_disabled_names() {
    for name in ["media", "Media", " MEDIA ", "m e d i a", "\tMEdiA\t"] {
        let source = format!("[{name}] trailing text\n path = /srv/operator\n");
        for enabled in [true, false] {
            let mut desired = config("media");
            desired.shares[0].enabled = enabled;
            assert_eq!(
                plan_document_edit(source.as_bytes(), None, &desired).unwrap_err()[0].field,
                "samba.document.collision"
            );
        }
    }
}

#[test]
fn removing_a_name_still_refuses_a_foreign_duplicate_of_the_old_name() {
    let previous = config("old_share");
    let desired = config("new_share");
    let source = format!(
        "[OLD_SHARE]\n path = /srv/operator\n{}",
        previous.render_sections().unwrap()
    );
    assert_eq!(
        plan_document_edit(source.as_bytes(), Some(&previous), &desired).unwrap_err()[0].field,
        "samba.document.collision"
    );
}

#[test]
fn bracket_text_in_parameters_and_comments_is_preserved_without_becoming_a_section() {
    let desired = imported();
    let source = "# [media]\n; [docs]\n[operator]\n path = /srv/[media]\n comment = [media] text\n include = /operator/never-opened.conf\n";
    let plan = plan_document_edit(source.as_bytes(), None, &desired).unwrap();
    assert!(plan.candidate().starts_with(source.as_bytes()));
}

#[test]
fn suffix_parameters_must_have_an_explicit_operator_section_boundary() {
    let previous = imported();
    let desired = changed();
    let owned = previous.render_sections().unwrap();
    for suffix in [
        " comment = do not move\n",
        "\n# operator note\n read only = no\n",
        "include = /operator/secret.conf\n",
    ] {
        let source = format!("{owned}{suffix}");
        assert_eq!(
            plan_document_edit(source.as_bytes(), Some(&previous), &desired).unwrap_err()[0].field,
            "samba.document.suffix"
        );
    }
    let suffix = "\n# Note\n[operator]\n comment = stays here\n";
    let source = format!("{owned}{suffix}");
    assert!(
        plan_document_edit(source.as_bytes(), Some(&previous), &desired)
            .unwrap()
            .candidate()
            .ends_with(suffix.as_bytes())
    );
}

#[test]
fn continued_headers_cannot_hide_a_managed_name_collision() {
    let desired = config("media");
    for source in ["[me\\\n dia]\n", "[m\\ \n e d i a]\n"] {
        assert_eq!(
            plan_document_edit(source.as_bytes(), None, &desired).unwrap_err()[0].field,
            "samba.document.collision"
        );
    }
    let source = "[operator]\n comment = multi\\\n [media] is a literal value\n";
    assert!(
        plan_document_edit(source.as_bytes(), None, &desired)
            .unwrap()
            .candidate()
            .starts_with(source.as_bytes())
    );
}

#[test]
fn markers_inside_continuations_and_ambiguous_comment_continuations_refuse() {
    let desired = imported();
    let owned = desired.render_sections().unwrap();
    let source = format!("[global]\n comment = value\\\n{owned}");
    assert_eq!(
        plan_document_edit(source.as_bytes(), Some(&desired), &desired).unwrap_err()[0].field,
        "samba.document.markers"
    );
    for source in [
        "# comment\\\n[media]\n",
        "; comment\\\n[media]\n",
        "[operator]\n comment = unfinished\\",
    ] {
        assert!(plan_document_edit(source.as_bytes(), None, &desired).is_err());
    }
}

#[test]
fn malformed_or_unqualified_section_names_refuse_without_echoing_input() {
    let desired = imported();
    for source in ["[missing-close\n", "[]\n", "[privé]\n"] {
        let errors = plan_document_edit(source.as_bytes(), None, &desired).unwrap_err();
        assert_eq!(errors[0].field, "samba.document.section");
        assert!(!format!("{errors:?}").contains("privé"));
    }
}

#[test]
fn invalid_encoding_and_controls_produce_no_candidate() {
    let desired = imported();
    for source in [
        b"[global]\0".as_slice(),
        b"[global]\x01",
        b"[global]\x7f",
        b"[global]\xff",
    ] {
        assert_eq!(
            plan_document_edit(source, None, &desired).unwrap_err()[0].code,
            StoragePlanningFindingCode::Malformed
        );
    }
}

#[test]
fn source_and_candidate_byte_limits_are_independent_and_exact() {
    let previous = imported();
    let old = previous.render_sections().unwrap();
    let source = format!("{}{}", padded(SAMBA_DOCUMENT_MAX_BYTES - old.len()), old);
    let plan = plan_document_edit(source.as_bytes(), Some(&previous), &previous).unwrap();
    assert_eq!(plan.candidate().len(), SAMBA_DOCUMENT_MAX_BYTES);
    let mut larger = previous.clone();
    larger.shares[0].comment = "larger candidate".into();
    assert_eq!(
        plan_document_edit(source.as_bytes(), Some(&previous), &larger).unwrap_err()[0].field,
        "samba.document.candidate"
    );
    let oversized = format!("{source}\n");
    assert_eq!(
        plan_document_edit(oversized.as_bytes(), Some(&previous), &previous).unwrap_err()[0].field,
        "samba.document.bytes"
    );
}

#[test]
fn logical_line_and_external_section_bounds_refuse_before_unbounded_growth() {
    let desired = SambaSharesConfig::default();
    let exact = format!(
        "#{}\n",
        "x".repeat(SAMBA_DOCUMENT_MAX_LOGICAL_LINE_BYTES - 1)
    );
    plan_document_edit(exact.as_bytes(), None, &desired).unwrap();
    let excess = format!("#{}\n", "x".repeat(SAMBA_DOCUMENT_MAX_LOGICAL_LINE_BYTES));
    assert_eq!(
        plan_document_edit(excess.as_bytes(), None, &desired).unwrap_err()[0].field,
        "samba.document.line"
    );
    let sections = (0..SAMBA_DOCUMENT_MAX_EXTERNAL_SECTIONS)
        .map(|index| format!("[op{index}]\n"))
        .collect::<String>();
    plan_document_edit(sections.as_bytes(), None, &desired).unwrap();
    assert_eq!(
        plan_document_edit(format!("{sections}[another]\n").as_bytes(), None, &desired)
            .unwrap_err()[0]
            .field,
        "samba.document.sections"
    );
}

#[test]
fn summary_and_debug_expose_counts_without_operator_or_share_text() {
    let desired = changed();
    let source = b"[global]\n private credential = synthetic-private-value\n";
    let plan = plan_document_edit(source, None, &desired).unwrap();
    let debug = format!("{plan:?}");
    let json = serde_json::to_string(&plan.summary()).unwrap();
    for private in [
        "synthetic-private-value",
        "credential",
        "Épisodes",
        "/srv/Media Library",
    ] {
        assert!(!debug.contains(private));
        assert!(!json.contains(private));
    }
    assert_eq!(plan.summary().before_bytes, source.len());
    assert_eq!(plan.summary().candidate_bytes, plan.candidate().len());
}

#[test]
fn desired_and_previous_typed_configuration_are_validated_before_editing() {
    let previous = imported();
    let source = previous.render_sections().unwrap();
    let mut invalid = previous.clone();
    invalid.shares[0].valid_users = "pi\n[global]".into();
    assert!(plan_document_edit(source.as_bytes(), Some(&previous), &invalid).is_err());
    assert!(plan_document_edit(source.as_bytes(), Some(&invalid), &previous).is_err());
}
