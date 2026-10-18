use super::*;
use limeos_domain::deployment::notifications::{IgnoreReason, StackNotification};
use serde_json::{Value, json};

fn outcome(body: Value) -> Result<Normalization, DecodeError> {
    decode(&serde_json::to_vec(&body).unwrap(), Mode::Quiet, "stack")
}
fn note(body: Value) -> StackNotification {
    let Normalization::Notification { notification } = outcome(body).unwrap() else {
        panic!()
    };
    notification
}
#[test]
fn frozen_payload_fixtures() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/deployment/stack-notifications.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let actual = note(case["body"].clone());
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            case["expected"],
            "{}",
            case["name"]
        );
    }
}
#[test]
fn every_frozen_alias_under_both_policies() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/deployment/stack-notifications.json"
    ))
    .unwrap();
    for row in fixture["aliases"].as_array().unwrap() {
        let body = serde_json::to_vec(
            &json!({"eventType": format!(" {} ", row[0].as_str().unwrap().to_uppercase())}),
        )
        .unwrap();
        for mode in [Mode::Quiet, Mode::Verbose] {
            let actual = decode(&body, mode, "stack").unwrap();
            if row[1].is_null() {
                assert_eq!(
                    actual,
                    Normalization::Ignored {
                        reason: IgnoreReason::ConnectionTest
                    }
                );
            } else if mode == Mode::Quiet && !row[2].as_bool().unwrap() {
                assert_eq!(
                    actual,
                    Normalization::Ignored {
                        reason: IgnoreReason::Policy
                    }
                );
            } else {
                let Normalization::Notification { notification } = actual else {
                    panic!("{row}")
                };
                assert_eq!(serde_json::to_value(notification.event()).unwrap(), row[1]);
            }
        }
    }
}
#[test]
fn missing_null_and_unknown_event_are_intentionally_ignored() {
    for body in [
        json!({}),
        json!({"eventType": null}),
        json!({"eventType": "future"}),
    ] {
        assert_eq!(
            outcome(body).unwrap(),
            Normalization::Ignored {
                reason: IgnoreReason::UnknownEvent
            }
        );
    }
}
#[test]
fn malformed_bodies_are_not_suppression() {
    for bytes in [
        b"null".as_slice(),
        b"[]",
        b"true",
        b"not json",
        b"{} {}",
        b"{",
        b"\xff",
    ] {
        assert_eq!(
            decode(bytes, Mode::Quiet, "stack"),
            Err(DecodeError::InvalidBody),
            "body: {bytes:?}"
        );
    }
}
#[test]
fn selected_duplicate_keys_including_null_are_refused() {
    for body in [
        r#"{"eventType":"Download","eventType":"Test"}"#,
        r#"{"eventType":null,"eventType":"Download"}"#,
        r#"{"eventType":"Download","movie":{"title":null,"title":"other"}}"#,
        r#"{"eventType":"Download","movieFile":{"quality":{"name":null,"name":"HD"}}}"#,
        r#"{"eventType":"Download","isUpgrade":null,"isUpgrade":true}"#,
    ] {
        assert_eq!(
            decode(body.as_bytes(), Mode::Quiet, "stack"),
            Err(DecodeError::InvalidBody)
        );
    }
}
#[test]
fn malformed_selected_types_are_errors_even_for_suppressed_events() {
    for body in [
        json!({"eventType": 3}),
        json!({"eventType":"Test","message": []}),
        json!({"eventType":"Download","isUpgrade":"true"}),
        json!({"eventType":"Download","movie": []}),
        json!({"eventType":"Download","series": []}),
        json!({"eventType":"Download","episodes": [[]]}),
        json!({"eventType":"Download","movieFile": []}),
        json!({"eventType":"Download","release": []}),
        json!({"eventType":"Download","episodes": {}}),
        json!({"eventType":"Download","movieFile":{"quality": []}}),
        json!({"eventType":"Download","instanceName": {}}),
    ] {
        assert_eq!(
            outcome(body.clone()),
            Err(DecodeError::InvalidBody),
            "body: {body}"
        );
    }
}
#[test]
fn quality_file_order_and_release_fallback() {
    for (field, expected) in [
        ("movieFile", "movie"),
        ("episodeFile", "episode"),
        ("trackFile", "track"),
        ("bookFile", "book"),
    ] {
        let mut body = json!({"eventType":"Download","release":{"quality":"release"}});
        body[field] = json!({"quality":{"quality":{"name":expected}}});
        assert_eq!(note(body).detail(), expected);
    }
    for quality in [
        json!("direct"),
        json!({"name":"outer"}),
        json!({"quality":{"name":"nested"}}),
    ] {
        let expected = match &quality {
            Value::String(s) => s.as_str(),
            _ if quality.get("name").is_some() => "outer",
            _ => "nested",
        };
        assert_eq!(
            note(json!({"eventType":"Download","release":{"quality":quality}})).detail(),
            expected
        );
    }
}
#[test]
fn file_and_release_quality_use_their_distinct_reference_precedence() {
    assert_eq!(note(json!({"eventType":"Download","movieFile":{"quality":"ignored"},"release":{"quality":"fallback"}})).detail(), "fallback");
    assert_eq!(note(json!({"eventType":"Download","movieFile":{"quality":{"quality":"inner","name":"outer"}}})).detail(), "inner");
    assert_eq!(note(json!({"eventType":"Download","movieFile":{"quality":{"quality":{},"name":"not selected"}},"release":{"quality":{"quality":{},"name":"selected"}}})).detail(), "selected");
    assert_eq!(note(json!({"eventType":"Download","release":{"quality":{"quality":"not selected","name":"selected"}}})).detail(), "selected");
}
#[test]
fn movie_precedes_series_and_first_episode_is_selected() {
    assert_eq!(note(json!({"eventType":"Download","movie":{"title":"Movie","year":"2026"},"series":{"title":"Series"}})).detail(), "Movie (2026)");
    assert_eq!(note(json!({"eventType":"Download","series":{"title":"Series"},"episodes":[{"seasonNumber":"02","episodeNumber":"03"},{"seasonNumber":9,"episodeNumber":9}]})).detail(), "Series S02E03");
}
#[test]
fn missing_episode_parts_have_explicit_reference_fallbacks() {
    for (episodes, expected) in [
        (json!([]), "Series"),
        (json!([null]), "Series"),
        (json!([{}]), "Series"),
        (json!([{"seasonNumber":2}]), "Series S02E00"),
        (json!([{"episodeNumber":3}]), "Series"),
    ] {
        assert_eq!(
            note(json!({"eventType":"Download","series":{"title":"Series"},"episodes":episodes}))
                .detail(),
            expected
        );
    }
}
#[test]
fn invalid_numbers_are_not_coerced_to_labels() {
    for value in [
        json!(-1),
        json!(1.5),
        json!(true),
        json!(65536),
        json!("+2"),
        json!("2.0"),
        json!("999999"),
    ] {
        assert_eq!(
            outcome(
                json!({"eventType":"Download","series":{"title":"Series"},"episodes":[{"seasonNumber":value}]})
            ),
            Err(DecodeError::InvalidBody)
        );
    }
    assert_eq!(
        outcome(json!({"eventType":"Download","movie":{"year":10000}})),
        Err(DecodeError::InvalidInput)
    );
}
#[test]
fn episode_count_boundary_retains_only_first_label() {
    let mut episodes = vec![json!({"seasonNumber":1,"episodeNumber":2}); MAX_EPISODES];
    assert_eq!(
        note(json!({"eventType":"Download","series":{"title":"Series"},"episodes":episodes}))
            .detail(),
        "Series S01E02"
    );
    episodes.push(json!({}));
    assert_eq!(
        outcome(json!({"eventType":"Download","episodes":episodes})),
        Err(DecodeError::InvalidBody)
    );
}
#[test]
fn body_size_is_checked_before_parsing() {
    let mut bytes = br#"{"eventType":"Test"}"#.to_vec();
    bytes.resize(MAX_BODY_BYTES, b' ');
    assert!(matches!(
        decode(&bytes, Mode::Verbose, "stack"),
        Ok(Normalization::Ignored {
            reason: IgnoreReason::ConnectionTest
        })
    ));
    bytes.push(b' ');
    assert_eq!(
        decode(&bytes, Mode::Verbose, "stack"),
        Err(DecodeError::BodyTooLarge)
    );
}
#[test]
fn selected_string_bound_and_control_characters() {
    assert_eq!(
        note(json!({"eventType":"Health","message":"x".repeat(MAX_FIELD_BYTES)}))
            .detail()
            .len(),
        500
    );
    for message in [
        "x".repeat(MAX_FIELD_BYTES + 1),
        "bad\0text".into(),
        "bad\u{1b}text".into(),
    ] {
        assert_eq!(
            outcome(json!({"eventType":"Health","message":message})),
            Err(DecodeError::InvalidBody)
        );
    }
}
#[test]
fn unknown_credentials_and_large_unknown_text_are_not_retained() {
    let notification = note(
        json!({"eventType":"Download","movie":{"title":"Example","apiKey":"nested-secret"},"password":"private-password","unknown":"x".repeat(100_000)}),
    );
    let output = serde_json::to_string(&notification).unwrap();
    assert!(!output.contains("secret"));
    assert!(!output.contains("password"));
    assert!(!format!("{notification:?}").contains("Example"));
    assert!(output.len() < 256);
}
#[test]
fn excessive_unknown_nesting_is_refused() {
    let body = format!(
        "{{\"eventType\":\"Test\",\"unknown\":{}0{}}}",
        "[".repeat(256),
        "]".repeat(256)
    );
    assert_eq!(
        decode(body.as_bytes(), Mode::Quiet, "stack"),
        Err(DecodeError::InvalidBody)
    );
}

#[test]
fn exact_nesting_boundary_counts_unknown_containers_only_outside_strings() {
    let body = |arrays: usize| {
        format!(
            "{{\"eventType\":\"Test\",\"unknown\":{}0{}}}",
            "[".repeat(arrays),
            "]".repeat(arrays)
        )
    };
    assert!(decode(body(MAX_JSON_DEPTH - 1).as_bytes(), Mode::Quiet, "stack").is_ok());
    assert_eq!(
        decode(body(MAX_JSON_DEPTH).as_bytes(), Mode::Quiet, "stack"),
        Err(DecodeError::InvalidBody)
    );
    let body = json!({"eventType":"Health","message":"[[[{{{ \" quote \\ slash }}}]]]"});
    assert_eq!(
        note(body.clone()).detail(),
        body["message"].as_str().unwrap()
    );
}
