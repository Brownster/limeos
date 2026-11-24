use super::*;
use limeos_domain::{ContainerJob, DeclaredOutcome, ResolutionEvidence};

pub(crate) const UNSETTLED: [JobState; 7] = [
    JobState::Planned,
    JobState::WaitingApproval,
    JobState::Queued,
    JobState::Running,
    JobState::Verifying,
    JobState::OutcomeUnknown,
    JobState::NeedsIntervention,
];
const ALL: [JobState; 12] = [
    JobState::Planned,
    JobState::WaitingApproval,
    JobState::Queued,
    JobState::Running,
    JobState::Verifying,
    JobState::OutcomeUnknown,
    JobState::NeedsIntervention,
    JobState::Succeeded,
    JobState::Failed,
    JobState::Canceled,
    JobState::PreconditionChanged,
    JobState::Resolved,
];

/// Every job in `states`, no other filter, from the start.
pub(crate) fn only(states: &[JobState]) -> OperatorJobFilter {
    OperatorJobFilter {
        states: states.to_vec(),
        owner: None,
        family: None,
        job: None,
        after: None,
    }
}
fn locks(s: &Store, job: &str) -> i64 {
    s.conn
        .query_row(
            "SELECT count(*) FROM resource_locks WHERE job=?",
            [job],
            |r| r.get(0),
        )
        .unwrap()
}
fn claims(s: &Store, job: &str) -> i64 {
    s.conn
        .query_row(
            "SELECT count(*) FROM job_resources WHERE job=?",
            [job],
            |r| r.get(0),
        )
        .unwrap()
}
fn events_of(s: &Store, job: &str) -> Vec<(Option<String>, EventKind)> {
    s.conn
        .prepare("SELECT principal,kind FROM events WHERE job=? ORDER BY cursor")
        .unwrap()
        .query_map([job], |r| {
            Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?))
        })
        .unwrap()
        .map(|row| {
            let (principal, kind) = row.unwrap();
            (principal, parse(&kind).unwrap())
        })
        .collect()
}

/// `admin` cancels queued job `job` of family `family`, and everything the
/// operator is told holds: the job is canceled at once, the owner's history
/// shows the transition and the administrator is recorded as the actor, in
/// that order and nothing else; no lock was or is held, its claims and its
/// consumed plan are untouched, a repeat writes nothing, and no generic
/// transition leaves `canceled`. Each family's own claim is checked by the
/// caller.
pub(crate) fn cancel_on_behalf(
    s: &mut Store,
    admin: &Principal,
    job: &str,
    family: JobFamily,
    now: i64,
) {
    let owner: String = s
        .conn
        .query_row("SELECT principal FROM jobs WHERE id=?", [job], |r| r.get(0))
        .unwrap();
    assert_eq!(s.job_state(job).unwrap(), "queued");
    assert_eq!(locks(s, job), 0, "a queued job holds no lock");
    let (claimed, history) = (claims(s, job), events_of(s, job));
    assert_eq!(
        s.operator_cancel_job(admin, job, now).unwrap(),
        OperatorCancel {
            job: job.into(),
            family,
            state: JobState::Canceled,
            outcome: CancelOutcome::Canceled,
            refusal: None,
        }
    );
    assert_eq!(s.job_state(job).unwrap(), "canceled");
    let settled: Option<i64> = s
        .conn
        .query_row("SELECT settled FROM jobs WHERE id=?", [job], |r| r.get(0))
        .unwrap();
    assert_eq!(settled, Some(now));
    let after = events_of(s, job);
    assert_eq!(after[..history.len()], history[..]);
    assert_eq!(
        after[history.len()..],
        [
            (
                Some(owner),
                EventKind::JobTransition {
                    state: JobState::Canceled
                }
            ),
            (
                Some(admin.id.clone()),
                EventKind::JobCanceledByAdministrator
            ),
        ]
    );
    assert_eq!(locks(s, job), 0);
    assert_eq!(claims(s, job), claimed);
    let changes = s.conn.total_changes();
    let again = s.operator_cancel_job(admin, job, now + 1).unwrap();
    assert_eq!(again.outcome, CancelOutcome::AlreadyCanceled);
    assert_eq!(again.state, JobState::Canceled);
    assert_eq!(s.conn.total_changes(), changes, "a repeat writes nothing");
    for to in ALL {
        assert!(s.transition(job, JobState::Canceled, to, now + 2).is_err());
    }
    assert_eq!(s.job_state(job).unwrap(), "canceled");
}

