use super::*;

fn address(value: &str) -> ProbeOutcome {
    ProbeOutcome::Address(value.parse().unwrap())
}
#[test]
fn supplied_response_fixtures_preserve_terminal_failures() {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Case {
        name: String,
        kind: String,
        status: Option<i64>,
        body: String,
        expected: String,
        address: Option<String>,
    }
    let cases: Vec<Case> = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/deployment/egress/responses.json"
    ))
    .unwrap();
    assert_eq!(cases.len(), 16);
    for case in cases {
        let outcome = match case.kind.as_str() {
            "http" => parse_http_address(
                case.status.map(|code| code.try_into().unwrap()),
                case.body.as_bytes(),
            ),
            "exec" => parse_exec_address(case.status, case.body.as_bytes()),
            _ => panic!("unknown fixture kind"),
        };
        let json = serde_json::to_value(outcome).unwrap();
        assert_eq!(json["state"], case.expected, "{}", case.name);
        if let Some(ip) = case.address {
            assert_eq!(json["address"], ip, "{}", case.name);
        } else {
            assert!(json.get("address").is_none(), "{}", case.name);
        }
    }
}
#[test]
fn only_exact_http_success_and_present_zero_exit_admit_output() {
    for status in [0, 100, 201, 204, 301, 400, 401, 429, 500, u16::MAX] {
        assert_eq!(
            parse_http_address(Some(status), b"1.1.1.1"),
            ProbeOutcome::Failed
        );
    }
    for code in [i64::MIN, -1, 1, 2, 127, 137, i64::MAX] {
        assert_eq!(
            parse_exec_address(Some(code), b"1.1.1.1"),
            ProbeOutcome::Failed
        );
    }
    assert_eq!(
        parse_exec_address(None, b"1.1.1.1"),
        ProbeOutcome::MissingTerminalStatus
    );
    assert_eq!(
        parse_http_address(None, b"1.1.1.1"),
        ProbeOutcome::MissingTerminalStatus
    );
}
#[test]
fn raw_response_limit_precedes_trimming_or_status_interpretation() {
    let mut bytes = b"1.1.1.1".to_vec();
    bytes.resize(MAX_RESPONSE_BYTES, b' ');
    assert_eq!(parse_exec_address(Some(0), &bytes), address("1.1.1.1"));
    assert_eq!(parse_http_address(Some(200), &bytes), address("1.1.1.1"));
    bytes.push(b' ');
    for code in [Some(0), Some(1), None] {
        assert_eq!(
            parse_exec_address(code, &bytes),
            ProbeOutcome::ResponseLimit
        );
    }
    for status in [Some(200), Some(500), None] {
        assert_eq!(
            parse_http_address(status, &bytes),
            ProbeOutcome::ResponseLimit
        );
    }
}
#[test]
fn empty_non_utf8_and_non_ascii_whitespace_are_not_addresses() {
    for bytes in [
        b"".as_slice(),
        b" \r\n\t",
        b"\xff1.1.1.1",
        "\u{a0}1.1.1.1".as_bytes(),
        "1.1.1.1\u{2003}".as_bytes(),
    ] {
        assert_eq!(
            parse_http_address(Some(200), bytes),
            ProbeOutcome::InvalidResponse
        );
    }
}
#[test]
fn error_bodies_do_not_enter_outcome_debug_or_serialization() {
    let bytes = b"fixture-password=do-not-retain";
    for outcome in [
        parse_http_address(Some(500), bytes),
        parse_exec_address(None, bytes),
        parse_exec_address(Some(0), bytes),
    ] {
        for value in [
            format!("{outcome:?}"),
            serde_json::to_string(&outcome).unwrap(),
        ] {
            assert!(!value.contains("password") && !value.contains("do-not-retain"));
        }
    }
}
#[test]
fn urls_ports_prefixes_multiple_values_and_control_text_are_refused() {
    for value in [
        "https://1.1.1.1",
        "1.1.1.1/32",
        "[2606:4700:4700::1111]:443",
        "1.1.1.1:443",
        "IP: 1.1.1.1",
        "1.1.1.1 9.9.9.9",
        "1.1.1.1\n9.9.9.9",
        "1.1.1.1\0",
        "01.1.1.1",
        "HTTP/1.1 200 OK\r\n\r\n1.1.1.1",
    ] {
        assert_eq!(
            parse_exec_address(Some(0), value.as_bytes()),
            ProbeOutcome::InvalidResponse,
            "{value:?}"
        );
    }
}
#[test]
fn obvious_non_egress_literals_are_refused_after_mapped_normalization() {
    for value in [
        "0.0.0.0",
        "::",
        "127.0.0.1",
        "::1",
        "224.0.0.1",
        "ff02::1",
        "255.255.255.255",
        "::ffff:0.0.0.0",
        "::ffff:127.0.0.1",
        "::ffff:224.0.0.1",
        "::ffff:255.255.255.255",
    ] {
        assert_eq!(
            parse_exec_address(Some(0), value.as_bytes()),
            ProbeOutcome::InvalidResponse,
            "{value}"
        );
    }
}
#[test]
fn ipv6_is_canonicalized_and_ipv4_mapped_equality_is_preserved() {
    assert_eq!(
        parse_http_address(Some(200), b"2606:4700:4700:0000:0000:0000:0000:1111\r\n"),
        address("2606:4700:4700::1111")
    );
    assert_eq!(
        parse_exec_address(Some(0), b" \t::ffff:1.1.1.1\r\n"),
        address("1.1.1.1")
    );
    assert_eq!(
        parse_exec_address(Some(0), b"::ffff:101:101"),
        address("1.1.1.1")
    );
}
#[test]
fn supplied_response_failure_cannot_create_a_comparable_pair() {
    use limeos_domain::{ContainerSnapshot, deployment::egress::*};
    let target = Target {
        host_boot_id: "a1111111-2222-4333-8444-555555555555".into(),
        engine_id: "engine-a".into(),
        provider: ContainerSnapshot {
            resource: format!("container:{}", "a".repeat(64)),
            image: format!("sha256:{}", "b".repeat(64)),
            started_at: "2026-10-16T01:02:03.000000000Z".into(),
            running: true,
        },
    };
    let host = HostObservation {
        host_boot_id: target.host_boot_id.clone(),
        sample: Sample {
            sampled_at: 100,
            outcome: parse_http_address(Some(200), b"1.1.1.1"),
        },
    };
    for outcome in [
        parse_exec_address(None, b"9.9.9.9"),
        parse_exec_address(Some(1), b"9.9.9.9"),
        parse_exec_address(Some(0), b"<error>9.9.9.9</error>"),
    ] {
        let provider = ProviderObservation {
            target: target.clone(),
            sample: Sample {
                sampled_at: 100,
                outcome,
            },
        };
        let report = compare(100, &target, Some(&host), Some(&provider)).unwrap();
        assert_eq!(
            report.comparison,
            Comparison::Unknown {
                reason: UnknownReason::UnusableSources
            }
        );
        assert!(report.addresses.is_none());
        assert_eq!(report.vpn_evidence, VpnEvidence::NotEstablished);
    }
}
