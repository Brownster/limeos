use super::*;
use crate::ErrorCode;

fn schedule() -> Schedule {
    Schedule {
        id: "health".into(),
        revision: 1,
        paused: false,
        settled_through: None,
        definition: ScheduleDefinition {
            operation: ScheduledOperation::ObserveHealth,
            anchor: 1000,
            interval_seconds: 60,
            grace_seconds: 30,
            missed: MissedRunPolicy::CoalesceLatest,
            window: None,
        },
    }
}
fn signal(status: SignalStatus) -> AlertSignal {
    AlertSignal {
        key: "mount:/mnt/Media".into(),
        kind: AlertKind::Mount,
        status,
        severity: Severity::Critical,
        summary: "Required media mount is missing".into(),
    }
}
fn notification(channel: DeliveryChannel) -> Notification {
    Notification {
        id: "note-1".into(),
        revision: 1,
        event: AlertEvent {
            key: "mount:/mnt/Media".into(),
            generation: 2,
            event: AlertEventKind::Incident,
            kind: AlertKind::Mount,
            severity: Severity::Critical,
            summary: "Missing mount".into(),
            at: 1000,
        },
        channel,
        delivery: DeliveryState::Pending,
        attempts: 0,
        acknowledged_at: None,
    }
}

#[test]
fn frozen_due_cases() {
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        now: i64,
        last: Option<i64>,
        paused: bool,
        missed: MissedRunPolicy,
        expect: String,
        due: Option<i64>,
    }
    let cases: Vec<Case> = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/automation/due-cases.json"
    ))
    .unwrap();
    assert_eq!(cases.len(), 10);
    for case in cases {
        let mut row = schedule();
        row.settled_through = case.last;
        row.paused = case.paused;
        row.definition.missed = case.missed;
        let result = row.due(case.now).unwrap();
        let (name, key) = match result {
            DueDecision::Ready(key) => ("ready", Some(key)),
            DueDecision::Skipped(key) => ("skipped", Some(key)),
            DueDecision::NotDue => ("not_due", None),
            DueDecision::Paused => ("paused", None),
            DueDecision::ClockRollback => ("clock_rollback", None),
        };
        assert_eq!(name, case.expect, "{}", case.name);
        assert_eq!(key.map(|key| key.due), case.due, "{}", case.name);
    }
}
#[test]
fn midnight_window_and_half_open_boundaries() {
    let window = MaintenanceWindow {
        start_minute: 22 * 60,
        end_minute: 2 * 60,
    };
    window.validate().unwrap();
    assert!(!window.contains(22 * 3600 - 1));
    assert!(window.contains(22 * 3600));
    assert!(window.contains(86400 + 2 * 3600 - 1));
    assert!(!window.contains(86400 + 2 * 3600));
    let mut row = schedule();
    row.definition.anchor = 0;
    row.definition.window = Some(window);
    assert!(matches!(row.due(22 * 3600), Ok(DueDecision::Ready(_))));
    assert!(matches!(row.due(2 * 3600), Ok(DueDecision::Skipped(_))));
}
#[test]
fn old_occurrence_does_not_dispatch_outside_current_window() {
    let mut row = schedule();
    row.definition.anchor = 0;
    row.definition.interval_seconds = 3600;
    row.definition.grace_seconds = 3599;
    row.definition.window = Some(MaintenanceWindow {
        start_minute: 0,
        end_minute: 1,
    });
    assert!(matches!(row.due(120), Ok(DueDecision::Skipped(_))));
    assert!(matches!(row.due(30), Ok(DueDecision::Ready(_))));
}
#[test]
fn forward_jump_is_bounded_and_revision_changes_identity() {
    let mut row = schedule();
    row.definition.anchor = 0;
    row.definition.interval_seconds = 30;
    row.definition.grace_seconds = 29;
    let DueDecision::Ready(first) = row.due(MAX_TIME).unwrap() else {
        panic!()
    };
    assert_eq!(first.due, MAX_TIME - MAX_TIME % 30);
    row.revision += 1;
    let DueDecision::Ready(second) = row.due(MAX_TIME).unwrap() else {
        panic!()
    };
    assert_ne!(first, second);
}
#[test]
fn corrupt_watermark_and_unbounded_schedule_refused() {
    let mut row = schedule();
    row.settled_through = Some(1001);
    assert_eq!(row.validate().unwrap_err().0, ErrorCode::StateNotDurable);
    row.settled_through = None;
    row.definition.interval_seconds = 0;
    assert!(row.validate().is_err());
    row = schedule();
    row.definition.grace_seconds = 60;
    assert!(row.validate().is_err());
    row = schedule();
    row.definition.window = Some(MaintenanceWindow {
        start_minute: 0,
        end_minute: 0,
    });
    assert!(row.validate().is_err());
    assert!(schedule().due(-1).is_err());
}
#[test]
fn threshold_one_open_repeat_no_change_one_recovery() {
    let policy = AlertPolicy::default();
    let fail = signal(SignalStatus::Failing);
    let first = fold_alert(None, &fail, &policy, 1, 1000).unwrap();
    assert!(first.changed);
    assert!(first.event.is_none());
    let opened = fold_alert(Some(&first.next), &fail, &policy, 2, 1010).unwrap();
    assert_eq!(opened.event.unwrap().event, AlertEventKind::Incident);
    let repeat = fold_alert(Some(&opened.next), &fail, &policy, 3, 1020).unwrap();
    assert!(!repeat.changed);
    assert!(repeat.event.is_none());
    let recovered = fold_alert(
        Some(&repeat.next),
        &signal(SignalStatus::Healthy),
        &policy,
        4,
        1030,
    )
    .unwrap();
    assert_eq!(recovered.event.unwrap().generation, 2);
    let healthy = fold_alert(
        Some(&recovered.next),
        &signal(SignalStatus::Healthy),
        &policy,
        5,
        1040,
    )
    .unwrap();
    assert!(!healthy.changed);
    assert!(healthy.event.is_none());
}
#[test]
fn restart_round_trip_and_old_sequence_no_duplicate() {
    let policy = AlertPolicy {
        fail_threshold: 1,
        ..Default::default()
    };
    let opened = fold_alert(None, &signal(SignalStatus::Failing), &policy, 7, 1000).unwrap();
    let bytes = serde_json::to_vec(&opened.next).unwrap();
    let persisted: AlertState = serde_json::from_slice(&bytes).unwrap();
    for seq in [7, 6, 8] {
        let folded = fold_alert(
            Some(&persisted),
            &signal(SignalStatus::Failing),
            &policy,
            seq,
            1010,
        )
        .unwrap();
        assert!(!folded.changed);
        assert!(folded.event.is_none());
    }
}
#[test]
fn unavailable_does_not_reset_streak_or_recover() {
    let policy = AlertPolicy::default();
    let first = fold_alert(None, &signal(SignalStatus::Failing), &policy, 1, 1000).unwrap();
    let unavailable = fold_alert(
        Some(&first.next),
        &signal(SignalStatus::Unavailable),
        &policy,
        2,
        1001,
    )
    .unwrap();
    assert!(!unavailable.changed);
    let opened = fold_alert(
        Some(&unavailable.next),
        &signal(SignalStatus::Failing),
        &policy,
        3,
        1002,
    )
    .unwrap();
    assert!(opened.event.is_some());
    let partial = fold_alert(
        Some(&opened.next),
        &signal(SignalStatus::Unavailable),
        &policy,
        4,
        1003,
    )
    .unwrap();
    assert!(!partial.changed);
    assert!(matches!(partial.next.incident, IncidentState::Open { .. }));
}
#[test]
fn silence_tracks_fault_and_announces_once_on_expiry() {
    let mut policy = AlertPolicy {
        fail_threshold: 1,
        ..Default::default()
    };
    policy.silences.push(Silence {
        kind: AlertKind::Mount,
        key: signal(SignalStatus::Failing).key,
        expires_at: Some(1010),
    });
    let muted = fold_alert(None, &signal(SignalStatus::Failing), &policy, 1, 1000).unwrap();
    assert!(muted.event.is_none());
    let announced = fold_alert(
        Some(&muted.next),
        &signal(SignalStatus::Failing),
        &policy,
        2,
        1010,
    )
    .unwrap();
    assert_eq!(announced.event.unwrap().generation, 1);
    assert!(
        fold_alert(
            Some(&announced.next),
            &signal(SignalStatus::Failing),
            &policy,
            3,
            1020
        )
        .unwrap()
        .event
        .is_none()
    );
}
#[test]
fn never_announced_incident_does_not_send_recovery() {
    let policy = AlertPolicy {
        fail_threshold: 1,
        disabled: vec![AlertKind::Mount],
        silences: vec![],
    };
    let muted = fold_alert(None, &signal(SignalStatus::Failing), &policy, 1, 1000).unwrap();
    let recovered = fold_alert(
        Some(&muted.next),
        &signal(SignalStatus::Healthy),
        &AlertPolicy::default(),
        2,
        1001,
    )
    .unwrap();
    assert!(recovered.event.is_none());
}
#[test]
fn signal_bounds_case_sensitive_keys_and_closed_input() {
    let mut input = signal(SignalStatus::Failing);
    input.summary = "x".repeat(513);
    assert!(fold_alert(None, &input, &AlertPolicy::default(), 1, 1000).is_err());
    input = signal(SignalStatus::Failing);
    input.key = "mount:/mnt/media".into();
    let first = fold_alert(
        None,
        &signal(SignalStatus::Failing),
        &AlertPolicy::default(),
        1,
        1000,
    )
    .unwrap();
    assert!(fold_alert(Some(&first.next), &input, &AlertPolicy::default(), 2, 1001).is_err());
    assert!(
        serde_json::from_str::<CreateSchedule>(r#"{"definition":{},"principal":"admin"}"#).is_err()
    );
}
#[test]
fn retry_limits_delays_and_state_round_trip() {
    let mut note = notification(DeliveryChannel::InApp);
    let mut now = 1000;
    for delay in [Some(30), Some(120), Some(600), None] {
        let in_flight = note.begin(now).unwrap();
        assert!(!in_flight.retry_due(now).unwrap());
        note = in_flight.finish(DeliveryResult::Rejected, now).unwrap();
        let encoded = serde_json::to_vec(&note).unwrap();
        note = serde_json::from_slice(&encoded).unwrap();
        assert!(!note.retainable(MAX_TIME).unwrap());
        if let Some(delay) = delay {
            assert!(!note.retry_due(now + delay - 1).unwrap());
            now += delay;
        }
    }
    assert_eq!(note.attempts, 4);
    assert!(!note.retry_due(MAX_TIME).unwrap());
}
#[test]
fn ambiguous_external_delivery_is_not_retried() {
    let unknown = notification(DeliveryChannel::External)
        .begin(1000)
        .unwrap()
        .finish(DeliveryResult::Unknown, 1001)
        .unwrap();
    assert!(!unknown.retry_due(MAX_TIME).unwrap());
    assert!(!unknown.retainable(MAX_TIME).unwrap());
    let mut idempotent = unknown;
    idempotent.channel = DeliveryChannel::IdempotentReceiver;
    assert!(!idempotent.retry_due(1600).unwrap());
    assert!(idempotent.retry_due(1601).unwrap());
}
#[test]
fn verified_retention_ack_and_clock_rollback() {
    let mut delivered = notification(DeliveryChannel::InApp)
        .begin(1000)
        .unwrap()
        .finish(DeliveryResult::Verified, 1001)
        .unwrap();
    assert!(
        !delivered
            .retainable(1001 + DELIVERED_RETENTION_SECONDS - 1)
            .unwrap()
    );
    assert!(
        delivered
            .retainable(1001 + DELIVERED_RETENTION_SECONDS)
            .unwrap()
    );
    delivered.acknowledged_at = Some(1100);
    assert!(
        !delivered
            .retainable(1001 + DELIVERED_RETENTION_SECONDS)
            .unwrap()
    );
    assert!(
        notification(DeliveryChannel::InApp)
            .begin(1000)
            .unwrap()
            .finish(DeliveryResult::Verified, 999)
            .is_err()
    );
}

#[test]
fn frozen_long_running_container_rules_ignore_one_shots() {
    for policy in [
        RestartPolicy::No,
        RestartPolicy::OnFailure,
        RestartPolicy::Unknown,
    ] {
        assert!(
            container_signal("nightly", Some(false), ContainerHealth::Unreported, policy)
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(
        container_signal(
            "media",
            Some(false),
            ContainerHealth::Unreported,
            RestartPolicy::Always
        )
        .unwrap()
        .unwrap()
        .status,
        SignalStatus::Failing
    );
    assert_eq!(
        container_signal(
            "media",
            Some(true),
            ContainerHealth::Unhealthy,
            RestartPolicy::UnlessStopped
        )
        .unwrap()
        .unwrap()
        .severity,
        Severity::Warning
    );
    assert_eq!(
        container_signal(
            "media",
            Some(true),
            ContainerHealth::Starting,
            RestartPolicy::Always
        )
        .unwrap()
        .unwrap()
        .status,
        SignalStatus::Healthy
    );
}
#[test]
fn collector_gaps_are_unavailable_and_mount_paths_remain_literal() {
    assert_eq!(
        container_signal(
            "media",
            None,
            ContainerHealth::Unreported,
            RestartPolicy::Always
        )
        .unwrap()
        .unwrap()
        .status,
        SignalStatus::Unavailable
    );
    assert_eq!(
        smart_signal("/dev/sda", None, None).unwrap().status,
        SignalStatus::Unavailable
    );
    assert_eq!(
        required_mount_signal("/mnt/Media", None).unwrap().status,
        SignalStatus::Unavailable
    );
    assert_ne!(
        required_mount_signal("/mnt/Media", Some(true)).unwrap().key,
        required_mount_signal("/mnt/media", Some(true)).unwrap().key
    );
    assert!(required_mount_signal("relative", Some(true)).is_err());
    assert!(smart_signal(&"x".repeat(256), Some(false), None).is_err());
}
#[test]
fn frozen_protection_status_severity_without_new_thresholds() {
    assert!(
        protection_signal(ProtectionHealth::Unconfigured, None)
            .unwrap()
            .is_none()
    );
    let sync = protection_signal(ProtectionHealth::NeedsSync, None)
        .unwrap()
        .unwrap();
    assert_eq!(sync.severity, Severity::Warning);
    assert_eq!(sync.status, SignalStatus::Failing);
    assert_eq!(
        protection_signal(ProtectionHealth::Error, Some("Existing protection error"))
            .unwrap()
            .unwrap()
            .severity,
        Severity::Critical
    );
    assert_eq!(
        protection_signal(ProtectionHealth::Unavailable, None)
            .unwrap()
            .unwrap()
            .status,
        SignalStatus::Unavailable
    );
    assert!(protection_signal(ProtectionHealth::Error, Some(&"x".repeat(513))).is_err());
}

#[test]
fn arithmetic_due_matches_enumerated_window_candidates() {
    for window in [
        MaintenanceWindow {
            start_minute: 0,
            end_minute: 1,
        },
        MaintenanceWindow {
            start_minute: 1380,
            end_minute: 0,
        },
        MaintenanceWindow {
            start_minute: 1320,
            end_minute: 120,
        },
        MaintenanceWindow {
            start_minute: 1,
            end_minute: 1439,
        },
    ] {
        for interval in [37, 3600, 86400] {
            let mut row = schedule();
            row.definition.anchor = 7;
            row.definition.interval_seconds = interval;
            row.definition.grace_seconds = (interval - 1).min(600);
            row.definition.window = Some(window);
            for now in (0..172800).step_by(139) {
                let eligible = (7..=now)
                    .step_by(interval as usize)
                    .filter(|due| {
                        now - due <= i64::from(row.definition.grace_seconds)
                            && window.contains(*due)
                            && window.contains(now)
                    })
                    .last();
                let actual = match row.due(now).unwrap() {
                    DueDecision::Ready(key) => Some(key.due),
                    _ => None,
                };
                assert_eq!(actual, eligible, "{window:?} interval={interval} now={now}");
            }
        }
    }
}

#[test]
fn idle_alert_material_is_unchanged_over_a_day() {
    let policy = AlertPolicy {
        fail_threshold: 1,
        ..Default::default()
    };
    let fail = signal(SignalStatus::Failing);
    let mut state = fold_alert(None, &fail, &policy, 1, 1000).unwrap().next;
    for sequence in 2..=2881 {
        let result = fold_alert(
            Some(&state),
            &fail,
            &policy,
            sequence,
            1000 + (sequence - 1) * 30,
        )
        .unwrap();
        assert!(!result.changed);
        assert!(result.event.is_none());
        state = result.next;
    }
    // This proves leaf output only. Schedule/job transactions still require idle-write measurement.
}

#[test]
fn wire_integer_and_corrupt_state_bounds_refuse_before_copy() {
    let mut row = schedule();
    row.revision = MAX_REVISION + 1;
    assert!(row.validate().is_err());
    let mut note = notification(DeliveryChannel::InApp);
    note.revision = MAX_REVISION;
    assert!(note.begin(1000).is_err());
    let state = AlertState {
        key: "mount:/mnt/Media".into(),
        kind: AlertKind::Mount,
        last_sequence: 1,
        incident: IncidentState::Open {
            generation: 1,
            severity: Severity::Critical,
            summary: "x".repeat(513),
            announced: true,
        },
    };
    assert!(
        fold_alert(
            Some(&state),
            &signal(SignalStatus::Failing),
            &AlertPolicy::default(),
            2,
            1000
        )
        .is_err()
    );
    assert!(
        fold_alert(
            None,
            &signal(SignalStatus::Failing),
            &AlertPolicy::default(),
            MAX_REVISION + 1,
            1000
        )
        .is_err()
    );
}
