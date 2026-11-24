//! The root-peer rule is tested both ways through `respond(core, uid, ..)`,
//! and a non-root peer's refusal over a real socket pair. Root acceptance
//! over a real socket needs uid 0, which this suite never has: the VM guest
//! `tests/privileged_vm/p06_job_resolution_guest.py` runs `limeosctl job list`
//! and `job cancel` as root against the installed core.
use super::*;
use limeos_domain::{
    CancelOutcome, CancelRefusal, ContainerSnapshot, JobFamily, JobState, OperatorJob,
};

const UNSETTLED: [JobState; 7] = [
    JobState::Planned,
    JobState::WaitingApproval,
    JobState::Queued,
    JobState::Running,
    JobState::Verifying,
    JobState::OutcomeUnknown,
    JobState::NeedsIntervention,
];

fn snapshot() -> ContainerSnapshot {
    ContainerSnapshot {
        resource: format!("container:{}", "b".repeat(64)),
        image: format!("sha256:{}", "c".repeat(64)),
        started_at: "2026-10-05T07:00:00Z".into(),
        running: true,
    }
}

/// A core whose store has the administrator `admin`, the operator `bob`
/// and `bob`'s queued restart, whose ID is returned.
async fn fixture() -> (tempfile::TempDir, Core, String) {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("core.sqlite")).unwrap();
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/werkzeug-hashes.json"
    ))
    .unwrap();
    let legacy = fixtures[0]["hash"].as_str().unwrap().to_owned();
    let job = db
        .call(move |s| {
            s.issue_bootstrap("bootstrap", now())?;
            s.enroll("bootstrap", "admin", "$argon2id$fixture", now())?;
            s.import_legacy_user("bob", &legacy, now())?;
            let bob = s.login_record("bob")?.unwrap().principal;
            s.revise_grants(
                &bob.id,
                limeos_domain::Role::Operator,
                &[Scope {
                    operation: limeos_domain::Operation::ContainerManage,
                    resource: "*".into(),
                }],
                now(),
            )?;
            let bob = s.login_record("bob")?.unwrap().principal;
            let plan = s.plan_restart(&bob, &snapshot(), now())?;
            let approval = s.approve_container(&bob, &plan.plan.id, &plan.digest, now())?;
            Ok(
                s.queue_container(&bob, "bob", &plan, &approval.token, &snapshot(), now())?
                    .id,
            )
        })
        .await
        .unwrap();
    (dir, crate::automation_schedules::tests::core(db), job)
}
fn filter(states: &[JobState]) -> OperatorJobFilter {
    OperatorJobFilter {
        states: states.to_vec(),
        owner: None,
        family: None,
        job: None,
        after: None,
    }
}
fn list() -> CoreRequest {
    CoreRequest::ListJobs {
        version: VERSION,
        filter: filter(&UNSETTLED),
        limit: 32,
    }
}
fn cancel(job: &str, admin: &str) -> CoreRequest {
    CoreRequest::CancelJob {
        version: VERSION,
        admin: admin.into(),
        job: job.into(),
    }
}
async fn state(core: &Core, job: &str) -> String {
    let job = job.to_owned();
    core.db.call(move |s| s.job_state(&job)).await.unwrap()
}

#[tokio::test]
async fn list_and_cancel_are_served_to_a_root_peer_only() {
    let (_dir, core, job) = fixture().await;
    for uid in [1, 999, 1000, 65534, u32::MAX] {
        for request in [list(), cancel(&job, "admin")] {
            assert_eq!(
                respond(&core, uid, request).await.err(),
                Some(Error(ErrorCode::Forbidden)),
                "uid {uid}"
            );
        }
        assert_eq!(state(&core, &job).await, "queued");
    }
    // Another protocol version is refused like an unknown request.
    let old = CoreRequest::ListJobs {
        version: VERSION + 1,
        filter: filter(&UNSETTLED),
        limit: 32,
    };
    assert_eq!(
        respond(&core, 0, old).await.err(),
        Some(Error(ErrorCode::Forbidden))
    );
    let Ok(CoreResponse::Jobs(page)) = respond(&core, 0, list()).await else {
        panic!("root peer was refused");
    };
    assert_eq!(page.matching, 1);
    let OperatorJob {
        job: listed,
        owner,
        family,
        state: listed_state,
        ..
    } = page.jobs[0].clone();
    assert_eq!(
        (listed.as_str(), owner.as_str(), family, listed_state),
        (job.as_str(), "bob", JobFamily::Container, JobState::Queued)
    );
    let Ok(CoreResponse::JobCanceled(answer)) = respond(&core, 0, cancel(&job, "admin")).await
    else {
        panic!("root peer was refused");
    };
    assert_eq!(answer.outcome, CancelOutcome::Canceled);
    assert_eq!(state(&core, &job).await, "canceled");
}

