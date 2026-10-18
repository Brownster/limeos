use super::*;
use serde_json::Value;

#[test]
fn frozen_config_fixtures_and_regressions() {
    let cases: Vec<Value> = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/deployment/tools/config.json"
    ))
    .unwrap();
    assert_eq!(cases.len(), 27);
    for case in cases {
        let read = match case["read"].as_str() {
            Some("absent") => ConfigRead::Absent,
            Some("unavailable") => ConfigRead::Unavailable,
            Some("incomplete") => ConfigRead::Incomplete,
            _ => ConfigRead::Bytes(case["body"].as_str().unwrap().as_bytes()),
        };
        let state = config(read);
        assert_eq!(
            serde_json::to_value(&state).unwrap(),
            case["expected"],
            "{}",
            case["name"]
        );
        assert!(!format!("{state:?}").contains("customer-secret"));
        assert!(
            !serde_json::to_string(&state)
                .unwrap()
                .contains("customer-secret")
        );
    }
}
#[test]
fn service_property_fixtures() {
    let cases: Vec<Value> = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/deployment/tools/services.json"
    ))
    .unwrap();
    assert_eq!(cases.len(), 20);
    for case in cases {
        let state = service(ServiceCapture {
            outcome: CaptureOutcome::Exited(0),
            complete: true,
            stdout: case["body"].as_str().unwrap().as_bytes(),
        });
        assert_eq!(
            serde_json::to_value(state).unwrap(),
            case["expected"],
            "{}",
            case["name"]
        );
    }
}
#[test]
fn failed_timed_out_pending_and_nonzero_captures_never_become_inactive() {
    for outcome in [
        CaptureOutcome::Exited(1),
        CaptureOutcome::Exited(-1),
        CaptureOutcome::TimedOut,
        CaptureOutcome::Failed,
        CaptureOutcome::Pending,
    ] {
        let state = service(ServiceCapture {
            outcome,
            complete: true,
            stdout: b"LoadState=not-found\nActiveState=inactive\n",
        });
        assert_eq!(state, ServiceState::Unknown(ServiceIssue::Unavailable));
    }
}
#[test]
fn incomplete_capture_never_uses_plausible_properties() {
    assert_eq!(
        service(ServiceCapture {
            outcome: CaptureOutcome::Exited(0),
            complete: false,
            stdout: b"LoadState=loaded\nActiveState=active\n"
        }),
        ServiceState::Unknown(ServiceIssue::Incomplete)
    );
}
#[test]
fn exact_config_byte_limit_is_inclusive() {
    let mut bytes = b"{}".to_vec();
    bytes.resize(MAX_CONFIG_BYTES, b' ');
    assert!(matches!(
        config(ConfigRead::Bytes(&bytes)),
        ConfigState::Stored { .. }
    ));
    bytes.push(b' ');
    assert_eq!(
        config(ConfigRead::Bytes(&bytes)),
        ConfigState::Unknown {
            reason: ConfigIssue::Limit
        }
    );
}
#[test]
fn oversized_property_capture_is_unknown_even_with_valid_prefix() {
    let mut bytes = b"LoadState=loaded\nActiveState=active\n".to_vec();
    bytes.resize(MAX_SERVICE_BYTES + 1, b' ');
    assert_eq!(
        service(ServiceCapture {
            outcome: CaptureOutcome::Exited(0),
            complete: true,
            stdout: &bytes
        }),
        ServiceState::Unknown(ServiceIssue::Limit)
    );
}
#[test]
fn invalid_utf8_and_control_properties_are_refused() {
    for bytes in [
        b"\xff".as_slice(),
        b"LoadState=loaded\nActiveState=active\0\n",
        b"LoadState=loaded\nActiveState=active\t\n",
    ] {
        assert_eq!(
            service(ServiceCapture {
                outcome: CaptureOutcome::Exited(0),
                complete: true,
                stdout: bytes
            }),
            ServiceState::Unknown(ServiceIssue::InvalidProperties)
        );
    }
    assert_eq!(
        config(ConfigRead::Bytes(b"\xff")),
        ConfigState::Unknown {
            reason: ConfigIssue::InvalidDocument
        }
    );
}
#[test]
fn debug_never_retains_raw_private_bytes() {
    let raw = b"customer-secret";
    assert!(!format!("{:?}", ConfigRead::Bytes(raw)).contains("customer-secret"));
    let capture = ServiceCapture {
        outcome: CaptureOutcome::Failed,
        complete: false,
        stdout: raw,
    };
    assert!(!format!("{capture:?}").contains("customer-secret"));
    assert!(
        !serde_json::to_string(&service(capture))
            .unwrap()
            .contains("customer-secret")
    );
}
#[test]
fn duplicate_path_and_flags_do_not_override_earlier_values() {
    for bytes in [
        b"{\"share_path\":\"/data\",\"share_path\":\"/etc\"}".as_slice(),
        b"{\"extra_args\":\"\",\"extra_args\":\"--password customer-secret\"}",
    ] {
        assert_eq!(
            config(ConfigRead::Bytes(bytes)),
            ConfigState::Unknown {
                reason: ConfigIssue::InvalidDocument
            }
        );
    }
}
#[test]
fn whitespace_document_and_deep_wrong_shapes_never_supply_defaults() {
    for bytes in [
        b" ".as_slice(),
        b"\xef\xbb\xbf{}",
        b"{\"port\":{\"value\":3923}}",
        b"{\"extra_args\":[\"-v\"]}",
    ] {
        assert_eq!(
            config(ConfigRead::Bytes(bytes)),
            ConfigState::Unknown {
                reason: ConfigIssue::InvalidDocument
            }
        );
    }
}
