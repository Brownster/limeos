//! Admission precedes protected reads; actual scopes are bound at completion.
use super::*;

#[derive(Clone, Debug)]
enum Origin {
    Poll(Box<ObservationPollLease>),
    Queued(Box<ObservationLease>),
}

/// Private original admission, never serialized or accepted from HTTP.
/// Concrete container scope authority is still checked in the completion transaction.
///
/// ```compile_fail
/// let _: limeos_persistence::ContainerReadPermit = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Clone, Debug)]
pub struct ContainerReadPermit {
    origin: Origin,
}
impl ContainerReadPermit {
    pub fn deadline(&self) -> Instant {
        match &self.origin {
            Origin::Poll(lease) => lease.deadline(),
            Origin::Queued(lease) => lease.deadline(),
        }
    }
}

#[derive(Debug)]
pub enum ContainerReadAdmission {
    Read(Box<ContainerReadPermit>),
    Existing(ObservationResult),
    NotDue,
    Paused,
    ClockRollback,
    Skipped(OccurrenceKey),
}
#[derive(Debug)]
pub enum ContainerReadOutcome {
    Poll(ObservationPollOutcome),
    Queued(ObservationResult),
}

impl Store {
    pub fn automation_container_poll_begin(
        &mut self,
        id: &str,
        clock: impl Fn() -> i64,
        deadline: Instant,
    ) -> Result<ContainerReadAdmission> {
        Ok(
            match self.automation_poll_begin(id, vec!["host:system".into()], clock, deadline)? {
                ObservationPollAdmission::Lease(lease) => {
                    ContainerReadAdmission::Read(Box::new(ContainerReadPermit {
                        origin: Origin::Poll(lease),
                    }))
                }
                ObservationPollAdmission::NotDue => ContainerReadAdmission::NotDue,
                ObservationPollAdmission::Paused => ContainerReadAdmission::Paused,
                ObservationPollAdmission::ClockRollback => ContainerReadAdmission::ClockRollback,
                ObservationPollAdmission::Skipped(key) => ContainerReadAdmission::Skipped(key),
            },
        )
    }

    pub fn automation_container_job_begin(
        &mut self,
        job: &ObservationJob,
        clock: impl Fn() -> i64,
        deadline: Instant,
    ) -> Result<ContainerReadAdmission> {
        Ok(
            match self.automation_observation_begin(
                job,
                vec!["host:system".into()],
                clock,
                deadline,
            )? {
                ObservationAdmission::Lease(lease) => {
                    ContainerReadAdmission::Read(Box::new(ContainerReadPermit {
                        origin: Origin::Queued(Box::new(lease)),
                    }))
                }
                ObservationAdmission::Existing(receipt) => {
                    ContainerReadAdmission::Existing(receipt)
                }
            },
        )
    }

    pub fn automation_container_read_complete(
        &mut self,
        permit: &ContainerReadPermit,
        cohort: &ContainerHealthCohort,
        clock: impl Fn() -> i64,
        read_deadline: Instant,
    ) -> Result<ContainerReadOutcome> {
        let mut scopes = vec!["host:system".into()];
        scopes.extend(cohort.members().iter().map(|m| m.resource.clone()));
        scopes.sort();
        // Binding does not mutate the original private nonce/deadline or active
        // owner entry. Both completion paths reauthenticate every actual scope.
        let answer = match &permit.origin {
            Origin::Poll(original) => {
                let bound = original.with_resources(scopes);
                self.automation_poll_containers_complete_before(
                    &bound,
                    cohort,
                    clock,
                    read_deadline,
                )
                .map(ContainerReadOutcome::Poll)
            }
            Origin::Queued(original) => {
                let mut bound = (**original).clone();
                bound.resources = scopes;
                let batch = HealthObservation {
                    complete: false,
                    signals: cohort
                        .members()
                        .iter()
                        .filter_map(|m| {
                            m.signal.clone().map(|signal| ScopedHealthSignal {
                                resource: m.resource.clone(),
                                signal,
                            })
                        })
                        .collect(),
                };
                self.complete_observation_before(&bound, &batch, clock, read_deadline, Some(cohort))
                    .map(ContainerReadOutcome::Queued)
            }
        };
        self.automation_container_read_abandon(permit);
        answer
    }

    /// Cleanup releases only matching volatile admission; it settles no job.
    pub fn automation_container_read_abandon(&mut self, permit: &ContainerReadPermit) {
        match &permit.origin {
            Origin::Poll(lease) => self.automation_poll_abandon(lease),
            Origin::Queued(lease) => {
                if self.generation == lease.generation
                    && self
                        .observation_leases
                        .get(&lease.owner)
                        .is_some_and(|entry| {
                            entry.nonce == lease.nonce
                                && entry.job.as_deref() == Some(lease.job.id.as_str())
                                && entry.deadline == lease.deadline
                        })
                {
                    self.release_observation(lease);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
