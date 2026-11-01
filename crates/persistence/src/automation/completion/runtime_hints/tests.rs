use super::super::tests::{deadline, setup};
use super::*;

fn schedule(store: &mut Store, actor: &Principal, at: i64) -> Schedule {
    store
        .automation_create(
            actor,
            &CreateSchedule {
                definition: ScheduleDefinition {
                    operation: ScheduledOperation::ObserveHealth,
                    anchor: at,
                    interval_seconds: 30,
                    grace_seconds: 5,
                    missed: MissedRunPolicy::CoalesceLatest,
                    window: None,
                },
            },
            at,
        )
        .unwrap()
        .schedule
}
fn owner(store: &mut Store, index: usize) -> Principal {
    let actor = store
        .import_legacy_user(
            &format!("fixture_{index}"),
            &format!("pbkdf2:sha256:1$salt${}", "00".repeat(32)),
            120,
        )
        .unwrap();
    store
        .revise_grants(
            &actor.id,
            Role::Operator,
            &[
                Scope {
                    operation: Operation::HealthRead,
                    resource: "host:system".into(),
                },
                Scope {
                    operation: Operation::AutomationManage,
                    resource: "automation:observe-health".into(),
                },
            ],
            121,
        )
        .unwrap();
    store
        .login_record(&format!("fixture_{index}"))
        .unwrap()
        .unwrap()
        .principal
}

#[test]
fn owner_pages_are_bounded_query_only_and_cover_more_than_one_page() {
    let (_dir, mut store, actor) = setup();
    let mut owners = vec![actor.id.clone()];
    schedule(&mut store, &actor, 200);
    for i in 0..32 {
        let actor = owner(&mut store, i);
        owners.push(actor.id.clone());
        schedule(&mut store, &actor, 200);
    }
    owners.sort();
    let before = store.conn.total_changes();
    store.conn.execute_batch("PRAGMA query_only=ON").unwrap();
    let first = store.automation_observation_owner_page(None, 32).unwrap();
    assert_eq!(first.owners, owners[..32]);
    assert_eq!(first.next.as_ref(), Some(&owners[31]));
    let last = store
        .automation_observation_owner_page(first.next.as_deref(), 32)
        .unwrap();
    assert_eq!(last.owners, owners[32..]);
    assert_eq!(last.next, None);
    for owner in &owners {
        assert!(matches!(
            store
                .automation_observation_owner_work(owner, None)
                .unwrap(),
            Some(ObservationWorkHint::Schedule(_))
        ));
    }
    for limit in [0, 33, usize::MAX] {
        assert!(matches!(
            store.automation_observation_owner_page(None, limit),
            Err(Error(ErrorCode::InvalidInput))
        ));
    }
    assert!(matches!(
        store.automation_observation_owner_page(Some("/bad"), 32),
        Err(Error(ErrorCode::InvalidInput))
    ));
    assert!(matches!(
        store.automation_observation_owner_work(&actor.id, Some("*")),
        Err(Error(ErrorCode::InvalidInput))
    ));
    assert_eq!(store.conn.total_changes(), before);
}

#[test]
fn each_owner_rotates_schedules_and_original_oldest_jobs_take_priority() {
    let (_dir, mut store, actor) = setup();
    let mut rows: Vec<_> = (0..3).map(|_| schedule(&mut store, &actor, 200)).collect();
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    for i in 0..3 {
        assert_eq!(
            store
                .automation_observation_owner_work(&actor.id, Some(&rows[i].id))
                .unwrap(),
            Some(ObservationWorkHint::Schedule(rows[(i + 1) % 3].clone()))
        );
    }
    let first_key = proposed(&rows[2], 200).unwrap().unwrap();
    let Reservation::Queued(first) = store
        .automation_reserve_current(&rows[2], &first_key, || 200, deadline())
        .unwrap()
    else {
        panic!("first")
    };
    let second_key = proposed(&rows[0], 201).unwrap().unwrap();
    let Reservation::Queued(second) = store
        .automation_reserve_current(&rows[0], &second_key, || 201, deadline())
        .unwrap()
    else {
        panic!("second")
    };
    let before = store.conn.total_changes();
    for after in [None, Some(rows[1].id.as_str())] {
        assert_eq!(
            store
                .automation_observation_owner_work(&actor.id, after)
                .unwrap(),
            Some(ObservationWorkHint::Queued(first.clone()))
        );
    }
    assert_eq!(store.conn.total_changes(), before);
    store
        .automation_observation_discard(&first, || 270, deadline())
        .unwrap();
    assert_eq!(
        store
            .automation_observation_owner_work(&actor.id, None)
            .unwrap(),
        Some(ObservationWorkHint::Queued(second.clone()))
    );
    store
        .automation_observation_discard(&second, || 270, deadline())
        .unwrap();
    let mut expected = rows[0].clone();
    expected.settled_through = Some(200);
    assert_eq!(
        store
            .automation_observation_owner_work(&actor.id, Some(&rows[2].id))
            .unwrap(),
        Some(ObservationWorkHint::Schedule(expected))
    );
}

#[test]
fn invalid_queued_hint_bodies_are_refused_before_authority_or_collection() {
    let (_dir, mut store, actor) = setup();
    let row = schedule(&mut store, &actor, 200);
    let settlement = proposed(&row, 200).unwrap().unwrap();
    let Reservation::Queued(job) = store
        .automation_reserve_current(&row, &settlement, || 200, deadline())
        .unwrap()
    else {
        panic!("job")
    };
    let body: String = store
        .conn
        .query_row("SELECT intent FROM jobs WHERE id=?", [&job.id], |row| {
            row.get(0)
        })
        .unwrap();
    let mut wire: serde_json::Value = serde_json::from_str(&body).unwrap();
    wire["occurrence"]["revision"] = serde_json::json!(0);
    assert!(
        store
            .conn
            .execute(
                "UPDATE jobs SET intent=? WHERE id=?",
                params![wire.to_string(), job.id],
            )
            .is_err()
    );
    assert!(matches!(
        queued_hint(&job.id, &wire.to_string()),
        Err(Error(ErrorCode::StateNotDurable))
    ));
    let oversized = format!(
        "{{\"operation\":\"automation_observe_health\",\"padding\":\"{}\"}}",
        "x".repeat(DEFINITION_BYTES)
    );
    assert!(
        store
            .conn
            .execute(
                "UPDATE jobs SET intent=? WHERE id=?",
                params![oversized, job.id],
            )
            .is_err()
    );
    assert!(matches!(
        queued_hint(&job.id, &oversized),
        Err(Error(ErrorCode::StateNotDurable))
    ));
    assert_eq!(
        store
            .automation_observation_owner_work(&actor.id, None)
            .unwrap(),
        Some(ObservationWorkHint::Queued(job))
    );
}
