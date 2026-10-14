use super::*;
use crate::compose_management::sources::lifecycle::{
    self, ComposeSourceChange,
    tests::{LocalProposal, counted, propose},
};
use limeos_domain::{ComposeProject, ComposeService, ComposeUser};
use sha2::{Digest, Sha256};

fn worker() -> ComposeService {
    ComposeService {
        name: "worker".into(),
        image: format!("worker@sha256:{}", "c".repeat(64)),
        user: Some(ComposeUser { uid: 42, gid: 43 }),
        restart: limeos_domain::ComposeRestart::No,
        privileged: false,
        host_network: false,
        ports: vec![],
        mounts: vec![],
        devices: vec![],
    }
}
async fn proposal(f: &Fixture, add_worker: bool) -> LocalProposal {
    let source = acquire(f, true).await;
    let mut desired = source.owner.snapshot().sources.current.clone();
    desired.services[0].image = format!("updated@sha256:{}", "d".repeat(64));
    if add_worker {
        desired.services.push(worker());
    }
    let p = propose(source, &desired).unwrap();
    p.revalidate(&|| false).await.unwrap();
    p
}

#[tokio::test]
async fn lifecycle_changes_keep_full_replicas_oneoffs_stopped_unrelated_and_noncompose_rows() {
    let f = fixture(Config {
        count: 5,
        running_count: Some(2),
        mounts: 0,
        compose_labels: Some(vec![
            selected("api", true, false),
            selected("api", true, false),
            selected("api", true, true),
            labels(
                "other",
                "outside",
                "/elsewhere",
                "/elsewhere/compose.yaml",
                false,
            ),
            Value::Null,
        ]),
        ..config(true)
    })
    .await;
    let p = proposal(&f, true).await;
    let s = p.owner.summary();
    assert_eq!(s.changes.len(), 2);
    assert_eq!(s.changes[0].service, "api");
    assert_eq!(s.changes[0].source_change, ComposeSourceChange::Change);
    assert_eq!(s.changes[0].member_indices, [0, 1, 2]);
    assert_eq!(s.changes[1].source_change, ComposeSourceChange::Add);
    assert!(s.changes[1].member_indices.is_empty());
    assert_eq!(s.evidence.binding.engine.containers.len(), 5);
    assert!(!s.evidence.binding.engine.containers[2].container.running);
    assert_eq!(
        s.proposal.desired.services[1].user,
        Some(ComposeUser { uid: 42, gid: 43 })
    );
    assert_eq!(s.proposal.before.services[0].user.unwrap().gid, u32::MAX);
    assert_eq!(
        s.artifact_sha256,
        hex::encode(Sha256::digest(p.owner.artifact()))
    );
    assert_eq!(s.proposal.artifact_bytes, p.owner.artifact().len());
    assert_eq!(s.unresolved.len(), 8);
    assert_eq!(s.proposal.unresolved.len(), 8);
    assert!(p.owner.retained_descriptor_count() <= 38);
    let before = serde_json::to_vec(&s).unwrap();
    let artifact = p.owner.artifact().to_vec();
    p.revalidate(&|| false).await.unwrap();
    assert_eq!(before, serde_json::to_vec(&p.owner.summary()).unwrap());
    assert_eq!(artifact, p.owner.artifact());
    let text = String::from_utf8(before).unwrap();
    for secret in ["private-source-value", "SYNTHETIC_SECRET", "approval_token"] {
        assert!(!text.contains(secret));
    }
    assert!(f.requests().iter().all(|r| r.starts_with("GET ")));
}

