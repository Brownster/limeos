use super::*;
use serde_json::json;
fn member(index: usize, status: SignalStatus) -> ContainerHealthMember {
    let resource = format!("container:{index:064x}");
    ContainerHealthMember {
        signal: Some(AlertSignal {
            key: resource.clone(),
            kind: AlertKind::Container,
            status,
            severity: Severity::Warning,
            summary: "Fixture container health".into(),
        }),
        resource,
    }
}
fn cohort(count: usize) -> ContainerHealthCohort {
    ContainerHealthCohort::new(
        "a".repeat(64),
        "fixture-engine".into(),
        120,
        (0..count)
            .map(|i| member(i, SignalStatus::Healthy))
            .collect(),
    )
    .unwrap()
}
#[test]
fn complete_empty_stopped_and_unavailable_batches_round_trip_under_closed_bounds() {
    for count in [0, 1, 16] {
        let value = cohort(count);
        assert!(value.complete());
        assert_eq!(
            ContainerHealthCohort::decode(&value.encode().unwrap()).unwrap(),
            value
        );
        assert!(value.encode().unwrap().len() <= CONTAINER_HEALTH_BYTES);
    }
    let value = ContainerHealthCohort::new(
        "a".repeat(64),
        "engine".into(),
        120,
        vec![member(0, SignalStatus::Unavailable)],
    )
    .unwrap();
    assert!(!value.complete());
    let mut wire = serde_json::to_value(&value).unwrap();
    wire["complete"] = json!(true);
    assert!(serde_json::from_value::<ContainerHealthCohort>(wire).is_err());
}
#[test]
fn full_ids_order_kind_and_signal_binding_are_required_without_clipping() {
    let mut cases = vec![
        vec![member(0, SignalStatus::Healthy); 17],
        vec![
            member(1, SignalStatus::Healthy),
            member(0, SignalStatus::Healthy),
        ],
        vec![member(0, SignalStatus::Healthy); 2],
    ];
    for mutation in 0..4 {
        let mut m = member(0, SignalStatus::Healthy);
        match mutation {
            0 => m.resource = "container:plex".into(),
            1 => m.signal.as_mut().unwrap().key = "container:plex".into(),
            2 => m.signal.as_mut().unwrap().kind = AlertKind::Host,
            _ => m.signal.as_mut().unwrap().summary = "x".repeat(513),
        }
        cases.push(vec![m]);
    }
    for members in cases {
        assert!(ContainerHealthCohort::new("a".repeat(64), "engine".into(), 120, members).is_err());
    }
}
#[test]
fn wall_time_is_original_request_bound_and_never_rebased_at_receipt() {
    let value = cohort(1);
    assert!(value.check_time(119, 120).is_ok());
    assert!(value.check_time(120, 125).is_ok());
    for (requested, received) in [
        (121, 122),
        (120, 119),
        (120, 126),
        (-1, 120),
        (120, MAX_TIME + 1),
    ] {
        assert!(value.check_time(requested, received).is_err());
    }
}
#[test]
fn malformed_and_oversized_nested_wire_values_refuse() {
    let original = serde_json::to_value(cohort(1)).unwrap();
    for (pointer, value) in [
        ("/version", json!(2)),
        ("/nonce", json!("short")),
        ("/engine_id", json!("x".repeat(129))),
        ("/members/0/resource", json!("x".repeat(75))),
        ("/members/0/signal/summary", json!("x".repeat(513))),
        ("/members/0/signal/key", json!("x".repeat(75))),
        ("/complete", json!(false)),
    ] {
        let mut wire = original.clone();
        *wire.pointer_mut(pointer).unwrap() = value;
        assert!(
            serde_json::from_value::<ContainerHealthCohort>(wire).is_err(),
            "{pointer}"
        );
    }
    let mut unknown = original.clone();
    unknown["command"] = json!("must not be accepted");
    assert!(serde_json::from_value::<ContainerHealthCohort>(unknown).is_err());
    let text = String::from_utf8(cohort(1).encode().unwrap()).unwrap();
    assert!(
        ContainerHealthCohort::decode(
            text.replacen("\"version\":1", "\"version\":1,\"version\":1", 1)
                .as_bytes()
        )
        .is_err()
    );
    assert!(ContainerHealthCohort::decode(&vec![b' '; CONTAINER_HEALTH_BYTES + 1]).is_err());
    let mut missing = original.clone();
    missing["members"][0]
        .as_object_mut()
        .unwrap()
        .remove("signal");
    assert!(serde_json::from_value::<ContainerHealthCohort>(missing).is_err());
    for (pointer, value) in [
        ("/members/0", json!([format!("container:{:064x}", 0), null])),
        (
            "/members/0/signal",
            json!(["key", "container", "healthy", "warning", "summary"]),
        ),
    ] {
        let mut array = original.clone();
        *array.pointer_mut(pointer).unwrap() = value;
        assert!(serde_json::from_value::<ContainerHealthCohort>(array).is_err());
    }
}
#[test]
fn next_member_is_refused_before_decoding_its_payload() {
    let mut wire = String::from_utf8(cohort(16).encode().unwrap()).unwrap();
    assert!(wire.ends_with("]}"));
    wire.truncate(wire.len() - 2);
    wire.push_str(",\"unterminated seventeenth value");
    let error = serde_json::from_str::<ContainerHealthCohort>(&wire)
        .unwrap_err()
        .to_string();
    assert!(error.contains("health member limit"), "{error}");
}
