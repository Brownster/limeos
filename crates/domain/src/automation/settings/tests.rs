use super::*;
use crate::automation::{
    AlertEventKind, AlertKind, AlertPolicy, AlertSignal, IncidentState, Severity, SignalStatus,
    Silence, fold_alert,
};
use serde_json::{Value, json};

#[cfg(feature = "deferred")]
const CREATED: &str = "2026-10-20T12:00:00+00:00";
#[cfg(feature = "deferred")]
fn plan(body: &str) -> Result<PlannedHealthSettings> {
    plan_health_settings(
        LegacyHealthEnvironment::default(),
        LegacyPolicyRead::Contents(body),
        CREATED,
    )
}
fn fixture() -> Vec<Value> {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/automation/frozen-notification-settings.json"
    ))
    .unwrap();
    assert_eq!(
        fixture["reference_commit"],
        "80593b29443ea9b81eb60ed4e5c3c3ac7a236204"
    );
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 32);
    fixture["cases"].as_array().unwrap().clone()
}
#[cfg(feature = "deferred")]
fn silence_value() -> Value {
    json!({
        "kind":"mount", "key":"mount:/mnt/Media", "created_at": CREATED,
        "expires_at":"2026-10-20T13:00:00+00:00", "reason":"  disk maintenance  "
    })
}
#[cfg(feature = "deferred")]
fn with_silence(silence: Value) -> Result<PlannedHealthSettings> {
    plan(&json!({"silences":[silence]}).to_string())
}
#[cfg(feature = "deferred")]
fn assert_issue<T: std::fmt::Debug>(result: Result<T>, field: SettingField, reason: ReviewReason) {
    let error = result.unwrap_err();
    assert_eq!((error.field, error.reason), (field, reason));
}

#[cfg(feature = "deferred")]
#[test]
fn frozen_daemon_cases_preserve_supported_values_or_require_explicit_review() {
    let mut cases = 0;
    for case in fixture() {
        let name = case["id"].as_str().unwrap();
        if !name.starts_with("daemon-") {
            continue;
        }
        cases += 1;
        let env = &case["input"]["environment"];
        let result = plan_health_settings(
            LegacyHealthEnvironment {
                poll_seconds: env["LIMEOS_ALERT_POLL_SECONDS"].as_str(),
                fail_threshold: env["LIMEOS_ALERT_FAIL_THRESHOLD"].as_str(),
                required_mounts: env["LIMEOS_ALERT_REQUIRED_MOUNTS"].as_str(),
            },
            LegacyPolicyRead::NotConfigured,
            CREATED,
        );
        if name == "daemon-clamps-zero" {
            assert_eq!(case["output"]["poll_seconds"], 1);
            assert_issue(
                result,
                SettingField::PollSeconds,
                ReviewReason::OutsideRuntimeBounds,
            );
        } else if name == "daemon-large-threshold" {
            assert_eq!(case["output"]["fail_threshold"], 100);
            assert_issue(
                result,
                SettingField::FailThreshold,
                ReviewReason::OutsideRuntimeBounds,
            );
        } else {
            let candidate = result.unwrap();
            assert_eq!(
                json!(candidate.poll_seconds()),
                case["output"]["poll_seconds"],
                "{name}"
            );
            assert_eq!(
                json!(candidate.fail_threshold()),
                case["output"]["fail_threshold"],
                "{name}"
            );
            let mut mounts = Vec::new();
            for value in case["output"]["required_mounts"].as_array().unwrap() {
                if !mounts.contains(value) {
                    mounts.push(value.clone());
                }
            }
            assert_eq!(json!(candidate.required_mounts()), json!(mounts), "{name}");
            assert!(candidate.alert_policy().validate().is_ok());
            assert_eq!(
                candidate.origin(),
                PolicyOrigin::Environment(PolicyFallback::NotConfigured)
            );
            let output = serde_json::to_string(&candidate).unwrap();
            assert!(!output.contains("present-secret"));
            assert!(!output.contains("webhook"));
        }
    }
    assert_eq!(cases, 5);
}

