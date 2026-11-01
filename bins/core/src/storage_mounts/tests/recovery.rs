//! Real Store and owned Unix RPC, using private task-scoped peer credentials.
//! Root receipt facts are synthetic; installed guardian trust is not claimed.
use super::*;
use limeos_domain::{
    MountEffectQuiescence, MountHelperIdentity, MountHelperOutcome, StorageMountReceipt,
};

#[tokio::test]
async fn original_mount_recovery_refines_only_bound_quiescence_and_requires_independent_absence() {
    for mode in [
        "fresh",
        "missing_current",
        "denied",
        "pending",
        "wrong_digest",
    ] {
        let (dir, mut core, _, _, principal) = fixture(limeos_domain::Role::Administrator).await;
        let saved = principal.clone();
        let job = core
            .db
            .call(move |store| {
                let at = now() - 1000;
                let facts = inspection(at);
                let proposal = store.plan_storage_mount(&saved, &facts, at)?;
                let approval = store.approve_storage_mount(
                    &saved,
                    &proposal.plan.id,
                    &proposal.digest,
                    &facts,
                    at + 1,
                )?;
                let queued = store.queue_storage_mount(
                    &saved,
                    "original-mount-recovery",
                    &proposal,
                    &approval.token,
                    &facts,
                    at + 2,
                )?;
                let mut job = store.claim_storage_mount(&saved, &queued.id, &facts, at + 3)?;
                store.transition(
                    &job.id,
                    JobState::Running,
                    JobState::NeedsIntervention,
                    at + 4,
                )?;
                job.state = JobState::NeedsIntervention;
                Ok(job)
            })
            .await
            .unwrap();
        assert!(job.plan.effect.expires_at < now());
        let path = dir.path().join("root-fixture.sock");
        let listener = UnixListener::bind(&path).unwrap();
        core.storage_mount_socket = Arc::new(path);
        let expected = job.clone();
        let server = tokio::spawn(async move {
            let mut receipt = StorageMountReceipt {
                plan: expected.plan.effect.clone(),
                digest: limeos_identity::digest(
                    &serde_json::to_string(&expected.plan.effect).unwrap(),
                ),
                state: MountExecutionState::OutcomeUnknown,
                quiescence: MountEffectQuiescence::Reaped {
                    helper: MountHelperIdentity {
                        pid: 123,
                        start_time_ticks: 1,
                        mount_namespace_inode: 1,
                        cgroup: "/fixture".into(),
                        image_digest: "f".repeat(64),
                    },
                    outcome: MountHelperOutcome::NotStarted {},
                },
                after: None,
            };
            if mode == "pending" {
                receipt.quiescence = MountEffectQuiescence::Pending { helper: None };
            }
            if mode == "wrong_digest" {
                receipt.digest = "0".repeat(64);
            }
            let count = if matches!(mode, "pending" | "wrong_digest") {
                1
            } else if mode == "denied" {
                2
            } else {
                3
            };
            let mut requests = Vec::new();
            for index in 0..count {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request: Request = limeos_contracts::read_frame(&mut stream).await.unwrap();
                let mut reply = Receipt::empty();
                match request {
                    Request::StorageMountReceipt { action, digest, .. } if index == 0 => {
                        assert_eq!(action, expected.id);
                        assert_eq!(
                            digest,
                            limeos_identity::digest(
                                &serde_json::to_string(&expected.plan.effect).unwrap()
                            )
                        );
                        requests.push("receipt");
                        reply.storage_mount = Some(receipt.clone());
                    }
                    Request::ReconcileStorageMount { action, digest, .. } if index == 1 => {
                        assert_eq!(action, expected.id);
                        assert_eq!(digest, receipt.digest);
                        requests.push("reconcile");
                        if mode == "denied" {
                            reply.ready = false;
                            reply.error = Some(ErrorCode::Forbidden);
                        } else {
                            receipt.state = MountExecutionState::PreconditionChanged;
                            reply.storage_mount = Some(receipt.clone());
                        }
                    }
                    Request::InspectStorageMount { selection, .. } if index == 2 => {
                        assert_eq!(selection.target_id, expected.plan.effect.device.id);
                        requests.push("independent");
                        if mode == "missing_current" {
                            reply.ready = false;
                            reply.error = Some(ErrorCode::Unavailable);
                        } else {
                            reply.storage_mount_inspection = Some(inspection(now()));
                        }
                    }
                    _ => panic!("recovery must never execute or replace a mount"),
                }
                limeos_contracts::write_frame(&mut stream, &reply)
                    .await
                    .unwrap();
            }
            requests
        });
        let result = ROOT_UID
            .scope(
                rustix::process::geteuid().as_raw(),
                core.dispatch_mount(principal.clone(), job.clone()),
            )
            .await;
        if mode == "wrong_digest" {
            assert!(result.is_err());
        } else {
            result.unwrap();
        }
        let requests = tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
        let saved = principal.clone();
        let id = job.id.clone();
        let actual = core
            .db
            .call(move |store| store.storage_mount_job(&saved, &id))
            .await
            .unwrap();
        assert_eq!(actual.id, job.id);
        assert_eq!(actual.plan, job.plan);
        let connection = rusqlite::Connection::open_with_flags(
            dir.path().join("core.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let locks: i64 = connection
            .query_row(
                "SELECT count(*) FROM resource_locks WHERE job=?",
                [&job.id],
                |r| r.get(0),
            )
            .unwrap();
        if mode == "fresh" {
            assert_eq!(actual.state, JobState::PreconditionChanged);
            assert_eq!(locks, 0);
            assert_eq!(requests, ["receipt", "reconcile", "independent"]);
        } else {
            assert_eq!(actual.state, JobState::NeedsIntervention);
            assert!(locks > 0);
        }
    }
}
