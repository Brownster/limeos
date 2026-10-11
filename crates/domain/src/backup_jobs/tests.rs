use super::*;
fn binding() -> BackupPreparationBinding {
    BackupPreparationBinding {
        custody: CustodyBinding {
            format_version: 1,
            record_id: "a".repeat(32),
            archive_sha256: "b".repeat(64),
            archive_bytes: 123,
            policy_identity: "c".repeat(64),
            manifest_identity: "d".repeat(64),
        },
        preparation_identity: "e".repeat(64),
        selected_resources: vec!["limeos-config".into(), "app-config".into()],
        workspace_policy_identity: "f".repeat(64),
    }
}
#[test]
fn custody_encoding_and_closed_form_remain_compatible() {
    let b = binding().custody;
    let expected = format!(
        "{{\"format_version\":1,\"record_id\":\"{}\",\"archive_sha256\":\"{}\",\"archive_bytes\":123,\"policy_identity\":\"{}\",\"manifest_identity\":\"{}\"}}",
        "a".repeat(32),
        "b".repeat(64),
        "c".repeat(64),
        "d".repeat(64)
    );
    assert_eq!(serde_json::to_string(&b).unwrap(), expected);
    assert_eq!(
        serde_json::from_str::<CustodyBinding>(&expected).unwrap(),
        b
    );
    assert!(
        serde_json::from_str::<CustodyBinding>(&expected.replace(
            "\"archive_bytes\":123",
            "\"archive_bytes\":123,\"path\":\"/\""
        ))
        .is_err()
    );
    assert!(
        serde_json::from_str::<CustodyBinding>(&expected.replace(
            "\"archive_bytes\":123",
            "\"archive_bytes\":123,\"archive_bytes\":124"
        ))
        .is_err()
    );
    let mut b = b;
    b.record_id.make_ascii_uppercase();
    assert!(!b.well_formed());
}
#[test]
fn selection_bounds_and_order_are_binding_facts() {
    let b = binding();
    b.validate().unwrap();
    assert_eq!(
        b.resources(),
        vec![
            format!("backup:custody:{}", "a".repeat(32)),
            "backup:preparation".into(),
            "storage:configuration".into()
        ]
    );
    for ids in [
        vec![],
        vec!["a".into(), "a".into()],
        vec!["a/b".into()],
        vec!["a".repeat(129)],
        (0..65).map(|n| n.to_string()).collect(),
    ] {
        assert!(
            (BackupPreparationBinding {
                selected_resources: ids,
                ..b.clone()
            })
            .validate()
            .is_err()
        );
    }
    let mut reversed = b.clone();
    reversed.selected_resources.reverse();
    assert_ne!(b, reversed);
    assert_ne!(
        serde_json::to_string(&b).unwrap(),
        serde_json::to_string(&reversed).unwrap()
    );
}
#[test]
fn plan_time_arithmetic_and_full_current_binding_are_checked() {
    let p = BackupPreparationJobPlan {
        id: "a".repeat(64),
        version: 1,
        action: "b".repeat(64),
        principal: "alice".into(),
        grant_revision: 1,
        created_at: 100,
        expires_at: 400,
        binding: binding(),
    };
    p.check_current(&p.binding, 399).unwrap();
    assert_eq!(p.validate(400).unwrap_err().0, ErrorCode::Expired);
    assert!(p.validate(99).is_err());
    let mut changed = p.binding.clone();
    changed.workspace_policy_identity = "a".repeat(64);
    assert_eq!(
        p.check_current(&changed, 200).unwrap_err().0,
        ErrorCode::Conflict
    );
    let mut overflow = p.clone();
    overflow.created_at = i64::MAX;
    overflow.expires_at = i64::MAX;
    assert!(overflow.validate(i64::MAX).is_err());
    let mut value = serde_json::to_value(&p).unwrap();
    value["claims"] = serde_json::json!(["storage:root"]);
    assert!(serde_json::from_value::<BackupPreparationJobPlan>(value).is_err());
}
