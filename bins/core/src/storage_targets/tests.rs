use super::*;
use std::os::unix::fs::MetadataExt;
tokio::task_local! { pub(super) static ROOT_UID: u32; }
async fn core(
    dir: &tempfile::TempDir,
    role: limeos_domain::Role,
) -> (Core, String, String, Principal) {
    let db = Database::open(&dir.path().join("core.sqlite")).unwrap();
    let token = "a".repeat(64);
    let csrf = "b".repeat(64);
    let digest = limeos_identity::digest(&token);
    let csrf_hash = limeos_identity::digest(&csrf);
    let p = db
        .call(move |s| {
            s.issue_bootstrap("bootstrap", now())?;
            s.enroll("bootstrap", "admin", "$argon2id$fixture", now())?;
            let id = s.login_record("admin")?.unwrap().principal.id;
            s.revise_grants(&id, role, &[limeos_persistence::storage_scope()], now())?;
            let record = s.login_record("admin")?.unwrap();
            s.create_session(&record, None, &digest, &csrf_hash, now())?;
            Ok(record.principal)
        })
        .await
        .unwrap();
    (
        Core {
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
        },
        token,
        csrf,
        p,
    )
}
fn fixture() -> StorageContract {
    let value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/storage-contracts.json"
    ))
    .unwrap();
    serde_json::from_value(value["cases"][0]["contract"].clone()).unwrap()
}
#[tokio::test]
async fn target_authority_and_csrf_are_checked_before_reader_access_and_tasks_cannot_approve() {
    for role in [
        limeos_domain::Role::Viewer,
        limeos_domain::Role::Operator,
        limeos_domain::Role::Administrator,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (c, token, csrf, _) = core(&dir, role).await;
        assert_eq!(
            c.human_plan_targets(token, csrf, fixture())
                .await
                .unwrap_err()
                .0,
            if role == limeos_domain::Role::Administrator {
                ErrorCode::Unavailable
            } else {
                ErrorCode::Forbidden
            }
        );
        assert_eq!(
            c.human_plan_targets("a".repeat(64), "bad-csrf".into(), fixture())
                .await
                .unwrap_err()
                .0,
            ErrorCode::Forbidden
        );
    }
    for request in [
        "approve_storage_targets",
        "queue_storage_targets",
        "prepare_storage_targets",
    ] {
        let raw =
            format!(r#"{{"request":"{request}","version":1,"token":"task","principal":"admin"}}"#);
        assert!(serde_json::from_str::<CoreRequest>(&raw).is_err());
    }
}
#[tokio::test]
async fn an_unprivileged_target_socket_cannot_forge_evidence_and_receives_no_frame() {
    let dir = tempfile::tempdir().unwrap();
    if dir.path().metadata().unwrap().uid() == 0 {
        return;
    }
    let path = dir.path().join("target.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let (mut c, _, _, _) = core(&dir, limeos_domain::Role::Administrator).await;
    c.storage_target_socket = Arc::new(path);
    assert_eq!(
        c.target_snapshot(&fixture()).await.unwrap_err().0,
        ErrorCode::Forbidden
    );
    let (mut stream, _) = listener.accept().await.unwrap();
    use tokio::io::AsyncReadExt;
    assert_eq!(stream.read(&mut [0u8; 4]).await.unwrap(), 0);
}

/// The storage target fixture's current state, as the storage reader reports it.
fn snapshot(contract: &StorageContract) -> StorageTargetSnapshot {
    StorageTargetSnapshot {
        inventory: limeos_domain::StorageInventory {
            host_root_mount_id: 1,
            topology_digest: "a".repeat(64),
            mounts_digest: "b".repeat(64),
            fstab_digest: "c".repeat(64),
            fstab_entries: vec![],
            devices: vec![limeos_domain::StorageObservedDevice {
                device: limeos_domain::StorageBlockDevice { major: 8, minor: 1 },
                filesystem_uuid: contract.devices[0].filesystem_uuid.clone(),
                filesystem: "ext4".into(),
                serial: None,
                boot_backing: false,
                in_use_as_swap: false,
                mounts: vec![],
            }],
        },
        targets: vec![limeos_domain::TargetEvidence {
            mountpoint: contract.devices[0].mountpoint.clone(),
            parent: limeos_domain::DirectoryIdentity {
                major: 8,
                minor: 0,
                mount_id: 1,
                inode: 5,
                mode: 0o40755,
                uid: 0,
                gid: 0,
            },
            existing: None,
        }],
    }
}

type Requests = Arc<std::sync::Mutex<Vec<&'static str>>>;
/// A queued storage target job, with a root fixture that reports the
/// current state and answers a preparation as refused on a changed
/// precondition; it lists the requests it served.
async fn queued_target(
    dir: &tempfile::TempDir,
) -> (
    Core,
    Principal,
    StorageTargetJob,
    Requests,
    tokio::task::JoinHandle<()>,
) {
    let (mut c, _, _, principal) = core(dir, limeos_domain::Role::Administrator).await;
    let contract = fixture();
    let current = snapshot(&contract);
    let (p, facts) = (principal.clone(), current.clone());
    let job =
        c.db.call(move |s| {
            let proposal = s.plan_storage_targets(&p, &contract, &facts, now())?;
            let approval =
                s.approve_storage_targets(&p, &proposal.plan.id, &proposal.digest, &facts, now())?;
            s.queue_storage_targets(&p, "in-doubt", &proposal, &approval.token, &facts, now())
        })
        .await
        .unwrap();
    let socket = dir.path().join("targets.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    c.storage_target_socket = Arc::new(socket);
    let requests = Requests::default();
    let seen = requests.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request: Request = limeos_contracts::read_frame(&mut stream).await.unwrap();
            let mut reply = Receipt::empty();
            match request {
                Request::InspectStorageTargets { .. } => {
                    seen.lock().unwrap().push("inspect");
                    reply.storage_targets = Some(current.clone());
                }
                Request::PrepareStorageTargets { plan, .. } => {
                    seen.lock().unwrap().push("prepare");
                    reply.target_preparation = Some(limeos_domain::TargetPreparationReceipt {
                        digest: limeos_identity::digest(&serde_json::to_string(&plan).unwrap()),
                        plan: *plan,
                        state: limeos_domain::PreparationState::PreconditionChanged,
                        after: vec![],
                    });
                }
                _ => seen.lock().unwrap().push("other"),
            }
            limeos_contracts::write_frame(&mut stream, &reply)
                .await
                .unwrap();
        }
    });
    (c, principal, job, requests, server)
}
async fn target_state(c: &Core, principal: &Principal, id: &str) -> JobState {
    let (p, id) = (principal.clone(), id.to_owned());
    c.db.call(move |s| s.storage_target_job(&p, &id))
        .await
        .unwrap()
        .state
}

