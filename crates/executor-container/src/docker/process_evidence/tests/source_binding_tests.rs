use super::*;
use crate::{
    compose_management::{
        sources::{
            self,
            tests::{MANAGED, OPERATOR, from_binding, inputs},
        },
        tests::{bind, refusal},
    },
    docker::compose_evidence::EngineComposeEvidence,
};
use std::{fs, sync::atomic::AtomicBool};
mod lifecycle_tests;

fn labels(project: &str, service: &str, directory: &str, files: &str, oneoff: bool) -> Value {
    json!({"com.docker.compose.project":project,"com.docker.compose.service":service,
        "com.docker.compose.project.working_dir":directory,"com.docker.compose.project.config_files":files,
        "com.docker.compose.oneoff":if oneoff {"True"} else {"False"},"credential":"private-source-value"})
}
fn selected(service: &str, managed: bool, oneoff: bool) -> Value {
    labels(
        "media",
        service,
        "/opt/stacks/media",
        if managed {
            "/opt/stacks/media/compose.yaml,/opt/stacks/media/limeos.override.json"
        } else {
            "/opt/stacks/media/compose.yaml"
        },
        oneoff,
    )
}
fn config(managed: bool) -> Config {
    Config {
        compose_labels: Some(vec![selected("api", managed, false); 2]),
        ..Config::default()
    }
}
async fn engine(f: &Fixture) -> Result<EngineComposeEvidence> {
    EngineComposeEvidence::from_storage(
        inspect_selected(
            &f.docker,
            f.policy,
            TimeSource::default(),
            Some(Admission::new(EngineReadProfile::CombinedV1)),
            true,
        )
        .await?,
    )
}
async fn acquire(f: &Fixture, managed: bool) -> sources::tests::LocalSources {
    from_binding(
        bind(
            inputs(OPERATOR, managed.then_some(MANAGED), |_| {}).unwrap(),
            engine(f).await.unwrap(),
        )
        .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn genuine_bound_json_preserves_full_members_and_original_report_times_and_ledgers() {
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
    let b = acquire(&f, true).await;
    b.revalidate(&|| false).await.unwrap();
    let snapshot = b.owner.snapshot();
    assert_eq!(snapshot.binding.member_indices, [0, 1, 2]);
    assert_eq!(snapshot.binding.engine.containers.len(), 5);
    assert_eq!(snapshot.sources.current.services[0].name, "api");
    assert_eq!(snapshot.sources.current.services[0].mounts.len(), 2);
    assert!(!snapshot.binding.engine.containers[2].container.running);
    assert!(matches!(
        snapshot.binding.engine.containers[2].ownership,
        crate::docker::compose_evidence::ComposeOwnership::Compose { one_off: true, .. }
    ));
    assert!(b.owner.retained_descriptor_count() <= 38);
    let before = serde_json::to_vec(&snapshot).unwrap();
    b.revalidate(&|| false).await.unwrap();
    assert_eq!(serde_json::to_vec(&b.owner.snapshot()).unwrap(), before);
    let text = String::from_utf8(before).unwrap();
    for secret in ["private-source-value", "approval_token", "SYNTHETIC_SECRET"] {
        assert!(!text.contains(secret));
    }
    assert!(
        f.requests()
            .iter()
            .all(|request| request.starts_with("GET "))
    );
}

#[tokio::test]
async fn explicit_absent_managed_and_zero_selected_members_are_past_facts() {
    for count in [0, 2] {
        let f = fixture(Config {
            count,
            first_running: false,
            pid: Some(json!(0)),
            mounts: 0,
            ..config(false)
        })
        .await;
        let b = acquire(&f, false).await;
        b.revalidate(&|| false).await.unwrap();
        assert!(b.owner.snapshot().sources.managed_fingerprint.is_none());
        assert_eq!(b.owner.snapshot().binding.member_indices.len(), count);
        assert_eq!(b.owner.snapshot().sources.current.services.len(), 1);
    }
}

#[tokio::test]
async fn defined_unregistered_service_or_registered_observed_orphan_refuses() {
    let f = fixture(Config {
        count: 0,
        ..config(true)
    })
    .await;
    let input = inputs(OPERATOR, Some(MANAGED), |c| {
        c["stacks"][0]["services"] = json!(["worker"])
    })
    .unwrap();
    refusal(
        from_binding(bind(input, engine(&f).await.unwrap()).unwrap()),
        ErrorCode::Conflict,
    );
    let f = fixture(Config {
        compose_labels: Some(vec![selected("worker", true, false); 2]),
        ..config(true)
    })
    .await;
    let input = inputs(OPERATOR, Some(MANAGED), |_| {}).unwrap();
    refusal(
        from_binding(bind(input, engine(&f).await.unwrap()).unwrap()),
        ErrorCode::Conflict,
    );
}

#[tokio::test]
async fn raw_source_hashes_cannot_be_replaced_by_pure_imported_json_data() {
    let f = fixture(config(true)).await;
    let binding = bind(
        inputs(OPERATOR, Some(MANAGED), |_| {}).unwrap(),
        engine(&f).await.unwrap(),
    )
    .unwrap();
    let source = crate::compose_source::admit_json_sources(OPERATOR, None).unwrap();
    refusal(
        sources::tests::assemble_bound(binding.owner, source),
        ErrorCode::Conflict,
    );
}

#[tokio::test]
async fn protected_but_unsupported_operator_or_managed_source_refuses() {
    for managed_bad in [false, true] {
        let operator = if managed_bad {
            OPERATOR
        } else {
            b"services: {}\n"
        };
        let managed = if managed_bad {
            b"{\"services\":{},\"include\":[]}".as_slice()
        } else {
            MANAGED
        };
        let f = fixture(config(true)).await;
        let binding = bind(
            inputs(operator, Some(managed), |_| {}).unwrap(),
            engine(&f).await.unwrap(),
        )
        .unwrap();
        refusal(from_binding(binding), ErrorCode::InvalidInput);
    }
}

#[tokio::test]
async fn catalog_file_and_engine_changes_poison_bound_source_without_retry_http() {
    for change in 0..3 {
        let f = fixture(config(true)).await;
        let b = acquire(&f, true).await;
        b.revalidate(&|| false).await.unwrap();
        match change {
            0 => {
                let path = b.tree.path().join("config/catalog.json");
                let mut raw = fs::read(&path).unwrap();
                raw.push(b' ');
                fs::write(path, raw).unwrap();
            }
            1 => fs::write(
                b.tree.path().join("opt/stacks/media/limeos.override.json"),
                b"{}",
            )
            .unwrap(),
            _ => f.set(|config| config.changed = Some(Change::Pid)),
        }
        assert!(b.revalidate(&|| false).await.is_err());
        let requests = f.requests().len();
        refusal(b.revalidate(&|| false).await, ErrorCode::Conflict);
        assert_eq!(f.requests().len(), requests);
        assert_eq!(b.owner.snapshot().sources.current.services[0].name, "api");
    }
}

#[tokio::test]
async fn late_cancellation_after_genuine_engine_check_permanently_poisons() {
    let f = fixture(config(true)).await;
    let b = acquire(&f, true).await;
    let before = f.requests().len();
    refusal(
        b.revalidate(&|| f.requests().len() > before).await,
        ErrorCode::Conflict,
    );
    refusal(b.revalidate(&|| false).await, ErrorCode::Conflict);
}

#[tokio::test]
async fn dropping_held_engine_validation_closes_connection_and_poisons_new_owner() {
    let f = fixture(config(true)).await;
    let b = acquire(&f, true).await;
    f.set(|c| c.hold = true);
    let mut check = Box::pin(b.revalidate(&|| false));
    tokio::select! { result=&mut check=>panic!("expected held request: {result:?}"), ()=f.held.notified()=>{} }
    drop(check);
    tokio::time::timeout(Duration::from_secs(1), f.eof.notified())
        .await
        .unwrap();
    f.set(|c| c.hold = false);
    let requests = f.requests().len();
    refusal(b.revalidate(&|| false).await, ErrorCode::Conflict);
    assert_eq!(f.requests().len(), requests);
}

#[tokio::test]
async fn original_deadline_and_wall_rollback_are_not_renewed_by_source_admission() {
    for monotonic in [true, false] {
        let f = fixture(config(true)).await;
        let mut b = acquire(&f, true).await;
        if monotonic {
            b.expire(Duration::from_millis(40));
            f.set(|c| c.hold = true);
        } else {
            b.set_wall_time(crate::compose_files::now().unwrap() + 60);
        }
        refusal(b.revalidate(&|| false).await, ErrorCode::Expired);
        if monotonic {
            tokio::time::timeout(Duration::from_secs(1), f.eof.notified())
                .await
                .unwrap();
        }
        refusal(b.revalidate(&|| false).await, ErrorCode::Conflict);
    }
}

#[tokio::test]
async fn concurrent_reentrant_and_unwound_source_checks_cannot_restore_readiness() {
    let f = fixture(config(true)).await;
    let b = acquire(&f, true).await;
    let (a, c) = tokio::join!(b.revalidate(&|| false), b.revalidate(&|| false));
    refusal(a, ErrorCode::Conflict);
    refusal(c, ErrorCode::Conflict);
    let b = acquire(&f, true).await;
    let called = AtomicBool::new(false);
    refusal(
        b.revalidate(&|| {
            if !called.swap(true, Ordering::SeqCst) {
                let mut nested = Box::pin(b.revalidate(&|| false));
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
    let b = acquire(&f, true).await;
    let cancelled = || panic!("owned retained source unwind");
    let mut check = Box::pin(b.revalidate(&cancelled));
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
            std::future::Future::poll(check.as_mut(), &mut cx)
        }))
        .is_err()
    );
    drop(check);
    refusal(b.revalidate(&|| false).await, ErrorCode::Conflict);
}

#[tokio::test]
async fn combined_borrowed_report_counts_exact_bytes_and_never_exports_body_or_approval() {
    let f = fixture(config(true)).await;
    let b = acquire(&f, true).await;
    let snapshot = b.owner.snapshot();
    let bytes = serde_json::to_vec(&snapshot).unwrap().len();
    sources::tests::counted_report(&snapshot, bytes).unwrap();
    refusal(
        sources::tests::counted_report(&snapshot, bytes - 1),
        ErrorCode::Unavailable,
    );
    let value = serde_json::to_value(snapshot).unwrap();
    for key in ["body", "approval", "receipt", "artifact"] {
        assert!(value.get(key).is_none());
    }
}

#[tokio::test]
async fn public_bound_json_nonroot_refusal_precedes_callback_target_and_engine_access() {
    assert!(!rustix::process::geteuid().is_root());
    let called = AtomicBool::new(false);
    refusal(
        sources::inspect_bound_compose_json(
            std::path::Path::new("/limeos-missing-source-catalog.json"),
            "media-stack",
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
async fn valid_child_binding_cannot_restore_a_source_owner_cancelled_at_entry() {
    let f = fixture(config(true)).await;
    let b = acquire(&f, true).await;
    refusal(b.revalidate(&|| true).await, ErrorCode::Conflict);
    b.check_binding_only(&|| false).await.unwrap();
    let requests = f.requests().len();
    refusal(b.revalidate(&|| false).await, ErrorCode::Conflict);
    assert_eq!(f.requests().len(), requests);
}