#[tokio::test]
async fn a_non_root_socket_peer_is_refused_on_the_wire_and_nothing_changes() {
    if rustix::process::getuid().is_root() {
        return;
    }
    let (_dir, core, job) = fixture().await;
    for request in [list(), cancel(&job, "admin")] {
        let (mut client, server) = UnixStream::pair().unwrap();
        let worker = tokio::spawn(handle_rpc(core.clone(), server));
        limeos_contracts::write_frame(&mut client, &request)
            .await
            .unwrap();
        let response: CoreResponse = limeos_contracts::read_frame(&mut client).await.unwrap();
        let CoreResponse::Error(error) = response else {
            panic!("non-root peer was served");
        };
        assert_eq!(error.code, ErrorCode::Forbidden);
        let _ = worker.await;
    }
    assert_eq!(state(&core, &job).await, "queued");
}

#[tokio::test]
async fn the_named_user_must_be_a_current_administrator_and_fields_are_checked() {
    let (_dir, core, job) = fixture().await;
    for (admin, code) in [
        ("bob", ErrorCode::Forbidden),
        ("nobody", ErrorCode::NotFound),
        ("not a name", ErrorCode::InvalidInput),
        ("", ErrorCode::InvalidInput),
    ] {
        assert_eq!(
            core.rpc_cancel_job(admin.into(), job.clone()).await.err(),
            Some(Error(code)),
            "{admin:?}"
        );
    }
    for bad in ["", "short", &"G".repeat(64), &"a".repeat(65)] {
        assert_eq!(
            core.rpc_cancel_job("admin".into(), bad.into()).await.err(),
            Some(Error(ErrorCode::InvalidInput))
        );
    }
    assert_eq!(
        core.rpc_cancel_job("admin".into(), "f".repeat(64))
            .await
            .err(),
        Some(Error(ErrorCode::NotFound))
    );
    for (states, limit) in [
        (UNSETTLED.to_vec(), 0),
        (UNSETTLED.to_vec(), MAX_OPERATOR_JOBS + 1),
        (Vec::new(), 10),
        (vec![JobState::Queued; MAX_STATES + 1], 10),
    ] {
        assert_eq!(
            core.rpc_list_jobs(filter(&states), limit).await.err(),
            Some(Error(ErrorCode::InvalidInput))
        );
    }
    assert_eq!(state(&core, &job).await, "queued");
    // Recorded under the administrator, beside the owner's transition.
    let answer = core
        .rpc_cancel_job("admin".into(), job.clone())
        .await
        .unwrap();
    assert_eq!(
        (answer.outcome, answer.refusal),
        (CancelOutcome::Canceled, None)
    );
    let id = job.clone();
    let actors: Vec<(String, String)> = core
        .db
        .call(move |s| {
            Ok(s.operator_jobs(&filter(&[JobState::Canceled]), 1)?
                .jobs
                .into_iter()
                .map(|j| (j.job, j.owner))
                .collect())
        })
        .await
        .unwrap();
    assert_eq!(actors, vec![(id, "bob".to_owned())]);
    let again = core.rpc_cancel_job("admin".into(), job).await.unwrap();
    assert_eq!(again.outcome, CancelOutcome::AlreadyCanceled);
    assert_ne!(again.refusal, Some(CancelRefusal::NotQueued));
}

#[test]
fn the_longest_listing_fits_one_control_socket_frame() {
    let job = OperatorJob {
        job: "a".repeat(64),
        owner: "o".repeat(128),
        family: JobFamily::BackupPreparation,
        operation: "storage_prepare_targets".into(),
        state: JobState::NeedsIntervention,
        resource: "r".repeat(128),
        claims: 128,
        created_at: i64::MIN,
        deadline: i64::MIN,
        resolution_open: true,
    };
    let page = CoreResponse::Jobs(limeos_domain::OperatorJobPage {
        jobs: vec![job; MAX_OPERATOR_JOBS as usize],
        matching: u32::MAX,
        remaining: u32::MAX,
        next: Some(i64::MAX),
    });
    let bytes = serde_json::to_vec(&page).unwrap();
    assert!(
        bytes.len() < limeos_contracts::FRAME_LIMIT,
        "{} bytes",
        bytes.len()
    );
}