/// An authorization read stuck behind a stalled writer leaves the job
/// queued, with nothing left to run later, and the job runs once the store
/// moves again.
#[tokio::test]
async fn a_target_authorization_read_behind_a_stalled_writer_leaves_the_job_queued() {
    use crate::operation_tests::{SHORT_CALL, stall_writer, writer_idle};
    let dir = tempfile::tempdir().unwrap();
    let (mut c, principal, job, requests, server) = queued_target(&dir).await;
    let uid = rustix::process::geteuid().as_raw();
    c.db = c.db.clone().with_call_deadline(SHORT_CALL);
    let release = stall_writer(&c.db).await;
    let pass = c.dispatch_target(principal.clone(), job.clone());
    let refused = ROOT_UID.scope(uid, pass).await.unwrap_err();
    assert_eq!(refused.0, ErrorCode::Unavailable);
    release.send(()).unwrap();
    writer_idle(&c.db).await;
    // The short deadline was for the stalled call only: a slow machine must
    // not time out the ordinary calls that follow.
    c.db =
        c.db.clone()
            .with_call_deadline(limeos_persistence::CALL_DEADLINE);
    assert_eq!(
        target_state(&c, &principal, &job.id).await,
        JobState::Queued
    );
    assert!(requests.lock().unwrap().is_empty());
    let pass = c.dispatch_target(principal.clone(), job.clone());
    ROOT_UID.scope(uid, pass).await.unwrap();
    assert_eq!(
        target_state(&c, &principal, &job.id).await,
        JobState::PreconditionChanged
    );
    assert_eq!(*requests.lock().unwrap(), ["inspect", "prepare"]);
    server.abort();
}

/// A storage target claim that commits after core gave up on it holds the
/// job `running` with its locks and no preparation sent. The next pass
/// sends that preparation, once, instead of asking for a receipt that
/// cannot exist, and the job settles and frees its locks.
#[tokio::test]
async fn a_target_claim_that_commits_after_core_gave_up_is_prepared_once_on_the_next_pass() {
    use crate::operation_tests::{SHORT_CALL, locks, with_claim_in_doubt};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let (mut c, principal, job, requests, server) = queued_target(&dir).await;
    let uid = rustix::process::geteuid().as_raw();
    c.db = c.db.clone().with_call_deadline(SHORT_CALL);
    let (pass, p, queued) = (c.clone(), principal.clone(), job.clone());
    let result = with_claim_in_doubt(
        &path,
        &c.db,
        ROOT_UID.scope(uid, async move { pass.dispatch_target(p, queued).await }),
    )
    .await;
    assert_eq!(result.unwrap_err().0, ErrorCode::StateNotDurable);
    // The short deadline was for the stalled call only: a slow machine must
    // not time out the ordinary calls that follow.
    c.db =
        c.db.clone()
            .with_call_deadline(limeos_persistence::CALL_DEADLINE);
    assert_eq!(
        target_state(&c, &principal, &job.id).await,
        JobState::Running
    );
    assert!(locks(&path, &job.id) > 0);
    assert_eq!(*requests.lock().unwrap(), ["inspect"]);
    let (p, listed) =
        c.db.call(|s| s.storage_target_candidates())
            .await
            .unwrap()
            .into_iter()
            .find(|(_, candidate)| candidate.id == job.id)
            .unwrap();
    ROOT_UID
        .scope(uid, c.dispatch_target(p, listed))
        .await
        .unwrap();
    assert_eq!(
        target_state(&c, &principal, &job.id).await,
        JobState::PreconditionChanged
    );
    assert_eq!(locks(&path, &job.id), 0);
    assert_eq!(*requests.lock().unwrap(), ["inspect", "prepare"]);
    server.abort();
}
