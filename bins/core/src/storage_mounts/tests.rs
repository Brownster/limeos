use super::*;
use limeos_api::Backend;
use std::os::unix::fs::MetadataExt;
tokio::task_local! { pub(super) static ROOT_UID: u32; }
mod recovery;

async fn fixture(
    role: limeos_domain::Role,
) -> (tempfile::TempDir, Core, String, String, Principal) {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("core.sqlite")).unwrap();
    let token = "a".repeat(64);
    let csrf = "b".repeat(64);
    let token_hash = limeos_identity::digest(&token);
    let csrf_hash = limeos_identity::digest(&csrf);
    let principal = db
        .call(move |store| {
            store.issue_bootstrap("bootstrap", now())?;
            store.enroll("bootstrap", "admin", "$argon2id$fixture", now())?;
            let id = store.login_record("admin")?.unwrap().principal.id;
            store.revise_grants(&id, role, &[limeos_persistence::storage_scope()], now())?;
            let record = store.login_record("admin")?.unwrap();
            store.create_session(&record, None, &token_hash, &csrf_hash, now())?;
            Ok(record.principal)
        })
        .await
        .unwrap();
    let core = Core {
        automation_status: Default::default(),
        reader_health: Default::default(),
        db,
        storage_socket: Arc::new("/unused-reader".into()),
        storage_target_socket: Arc::new("/unused-targets".into()),
        storage_mount_socket: Arc::new("/unused-mounts".into()),
        storage_readers: Arc::new(Semaphore::new(1)),
        claims_in_doubt: Default::default(),
        stacks: Default::default(),
        container_socket: Arc::new("/unused-container".into()),
        compose_catalog: None,
        password_workers: Arc::new(PasswordWorkers::new(2)),
        dummy_hash: "unused".into(),
        observations: Default::default(),
        telemetry: None,
    };
    (dir, core, token, csrf, principal)
}

fn inspection(at: i64) -> StorageMountInspection {
    let identity = serde_json::json!({"major":8,"minor":1,"mount_id":10,"inode":2,"mode":16877,"uid":0,"gid":0});
    // Past facts for DB/read tests only; no helper or physical admission exists.
    serde_json::from_value(serde_json::json!({
        "target_id":"data", "device":{"id":"data","role":"data","filesystem_uuid":"uuid-data","filesystem":"ext4","mountpoint":"/mnt/data","serial":null},
        "expected":{"host_root_mount_id":10,"topology_digest":"a".repeat(64),"mounts_digest":"b".repeat(64),"fstab_digest":"c".repeat(64),"fstab_entries":[],
            "devices":[{"device":{"major":8,"minor":16},"filesystem_uuid":"uuid-data","filesystem":"ext4","serial":null,"boot_backing":false,"mounts":[]}]},
        "target":{"mountpoint":"/mnt/data","parent":identity,"existing":identity},
        "physical_devices":[{"major":8,"minor":16}],"consumer_digest":"d".repeat(64),"observed_at":at
    })).unwrap()
}