fn setup() -> (tempfile::TempDir, Store, Principal) {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("core.sqlite")).unwrap();
    s.issue_bootstrap("bootstrap", 100).unwrap();
    s.enroll("bootstrap", "alice", "$argon2id$fixture", 101)
        .unwrap();
    let p = s.login_record("alice").unwrap().unwrap().principal;
    (dir, s, p)
}
/// A user with a role other than administrator and every container grant.
fn user(s: &Store, name: &str, role: &str) -> Principal {
    s.conn
        .execute(
            "INSERT INTO users VALUES(?1,?1,'$argon2id$fixture',?2,1)",
            params![name, format!("\"{role}\"")],
        )
        .unwrap();
    s.conn
        .execute(
            "INSERT INTO grants VALUES(?,'\"container_manage\"','*')",
            [name],
        )
        .unwrap();
    s.login_record(name).unwrap().unwrap().principal
}
fn snapshot() -> limeos_domain::ContainerSnapshot {
    limeos_domain::ContainerSnapshot {
        resource: format!("container:{}", "a".repeat(64)),
        image: format!("sha256:{}", "b".repeat(64)),
        started_at: "2026-10-05T07:00:00.000000000Z".into(),
        running: true,
    }
}
/// A queued restart planned at `at` and queued at `at + 2`.
fn queued(s: &mut Store, p: &Principal, key: &str, at: i64) -> ContainerJob {
    let plan = s.plan_restart(p, &snapshot(), at).unwrap();
    let approval = s
        .approve_container(p, &plan.plan.id, &plan.digest, at + 1)
        .unwrap();
    s.queue_container(p, key, &plan, &approval.token, &snapshot(), at + 2)
        .unwrap()
}

