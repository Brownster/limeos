use super::*;
use crate::automation::{AlertEvent, AlertKind, Severity};

fn incident(channel: DeliveryChannel) -> Notification {
    Notification {
        id: "opening-1".into(),
        revision: 1,
        event: AlertEvent {
            key: "mount:/mnt/Media".into(),
            generation: 7,
            event: AlertEventKind::Incident,
            kind: AlertKind::Mount,
            severity: Severity::Critical,
            summary: "Required mount missing".into(),
            at: 1000,
        },
        channel,
        delivery: DeliveryState::Pending,
        attempts: 0,
        acknowledged_at: None,
    }
}
fn recovery() -> Notification {
    let mut row = incident(DeliveryChannel::InApp);
    row.id = "recovery-1".into();
    row.event.event = AlertEventKind::Recovery;
    row.event.summary = "Mount recovered".into();
    row.event.at = 1010;
    row
}

#[test]
fn incident_in_app_only_and_missing_recovery_receipt_stays_visible() {
    assert_eq!(
        plan_in_app_delivery(&incident(DeliveryChannel::InApp), None, 1000).unwrap(),
        DeliveryDecision::PublishInApp
    );
    for channel in [
        DeliveryChannel::External,
        DeliveryChannel::IdempotentReceiver,
    ] {
        assert_eq!(
            plan_in_app_delivery(&incident(channel), None, 1000).unwrap(),
            DeliveryDecision::AdapterUnavailable
        );
    }
    assert_eq!(
        plan_in_app_delivery(&recovery(), None, 1010).unwrap(),
        DeliveryDecision::OpeningMissing
    );
}

#[test]
fn recovery_waits_for_verified_opening_in_same_channel_and_generation() {
    let mut opening = incident(DeliveryChannel::InApp);
    assert_eq!(
        plan_in_app_delivery(&recovery(), Some(&opening), 1010).unwrap(),
        DeliveryDecision::OpeningPending
    );
    opening = opening.begin(1000).unwrap();
    assert_eq!(
        plan_in_app_delivery(&recovery(), Some(&opening), 1010).unwrap(),
        DeliveryDecision::OpeningPending
    );
    let delivered = opening.finish(DeliveryResult::Verified, 1011).unwrap();
    assert_eq!(
        plan_in_app_delivery(&recovery(), Some(&delivered), 1010).unwrap(),
        DeliveryDecision::ClockRollback
    );
    assert_eq!(
        plan_in_app_delivery(&recovery(), Some(&delivered), 1011).unwrap(),
        DeliveryDecision::PublishInApp
    );
}

#[test]
fn failed_or_acknowledged_opening_is_not_delivery_proof() {
    let mut opening = incident(DeliveryChannel::InApp)
        .begin(1000)
        .unwrap()
        .finish(DeliveryResult::Rejected, 1001)
        .unwrap();
    assert_eq!(
        plan_in_app_delivery(&recovery(), Some(&opening), 1010).unwrap(),
        DeliveryDecision::OpeningPending
    );
    opening.delivery = DeliveryState::Failed { retry_at: None };
    opening.acknowledged_at = Some(1005);
    assert_eq!(
        plan_in_app_delivery(&recovery(), Some(&opening), 1010).unwrap(),
        DeliveryDecision::OpeningBlocked
    );
}

#[test]
fn interrupted_delivery_preserves_original_identity_without_resend() {
    let started = incident(DeliveryChannel::External).begin(1000).unwrap();
    let recovered = started.recover_interrupted(1001).unwrap();
    assert_eq!(recovered.id, started.id);
    assert_eq!(recovered.event, started.event);
    assert_eq!(recovered.attempts, started.attempts);
    assert_eq!(recovered.revision, started.revision + 1);
    assert_eq!(
        plan_in_app_delivery(&recovered, None, 1010).unwrap(),
        DeliveryDecision::NeedsReconciliation
    );
    assert!(!recovered.retry_due(5000).unwrap());
    assert!(recovered.recover_interrupted(1010).is_err());
    assert!(started.recover_interrupted(999).is_err());
}

#[test]
fn ambiguous_opening_never_releases_recovery() {
    let opening = incident(DeliveryChannel::InApp)
        .begin(1000)
        .unwrap()
        .recover_interrupted(1001)
        .unwrap();
    assert_eq!(
        plan_in_app_delivery(&recovery(), Some(&opening), 1010).unwrap(),
        DeliveryDecision::OpeningAmbiguous
    );
}

#[test]
fn mismatched_opening_receipts_and_chronology_refused() {
    let opening = incident(DeliveryChannel::InApp)
        .begin(1000)
        .unwrap()
        .finish(DeliveryResult::Verified, 1001)
        .unwrap();
    for variant in 0..6 {
        let mut wrong = opening.clone();
        match variant {
            0 => wrong.event.key = "mount:/mnt/Other".into(),
            1 => wrong.event.generation += 1,
            2 => wrong.channel = DeliveryChannel::External,
            3 => wrong.event.event = AlertEventKind::Recovery,
            4 => {
                wrong.event.at = 1020;
                wrong.delivery = DeliveryState::Delivered { at: 1021 };
            }
            _ => wrong.id = recovery().id,
        }
        assert!(plan_in_app_delivery(&recovery(), Some(&wrong), 1030).is_err());
    }
    assert!(plan_in_app_delivery(&incident(DeliveryChannel::InApp), Some(&opening), 1030).is_err());
}

#[test]
fn retry_wait_and_terminal_results_do_not_start_a_second_attempt() {
    let started = incident(DeliveryChannel::InApp).begin(1000).unwrap();
    assert_eq!(
        plan_in_app_delivery(&started, None, 1000).unwrap(),
        DeliveryDecision::InFlight
    );
    let failed = started.finish(DeliveryResult::Rejected, 1000).unwrap();
    assert_eq!(
        plan_in_app_delivery(&failed, None, 1029).unwrap(),
        DeliveryDecision::WaitingRetry
    );
    assert_eq!(
        plan_in_app_delivery(&failed, None, 1030).unwrap(),
        DeliveryDecision::PublishInApp
    );
    let delivered = started.finish(DeliveryResult::Verified, 1001).unwrap();
    assert_eq!(
        plan_in_app_delivery(&delivered, None, 1001).unwrap(),
        DeliveryDecision::Complete
    );
}