#[tokio::test]
async fn mount_current_human_authority_and_csrf_precede_reader_access() {
    for role in [
        limeos_domain::Role::Viewer,
        limeos_domain::Role::Operator,
        limeos_domain::Role::Administrator,
    ] {
        let (_dir, core, token, csrf, _) = fixture(role).await;
        let expected = if role == limeos_domain::Role::Administrator {
            ErrorCode::Unavailable
        } else {
            ErrorCode::Forbidden
        };
        assert_eq!(
            core.human_plan_mount(
                token.clone(),
                csrf,
                StorageMountSelection {
                    target_id: "data".into()
                }
            )
            .await
            .unwrap_err()
            .0,
            expected
        );
        assert_eq!(
            core.human_plan_mount(
                token,
                "wrong-csrf".into(),
                StorageMountSelection {
                    target_id: "data".into()
                }
            )
            .await
            .unwrap_err()
            .0,
            ErrorCode::Forbidden
        );
    }
    for request in [
        "plan_storage_mount",
        "approve_storage_mount",
        "queue_storage_mount",
        "execute_storage_mount",
    ] {
        let raw =
            format!(r#"{{"request":"{request}","version":1,"token":"task","principal":"admin"}}"#);
        assert!(serde_json::from_str::<CoreRequest>(&raw).is_err());
    }
}

#[tokio::test]
async fn mount_stored_get_list_and_expired_queue_replay_never_contact_executor() {
    let (dir, mut core, token, csrf, principal) = fixture(limeos_domain::Role::Administrator).await;
    let path = dir.path().join("unused-mount.sock");
    let listener = UnixListener::bind(&path).unwrap();
    core.storage_mount_socket = Arc::new(path);
    core.storage_readers = Arc::new(Semaphore::new(0));
    let (input, job) = core
        .db
        .call(move |store| {
            let past = now() - 1000;
            let facts = inspection(past);
            let proposal = store.plan_storage_mount(&principal, &facts, past)?;
            let approval = store.approve_storage_mount(
                &principal,
                &proposal.plan.id,
                &proposal.digest,
                &facts,
                past + 1,
            )?;
            let job = store.queue_storage_mount(
                &principal,
                "same-mount-request",
                &proposal,
                &approval.token,
                &facts,
                past + 2,
            )?;
            Ok((
                limeos_contracts::QueueStorageMountInput {
                    proposal,
                    approval: approval.token,
                    idempotency_key: "same-mount-request".into(),
                },
                job,
            ))
        })
        .await
        .unwrap();
    assert!(job.plan.effect.expires_at < now());
    assert_eq!(
        core.storage_mount_plan(token.clone(), job.plan.id.clone())
            .await
            .unwrap(),
        input.proposal
    );
    assert_eq!(
        core.storage_mount_job(token.clone(), job.id.clone())
            .await
            .unwrap(),
        job
    );
    assert_eq!(
        core.storage_mount_jobs(token.clone()).await.unwrap(),
        vec![job.clone()]
    );
    assert_eq!(
        core.human_queue_mount(token, csrf, input).await.unwrap(),
        job
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn mount_unprivileged_executor_cannot_forge_inspection_and_receives_no_request() {
    let (dir, mut core, _, _, _) = fixture(limeos_domain::Role::Administrator).await;
    if dir.path().metadata().unwrap().uid() == 0 {
        return;
    }
    let path = dir.path().join("unprivileged.sock");
    let listener = UnixListener::bind(&path).unwrap();
    core.storage_mount_socket = Arc::new(path);
    assert_eq!(
        core.inspect_mount(StorageMountSelection {
            target_id: "data".into()
        })
        .await
        .unwrap_err()
        .0,
        ErrorCode::Forbidden
    );
    let (mut stream, _) = listener.accept().await.unwrap();
    use tokio::io::AsyncReadExt;
    assert_eq!(stream.read(&mut [0; 4]).await.unwrap(), 0);
}

type Requests = Arc<std::sync::Mutex<Vec<&'static str>>>;
/// A queued storage mount job, with a root fixture whose helper never
/// starts: an execute is reaped unstarted and reconciles as a changed
/// precondition. It lists the requests it served.
async fn queued_mount() -> (
    tempfile::TempDir,
    Core,
    Principal,
    StorageMountJob,
    Requests,
    tokio::task::JoinHandle<()>,
) {
    use limeos_domain::{MountEffectQuiescence, MountHelperIdentity, MountHelperOutcome};
    let (dir, mut core, _, _, principal) = fixture(limeos_domain::Role::Administrator).await;
    let saved = principal.clone();
    let job = core
        .db
        .call(move |store| {
            let facts = inspection(now());
            let proposal = store.plan_storage_mount(&saved, &facts, now())?;
            let approval = store.approve_storage_mount(
                &saved,
                &proposal.plan.id,
                &proposal.digest,
                &facts,
                now(),
            )?;
            store.queue_storage_mount(
                &saved,
                "in-doubt",
                &proposal,
                &approval.token,
                &facts,
                now(),
            )
        })
        .await
        .unwrap();
    let socket = dir.path().join("root-fixture.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    core.storage_mount_socket = Arc::new(socket);
    let requests = Requests::default();
    let seen = requests.clone();
    let server = tokio::spawn(async move {
        let mut written = None;
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request: Request = limeos_contracts::read_frame(&mut stream).await.unwrap();
            let mut reply = Receipt::empty();
            match request {
                Request::InspectStorageMount { .. } => {
                    seen.lock().unwrap().push("inspect");
                    reply.storage_mount_inspection = Some(inspection(now()));
                }
                Request::ExecuteStorageMount { plan, .. } => {
                    seen.lock().unwrap().push("execute");
                    let receipt = limeos_domain::StorageMountReceipt {
                        digest: limeos_identity::digest(&serde_json::to_string(&plan).unwrap()),
                        plan: *plan,
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
                    written = Some(receipt.clone());
                    reply.storage_mount = Some(receipt);
                }
                Request::ReconcileStorageMount { .. } => {
                    seen.lock().unwrap().push("reconcile");
                    let mut receipt = written.clone().unwrap();
                    receipt.state = MountExecutionState::PreconditionChanged;
                    reply.storage_mount = Some(receipt);
                }
                _ => seen.lock().unwrap().push("other"),
            }
            limeos_contracts::write_frame(&mut stream, &reply)
                .await
                .unwrap();
        }
    });
    (dir, core, principal, job, requests, server)
}
async fn mount_state(core: &Core, principal: &Principal, id: &str) -> JobState {
    let (p, id) = (principal.clone(), id.to_owned());
    core.db
        .call(move |store| store.storage_mount_job(&p, &id))
        .await
        .unwrap()
        .state
}

/// An authorization read stuck behind a stalled writer leaves the job
/// queued, with nothing left to run later, and the job runs once the store
/// moves again.
#[tokio::test]
async fn a_mount_authorization_read_behind_a_stalled_writer_leaves_the_job_queued() {
    use crate::operation_tests::{SHORT_CALL, stall_writer, writer_idle};
    let (_dir, mut core, principal, job, requests, server) = queued_mount().await;
    let uid = rustix::process::geteuid().as_raw();
    core.db = core.db.clone().with_call_deadline(SHORT_CALL);
    let release = stall_writer(&core.db).await;
    let pass = core.dispatch_mount(principal.clone(), job.clone());
    let refused = ROOT_UID.scope(uid, pass).await.unwrap_err();
    assert_eq!(refused.0, ErrorCode::Unavailable);
    release.send(()).unwrap();
    writer_idle(&core.db).await;
    assert_eq!(
        mount_state(&core, &principal, &job.id).await,
        JobState::Queued
    );
    assert!(requests.lock().unwrap().is_empty());
    let pass = core.dispatch_mount(principal.clone(), job.clone());
    ROOT_UID.scope(uid, pass).await.unwrap();
    assert_eq!(
        mount_state(&core, &principal, &job.id).await,
        JobState::PreconditionChanged
    );
    assert_eq!(
        *requests.lock().unwrap(),
        ["inspect", "execute", "reconcile", "inspect"]
    );
    server.abort();
}

/// A storage mount claim that commits after core gave up on it holds the
/// job `running` with its claim set and no mount sent. The next pass sends
/// that mount, once, instead of asking for a receipt that cannot exist, and
/// the job settles and frees its claim set.
#[tokio::test]
async fn a_mount_claim_that_commits_after_core_gave_up_is_executed_once_on_the_next_pass() {
    use crate::operation_tests::{SHORT_CALL, locks, with_claim_in_doubt};
    let (dir, mut core, principal, job, requests, server) = queued_mount().await;
    let path = dir.path().join("core.sqlite");
    let uid = rustix::process::geteuid().as_raw();
    core.db = core.db.clone().with_call_deadline(SHORT_CALL);
    let (pass, p, queued) = (core.clone(), principal.clone(), job.clone());
    let result = with_claim_in_doubt(
        &path,
        &core.db,
        ROOT_UID.scope(uid, async move { pass.dispatch_mount(p, queued).await }),
    )
    .await;
    assert_eq!(result.unwrap_err().0, ErrorCode::StateNotDurable);
    assert_eq!(
        mount_state(&core, &principal, &job.id).await,
        JobState::Running
    );
    assert!(locks(&path, &job.id) > 0);
    assert_eq!(*requests.lock().unwrap(), ["inspect"]);
    let (p, listed) = core
        .db
        .call(|store| store.storage_mount_candidates())
        .await
        .unwrap()
        .into_iter()
        .find(|(_, candidate)| candidate.id == job.id)
        .unwrap();
    ROOT_UID
        .scope(uid, core.dispatch_mount(p, listed))
        .await
        .unwrap();
    assert_eq!(
        mount_state(&core, &principal, &job.id).await,
        JobState::PreconditionChanged
    );
    assert_eq!(locks(&path, &job.id), 0);
    assert_eq!(
        *requests.lock().unwrap(),
        ["inspect", "execute", "reconcile", "inspect"]
    );
    server.abort();
}