#[cfg(feature = "deferred")]
#[test]
fn frozen_policy_cases_preserve_categories_literal_mounts_and_silence_metadata() {
    let mut cases = 0;
    for case in fixture() {
        let name = case["id"].as_str().unwrap();
        if !name.starts_with("policy-") {
            continue;
        }
        cases += 1;
        let result = plan(&case["input"].to_string());
        if case["output"].get("error").is_some() {
            assert!(result.is_err(), "{name}");
            continue;
        }
        let candidate = result.unwrap();
        let mut category_values = Map::new();
        let policy = candidate.alert_policy();
        for (kind, key) in KINDS {
            category_values.insert(
                key.into(),
                json!({"enabled":!policy.disabled.contains(&kind)}),
            );
        }
        let silences: Vec<_> = candidate
            .silences()
            .iter()
            .map(|silence| {
                json!({
                    "kind":silence.kind, "key":silence.key,
                    "created_at":silence.created_at.text(),
                    "expires_at":silence.expires_at.as_ref().map(PlannedTimestamp::text),
                    "reason":silence.reason,
                })
            })
            .collect();
        assert_eq!(
            json!({
                "version":1, "categories":category_values,
                "required_mounts":candidate.required_mounts(), "silences":silences,
            }),
            case["output"]["value"],
            "{name}"
        );
        assert_eq!(
            candidate.fail_threshold(),
            2,
            "policy never overrides daemon threshold"
        );
    }
    assert_eq!(cases, 7);
}

#[cfg(feature = "deferred")]
#[test]
fn frozen_policy_file_precedence_keeps_empty_separate_from_missing() {
    let mut cases = 0;
    for case in fixture() {
        let Some(input) = case
            .get("input")
            .filter(|input| input.get("policy_json").is_some())
        else {
            continue;
        };
        cases += 1;
        let source = match (input["policy_json"].as_str(), input["read_error"].as_str()) {
            (_, Some(_)) => LegacyPolicyRead::Missing,
            (Some(body), _) => LegacyPolicyRead::Contents(body),
            _ => LegacyPolicyRead::NotConfigured,
        };
        let result = plan_health_settings(
            LegacyHealthEnvironment {
                required_mounts: Some("/mnt/Required"),
                ..Default::default()
            },
            source,
            CREATED,
        );
        if case["output"].get("error").is_some() {
            assert_issue(result, SettingField::Policy, ReviewReason::InvalidShape);
        } else {
            assert_eq!(
                json!(result.unwrap().required_mounts()),
                case["output"]["value"],
                "{}",
                case["id"]
            );
        }
    }
    assert_eq!(cases, 7);
}

/// The runtime form of `silence_value()` with a threshold of one.
fn expiring_policy() -> (AlertPolicy, i64) {
    let expiry = timestamp::parse("2026-10-20T13:00:00+00:00")
        .unwrap()
        .unix_seconds();
    let policy = AlertPolicy {
        fail_threshold: 1,
        disabled: vec![],
        silences: vec![Silence {
            kind: AlertKind::Mount,
            key: "mount:/mnt/Media".into(),
            expires_at: Some(expiry),
        }],
    };
    (policy, expiry)
}
#[cfg(feature = "deferred")]
#[test]
fn planned_silence_has_the_runtime_expiry() {
    let mut planned = with_silence(silence_value()).unwrap().alert_policy();
    planned.fail_threshold = 1;
    assert_eq!(planned, expiring_policy().0);
}
#[test]
fn frozen_exclusive_expiry_and_real_fold_do_not_erase_muted_incidents() {
    let (policy, expiry) = expiring_policy();
    let signal = AlertSignal {
        key: "mount:/mnt/Media".into(),
        kind: AlertKind::Mount,
        status: SignalStatus::Failing,
        severity: Severity::Critical,
        summary: "Missing required mount".into(),
    };
    for case in fixture()
        .iter()
        .filter(|case| case["id"].as_str().unwrap().starts_with("silence-"))
    {
        let now = if case["id"] == "silence-before-expiry" {
            expiry - 1
        } else {
            expiry
        };
        assert_eq!(json!(policy.allows(&signal, now)), case["output"]["allows"]);
    }
    let muted = fold_alert(None, &signal, &policy, 1, expiry - 1).unwrap();
    assert!(matches!(
        muted.next.incident,
        IncidentState::Open {
            announced: false,
            ..
        }
    ));
    assert!(muted.event.is_none());
    let announced = fold_alert(Some(&muted.next), &signal, &policy, 2, expiry).unwrap();
    assert_eq!(announced.event.unwrap().event, AlertEventKind::Incident);
}

