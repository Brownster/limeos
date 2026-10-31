use super::*;
use serde_json::json;

#[test]
fn planned_configuration_preserves_silence_metadata_and_literal_mounts() {
    let planned=settings::plan_health_settings(settings::LegacyHealthEnvironment {
        fail_threshold:Some("3"),poll_seconds:Some("30"),..Default::default()
    },settings::LegacyPolicyRead::Contents(r#"{"required_mounts":["/mnt/Media","/mnt/media"],"silences":[{"kind":"mount","key":"mount:/mnt/Media","created_at":"2026-10-20T12:00:00.123456Z","expires_at":"2026-10-20T13:00:00+00:00","reason":"  disk work  "}]}"#),"1970-01-01T00:00:00Z").unwrap();
    let definition = HealthPolicyDefinition::from_planned(&planned).unwrap();
    assert_eq!(definition.fail_threshold, 3);
    assert_eq!(definition.required_mounts, ["/mnt/Media", "/mnt/media"]);
    assert_eq!(
        definition.silences[0].created_at,
        "2026-10-20T12:00:00.123456Z"
    );
    assert_eq!(definition.silences[0].reason, "disk work");
    assert_eq!(definition.runtime_policy().unwrap(), planned.alert_policy());
    let bytes = serde_json::to_vec(&definition).unwrap();
    assert_eq!(
        serde_json::from_slice::<HealthPolicyDefinition>(&bytes).unwrap(),
        definition
    );
}
#[test]
fn policy_definition_bounds_and_runtime_predicate_are_validated() {
    let mut definition = HealthPolicyDefinition {
        required_mounts: vec!["/mnt/Media".into(), "/mnt/Media".into()],
        ..Default::default()
    };
    assert!(definition.validate().is_err());
    definition.required_mounts = vec!["/mnt/Media".into(), "/mnt/media".into()];
    definition.disabled = vec![AlertKind::Mount, AlertKind::Mount];
    assert!(definition.validate().is_err());
    definition.disabled = vec![];
    definition.silences = vec![HealthPolicySilence {
        kind: AlertKind::Mount,
        key: "mount:/mnt/Media".into(),
        created_at: "2026-10-20T12:00:00Z".into(),
        expires_at: Some("2026-10-20T13:00:00.1Z".into()),
        reason: "disk work".into(),
    }];
    assert!(definition.validate().is_err());
    definition.silences[0].expires_at = Some("2026-10-20T13:00:00Z".into());
    let policy = definition.runtime_policy().unwrap();
    assert!(!policy.allows_key(AlertKind::Mount, "mount:/mnt/Media", 1_792_501_199));
    assert!(policy.allows_key(AlertKind::Mount, "mount:/mnt/Media", 1_792_501_200));
    assert!(policy.allows_key(AlertKind::Mount, "mount:/mnt/media", 1_792_501_199));
    definition.required_mounts = (0..256)
        .map(|i| format!("/{i:03}{}", "x".repeat(246)))
        .collect();
    definition.silences = vec![definition.silences[0].clone(); 64];
    assert!(
        definition.validate().is_err(),
        "whole definition output limit"
    );
}
#[test]
fn implicit_view_and_replacement_revision_shapes_are_strict() {
    let mut view = HealthPolicyView {
        revision: 0,
        updated: None,
        definition: HealthPolicyDefinition::default(),
    };
    view.validate().unwrap();
    view.definition.fail_threshold = 3;
    assert!(view.validate().is_err());
    view.revision = 1;
    view.updated = Some(200);
    view.validate().unwrap();
    view.updated = None;
    assert!(view.validate().is_err());
    let replacement =
        json!({"revision":0,"definition":HealthPolicyDefinition::default(),"owner":"injected"});
    assert!(serde_json::from_value::<ReplaceHealthPolicy>(replacement).is_err());
    for revision in [-1, MAX_REVISION + 1] {
        assert!(
            ReplaceHealthPolicy {
                revision,
                definition: HealthPolicyDefinition::default()
            }
            .validate()
            .is_err()
        );
    }
}