#[test]
fn the_listing_spans_every_principal_beyond_the_per_user_page_and_says_how_many_more() {
    let (_dir, mut s, alice) = setup();
    let bob = user(&s, "bob", "operator");
    let bobs: Vec<ContainerJob> = (0..40)
        .map(|n| queued(&mut s, &bob, &format!("bob-{n}"), 200 + n))
        .collect();
    let mine = queued(&mut s, &alice, "alice", 300);
    let probe = s
        .queue_job(
            &alice,
            "probe",
            &Intent::HealthProbe {
                resource: "system".into(),
            },
            3_000,
            310,
        )
        .unwrap();
    // The owner's own listing stops at 32; the operator's does not.
    assert_eq!(s.container_jobs(&bob).unwrap().len(), 32);
    let changes = s.conn.total_changes();
    let page = s
        .operator_jobs(&only(&UNSETTLED), MAX_OPERATOR_JOBS)
        .unwrap();
    assert_eq!(page.matching, 42);
    assert_eq!(page.jobs.len(), 42);
    assert_eq!(
        page.jobs[0],
        OperatorJob {
            job: bobs[0].id.clone(),
            owner: "bob".into(),
            family: JobFamily::Container,
            operation: "container_restart".into(),
            state: JobState::Queued,
            resource: snapshot().resource,
            claims: 1,
            created_at: 202,
            deadline: bobs[0].plan.expires_at,
            resolution_open: false,
        }
    );
    // Oldest first, every owner.
    let ids: Vec<&str> = page.jobs.iter().map(|j| j.job.as_str()).collect();
    assert_eq!(ids[40], mine.id);
    assert_eq!(page.jobs[40].owner, "alice");
    assert_eq!(page.jobs[41].job, probe);
    assert_eq!(page.jobs[41].family, JobFamily::HealthProbe);
    assert_eq!(page.jobs[41].operation, "health_probe");
    assert_eq!(page.jobs[41].resource, "system");
    assert_eq!(page.jobs[41].deadline, 3_000);
    // Cut at the limit, with the full count.
    let first = s.operator_jobs(&only(&UNSETTLED), 5).unwrap();
    assert_eq!(first.jobs[..], page.jobs[..5]);
    assert_eq!((first.matching, first.remaining), (42, 37));
    assert!(first.next.is_some());
    assert_eq!((page.remaining, page.next), (0, None));
    assert_eq!(s.conn.total_changes(), changes, "listing writes nothing");

    // States filter: one canceled by its owner, one dispatched.
    s.cancel_container_job(&bob, &bobs[1].id, 400).unwrap();
    s.claim_container(&bob, &bobs[2].id, &snapshot(), &"1".repeat(64), 401)
        .unwrap();
    let canceled = s.operator_jobs(&only(&[JobState::Canceled]), 64).unwrap();
    assert_eq!(canceled.matching, 1);
    assert_eq!(canceled.jobs[0].job, bobs[1].id);
    let running = s.operator_jobs(&only(&[JobState::Running]), 64).unwrap();
    assert_eq!(running.jobs.len(), 1);
    assert_eq!(running.jobs[0].job, bobs[2].id);
    assert_eq!(running.jobs[0].state, JobState::Running);
    assert_eq!(s.operator_jobs(&only(&UNSETTLED), 64).unwrap().matching, 41);
    let queued_only = s
        .operator_jobs(&only(&[JobState::Queued, JobState::Queued]), 64)
        .unwrap();
    assert_eq!(queued_only.matching, 40);
    assert!(queued_only.jobs.iter().all(|j| j.state == JobState::Queued));
    assert_eq!(s.operator_jobs(&only(&ALL), 64).unwrap().matching, 42);
}

#[test]
fn the_listing_is_bounded_and_refuses_what_it_cannot_bound() {
    let (_dir, s, _) = setup();
    for limit in [0, MAX_OPERATOR_JOBS + 1, u32::MAX] {
        assert_eq!(
            s.operator_jobs(&only(&UNSETTLED), limit).unwrap_err().0,
            ErrorCode::InvalidInput
        );
    }
    assert_eq!(
        s.operator_jobs(&only(&[]), 10).unwrap_err().0,
        ErrorCode::InvalidInput
    );
    for bad in [
        OperatorJobFilter {
            owner: Some("not a name".into()),
            ..only(&ALL)
        },
        OperatorJobFilter {
            job: Some("short".into()),
            ..only(&ALL)
        },
        OperatorJobFilter {
            after: Some(-1),
            ..only(&ALL)
        },
    ] {
        assert_eq!(
            s.operator_jobs(&bad, 10).unwrap_err().0,
            ErrorCode::InvalidInput
        );
    }
    let thirteen = [JobState::Queued; 13];
    assert_eq!(
        s.operator_jobs(&only(&thirteen), 10).unwrap_err().0,
        ErrorCode::InvalidInput
    );
    let empty = s.operator_jobs(&only(&ALL), 10).unwrap();
    assert_eq!((empty.jobs.len(), empty.matching), (0, 0));
}

#[test]
fn an_open_resolution_request_is_shown() {
    let (_dir, mut s, alice) = setup();
    let job = queued(&mut s, &alice, "held", 200);
    let job = s
        .claim_container(&alice, &job.id, &snapshot(), &"1".repeat(64), 203)
        .unwrap();
    s.record_receipt_missing(&job.id, 204).unwrap();
    let held = s
        .operator_jobs(&only(&[JobState::NeedsIntervention]), 64)
        .unwrap();
    assert!(!held.jobs[0].resolution_open);
    let evidence = ResolutionEvidence::observe(
        job.plan.operation,
        &[&job.plan.expected],
        std::slice::from_ref(&Some(snapshot())),
    )
    .unwrap();
    s.request_resolution(
        &alice,
        &job.id,
        DeclaredOutcome::NotDone,
        "nothing ran",
        &evidence,
        &evidence.digest().unwrap(),
        205,
    )
    .unwrap();
    let held = s
        .operator_jobs(&only(&[JobState::NeedsIntervention]), 64)
        .unwrap();
    assert!(held.jobs[0].resolution_open);
    assert_eq!(held.jobs[0].claims, 1);
}

