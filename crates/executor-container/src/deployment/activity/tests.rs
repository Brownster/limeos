use super::*;
use serde_json::{Value, json};
const JF: &[u8] = include_bytes!("../../../../../tests/fixtures/deployment/activity/jellyfin.json");
const SAB: &[u8] = include_bytes!("../../../../../tests/fixtures/deployment/activity/sabnzbd.json");
const TR: &[u8] =
    include_bytes!("../../../../../tests/fixtures/deployment/activity/transmission.json");
const ARR: &[u8] = include_bytes!("../../../../../tests/fixtures/deployment/activity/arr.json");
fn project(provider: Provider, value: &Value) -> Result<ActivityData> {
    decode(provider, &serde_json::to_vec(value).unwrap())
}
#[test]
fn frozen_jellyfin_and_emby_stream_values_are_preserved() {
    for provider in [Provider::Jellyfin, Provider::Emby] {
        let ActivityData::Streams(rows) = decode(provider, JF).unwrap() else {
            panic!()
        };
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.title, "Reservoir Dogs");
        assert_eq!(r.subtitle, "Copycat Killers · S02E10");
        assert_eq!(r.user, "holly");
        assert!(r.transcoding);
        assert!(!r.paused);
        assert_eq!(
            r.transcode_reason.as_deref(),
            Some("VideoCodecNotSupported")
        );
        assert_eq!(r.progress_percent, Some(25.0));
        assert_eq!(r.duration_seconds, Some(3600.0));
        assert_eq!(r.position_seconds, Some(900.0));
    }
}
#[test]
fn frozen_sab_queue_uses_byte_units_and_only_active_row_gets_rate() {
    let ActivityData::Downloads {
        items,
        complete,
        aggregate_speed,
    } = decode(Provider::Sabnzbd, SAB).unwrap()
    else {
        panic!()
    };
    assert!(complete);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].progress_percent, Some(42.0));
    assert_eq!(items[0].size_bytes, Some(1024.0 * 1024.0 * 1024.0));
    assert_eq!(items[0].remaining_bytes, Some(512.0 * 1024.0 * 1024.0));
    assert_eq!(items[0].speed_bytes, Some(5120.0 * 1024.0));
    assert_eq!(items[1].speed_bytes, Some(0.0));
    assert_eq!(aggregate_speed, items[0].speed_bytes);
}
#[test]
fn frozen_transmission_filters_idle_and_preserves_download_metrics() {
    let ActivityData::Downloads {
        items,
        complete,
        aggregate_speed,
    } = decode(Provider::Transmission, TR).unwrap()
    else {
        panic!()
    };
    assert!(complete);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "Ubuntu ISO");
    assert_eq!(items[0].progress_percent, Some(50.0));
    assert_eq!(items[0].remaining_bytes, Some(2_000_000_000.0));
    assert_eq!(items[0].eta.as_deref(), Some("1h 30m"));
    assert_eq!(aggregate_speed, Some(2_000_000.0));
}
#[test]
fn all_frozen_arr_providers_count_problem_records_once() {
    for provider in [
        Provider::Sonarr,
        Provider::Radarr,
        Provider::Lidarr,
        Provider::Readarr,
    ] {
        let ActivityData::Queue(q) = decode(provider, ARR).unwrap() else {
            panic!()
        };
        assert_eq!(q.total, 3);
        assert_eq!(q.observed, 3);
        assert_eq!(q.problems, 2);
        assert!(q.complete);
        assert_eq!(q.items.len(), 3);
    }
}
#[test]
fn unknown_payload_values_never_escape_in_debug_or_fixed_failures() {
    for (provider, bytes) in [
        (Provider::Jellyfin, JF),
        (Provider::Sabnzbd, SAB),
        (Provider::Transmission, TR),
        (Provider::Sonarr, ARR),
    ] {
        assert!(
            !format!("{:?}", decode(provider, bytes).unwrap()).contains("ACTIVITY_PRIVATE_VALUE")
        );
    }
    let error = decode(
        Provider::Transmission,
        br#"{"result":"ACTIVITY_PRIVATE_VALUE","arguments":{"torrents":[]}}"#,
    )
    .unwrap_err();
    assert_eq!(error, ReadFailure::InvalidResponse);
    assert!(!format!("{error:?}").contains("ACTIVITY_PRIVATE_VALUE"));
}
#[test]
fn transmission_success_is_required_and_error_envelopes_are_not_empty_activity() {
    for body in [
        br#"{"arguments":{"torrents":[]}}"#.as_slice(),
        br#"{"result":"failure","arguments":{"torrents":[]}}"#,
        br#"{"result":"success"}"#,
    ] {
        assert!(decode(Provider::Transmission, body).is_err());
    }
    assert!(decode(Provider::Sabnzbd, br#"{"error":"API Key Required"}"#).is_err());
    assert!(decode(Provider::Sonarr, br#"{"message":"Unauthorized"}"#).is_err());
}
#[test]
fn sab_aggregate_rate_is_not_multiplied_across_active_slots() {
    let mut body: Value = serde_json::from_slice(SAB).unwrap();
    body["queue"]["slots"][1]["status"] = json!("Downloading");
    let ActivityData::Downloads {
        items,
        aggregate_speed,
        ..
    } = project(Provider::Sabnzbd, &body).unwrap()
    else {
        panic!()
    };
    assert_eq!(items[0].speed_bytes, aggregate_speed);
    assert_eq!(items[1].speed_bytes, None);
}
#[test]
fn absent_invalid_negative_and_overflow_metrics_stay_unknown() {
    let mut body: Value = serde_json::from_slice(SAB).unwrap();
    body["queue"]["kbpersec"] = json!("NaN");
    body["queue"]["slots"][0]["mb"] = json!("1e300");
    body["queue"]["slots"][0]["mbleft"] = json!("-1");
    let ActivityData::Downloads {
        items,
        aggregate_speed,
        ..
    } = project(Provider::Sabnzbd, &body).unwrap()
    else {
        panic!()
    };
    assert_eq!(aggregate_speed, None);
    assert_eq!(items[0].size_bytes, None);
    assert_eq!(items[0].remaining_bytes, None);
    assert_eq!(items[0].speed_bytes, None);
    let mut body: Value = serde_json::from_slice(TR).unwrap();
    body["arguments"]["torrents"][0]
        .as_object_mut()
        .unwrap()
        .remove("rateDownload");
    body["arguments"]["torrents"][0]["eta"] = json!(-1);
    let ActivityData::Downloads {
        items,
        aggregate_speed,
        ..
    } = project(Provider::Transmission, &body).unwrap()
    else {
        panic!()
    };
    assert_eq!(aggregate_speed, None);
    assert_eq!(items[0].speed_bytes, None);
    assert_eq!(items[0].eta, None);
}
#[test]
fn bounded_progress_avoids_negative_remaining_sizes() {
    let mut body: Value = serde_json::from_slice(TR).unwrap();
    body["arguments"]["torrents"][0]["percentDone"] = json!(1.5);
    let ActivityData::Downloads { items, .. } = project(Provider::Transmission, &body).unwrap()
    else {
        panic!()
    };
    assert_eq!(items[0].progress_percent, Some(100.0));
    assert_eq!(items[0].remaining_bytes, Some(0.0));
    let mut body: Value = serde_json::from_slice(JF).unwrap();
    body[0]["NowPlayingItem"]["RunTimeTicks"] = json!(0);
    let ActivityData::Streams(items) = project(Provider::Jellyfin, &body).unwrap() else {
        panic!()
    };
    assert_eq!(items[0].progress_percent, None);
}
#[test]
fn seeding_uses_upload_rate_and_idle_torrents_are_not_downloads() {
    let mut body: Value = serde_json::from_slice(TR).unwrap();
    body["arguments"]["torrents"][0]["status"] = json!(6);
    body["arguments"]["torrents"][0]["rateUpload"] = json!(12345);
    let ActivityData::Downloads {
        items,
        aggregate_speed,
        ..
    } = project(Provider::Transmission, &body).unwrap()
    else {
        panic!()
    };
    assert_eq!(items[0].state, "seeding");
    assert_eq!(items[0].speed_bytes, Some(12345.0));
    assert_eq!(aggregate_speed, Some(12345.0));
}
#[test]
fn pagination_and_missing_queue_status_do_not_claim_complete_problem_counts() {
    let mut body: Value = serde_json::from_slice(ARR).unwrap();
    body["totalRecords"] = json!(50);
    let ActivityData::Queue(q) = project(Provider::Sonarr, &body).unwrap() else {
        panic!()
    };
    assert!(!q.complete);
    assert_eq!(q.problems, 2);
    assert_eq!(q.observed, 3);
    let body = json!({"totalRecords":1,"records":[{}]});
    let ActivityData::Queue(q) = project(Provider::Sonarr, &body).unwrap() else {
        panic!()
    };
    assert!(!q.complete);
    let mut body: Value = serde_json::from_slice(SAB).unwrap();
    body["queue"].as_object_mut().unwrap().remove("start");
    let ActivityData::Downloads { complete, .. } = project(Provider::Sabnzbd, &body).unwrap()
    else {
        panic!()
    };
    assert!(!complete);
}
#[test]
fn duplicate_consumed_fields_and_array_objects_are_refused() {
    for (provider, text) in [
        (
            Provider::Jellyfin,
            r#"[{"NowPlayingItem":{"Name":"one","Name":"two"}}]"#,
        ),
        (Provider::Jellyfin, r#"[{"NowPlayingItem":["one"]}]"#),
        (Provider::Sabnzbd, r#"{"queue":{"slots":[],"slots":[]}}"#),
        (
            Provider::Transmission,
            r#"{"result":"success","result":"success","arguments":{"torrents":[]}}"#,
        ),
        (
            Provider::Sonarr,
            r#"{"totalRecords":0,"records":[],"records":[]}"#,
        ),
        (Provider::Sonarr, r#"[0,[]]"#),
    ] {
        assert!(decode(provider, text.as_bytes()).is_err());
    }
}
#[test]
fn wrong_shapes_required_fields_and_numeric_types_refuse() {
    for provider in [
        Provider::Jellyfin,
        Provider::Sabnzbd,
        Provider::Transmission,
        Provider::Sonarr,
    ] {
        for body in [b"null".as_slice(), b"true", b"garbage", b"{}extra"] {
            assert!(decode(provider, body).is_err());
        }
    }
    for body in [
        json!({"totalRecords":null,"records":[]}),
        json!({"totalRecords":true,"records":[]}),
        json!({"totalRecords":0}),
        json!({"totalRecords":0,"records":[{}]}),
    ] {
        assert!(project(Provider::Sonarr, &body).is_err());
    }
}
#[test]
fn raw_bytes_rows_and_reasons_have_explicit_caps() {
    let mut bytes = b"[]".to_vec();
    bytes.resize(MAX_RESPONSE_BYTES, b' ');
    assert!(decode(Provider::Jellyfin, &bytes).is_ok());
    bytes.push(b' ');
    assert!(decode(Provider::Jellyfin, &bytes).is_err());
    assert!(project(Provider::Jellyfin, &json!(vec![json!({}); MAX_ITEMS])).is_ok());
    assert!(project(Provider::Jellyfin, &json!(vec![json!({}); MAX_ITEMS + 1])).is_err());
    let mut body: Value = serde_json::from_slice(JF).unwrap();
    body[0]["TranscodingInfo"]["TranscodeReasons"] = json!(vec!["reason"; 9]);
    assert!(project(Provider::Jellyfin, &body).is_err());
}
#[test]
fn unicode_text_bounds_preserve_character_boundaries_and_refuse_controls() {
    let mut body: Value = serde_json::from_slice(JF).unwrap();
    body[0]["NowPlayingItem"]["Name"] = json!("映画".repeat(100));
    let ActivityData::Streams(items) = project(Provider::Jellyfin, &body).unwrap() else {
        panic!()
    };
    assert!(items[0].title.len() <= 200);
    assert!(!items[0].title.is_empty());
    body[0]["NowPlayingItem"]["Name"] = json!("private\ncontrol");
    assert_eq!(
        project(Provider::Jellyfin, &body).unwrap_err(),
        ReadFailure::InvalidResponse
    );
}
#[test]
fn empty_transcoding_and_idle_playing_objects_do_not_create_activity() {
    let body = json!([{"NowPlayingItem":{},"TranscodingInfo":{}},{"NowPlayingItem":{"Name":"Direct"},"TranscodingInfo":{}}]);
    let ActivityData::Streams(items) = project(Provider::Jellyfin, &body).unwrap() else {
        panic!()
    };
    assert_eq!(items.len(), 1);
    assert!(!items[0].transcoding);
    let body = json!([{"NowPlayingItem":{"Name":"Unknown transcode metadata"},"TranscodingInfo":{"unknown":"ACTIVITY_PRIVATE_VALUE"}}]);
    let ActivityData::Streams(items) = project(Provider::Jellyfin, &body).unwrap() else {
        panic!()
    };
    assert!(items[0].transcoding);
    assert!(!format!("{items:?}").contains("ACTIVITY_PRIVATE_VALUE"));
}