#[cfg(feature = "deferred")]
#[test]
fn integer_syntax_fallback_clamping_and_runtime_limits_are_distinct() {
    for (raw, expected) in [
        ("", 2),
        ("bad", 2),
        ("_3", 2),
        ("3_", 2),
        ("3__0", 2),
        ("0x10", 2),
        ("3.0", 2),
        ("  +03  ", 3),
        ("1_6", 16),
        ("0", 1),
        ("-999999999999999999999999", 1),
    ] {
        let candidate = plan_health_settings(
            LegacyHealthEnvironment {
                fail_threshold: Some(raw),
                ..Default::default()
            },
            LegacyPolicyRead::NotConfigured,
            CREATED,
        )
        .unwrap();
        assert_eq!(candidate.fail_threshold(), expected, "{raw}");
    }
    for raw in [
        "17",
        "100",
        "4294967296",
        "9999999999999999999999999999999999999",
    ] {
        assert_issue(
            plan_health_settings(
                LegacyHealthEnvironment {
                    fail_threshold: Some(raw),
                    ..Default::default()
                },
                LegacyPolicyRead::NotConfigured,
                CREATED,
            ),
            SettingField::FailThreshold,
            ReviewReason::OutsideRuntimeBounds,
        );
    }
    for raw in ["２", "١٢"] {
        assert_issue(
            plan_health_settings(
                LegacyHealthEnvironment {
                    fail_threshold: Some(raw),
                    ..Default::default()
                },
                LegacyPolicyRead::NotConfigured,
                CREATED,
            ),
            SettingField::FailThreshold,
            ReviewReason::UnsupportedInteger,
        );
    }
    for (raw, expected) in [
        ("3_0", Some(30)),
        ("2678400", Some(2_678_400)),
        ("29", None),
        ("2678401", None),
        ("-30", None),
        ("0", None),
    ] {
        let result = plan_health_settings(
            LegacyHealthEnvironment {
                poll_seconds: Some(raw),
                ..Default::default()
            },
            LegacyPolicyRead::NotConfigured,
            CREATED,
        );
        if let Some(expected) = expected {
            assert_eq!(result.unwrap().poll_seconds(), expected);
        } else {
            assert_issue(
                result,
                SettingField::PollSeconds,
                ReviewReason::OutsideRuntimeBounds,
            );
        }
    }
}

#[cfg(feature = "deferred")]
#[test]
fn policy_read_outcomes_and_legacy_falsey_values_are_explicit() {
    for (source, fallback) in [
        (LegacyPolicyRead::Missing, PolicyFallback::Missing),
        (LegacyPolicyRead::Unreadable, PolicyFallback::Unreadable),
        (LegacyPolicyRead::Contents("{"), PolicyFallback::Malformed),
        (LegacyPolicyRead::Contents("null"), PolicyFallback::Null),
    ] {
        let candidate = plan_health_settings(
            LegacyHealthEnvironment {
                required_mounts: Some("/env"),
                ..Default::default()
            },
            source,
            CREATED,
        )
        .unwrap();
        assert_eq!(candidate.required_mounts(), ["/env"]);
        assert_eq!(candidate.origin(), PolicyOrigin::Environment(fallback));
    }
    for body in [
        "{}",
        "false",
        "0",
        "0.0",
        "[]",
        r#""""#,
        r#"{"policy":null}"#,
        r#"{"policy":{}}"#,
    ] {
        let candidate = plan_health_settings(
            LegacyHealthEnvironment {
                required_mounts: Some("invalid/ignored"),
                ..Default::default()
            },
            LegacyPolicyRead::Contents(body),
            CREATED,
        )
        .unwrap();
        assert!(candidate.required_mounts().is_empty(), "{body}");
        assert_eq!(candidate.origin(), PolicyOrigin::Policy);
    }
    for body in ["true", "1", "[1]", r#""text""#] {
        assert_issue(plan(body), SettingField::Policy, ReviewReason::InvalidShape);
    }
    for body in ["1e400", r#"{"unused":"\ud800"}"#] {
        assert_issue(plan(body), SettingField::Policy, ReviewReason::InvalidValue);
    }
}

