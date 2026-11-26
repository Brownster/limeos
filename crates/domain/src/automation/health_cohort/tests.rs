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
    for count in [0, 1, 16, 17, 64] {
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
        (0..65).map(|i| member(i, SignalStatus::Healthy)).collect(),
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
    let mut wire = String::from_utf8(cohort(64).encode().unwrap()).unwrap();
    let end = wire.find("],\"truncated\":0}").unwrap();
    wire.truncate(end);
    wire.push_str(",\"unterminated sixty-fifth value");
    let error = serde_json::from_str::<ContainerHealthCohort>(&wire)
        .unwrap_err()
        .to_string();
    assert!(error.contains("health member limit"), "{error}");
}

fn members(count: usize) -> Vec<ContainerHealthMember> {
    (0..count)
        .map(|i| member(i, SignalStatus::Healthy))
        .collect()
}
#[test]
fn the_bound_is_the_products_sixty_four_containers_stopped_ones_included() {
    // The Pi that rolled the 26 November migration back has 18 containers,
    // 4 of them stopped; the earlier bound was 16.
    assert_eq!(
        CONTAINER_HEALTH_MEMBERS,
        crate::deployment::discovery::MAX_ENGINE_CONTAINERS
    );
    for count in [16, 17, 18, 64] {
        let value =
            ContainerHealthCohort::new("a".repeat(64), "engine".into(), 120, members(count));
        assert_eq!(value.map(|v| v.members().len()), Ok(count));
    }
    for count in [65, 66] {
        assert_eq!(
            ContainerHealthCohort::new("a".repeat(64), "engine".into(), 120, members(count)),
            Err(Error(ErrorCode::InvalidInput)),
            "{count}"
        );
    }
    // Stopped (failing) and unhealthy members are members like any other.
    let mut mixed = members(64);
    mixed[3].signal.as_mut().unwrap().status = SignalStatus::Failing;
    mixed[4].signal = None;
    let value = ContainerHealthCohort::new("a".repeat(64), "engine".into(), 120, mixed).unwrap();
    assert!(value.complete());
    assert_eq!(
        ContainerHealthCohort::decode(&value.encode().unwrap()).unwrap(),
        value
    );
}
#[test]
fn a_truncated_cohort_is_never_complete_and_says_how_many_it_left_out() {
    let value =
        ContainerHealthCohort::with_truncated("a".repeat(64), "engine".into(), 120, members(64), 1)
            .unwrap();
    assert_eq!(value.truncated(), 1);
    assert!(!value.complete());
    let bytes = value.encode().unwrap();
    assert_eq!(ContainerHealthCohort::decode(&bytes).unwrap(), value);
    // A truncated cohort cannot claim completeness on the wire, nor lose its
    // count while keeping its completeness.
    let mut wire = serde_json::to_value(&value).unwrap();
    wire["complete"] = json!(true);
    assert!(serde_json::from_value::<ContainerHealthCohort>(wire).is_err());
    let mut wire = serde_json::to_value(&value).unwrap();
    wire["truncated"] = json!(0);
    assert!(serde_json::from_value::<ContainerHealthCohort>(wire).is_err());
    let mut wire = serde_json::to_value(&value).unwrap();
    wire["truncated"] = json!(65536);
    assert!(serde_json::from_value::<ContainerHealthCohort>(wire).is_err());
    // An older reader's answer has no count: it never truncated.
    let older = cohort(2);
    let mut wire = serde_json::to_value(&older).unwrap();
    wire.as_object_mut().unwrap().remove("truncated");
    assert_eq!(
        serde_json::from_value::<ContainerHealthCohort>(wire).unwrap(),
        older
    );
}
#[test]
fn sixty_four_longest_named_unavailable_members_fit_one_executor_frame() {
    // The largest cohort a reader builds: every Engine name at its 255-byte
    // limit and every signal at its longest summary.
    let name = "n".repeat(255);
    let members: Vec<_> = (0..64)
        .map(|i| {
            let mut m = member(i, SignalStatus::Unavailable);
            m.signal.as_mut().unwrap().summary = format!("{name} observation unavailable");
            m
        })
        .collect();
    let value =
        ContainerHealthCohort::with_truncated("a".repeat(64), "e".repeat(128), 120, members, 960)
            .unwrap();
    let bytes = value.encode().unwrap();
    assert!(bytes.len() <= CONTAINER_HEALTH_BYTES, "{}", bytes.len());
    // `limeos_contracts::FRAME_LIMIT` (64 KiB) holds it with the receipt: a
    // compile-time assertion beside `limeos_executor_protocol::Receipt`.
    // Summaries the Engine cannot produce still cannot exceed the bound:
    // the cohort refuses as over its size instead of growing the frame.
    let members: Vec<_> = (0..64)
        .map(|i| {
            let mut m = member(i, SignalStatus::Healthy);
            m.signal.as_mut().unwrap().summary = "\"".repeat(512);
            m
        })
        .collect();
    assert_eq!(
        ContainerHealthCohort::new("a".repeat(64), "engine".into(), 120, members),
        Err(Error(ErrorCode::Overloaded))
    );
}