#[tokio::test]
async fn removing_managed_service_preserves_all_shutdown_candidates_and_unchanged_operator() {
    let f = fixture(Config {
        count: 4,
        running_count: Some(1),
        mounts: 0,
        compose_labels: Some(vec![
            selected("api", true, false),
            selected("worker", true, false),
            selected("worker", true, false),
            selected("worker", true, true),
        ]),
        ..config(true)
    })
    .await;
    let managed = include_bytes!(
        "../../../../../../../tests/fixtures/compose-override/new-service.override.json"
    );
    let source = from_binding(
        bind(
            inputs(OPERATOR, Some(managed), |_| {}).unwrap(),
            engine(&f).await.unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let desired = source.owner.snapshot().sources.operator.clone();
    let p = propose(source, &desired).unwrap();
    p.revalidate(&|| false).await.unwrap();
    let s = p.owner.summary();
    assert_eq!(s.changes.len(), 1);
    assert_eq!(s.changes[0].source_change, ComposeSourceChange::Remove);
    assert_eq!(s.changes[0].member_indices, [1, 2, 3]);
    assert_eq!(s.proposal.shutdown_required, ["worker"]);
    assert_eq!(s.evidence.binding.member_indices, [0, 1, 2, 3]);
    assert_eq!(p.owner.artifact(), b"{\"services\":{}}");
    assert_eq!(s.proposal.desired, s.evidence.sources.operator);
}

#[tokio::test]
async fn zero_observed_members_and_noop_source_proposals_do_not_invent_removal_receipts() {
    let f = fixture(Config {
        count: 0,
        ..config(true)
    })
    .await;
    let source = acquire(&f, true).await;
    let desired = source.owner.snapshot().sources.current.clone();
    let p = propose(source, &desired).unwrap();
    p.revalidate(&|| false).await.unwrap();
    let s = p.owner.summary();
    assert!(s.changes.is_empty());
    assert!(s.evidence.binding.member_indices.is_empty());
    assert!(s.proposal.shutdown_required.is_empty());
    assert_eq!(s.proposal.before, s.proposal.desired);
    let value = serde_json::to_value(s).unwrap();
    for key in [
        "approved",
        "receipt",
        "shutdown_verified",
        "safe_absence",
        "deletion",
        "artifact",
    ] {
        assert!(value.get(key).is_none());
    }
}

#[tokio::test]
async fn desired_unregistered_oversized_and_unrepresentable_projects_refuse_actual_binding() {
    let f = fixture(config(true)).await;
    for problem in 0..4 {
        let source = acquire(&f, true).await;
        let mut desired = source.owner.snapshot().sources.current.clone();
        let expected = match problem {
            0 => {
                let mut svc = worker();
                svc.name = "outside".into();
                desired.services.push(svc);
                ErrorCode::Conflict
            }
            1 => {
                desired.services = vec![worker(); 17];
                ErrorCode::InvalidInput
            }
            2 => {
                desired.services[0].ports.clear();
                ErrorCode::Conflict
            }
            _ => {
                desired.services[0].image = "x".repeat(265);
                ErrorCode::InvalidInput
            }
        };
        refusal(propose(source, &desired), expected);
    }
}

#[tokio::test]
async fn poisoned_source_cannot_construct_a_new_lifecycle_owner_before_desired_or_callback_work() {
    let f = fixture(config(true)).await;
    let source = acquire(&f, true).await;
    refusal(source.revalidate(&|| true).await, ErrorCode::Conflict);
    let called = AtomicBool::new(false);
    let desired = ComposeProject {
        services: vec![worker(); 17],
    };
    refusal(
        source
            .owner
            .propose_lifecycle(&desired, &|| {
                called.store(true, Ordering::SeqCst);
                false
            })
            .await,
        ErrorCode::Conflict,
    );
    assert!(!called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn lifecycle_catalog_file_and_engine_changes_permanently_refuse_without_retry_http() {
    for change in 0..3 {
        let f = fixture(config(true)).await;
        let p = proposal(&f, false).await;
        match change {
            0 => {
                let path = p.tree.path().join("config/catalog.json");
                let mut raw = fs::read(&path).unwrap();
                raw.push(b' ');
                fs::write(path, raw).unwrap();
            }
            1 => fs::write(
                p.tree.path().join("opt/stacks/media/limeos.override.json"),
                b"{}",
            )
            .unwrap(),
            _ => f.set(|c| c.changed = Some(Change::Membership)),
        }
        assert!(p.revalidate(&|| false).await.is_err());
        let requests = f.requests().len();
        refusal(p.revalidate(&|| false).await, ErrorCode::Conflict);
        assert_eq!(f.requests().len(), requests);
    }
}

#[tokio::test]
async fn cancellation_at_proposal_entry_cannot_be_restored_by_a_successful_child_check() {
    let f = fixture(config(true)).await;
    let p = proposal(&f, false).await;
    refusal(p.revalidate(&|| true).await, ErrorCode::Conflict);
    p.check_source_only(&|| false).await.unwrap();
    let requests = f.requests().len();
    refusal(p.revalidate(&|| false).await, ErrorCode::Conflict);
    assert_eq!(f.requests().len(), requests);
}

#[tokio::test]
async fn late_cancel_and_spent_original_file_ledger_permanently_refuse() {
    let f = fixture(config(true)).await;
    let p = proposal(&f, false).await;
    let requests = f.requests().len();
    refusal(
        p.revalidate(&|| f.requests().len() > requests).await,
        ErrorCode::Conflict,
    );
    refusal(p.revalidate(&|| false).await, ErrorCode::Conflict);
    let p = proposal(&f, false).await;
    p.exhaust_reads();
    refusal(p.revalidate(&|| false).await, ErrorCode::Unavailable);
    refusal(p.revalidate(&|| false).await, ErrorCode::Conflict);
}

#[tokio::test]
async fn dropped_and_timed_out_lifecycle_checks_close_actual_engine_connection_and_poison() {
    for timeout in [false, true] {
        let f = fixture(config(true)).await;
        let mut p = proposal(&f, false).await;
        if timeout {
            p.expire(Duration::from_millis(40));
        }
        f.set(|c| c.hold = true);
        if timeout {
            refusal(p.revalidate(&|| false).await, ErrorCode::Expired);
        } else {
            let mut check = Box::pin(p.revalidate(&|| false));
            tokio::select! { result=&mut check=>panic!("expected held request: {result:?}"), ()=f.held.notified()=>{} }
            drop(check);
        }
        tokio::time::timeout(Duration::from_secs(1), f.eof.notified())
            .await
            .unwrap();
        f.set(|c| c.hold = false);
        let requests = f.requests().len();
        refusal(p.revalidate(&|| false).await, ErrorCode::Conflict);
        assert_eq!(requests, f.requests().len());
    }
}

#[tokio::test]
async fn original_source_deadline_and_wall_rollback_are_not_renewed_for_proposals() {
    let f = fixture(config(true)).await;
    let mut source = acquire(&f, true).await;
    let desired = source.owner.snapshot().sources.current.clone();
    source.expire(Duration::ZERO);
    refusal(propose(source, &desired), ErrorCode::Expired);
    let mut p = proposal(&f, false).await;
    p.set_wall_time(crate::compose_files::now().unwrap() + 60);
    refusal(p.revalidate(&|| false).await, ErrorCode::Expired);
    refusal(p.revalidate(&|| false).await, ErrorCode::Conflict);
}

#[tokio::test]
async fn concurrent_reentrant_and_unwound_lifecycle_checks_permanently_refuse() {
    let f = fixture(config(true)).await;
    let p = proposal(&f, false).await;
    let (a, b) = tokio::join!(p.revalidate(&|| false), p.revalidate(&|| false));
    refusal(a, ErrorCode::Conflict);
    refusal(b, ErrorCode::Conflict);
    let p = proposal(&f, false).await;
    let called = AtomicBool::new(false);
    refusal(
        p.revalidate(&|| {
            if !called.swap(true, Ordering::SeqCst) {
                let mut nested = Box::pin(p.revalidate(&|| false));
                let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
                assert!(matches!(
                    std::future::Future::poll(nested.as_mut(), &mut cx),
                    std::task::Poll::Ready(Err(Error(ErrorCode::Conflict)))
                ));
            }
            false
        })
        .await,
        ErrorCode::Conflict,
    );
    let p = proposal(&f, false).await;
    let cancelled = || panic!("owned lifecycle unwind");
    let mut check = Box::pin(p.revalidate(&cancelled));
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
            std::future::Future::poll(check.as_mut(), &mut cx)
        }))
        .is_err()
    );
    drop(check);
    refusal(p.revalidate(&|| false).await, ErrorCode::Conflict);
}

