use super::*;

#[derive(Default)]
struct Stub {
    rows: Vec<Schedule>,
    calls: Vec<Settlement>,
    fail: bool,
    wrong: bool,
    completed: usize,
    published: usize,
}
impl ObservationStore for Stub {
    fn schedule_page(&mut self, _: Option<&str>, _: usize) -> Result<Vec<Schedule>> {
        Ok(self.rows.clone())
    }
    fn reserve(&mut self, _: &Schedule, settlement: &Settlement, _: i64) -> Result<Reservation> {
        self.calls.push(settlement.clone());
        if self.fail {
            return Err(Error(ErrorCode::Overloaded));
        }
        if self.wrong {
            return Ok(Reservation::Queued(ObservationJob {
                id: "wrong".into(),
                occurrence: settlement.key().clone(),
            }));
        }
        Ok(match settlement {
            Settlement::Observation(key) => Reservation::Queued(ObservationJob {
                id: "job-1".into(),
                occurrence: key.clone(),
            }),
            Settlement::Skipped(_) => Reservation::Skipped,
            Settlement::AdapterUnavailable(_) => Reservation::Blocked,
        })
    }
    fn complete_observation(
        &mut self,
        _: &ObservationJob,
        _: &ObservationBatch,
        _: i64,
    ) -> Result<()> {
        self.completed += 1;
        Ok(())
    }
    fn publish_in_app(&mut self, _: usize, _: i64) -> Result<usize> {
        Ok(self.published)
    }
}
fn row(id: &str, operation: ScheduledOperation) -> Schedule {
    Schedule {
        id: id.into(),
        revision: 1,
        paused: false,
        settled_through: None,
        definition: ScheduleDefinition {
            operation,
            anchor: 1000,
            interval_seconds: 60,
            grace_seconds: 30,
            missed: MissedRunPolicy::CoalesceLatest,
            window: None,
        },
    }
}
#[test]
fn effect_adapters_always_submit_blocked_records() {
    let mut store = Stub {
        rows: vec![
            row("a", ScheduledOperation::Backup),
            row("b", ScheduledOperation::ProtectionSync),
            row("c", ScheduledOperation::ProtectionScrub),
            row("d", ScheduledOperation::ContainerUpdate),
            row("e", ScheduledOperation::PackageReconcile),
            row("f", ScheduledOperation::ObserveHealth),
        ],
        ..Default::default()
    };
    let report = tick(&mut store, None, 1000).unwrap();
    assert_eq!(report.blocked, 5);
    assert_eq!(report.queued.len(), 1);
    assert!(
        store.calls[..5]
            .iter()
            .all(|call| matches!(call, Settlement::AdapterUnavailable(_)))
    );
}
#[test]
fn store_cannot_return_effect_job_as_observation() {
    let mut store = Stub {
        rows: vec![row("backup", ScheduledOperation::Backup)],
        wrong: true,
        ..Default::default()
    };
    assert_eq!(
        tick(&mut store, None, 1000).unwrap_err().0,
        ErrorCode::StateNotDurable
    );
}
#[test]
fn malformed_page_refused_before_first_reservation() {
    let mut store = Stub {
        rows: vec![
            row("a", ScheduledOperation::ObserveHealth),
            row("a", ScheduledOperation::ObserveHealth),
        ],
        ..Default::default()
    };
    assert!(tick(&mut store, None, 1000).is_err());
    assert!(store.calls.is_empty());
    store.rows = (0..33)
        .map(|i| row(&format!("a{i:02}"), ScheduledOperation::ObserveHealth))
        .collect();
    assert!(tick(&mut store, None, 1000).is_err());
    assert!(store.calls.is_empty());
    store.rows.truncate(32);
    let report = tick(&mut store, None, 1000).unwrap();
    assert_eq!(report.next_cursor.as_deref(), Some("a31"));
}
#[test]
fn paused_rollback_skip_and_queue_failure_are_visible() {
    let mut paused = row("a", ScheduledOperation::ObserveHealth);
    paused.paused = true;
    let mut rollback = row("b", ScheduledOperation::ObserveHealth);
    rollback.settled_through = Some(1060);
    let mut store = Stub {
        rows: vec![
            paused,
            rollback,
            row("c", ScheduledOperation::ObserveHealth),
        ],
        ..Default::default()
    };
    let report = tick(&mut store, None, 1031).unwrap();
    assert_eq!(report.clock_rollback, 1);
    assert_eq!(report.skipped, 1);
    assert_eq!(store.calls.len(), 1);
    store.rows = vec![row("a", ScheduledOperation::ObserveHealth)];
    store.fail = true;
    assert_eq!(
        tick(&mut store, None, 1000).unwrap_err().0,
        ErrorCode::Overloaded
    );
}
struct Reader(ObservationBatch);
impl HealthReader for Reader {
    fn observe(&mut self) -> Result<ObservationBatch> {
        Ok(self.0.clone())
    }
}
#[test]
fn partial_input_is_preserved_duplicate_keys_refused() {
    let signal = AlertSignal {
        key: "mount:/mnt/Media".into(),
        kind: AlertKind::Mount,
        status: SignalStatus::Unavailable,
        severity: Severity::Warning,
        summary: "Collection unavailable".into(),
    };
    let job = ObservationJob {
        id: "job-1".into(),
        occurrence: OccurrenceKey {
            schedule: "health".into(),
            revision: 1,
            due: 1000,
        },
    };
    let mut store = Stub::default();
    let mut reader = Reader(ObservationBatch {
        signals: vec![signal.clone()],
        complete: false,
    });
    observe_job(&mut store, &mut reader, &job, 1000).unwrap();
    assert_eq!(store.completed, 1);
    reader.0.signals.push(signal);
    assert!(observe_job(&mut store, &mut reader, &job, 1000).is_err());
    assert_eq!(store.completed, 1);
}
#[test]
fn notification_publication_count_is_bounded() {
    let mut store = Stub {
        published: 33,
        ..Default::default()
    };
    assert!(publish_notifications(&mut store, 1000).is_err());
    assert!(publish_notifications(&mut store, -1).is_err());
}
