use super::*;

const BOOT: &str = "a1111111-2222-4333-8444-555555555555";
const OTHER_BOOT: &str = "b1111111-2222-4333-8444-555555555555";
fn target() -> Target {
    Target {
        host_boot_id: BOOT.into(),
        engine_id: "engine-a".into(),
        provider: ContainerSnapshot {
            resource: format!("container:{}", "a".repeat(64)),
            image: format!("sha256:{}", "b".repeat(64)),
            started_at: "2026-10-16T01:02:03.000000000Z".into(),
            running: true,
        },
    }
}
fn sample(address: &str, at: i64) -> Sample {
    Sample {
        sampled_at: at,
        outcome: ProbeOutcome::Address(address.parse().unwrap()),
    }
}
fn observations() -> (HostObservation, ProviderObservation) {
    (
        HostObservation {
            host_boot_id: BOOT.into(),
            sample: sample("1.1.1.1", 100),
        },
        ProviderObservation {
            target: target(),
            sample: sample("9.9.9.9", 100),
        },
    )
}
fn assert_unknown(report: &EgressComparison, reason: UnknownReason) {
    assert_eq!(report.comparison, Comparison::Unknown { reason });
    assert!(report.addresses.is_none());
    assert_eq!(report.vpn_evidence, VpnEvidence::NotEstablished);
    let json = serde_json::to_string(report).unwrap();
    assert!(!json.contains("1.1.1.1") && !json.contains("9.9.9.9"));
}