#[cfg(feature = "deferred")]
#[test]
fn invalid_structures_refuse_without_partial_configuration_or_raw_diagnostics() {
    for (body, field) in [
        (json!({"categories":null}), SettingField::Categories),
        (
            json!({"categories":{"mount":false}}),
            SettingField::Categories,
        ),
        (
            json!({"categories":{"container":{"enabled":"false"}}}),
            SettingField::Categories,
        ),
        (
            json!({"required_mounts":null}),
            SettingField::RequiredMounts,
        ),
        (
            json!({"required_mounts":["/valid",7]}),
            SettingField::RequiredMounts,
        ),
        (json!({"silences":{}}), SettingField::Silences),
        (
            json!({"silences":[{"kind":"mount","key":"mount:/secret"},null]}),
            SettingField::Silences,
        ),
    ] {
        let error = plan(&body.to_string()).unwrap_err();
        assert_eq!(
            (error.field, error.reason),
            (field, ReviewReason::InvalidShape)
        );
        assert!(!serde_json::to_string(&error).unwrap().contains("secret"));
    }
    // Unknown legacy fields, including a misplaced threshold, had no effect.
    let candidate =
        plan(r#"{"version":99,"fail_threshold":9,"categories":{"host":{"enabled":false}}}"#)
            .unwrap();
    assert_eq!(candidate.alert_policy(), AlertPolicy::default());
}

#[cfg(feature = "deferred")]
#[test]
fn literal_mounts_keep_case_components_and_exact_only_deduplication() {
    let candidate =
        plan(r#"{"required_mounts":["/mnt/Media","/mnt/media","/mnt/Media","/a/../b","//b","/"]}"#)
            .unwrap();
    assert_eq!(
        candidate.required_mounts(),
        ["/mnt/Media", "/mnt/media", "/a/../b", "//b", "/"]
    );
    let candidate = plan_health_settings(
        LegacyHealthEnvironment {
            required_mounts: Some(" , /mnt/Media, /mnt/media , /mnt/Media, "),
            ..Default::default()
        },
        LegacyPolicyRead::NotConfigured,
        CREATED,
    )
    .unwrap();
    assert_eq!(candidate.required_mounts(), ["/mnt/Media", "/mnt/media"]);
    for path in ["relative", "", "/nul\0path"] {
        assert_issue(
            plan(&json!({"required_mounts":[path]}).to_string()),
            SettingField::RequiredMounts,
            ReviewReason::InvalidPath,
        );
    }
    let path = format!("/{}", "x".repeat(249));
    assert!(plan(&json!({"required_mounts":[path]}).to_string()).is_ok());
    for path in [format!("/{}", "x".repeat(250)), "/control\npath".into()] {
        assert_issue(
            plan(&json!({"required_mounts":[path]}).to_string()),
            SettingField::RequiredMounts,
            ReviewReason::OutsideRuntimeBounds,
        );
    }
}

#[cfg(feature = "deferred")]
#[test]
fn input_and_collection_limits_never_truncate_a_candidate() {
    for (field, environment) in [
        (
            SettingField::PollSeconds,
            LegacyHealthEnvironment {
                poll_seconds: Some(&"x".repeat(MAX_ENVIRONMENT_BYTES + 1)),
                ..Default::default()
            },
        ),
        (
            SettingField::FailThreshold,
            LegacyHealthEnvironment {
                fail_threshold: Some(&"x".repeat(MAX_ENVIRONMENT_BYTES + 1)),
                ..Default::default()
            },
        ),
        (
            SettingField::RequiredMounts,
            LegacyHealthEnvironment {
                required_mounts: Some(&"x".repeat(MAX_ENVIRONMENT_BYTES + 1)),
                ..Default::default()
            },
        ),
    ] {
        assert_issue(
            plan_health_settings(environment, LegacyPolicyRead::NotConfigured, CREATED),
            field,
            ReviewReason::InputLimit,
        );
    }
    assert_issue(
        plan(&" ".repeat(MAX_POLICY_BYTES + 1)),
        SettingField::Policy,
        ReviewReason::InputLimit,
    );
    let body = format!(
        "{}0{}",
        "[".repeat(MAX_JSON_DEPTH + 1),
        "]".repeat(MAX_JSON_DEPTH + 1)
    );
    assert_issue(plan(&body), SettingField::Policy, ReviewReason::InputLimit);
    // Brackets and escaped quotes inside a string do not spend nesting depth.
    assert!(
        plan(&json!({"unused":format!("{}\\\"{}","[".repeat(100),"]".repeat(100))}).to_string())
            .is_ok()
    );
    for (count, ok) in [(256, true), (257, false)] {
        let paths: Vec<_> = (0..count).map(|n| format!("/m{n}")).collect();
        let result = plan(&json!({"required_mounts":paths}).to_string());
        if ok {
            assert_eq!(result.unwrap().required_mounts().len(), count);
        } else {
            assert_issue(
                result,
                SettingField::RequiredMounts,
                ReviewReason::TooManyEntries,
            );
        }
    }
    for (count, ok) in [(64, true), (65, false)] {
        let result = plan(&json!({"silences":vec![silence_value();count]}).to_string());
        if ok {
            assert_eq!(result.unwrap().silences().len(), count);
        } else {
            assert_issue(result, SettingField::Silences, ReviewReason::TooManyEntries);
        }
    }
}

#[cfg(feature = "deferred")]
#[test]
fn silence_metadata_keeps_original_time_text_reason_and_duplicate_rows() {
    let mut raw = silence_value();
    raw["created_at"] = json!("2026-10-20T17:45:00.123456+05:45");
    raw["reason"] = json!(format!("  {}  ", "維".repeat(200)));
    raw["key"] = json!("  mount:/mnt/Media  ");
    let candidate = with_silence(raw.clone()).unwrap();
    let silence = &candidate.silences()[0];
    assert_eq!(silence.key, "mount:/mnt/Media");
    assert_eq!(silence.reason, "維".repeat(200));
    assert_eq!(
        silence.created_at.text(),
        "2026-10-20T17:45:00.123456+05:45"
    );
    assert_eq!(silence.created_at.micros(), 123456);
    assert_eq!(
        silence.created_at.unix_seconds(),
        timestamp::parse(CREATED).unwrap().unix_seconds()
    );
    raw["reason"] = json!("維".repeat(201));
    assert_issue(
        with_silence(raw),
        SettingField::SilenceReason,
        ReviewReason::InputLimit,
    );
    let raw = json!({"kind":"mount","key":"mount:/mnt/Media"});
    let candidate = plan(&json!({"silences":[raw.clone(),raw]}).to_string()).unwrap();
    assert_eq!(candidate.silences().len(), 2);
    assert_eq!(candidate.silences()[0].created_at.text(), CREATED);
    assert_eq!(candidate.silences()[0].reason, "");
    assert!(candidate.silences()[0].expires_at.is_none());
    assert!(!candidate.alert_policy().allows(
        &AlertSignal {
            key: "mount:/mnt/Media".into(),
            kind: AlertKind::Mount,
            status: SignalStatus::Healthy,
            severity: Severity::Warning,
            summary: "fixture".into()
        },
        253_402_300_799
    ));
}

#[cfg(feature = "deferred")]
#[test]
fn silence_key_category_and_expiry_review_boundaries_are_explicit() {
    for (field, value, expected) in [
        (
            "kind",
            json!("host"),
            (SettingField::SilenceKind, ReviewReason::InvalidValue),
        ),
        (
            "key",
            json!("container:Media"),
            (SettingField::SilenceKey, ReviewReason::InvalidValue),
        ),
        (
            "key",
            json!("mount:"),
            (SettingField::SilenceKey, ReviewReason::OutsideRuntimeBounds),
        ),
        (
            "key",
            json!(format!("mount:{}", "a".repeat(251))),
            (SettingField::SilenceKey, ReviewReason::OutsideRuntimeBounds),
        ),
        (
            "created_at",
            json!("2026-10-20T12:00:00"),
            (
                SettingField::SilenceCreatedAt,
                ReviewReason::UnsupportedTimestamp,
            ),
        ),
        (
            "expires_at",
            json!("2026-10-20T13:00:00.000001+00:00"),
            (
                SettingField::SilenceExpiresAt,
                ReviewReason::SubsecondExpiry,
            ),
        ),
        (
            "reason",
            json!(["not", "text"]),
            (SettingField::SilenceReason, ReviewReason::InvalidShape),
        ),
    ] {
        let mut raw = silence_value();
        raw[field] = value;
        let error = with_silence(raw).unwrap_err();
        assert_eq!((error.field, error.reason), expected, "{field}");
        assert_eq!(error.index, Some(0));
    }
    let mut raw = silence_value();
    raw["expires_at"] = json!("2026-10-20T13:00:00.000000Z");
    assert!(with_silence(raw).is_ok());
}

#[test]
fn calendar_offset_and_fraction_profile_matches_independent_known_instants() {
    // Constants independently computed with Python's datetime.fromisoformat.
    for (text, seconds) in [
        ("1970-01-01T00:00:00Z", 0),
        ("2000-02-29T12:34:56+00:00", 951_827_696),
        ("2026-10-20T12:00:00+00:00", 1_792_497_600),
        ("2100-03-01T00:00:00Z", 4_107_542_400),
        ("2024-02-29T23:59:59-03:30", 1_709_263_799),
        ("1970-01-01T23:59:00+23:59", 0),
        ("9999-12-31T23:59:59+00:00", 253_402_300_799),
        ("2026-10-20T13:00:00+05:45", 1_792_480_500),
    ] {
        let at = timestamp::parse(text).unwrap();
        assert_eq!(at.unix_seconds(), seconds, "{text}");
        assert_eq!(at.text(), text);
        assert_eq!(at.micros(), 0);
    }
    for (fraction, micros) in [("1", 100_000), ("12", 120_000), ("123456", 123_456)] {
        assert_eq!(
            timestamp::parse(&format!("2026-10-20 12:00:00.{fraction}Z"))
                .unwrap()
                .micros(),
            micros
        );
    }
    for text in [
        "1900-02-29T00:00:00Z",
        "2100-02-29T00:00:00Z",
        "2026-13-01T00:00:00Z",
        "2026-01-32T00:00:00Z",
        "2026-01-01T24:00:00Z",
        "2026-01-01T23:60:00Z",
        "2026-01-01T23:59:60Z",
        "2026-01-01T00:00:00+24:00",
        "2026-01-01T00:00:00+00:60",
        "2026-01-01T00:00:00.1234567Z",
        "2026-01-01T00:00:00.Z",
        "2026-W43-2T00:00:00Z",
        "20260101T000000Z",
        "1969-12-31T23:59:59Z",
        "9999-12-31T23:59:59-00:01",
        "0000-01-01T00:00:00Z",
        "2026-01-01X00:00:00Z",
        "2026-01-01T00:00:00Ztrailing",
        "２０２６-01-01T00:00:00Z",
    ] {
        assert!(timestamp::parse(text).is_none(), "{text}");
    }
}
