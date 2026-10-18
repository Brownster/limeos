use super::*;

fn note(input: Input<'_>) -> StackNotification {
    let Normalization::Notification { notification } =
        normalize(Mode::Quiet, "stack", input).unwrap()
    else {
        panic!("expected a notification")
    };
    notification
}
fn input() -> Input<'static> {
    Input {
        event_type: "Download",
        ..Input::default()
    }
}
#[test]
fn default_is_quiet() {
    assert_eq!(Mode::default(), Mode::Quiet);
    assert!(serde_json::from_str::<Mode>("\"loud\"").is_err());
}
#[test]
fn unknown_and_test_are_distinct_from_policy_suppression() {
    for (event_type, reason) in [
        ("future-event", IgnoreReason::UnknownEvent),
        (" Test ", IgnoreReason::ConnectionTest),
        ("Grab", IgnoreReason::Policy),
        ("", IgnoreReason::UnknownEvent),
    ] {
        assert_eq!(
            normalize(
                Mode::Quiet,
                "stack",
                Input {
                    event_type,
                    ..input()
                }
            )
            .unwrap(),
            Normalization::Ignored { reason }
        );
    }
}
#[test]
fn verbose_still_suppresses_connection_tests() {
    assert_eq!(
        normalize(
            Mode::Verbose,
            "stack",
            Input {
                event_type: "Test",
                ..input()
            }
        )
        .unwrap(),
        Normalization::Ignored {
            reason: IgnoreReason::ConnectionTest
        }
    );
}
#[test]
fn imported_upgrade_is_not_a_failed_event() {
    let notification = note(Input {
        is_upgrade: true,
        ..input()
    });
    assert_eq!(notification.event(), Event::Upgraded);
    assert_eq!(notification.severity(), Severity::Info);
    assert_eq!(notification.title(), "Upgraded");
}
#[test]
fn upgrade_flag_does_not_change_other_events() {
    let notification = note(Input {
        event_type: "ImportFailure",
        is_upgrade: true,
        ..input()
    });
    assert_eq!(notification.event(), Event::Failed);
    assert_eq!(notification.severity(), Severity::Warning);
}
#[test]
fn health_error_only_changes_health_severity() {
    for (event_type, expected) in [
        ("Health", Severity::Critical),
        ("HealthRestored", Severity::Info),
        ("DownloadFailure", Severity::Warning),
    ] {
        assert_eq!(
            note(Input {
                event_type,
                health_level: Some(" Error "),
                ..input()
            })
            .severity(),
            expected
        );
    }
}
#[test]
fn source_is_display_data_with_lowercase_and_fallback() {
    assert_eq!(
        note(Input {
            instance_name: Some("  My Radarr  "),
            ..input()
        })
        .source(),
        "my radarr"
    );
    assert_eq!(
        note(Input {
            instance_name: Some(" \t "),
            ..input()
        })
        .source(),
        "stack"
    );
    let Normalization::Notification { notification } =
        normalize(Mode::Quiet, " Media ", input()).unwrap()
    else {
        panic!()
    };
    assert_eq!(notification.source(), " Media ");
}
#[test]
fn lowercase_expansion_is_bounded_after_conversion() {
    let source = "İ".repeat(128);
    let notification = note(Input {
        instance_name: Some(&source),
        ..input()
    });
    assert_eq!(notification.source().chars().count(), MAX_SOURCE_CHARS);
    assert!(notification.source().len() <= 4 * MAX_SOURCE_CHARS);
}
#[test]
fn unicode_detail_uses_character_not_byte_limit() {
    let message = "🦀".repeat(800);
    let notification = note(Input {
        event_type: "Health",
        message: Some(&message),
        ..input()
    });
    assert_eq!(notification.detail(), "🦀".repeat(MAX_DETAIL_CHARS));
    assert_eq!(notification.detail().len(), 4 * MAX_DETAIL_CHARS);
}
#[test]
fn details_remain_untrusted_multiline_text() {
    let message = "<script>private-text</script>\nline two\tdata";
    let notification = note(Input {
        event_type: "Health",
        message: Some(message),
        ..input()
    });
    assert_eq!(notification.detail(), message);
    assert!(!format!("{notification:?}").contains("private-text"));
    assert!(
        serde_json::to_string(&notification)
            .unwrap()
            .contains("private-text")
    );
}
#[test]
fn health_uses_message_and_media_events_use_media() {
    let media = Media::Movie {
        title: Some("Example"),
        year: Some(2026),
    };
    assert_eq!(
        note(Input {
            media,
            message: Some("not used"),
            ..input()
        })
        .detail(),
        "Example (2026)"
    );
    assert_eq!(
        note(Input {
            event_type: "Health",
            media,
            message: Some("health"),
            ..input()
        })
        .detail(),
        "health"
    );
}
#[test]
fn missing_media_and_zero_year_match_reference_fallbacks() {
    assert_eq!(note(input()).detail(), "(no details)");
    assert_eq!(
        note(Input {
            media: Media::Movie {
                title: None,
                year: Some(0)
            },
            ..input()
        })
        .detail(),
        "?"
    );
    assert_eq!(
        note(Input {
            media: Media::Series {
                title: Some("Series"),
                episode: Some((2, 0))
            },
            ..input()
        })
        .detail(),
        "Series S02E00"
    );
}
#[test]
fn field_limits_controls_and_default_source_fail_explicitly() {
    let oversized = "x".repeat(MAX_FIELD_BYTES + 1);
    for message in [&oversized, "bad\0text", "bad\u{1b}text"] {
        assert!(
            normalize(
                Mode::Quiet,
                "stack",
                Input {
                    message: Some(message),
                    ..input()
                }
            )
            .is_err()
        );
    }
    assert!(normalize(Mode::Quiet, "  ", input()).is_err());
    assert!(
        normalize(
            Mode::Quiet,
            "stack",
            Input {
                media: Media::Movie {
                    title: None,
                    year: Some(10000)
                },
                ..input()
            }
        )
        .is_err()
    );
    let boundary = "x".repeat(MAX_FIELD_BYTES);
    assert_eq!(
        note(Input {
            quality: Some(&boundary),
            ..input()
        })
        .detail()
        .len(),
        MAX_DETAIL_CHARS
    );
}