#[tokio::test]
async fn borrowed_lifecycle_report_counts_exact_combined_bytes() {
    let f = fixture(config(true)).await;
    let p = proposal(&f, true).await;
    let s = p.owner.summary();
    let bytes = serde_json::to_vec(&s).unwrap().len();
    counted(&s, bytes).unwrap();
    refusal(counted(&s, bytes - 1), ErrorCode::Unavailable);
}

#[tokio::test]
async fn public_lifecycle_nonroot_refusal_precedes_callback_desired_target_and_engine() {
    assert!(!rustix::process::geteuid().is_root());
    let called = AtomicBool::new(false);
    refusal(
        lifecycle::inspect_compose_lifecycle_proposal(
            std::path::Path::new("/missing-lifecycle-catalog.json"),
            "media-stack",
            &ComposeProject {
                services: vec![worker(); 17],
            },
            &|| {
                called.store(true, Ordering::SeqCst);
                false
            },
        )
        .await,
        ErrorCode::Forbidden,
    );
    assert!(!called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn individually_admitted_sources_and_override_refuse_an_oversized_whole_lifecycle_report() {
    let f = fixture(Config {
        count: 0,
        ..config(false)
    })
    .await;
    let mut services = serde_json::Map::new();
    for n in 0..11 {
        let mounts: Vec<_> = (0..8)
            .map(|m| {
                json!({
                    "type":"bind", "source": format!("/srv/{m}/{}", "s".repeat(230)),
                    "target": format!("/media/{m}/{}", "t".repeat(230)), "read_only":true,
                    "bind":{"create_host_path":false}
                })
            })
            .collect();
        services.insert(
            format!("api{n}"),
            json!({
                "image":format!("base@sha256:{}", "a".repeat(64)),
                "user":"1000:1000", "volumes":mounts
            }),
        );
    }
    let names: Vec<_> = services.keys().cloned().collect();
    let raw = serde_json::to_vec(&json!({"services":services})).unwrap();
    assert!(raw.len() < 64 * 1024);
    let source = from_binding(
        bind(
            inputs(&raw, None, |c| {
                c["stacks"][0]["services"] = json!(names);
            })
            .unwrap(),
            engine(&f).await.unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let mut desired = source.owner.snapshot().sources.current.clone();
    for s in &mut desired.services {
        s.image = format!("next@sha256:{}", "b".repeat(64));
    }
    // Independently admit the real original bytes and prove the child plan fits its bound.
    let pure = crate::compose_source::admit_json_sources(&raw, None).unwrap();
    let plan = pure.plan(&desired).unwrap();
    assert!(serde_json::to_vec(&source.owner.snapshot()).unwrap().len() < 256 * 1024);
    assert!(serde_json::to_vec(&plan.summary()).unwrap().len() < 256 * 1024);
    refusal(propose(source, &desired), ErrorCode::Unavailable);
}
