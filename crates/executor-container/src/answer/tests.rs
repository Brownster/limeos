use super::*;
use limeos_domain::{ContainerAction, ContainerSnapshot, DeclaredOutcome, ResolveKind};

fn snapshot() -> ContainerSnapshot {
    ContainerSnapshot {
        resource: format!("container:{}", "a".repeat(64)),
        image: format!("sha256:{}", "b".repeat(64)),
        started_at: "2026-10-05T07:00:00Z".into(),
        running: true,
    }
}
fn restart() -> RestartReceipt {
    RestartReceipt {
        action: "d".repeat(64),
        plan_digest: "e".repeat(64),
        operation: ContainerAction::Restart,
        before: snapshot(),
        state: ExecutionState::PreconditionChanged,
        error: Some(ErrorCode::Expired),
    }
}
fn stack() -> StackReceipt {
    let before = limeos_domain::StackSnapshot::new("media", vec![snapshot()]).unwrap();
    let members = vec![limeos_domain::StackMemberReceipt {
        resource: snapshot().resource,
        state: ExecutionState::PreconditionChanged,
        error: Some(ErrorCode::Expired),
    }];
    StackReceipt {
        action: "d".repeat(64),
        plan_digest: "e".repeat(64),
        operation: ContainerAction::Restart,
        before,
        state: ExecutionState::PreconditionChanged,
        members,
        error: Some(ErrorCode::Expired),
    }
}
const ABSENCES: [ReceiptAbsence; 2] = [ReceiptAbsence::NeverRecorded, ReceiptAbsence::NoLongerHeld];

#[test]
fn a_container_lookup_fills_only_restart_or_absence_and_carries_closed() {
    for closed in [false, true] {
        assert_eq!(
            Answer::restart_lookup(Lookup::Receipt {
                receipt: restart(),
                closed
            }),
            Answer {
                restart: Some(restart()),
                closed,
                ..Answer::default()
            }
        );
        for absence in ABSENCES {
            assert_eq!(
                Answer::restart_lookup(Lookup::Absent { absence, closed }),
                Answer {
                    absence: Some(absence),
                    closed,
                    ..Answer::default()
                }
            );
        }
    }
}

#[test]
fn a_stack_lookup_fills_only_stack_or_absence_and_carries_closed() {
    for closed in [false, true] {
        assert_eq!(
            Answer::stack_lookup(Lookup::Receipt {
                receipt: stack(),
                closed
            }),
            Answer {
                stack: Some(stack()),
                closed,
                ..Answer::default()
            }
        );
        for absence in ABSENCES {
            assert_eq!(
                Answer::stack_lookup(Lookup::Absent { absence, closed }),
                Answer {
                    absence: Some(absence),
                    closed,
                    ..Answer::default()
                }
            );
        }
    }
}

#[test]
fn a_resolution_answers_its_confirmation_its_closed_absence_or_its_refusal() {
    let confirmation = ResolveReceipt {
        kind: ResolveKind::Container,
        action: "d".repeat(64),
        evidence_digest: "f".repeat(64),
        declared: DeclaredOutcome::NotDone,
        resolved_by: "a".repeat(64),
    };
    assert_eq!(
        Answer::resolve(ResolveOutcome::Resolved(confirmation.clone())),
        Answer {
            resolution: Some(confirmation),
            ..Answer::default()
        }
    );
    // Every absence `ResolveAction` answers has been closed first.
    for absence in ABSENCES {
        assert_eq!(
            Answer::resolve(ResolveOutcome::Absent(absence)),
            Answer {
                absence: Some(absence),
                closed: true,
                ..Answer::default()
            }
        );
    }
    let refusal = ResolveRefusal {
        recorded: ExecutionState::Verified,
        declared: DeclaredOutcome::NotDone,
        allowed: vec![
            DeclaredOutcome::VerifiedDone,
            DeclaredOutcome::AcceptedDivergence,
        ],
    };
    assert_eq!(
        Answer::resolve(ResolveOutcome::Refused(refusal.clone())),
        Answer {
            resolution_refusal: Some(refusal),
            ..Answer::default()
        }
    );
}