#[test]
fn frozen_comparison_fixtures_preserve_facts_without_vpn_claims() {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Case {
        name: String,
        host: Option<String>,
        provider: Option<String>,
        expected: String,
    }
    let cases: Vec<Case> = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/deployment/egress/comparisons.json"
    ))
    .unwrap();
    assert_eq!(cases.len(), 10);
    for case in cases {
        let host = case.host.map(|ip| HostObservation {
            host_boot_id: BOOT.into(),
            sample: sample(&ip, 100),
        });
        let provider = case.provider.map(|ip| ProviderObservation {
            target: target(),
            sample: sample(&ip, 100),
        });
        let report = compare(100, &target(), host.as_ref(), provider.as_ref()).unwrap();
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["comparison"]["state"], case.expected, "{}", case.name);
        assert_eq!(json["vpn_evidence"], "not_established");
        assert!(json.get("vpn_leak").is_none());
        assert_eq!(report.addresses.is_some(), case.expected != "unknown");
    }
}
#[test]
fn missing_reads_never_report_a_successful_comparison() {
    let (host, provider) = observations();
    for (host, provider) in [(None, None), (Some(&host), None), (None, Some(&provider))] {
        let report = compare(100, &target(), host, provider).unwrap();
        assert_unknown(&report, UnknownReason::UnusableSources);
        if host.is_none() {
            assert_eq!(
                report.host,
                SourceView {
                    sampled_at: None,
                    state: SourceState::Unavailable
                }
            );
        }
        if provider.is_none() {
            assert_eq!(
                report.provider,
                SourceView {
                    sampled_at: None,
                    state: SourceState::Unavailable
                }
            );
        }
    }
}
#[test]
fn each_unsuccessful_outcome_withholds_the_pair_on_either_side() {
    let outcomes = [
        (ProbeOutcome::Failed, SourceState::Failed),
        (
            ProbeOutcome::MissingTerminalStatus,
            SourceState::MissingTerminalStatus,
        ),
        (ProbeOutcome::TimedOut, SourceState::TimedOut),
        (ProbeOutcome::Incomplete, SourceState::Incomplete),
        (ProbeOutcome::InvalidResponse, SourceState::InvalidResponse),
        (ProbeOutcome::ResponseLimit, SourceState::ResponseLimit),
    ];
    for (outcome, state) in outcomes {
        let (mut host, mut provider) = observations();
        host.sample.outcome = outcome;
        let report = compare(100, &target(), Some(&host), Some(&provider)).unwrap();
        assert_eq!(report.host.state, state);
        assert_unknown(&report, UnknownReason::UnusableSources);
        host.sample = sample("1.1.1.1", 100);
        provider.sample.outcome = outcome;
        let report = compare(100, &target(), Some(&host), Some(&provider)).unwrap();
        assert_eq!(report.provider.state, state);
        assert_unknown(&report, UnknownReason::UnusableSources);
    }
}
#[test]
fn original_sample_age_is_inclusive_and_is_not_refreshed() {
    let (host, mut provider) = observations();
    provider.sample.sampled_at = 105;
    let report = compare(190, &target(), Some(&host), Some(&provider)).unwrap();
    assert_eq!(report.comparison, Comparison::DifferentAddresses);
    assert_eq!(report.host.sampled_at, Some(100));
    assert_eq!(report.provider.sampled_at, Some(105));
    let report = compare(191, &target(), Some(&host), Some(&provider)).unwrap();
    assert_eq!(report.host.state, SourceState::Stale);
    assert_eq!(report.provider.state, SourceState::Fresh);
    assert_unknown(&report, UnknownReason::UnusableSources);
}
#[test]
fn stale_provider_withholds_a_fresh_host_address() {
    let (mut host, provider) = observations();
    host.sample.sampled_at = 105;
    let report = compare(191, &target(), Some(&host), Some(&provider)).unwrap();
    assert_eq!(report.host.state, SourceState::Fresh);
    assert_eq!(report.provider.state, SourceState::Stale);
    assert_unknown(&report, UnknownReason::UnusableSources);
}
#[test]
fn future_samples_are_unknown_on_either_side() {
    for host_future in [true, false] {
        let (mut host, mut provider) = observations();
        if host_future {
            host.sample.sampled_at = 101;
        } else {
            provider.sample.sampled_at = 101;
        }
        let report = compare(100, &target(), Some(&host), Some(&provider)).unwrap();
        assert_eq!(
            if host_future {
                report.host.state
            } else {
                report.provider.state
            },
            SourceState::Future
        );
        assert_unknown(&report, UnknownReason::UnusableSources);
    }
}
#[test]
fn pair_skew_is_inclusive_and_symmetric() {
    for host_first in [true, false] {
        let (mut host, mut provider) = observations();
        if host_first {
            provider.sample.sampled_at = 105;
        } else {
            host.sample.sampled_at = 105;
        }
        assert_eq!(
            compare(110, &target(), Some(&host), Some(&provider))
                .unwrap()
                .comparison,
            Comparison::DifferentAddresses
        );
        if host_first {
            provider.sample.sampled_at = 106;
        } else {
            host.sample.sampled_at = 106;
        }
        let report = compare(110, &target(), Some(&host), Some(&provider)).unwrap();
        assert_eq!(
            (report.host.state, report.provider.state),
            (SourceState::Fresh, SourceState::Fresh)
        );
        assert_unknown(&report, UnknownReason::SampleSkew);
    }
}
#[test]
fn another_host_boot_cannot_supply_the_host_address() {
    let (mut host, provider) = observations();
    host.host_boot_id = OTHER_BOOT.into();
    let report = compare(100, &target(), Some(&host), Some(&provider)).unwrap();
    assert_eq!(report.host.state, SourceState::ScopeMismatch);
    assert_unknown(&report, UnknownReason::UnusableSources);
}
#[test]
fn every_provider_identity_component_must_match_exactly() {
    for field in 0..6 {
        let (host, mut provider) = observations();
        match field {
            0 => provider.target.host_boot_id = OTHER_BOOT.into(),
            1 => provider.target.engine_id = "engine-b".into(),
            2 => provider.target.provider.resource = format!("container:{}", "c".repeat(64)),
            3 => provider.target.provider.image = format!("sha256:{}", "d".repeat(64)),
            4 => provider.target.provider.started_at = "2026-10-16T01:02:04.000000000Z".into(),
            _ => provider.target.provider.running = false,
        }
        let report = compare(100, &target(), Some(&host), Some(&provider)).unwrap();
        assert_eq!(report.provider.state, SourceState::ScopeMismatch);
        assert_unknown(&report, UnknownReason::UnusableSources);
    }
}
#[test]
fn stopped_expected_provider_blocks_even_a_matching_fresh_pair() {
    let (host, mut provider) = observations();
    let mut expected = target();
    expected.provider.running = false;
    provider.target = expected.clone();
    let report = compare(100, &expected, Some(&host), Some(&provider)).unwrap();
    assert_unknown(&report, UnknownReason::ProviderNotRunning);
}
#[test]
fn mismatched_families_never_mean_different_vpn_egress() {
    for host_v6 in [true, false] {
        let (mut host, mut provider) = observations();
        if host_v6 {
            host.sample = sample("2606:4700:4700::1111", 100);
        } else {
            provider.sample = sample("2606:4700:4700::1111", 100);
        }
        assert_unknown(
            &compare(100, &target(), Some(&host), Some(&provider)).unwrap(),
            UnknownReason::AddressFamilyMismatch,
        );
    }
}
#[test]
fn direct_typed_inputs_cannot_bypass_mapped_ipv4_normalization() {
    let (host, mut provider) = observations();
    provider.sample = sample("::ffff:1.1.1.1", 100);
    let report = compare(100, &target(), Some(&host), Some(&provider)).unwrap();
    assert_eq!(report.comparison, Comparison::SameAddress);
    assert_eq!(report.addresses.unwrap().provider.to_string(), "1.1.1.1");
}
#[test]
fn direct_non_egress_literals_are_refused_even_when_out_of_scope() {
    for ip in [
        "0.0.0.0",
        "::",
        "127.0.0.1",
        "::1",
        "224.0.0.1",
        "ff02::1",
        "255.255.255.255",
        "::ffff:127.0.0.1",
    ] {
        let (mut host, mut provider) = observations();
        host.sample = sample(ip, 100);
        assert!(
            compare(100, &target(), Some(&host), Some(&provider)).is_err(),
            "{ip}"
        );
        host.sample = sample("1.1.1.1", 100);
        provider.sample = sample(ip, 100);
        provider.target.engine_id = "foreign".into();
        assert!(
            compare(100, &target(), Some(&host), Some(&provider)).is_err(),
            "{ip}"
        );
    }
}
#[test]
fn address_shape_admission_does_not_claim_public_routability() {
    for ip in [
        "10.0.0.1",
        "192.0.2.1",
        "169.254.1.1",
        "100.64.0.1",
        "fc00::1",
        "fe80::1",
        "2001:db8::1",
    ] {
        let (mut host, mut provider) = observations();
        host.sample = sample(ip, 100);
        provider.sample = sample(ip, 100);
        let report = compare(100, &target(), Some(&host), Some(&provider)).unwrap();
        assert_eq!(report.comparison, Comparison::SameAddress);
        assert_eq!(report.vpn_evidence, VpnEvidence::NotEstablished);
    }
}
#[test]
fn invalid_expected_and_supplied_identity_shapes_are_refused() {
    let (host, mut provider) = observations();
    for bad in [
        "",
        "host-a",
        "A1111111-2222-4333-8444-555555555555",
        "a1111111_2222-4333-8444-555555555555",
    ] {
        let mut expected = target();
        expected.host_boot_id = bad.into();
        assert!(compare(100, &expected, None, None).is_err());
        provider.target.host_boot_id = bad.into();
        assert!(compare(100, &target(), Some(&host), Some(&provider)).is_err());
        let bad_host = HostObservation {
            host_boot_id: bad.into(),
            sample: host.sample,
        };
        assert!(compare(100, &target(), Some(&bad_host), None).is_err());
    }
    for bad in ["".into(), "x".repeat(129), "engine\nsecret".into()] {
        let mut expected = target();
        expected.engine_id = bad;
        assert!(compare(100, &expected, None, None).is_err());
    }
}
#[test]
fn abbreviated_container_ids_and_invalid_instance_shapes_are_refused() {
    for field in 0..3 {
        let mut expected = target();
        match field {
            0 => expected.provider.resource = "container:aaaaaaaaaaaa".into(),
            1 => expected.provider.image = "sha256:broken".into(),
            _ => expected.provider.started_at = "not-a-start-time".into(),
        }
        assert!(compare(100, &expected, None, None).is_err());
    }
}
#[test]
fn invalid_clock_and_sample_bounds_are_refused_before_subtraction() {
    let (mut host, mut provider) = observations();
    for bad in [i64::MIN, -1, MAX_TIME + 1, i64::MAX] {
        assert!(compare(bad, &target(), None, None).is_err());
        host.sample.sampled_at = bad;
        assert!(compare(100, &target(), Some(&host), None).is_err());
        provider.sample.sampled_at = bad;
        assert!(compare(100, &target(), None, Some(&provider)).is_err());
    }
}
#[test]
fn inclusive_clock_extremes_are_valid() {
    for now in [0, MAX_TIME] {
        let (mut host, mut provider) = observations();
        host.sample.sampled_at = now;
        provider.sample.sampled_at = now;
        assert_eq!(
            compare(now, &target(), Some(&host), Some(&provider))
                .unwrap()
                .comparison,
            Comparison::DifferentAddresses
        );
    }
}
#[test]
fn generated_at_never_replaces_original_probe_times() {
    let (host, provider) = observations();
    let a = compare(100, &target(), Some(&host), Some(&provider)).unwrap();
    let b = compare(180, &target(), Some(&host), Some(&provider)).unwrap();
    assert_eq!((a.host, a.provider), (b.host, b.provider));
    assert_eq!(b.generated_at, 180);
    assert_eq!(b.host.sampled_at, Some(100));
}
#[test]
fn serialized_report_checks_size_without_retaining_unbounded_json() {
    let (host, provider) = observations();
    let mut report = compare(100, &target(), Some(&host), Some(&provider)).unwrap();
    assert!(serde_json::to_vec(&report).unwrap().len() <= MAX_REPORT_BYTES);
    report.target.engine_id = "x".repeat(MAX_REPORT_BYTES);
    assert_eq!(report.check_size().unwrap_err().0, ErrorCode::Unavailable);
}