#[test]
fn an_administrator_cancels_another_users_queued_container_job_for_good() {
    let (_dir, mut s, alice) = setup();
    let bob = user(&s, "bob", "operator");
    let job = queued(&mut s, &bob, "bob", 200);
    cancel_on_behalf(&mut s, &alice, &job.id, JobFamily::Container, 210);
    // The dispatcher's claim is a compare-and-set on `queued`: refused.
    assert_eq!(
        s.claim_container(&bob, &job.id, &snapshot(), &"1".repeat(64), 211)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert_eq!(locks(&s, &job.id), 0);
    // The plan stays consumed by the job: it cannot be queued again.
    let consumed: Option<String> = s
        .conn
        .query_row(
            "SELECT job FROM container_plans WHERE id=?",
            [&job.plan.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(consumed.as_deref(), Some(job.id.as_str()));
    // The owner sees it canceled in their own history.
    assert_eq!(
        s.container_job(&bob, &job.id).unwrap().state,
        JobState::Canceled
    );
    let theirs = s.container_events(&bob, &job.id, 0).unwrap();
    assert_eq!(
        theirs.last().unwrap().event,
        EventKind::JobTransition {
            state: JobState::Canceled
        }
    );
    // Not a candidate for any dispatcher.
    assert!(
        s.container_candidates()
            .unwrap()
            .iter()
            .all(|(_, j)| j.id != job.id)
    );
}

#[test]
fn only_a_current_administrator_may_cancel_and_refusals_change_nothing() {
    let (_dir, mut s, alice) = setup();
    let bob = user(&s, "bob", "operator");
    let carol = user(&s, "carol", "viewer");
    let job = queued(&mut s, &bob, "bob", 200);
    let changes = s.conn.total_changes();
    for actor in [&bob, &carol] {
        assert_eq!(
            s.operator_cancel_job(actor, &job.id, 210).unwrap_err().0,
            ErrorCode::Forbidden
        );
    }
    // A principal whose grants changed since it was read is stale.
    let stale = Principal {
        grant_revision: alice.grant_revision + 1,
        ..alice.clone()
    };
    assert!(s.operator_cancel_job(&stale, &job.id, 210).is_err());
    assert_eq!(
        s.operator_cancel_job(&alice, "not-an-id", 210)
            .unwrap_err()
            .0,
        ErrorCode::InvalidInput
    );
    assert_eq!(
        s.operator_cancel_job(&alice, &"f".repeat(64), 210)
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
    assert_eq!(s.conn.total_changes(), changes);
    assert_eq!(s.job_state(&job.id).unwrap(), "queued");
}

#[test]
fn a_dispatched_or_settled_job_is_refused_as_not_queued_with_nothing_written() {
    let (_dir, mut s, alice) = setup();
    let bob = user(&s, "bob", "operator");
    let job = queued(&mut s, &bob, "bob", 200);
    // Claimed first: the claim won, so cancel is refused and changes nothing.
    s.claim_container(&bob, &job.id, &snapshot(), &"1".repeat(64), 205)
        .unwrap();
    let changes = s.conn.total_changes();
    let refused = s.operator_cancel_job(&alice, &job.id, 206).unwrap();
    assert_eq!(
        refused,
        OperatorCancel {
            job: job.id.clone(),
            family: JobFamily::Container,
            state: JobState::Running,
            outcome: CancelOutcome::Refused,
            refusal: Some(CancelRefusal::NotQueued),
        }
    );
    assert_eq!(s.conn.total_changes(), changes);
    assert_eq!(locks(&s, &job.id), 1, "the claim's lock stays");
    s.record_receipt_missing(&job.id, 207).unwrap();
    assert_eq!(
        s.operator_cancel_job(&alice, &job.id, 208).unwrap().state,
        JobState::NeedsIntervention
    );
    // Settled: refused too.
    let other = queued(&mut s, &bob, "other", 300);
    s.transition(
        &other.id,
        JobState::Queued,
        JobState::PreconditionChanged,
        303,
    )
    .unwrap();
    let settled = s.operator_cancel_job(&alice, &other.id, 304).unwrap();
    assert_eq!(settled.outcome, CancelOutcome::Refused);
    assert_eq!(settled.refusal, Some(CancelRefusal::NotQueued));
    assert_eq!(settled.state, JobState::PreconditionChanged);
}

#[test]
fn the_operator_step_is_only_ever_a_queued_cancellation() {
    let (_dir, mut s, alice) = setup();
    let job = queued(&mut s, &alice, "own", 200);
    for (from, to) in [
        (JobState::Queued, JobState::Running),
        (JobState::Queued, JobState::PreconditionChanged),
        (JobState::Running, JobState::Canceled),
    ] {
        assert_eq!(
            s.transition_by(&job.id, from, to, Some(&alice.id), 210)
                .unwrap_err()
                .0,
            ErrorCode::Forbidden
        );
    }
    assert_eq!(s.job_state(&job.id).unwrap(), "queued");
}

#[test]
fn a_queued_health_probe_is_cancelled_for_good() {
    let (_dir, mut s, alice) = setup();
    let probe = s
        .queue_job(
            &alice,
            "probe",
            &Intent::HealthProbe {
                resource: "system".into(),
            },
            3_000,
            200,
        )
        .unwrap();
    cancel_on_behalf(&mut s, &alice, &probe, JobFamily::HealthProbe, 210);
    assert_eq!(
        s.transition(&probe, JobState::Queued, JobState::Running, 211)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
}

#[test]
fn the_cancellation_is_an_authority_event_kept_apart_from_bulk_history() {
    let (_dir, mut s, alice) = setup();
    let job = queued(&mut s, &alice, "own", 200);
    s.operator_cancel_job(&alice, &job.id, 210).unwrap();
    let kind: String = s
        .conn
        .query_row(
            "SELECT json_extract(kind,'$.kind') FROM events WHERE job=? ORDER BY cursor DESC LIMIT 1",
            [&job.id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(crate::retention::AUTHORITY_KINDS.contains(&kind.as_str()));
}

/// Every page in turn, from the start, `limit` at a time.
fn walk(s: &Store, filter: &OperatorJobFilter, limit: u32) -> Vec<OperatorJobPage> {
    let mut pages = Vec::new();
    let mut after = None;
    loop {
        let page = s
            .operator_jobs(
                &OperatorJobFilter {
                    after,
                    ..filter.clone()
                },
                limit,
            )
            .unwrap();
        after = page.next;
        pages.push(page);
        if after.is_none() {
            return pages;
        }
    }
}

#[test]
fn paging_reaches_every_held_job_past_the_first_page_and_its_last_page() {
    let (_dir, mut s, alice) = setup();
    // 70 held jobs: more than any one page.
    let mut held = Vec::new();
    for n in 0..70 {
        // Each on its own container, so each can hold its lock.
        let on = limeos_domain::ContainerSnapshot {
            resource: format!("container:{n:064x}"),
            ..snapshot()
        };
        let plan = s.plan_restart(&alice, &on, 200).unwrap();
        let approval = s
            .approve_container(&alice, &plan.plan.id, &plan.digest, 201)
            .unwrap();
        let job = s
            .queue_container(
                &alice,
                &format!("held-{n}"),
                &plan,
                &approval.token,
                &on,
                202,
            )
            .unwrap();
        s.claim_container(&alice, &job.id, &on, &"1".repeat(64), 203)
            .unwrap();
        s.record_receipt_missing(&job.id, 204).unwrap();
        held.push(job.id);
    }
    let filter = only(&[JobState::NeedsIntervention]);
    let pages = walk(&s, &filter, MAX_OPERATOR_JOBS);
    assert_eq!(pages.len(), 2);
    assert_eq!(
        (pages[0].jobs.len(), pages[0].matching, pages[0].remaining),
        (64, 70, 6)
    );
    assert_eq!(
        (
            pages[1].jobs.len(),
            pages[1].matching,
            pages[1].remaining,
            pages[1].next
        ),
        (6, 70, 0, None)
    );
    let seen: Vec<String> = pages
        .iter()
        .flat_map(|p| p.jobs.iter().map(|j| j.job.clone()))
        .collect();
    assert_eq!(seen, held);
    // A last page that is exactly full says nothing remains.
    let ten = walk(&s, &filter, 10);
    assert_eq!(ten.len(), 7);
    assert_eq!(ten[6].jobs.len(), 10);
    assert_eq!((ten[6].remaining, ten[6].next), (0, None));
}

#[test]
fn the_cursor_holds_while_jobs_are_cancelled_between_pages() {
    let (_dir, mut s, alice) = setup();
    let bob = user(&s, "bob", "operator");
    let jobs: Vec<String> = (0..12)
        .map(|n| queued(&mut s, &bob, &format!("q-{n}"), 200 + n).id)
        .collect();
    let filter = only(&[JobState::Queued]);
    let first = s.operator_jobs(&filter, 5).unwrap();
    assert_eq!((first.matching, first.remaining), (12, 7));
    // Cancel the whole first page and one job of the next.
    for job in jobs[..5].iter().chain(&jobs[6..7]) {
        s.operator_cancel_job(&alice, job, 300).unwrap();
    }
    let second = s
        .operator_jobs(
            &OperatorJobFilter {
                after: first.next,
                ..filter.clone()
            },
            5,
        )
        .unwrap();
    let ids: Vec<&str> = second.jobs.iter().map(|j| j.job.as_str()).collect();
    assert_eq!(ids, [&jobs[5], &jobs[7], &jobs[8], &jobs[9], &jobs[10]]);
    assert_eq!((second.matching, second.remaining), (6, 1));
    let last = s
        .operator_jobs(
            &OperatorJobFilter {
                after: second.next,
                ..filter
            },
            5,
        )
        .unwrap();
    assert_eq!(last.jobs.len(), 1);
    assert_eq!(last.jobs[0].job, jobs[11]);
    assert_eq!((last.remaining, last.next), (0, None));
}

#[test]
fn owner_family_and_job_filters_narrow_the_listing() {
    let (_dir, mut s, alice) = setup();
    let bob = user(&s, "bob", "operator");
    let bobs = queued(&mut s, &bob, "bob", 200);
    let mine = queued(&mut s, &alice, "alice", 210);
    let probe = s
        .queue_job(
            &alice,
            "probe",
            &Intent::HealthProbe {
                resource: "system".into(),
            },
            3_000,
            220,
        )
        .unwrap();
    let ids = |filter: OperatorJobFilter| -> (Vec<String>, u32) {
        let page = s.operator_jobs(&filter, 64).unwrap();
        (
            page.jobs.into_iter().map(|j| j.job).collect(),
            page.matching,
        )
    };
    assert_eq!(
        ids(OperatorJobFilter {
            owner: Some("bob".into()),
            ..only(&ALL)
        }),
        (vec![bobs.id.clone()], 1)
    );
    assert_eq!(
        ids(OperatorJobFilter {
            owner: Some("alice".into()),
            family: Some(JobFamily::Container),
            ..only(&ALL)
        }),
        (vec![mine.id.clone()], 1)
    );
    assert_eq!(
        ids(OperatorJobFilter {
            family: Some(JobFamily::HealthProbe),
            ..only(&ALL)
        }),
        (vec![probe.clone()], 1)
    );
    assert_eq!(
        ids(OperatorJobFilter {
            job: Some(mine.id.clone()),
            ..only(&ALL)
        }),
        (vec![mine.id.clone()], 1)
    );
    assert_eq!(
        ids(OperatorJobFilter {
            owner: Some("nobody".into()),
            ..only(&ALL)
        }),
        (vec![], 0)
    );
    assert_eq!(
        ids(OperatorJobFilter {
            family: Some(JobFamily::Stack),
            ..only(&ALL)
        }),
        (vec![], 0)
    );
}

#[test]
fn the_queued_time_is_the_queue_event_and_unknown_once_retention_took_it() {
    let (_dir, mut s, alice) = setup();
    let job = queued(&mut s, &alice, "own", 200);
    s.operator_cancel_job(&alice, &job.id, 500).unwrap();
    let listed = |s: &Store| {
        s.operator_jobs(&only(&[JobState::Canceled]), 1)
            .unwrap()
            .jobs
            .remove(0)
            .created_at
    };
    assert_eq!(listed(&s), 202);
    // Retention removes a settled job's history, oldest first, but never its
    // authority event: the cancellation's time must not stand in for it.
    s.conn
        .execute(
            "DELETE FROM events WHERE job=? AND json_extract(kind,'$.kind')<>'job_canceled_by_administrator'",
            [&job.id],
        )
        .unwrap();
    assert_eq!(events_of(&s, &job.id).len(), 1);
    assert_eq!(listed(&s), 0);
}

#[test]
fn a_failed_audit_insert_rolls_the_whole_cancellation_back() {
    let (_dir, mut s, alice) = setup();
    let bob = user(&s, "bob", "operator");
    let job = queued(&mut s, &bob, "bob", 200);
    let (history, audit) = (events_of(&s, &job.id), audit_bytes(&s));
    // The second insert, the administrator's event, fails after the state
    // change and the owner's event were written in the same transaction.
    s.conn
        .execute_batch(
            "CREATE TRIGGER force_failure BEFORE INSERT ON events WHEN json_extract(NEW.kind,'$.kind')='job_canceled_by_administrator' BEGIN SELECT RAISE(FAIL,'fixture audit failure'); END;",
        )
        .unwrap();
    assert_eq!(
        s.operator_cancel_job(&alice, &job.id, 210).unwrap_err().0,
        ErrorCode::StateNotDurable
    );
    assert_eq!(s.job_state(&job.id).unwrap(), "queued");
    let settled: Option<i64> = s
        .conn
        .query_row("SELECT settled FROM jobs WHERE id=?", [&job.id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(settled, None);
    assert_eq!(events_of(&s, &job.id), history);
    assert_eq!(audit_bytes(&s), audit);
    s.conn.execute_batch("DROP TRIGGER force_failure;").unwrap();
    // Still dispatchable, and still cancellable.
    cancel_on_behalf(&mut s, &alice, &job.id, JobFamily::Container, 220);
}
fn audit_bytes(s: &Store) -> i64 {
    s.conn
        .query_row("SELECT audit_bytes FROM meta WHERE id=1", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn an_administrator_demoted_after_the_lookup_cannot_cancel() {
    let (_dir, mut s, alice) = setup();
    let bob = user(&s, "bob", "operator");
    let job = queued(&mut s, &bob, "bob", 200);
    // What core looked up for `--admin alice`, before her role changed.
    let looked_up = s.login_record("alice").unwrap().unwrap().principal;
    s.revise_grants(&alice.id, limeos_domain::Role::Viewer, &[], 205)
        .unwrap();
    let changes = s.conn.total_changes();
    assert!(s.operator_cancel_job(&looked_up, &job.id, 210).is_err());
    let now = s.login_record("alice").unwrap().unwrap().principal;
    assert_eq!(
        s.operator_cancel_job(&now, &job.id, 210).unwrap_err().0,
        ErrorCode::Forbidden
    );
    assert_eq!(s.conn.total_changes(), changes);
    assert_eq!(s.job_state(&job.id).unwrap(), "queued");
}
