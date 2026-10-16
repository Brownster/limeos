use super::*;

fn parse(text: &str) -> Installed {
    parse_dpkg(
        "alpha",
        Architecture::Arm64,
        QueryOutput {
            exit_code: Some(0),
            stdout: text.as_bytes(),
            stderr: b"",
            truncated: false,
        },
    )
}

#[test]
fn installed_selection_and_architecture_survive_projection() {
    for (want, selection) in [
        ("install", Selection::Install),
        ("hold", Selection::Hold),
        ("deinstall", Selection::Deinstall),
        ("purge", Selection::Purge),
        ("unknown", Selection::Unknown),
    ] {
        assert_eq!(
            parse(&format!("alpha\tarm64\t{want}\tinstalled\tok\t1:2.0-1\n")),
            Installed::Present {
                version: "1:2.0-1".into(),
                selection
            }
        );
    }
    assert!(matches!(
        parse("alpha\tall\tinstall\tinstalled\tok\t1\n"),
        Installed::Present { .. }
    ));
    assert!(matches!(
        parse_dpkg(
            "alpha",
            Architecture::Amd64,
            QueryOutput {
                exit_code: Some(0),
                stdout: b"alpha\tamd64\tinstall\tinstalled\tok\t1\n",
                stderr: b"",
                truncated: false
            }
        ),
        Installed::Present { .. }
    ));
}

#[test]
fn only_explicit_absent_states_establish_absence() {
    assert_eq!(
        parse("alpha\tarm64\tunknown\tnot-installed\tok\t\n"),
        Installed::Absent {}
    );
    assert_eq!(
        parse("alpha\tarm64\tdeinstall\tconfig-files\tok\t1.0-1\n"),
        Installed::Absent {}
    );
    assert_eq!(parse(""), Installed::Unknown {});
    assert_eq!(
        parse("alpha\tarm64\tunknown\tnot-installed\tok\t1.0\n"),
        Installed::Unknown {}
    );
}

#[test]
fn partial_states_and_reinstall_required_need_recovery() {
    for state in [
        "half-installed",
        "unpacked",
        "half-configured",
        "triggers-awaited",
        "triggers-pending",
    ] {
        assert_eq!(
            parse(&format!("alpha\tarm64\tinstall\t{state}\tok\t1\n")),
            Installed::Broken {}
        );
    }
    assert_eq!(
        parse("alpha\tarm64\tinstall\tinstalled\treinstreq\t1\n"),
        Installed::Broken {}
    );
}

#[test]
fn failed_truncated_or_warned_queries_are_unknown_even_with_valid_stdout() {
    for (exit_code, truncated, stderr) in [
        (Some(1), false, b"".as_slice()),
        (None, false, b"".as_slice()),
        (Some(0), true, b"".as_slice()),
        (Some(0), false, b"warning".as_slice()),
    ] {
        assert_eq!(
            parse_dpkg(
                "alpha",
                Architecture::Arm64,
                QueryOutput {
                    exit_code,
                    stdout: b"alpha\tarm64\tinstall\tinstalled\tok\t1\n",
                    stderr,
                    truncated
                }
            ),
            Installed::Unknown {}
        );
    }
}

#[test]
fn wrong_identity_foreign_architecture_and_multirow_output_refuse() {
    for text in [
        "other\tarm64\tinstall\tinstalled\tok\t1\n",
        "alpha:arm64\tarm64\tinstall\tinstalled\tok\t1\n",
        "alpha\tamd64\tinstall\tinstalled\tok\t1\n",
        "alpha\tarm64\tinstall\tinstalled\tok\t1\nalpha\tarm64\tinstall\tinstalled\tok\t1\n",
    ] {
        assert_eq!(parse(text), Installed::Unknown {});
    }
}

#[test]
fn malformed_fields_controls_and_byte_limits_refuse() {
    for text in [
        "alpha\tarm64\tinstall\tinstalled\tok\t1",
        "alpha\tarm64\tinstall\tinstalled\tok\t1\r\n",
        "alpha\tarm64\tinstall\tinstalled\tok\t1\0\n",
        "alpha\tarm64\tinstall\tinstalled\tok\t\n",
        "alpha\tarm64\tinvalid\tinstalled\tok\t1\n",
        "alpha\tarm64\tinstall\tinstalled\tunknown\t1\n",
        "alpha\tarm64\tinstall\tinstalled\tok\t1\textra\n",
    ] {
        assert_eq!(parse(text), Installed::Unknown {});
    }
    assert_eq!(
        parse(&"x".repeat(MAX_QUERY_BYTES + 1)),
        Installed::Unknown {}
    );
    assert_eq!(
        parse_dpkg(
            "alpha",
            Architecture::Arm64,
            QueryOutput {
                exit_code: Some(0),
                stdout: &[255, b'\n'],
                stderr: b"",
                truncated: false
            }
        ),
        Installed::Unknown {}
    );
}
