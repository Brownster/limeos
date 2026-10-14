use super::*;
use crate::{
    compose_management::{
        self,
        tests::{bind, configuration, inputs_with, refusal},
    },
    docker::compose_evidence::EngineComposeEvidence,
};
use std::{fs, sync::atomic::AtomicBool};

fn labels(project: &str, service: &str, directory: &str, files: &str, oneoff: bool) -> Value {
    json!({"com.docker.compose.project":project,"com.docker.compose.service":service,
        "com.docker.compose.project.working_dir":directory,"com.docker.compose.project.config_files":files,
        "com.docker.compose.oneoff":if oneoff { "True" } else { "False" }, "credential":"private-management-value"})
}
fn selected(service: &str, oneoff: bool) -> Value {
    labels(
        "media",
        service,
        "/opt/stacks/media",
        "/opt/stacks/media/compose.yaml,/opt/stacks/media/limeos.override.json",
        oneoff,
    )
}
fn config() -> Config {
    Config {
        compose_labels: Some(vec![
            selected("media", false),
            selected("transcoder", false),
        ]),
        ..Config::default()
    }
}
async fn inspect(f: &Fixture) -> Result<EngineComposeEvidence> {
    let owner = inspect_selected(
        &f.docker,
        f.policy,
        TimeSource::default(),
        Some(Admission::new(EngineReadProfile::CombinedV1)),
        true,
    )
    .await?;
    EngineComposeEvidence::from_storage(owner)
}

#[tokio::test]
async fn genuine_binding_preserves_full_members_replicas_oneoffs_stopped_and_unrelated_rows() {
    let inputs = inputs_with(configuration()).unwrap();
    let f = fixture(Config {
        count: 5,
        running_count: Some(2),
        mounts: 0,
        compose_labels: Some(vec![
            selected("media", false),
            selected("media", false),
            selected("transcoder", true),
            labels(
                "other",
                "service",
                "/elsewhere",
                "/elsewhere/compose.yaml",
                false,
            ),
            Value::Null,
        ]),
        ..config()
    })
    .await;
    let engine = inspect(&f).await.unwrap();
    let original_time = engine.snapshot().containers_observed_at;
    let binding = bind(inputs, engine).unwrap();
    binding.revalidate(&|| false).await.unwrap();
    let report = binding.owner.snapshot();
    assert_eq!(report.stack_resource, "stack:media-stack");
    assert_eq!(report.member_indices, [0, 1, 2]);
    assert_eq!(report.engine.containers.len(), 5);
    assert_eq!(report.engine.containers_observed_at, original_time);
    assert!(!report.engine.containers[2].container.running);
    assert!(matches!(
        report.engine.containers[2].ownership,
        crate::docker::compose_evidence::ComposeOwnership::Compose { one_off: true, .. }
    ));
    assert!(matches!(
        report.engine.containers[4].ownership,
        crate::docker::compose_evidence::ComposeOwnership::NonCompose
    ));
    for index in report.member_indices {
        assert!(
            report.engine.containers[*index]
                .container
                .resource
                .starts_with("container:")
        );
    }
    assert!(binding.owner.retained_descriptor_count() <= 38);
    let original = serde_json::to_vec(&binding.owner.snapshot()).unwrap();
    binding.revalidate(&|| false).await.unwrap();
    assert_eq!(
        serde_json::to_vec(&binding.owner.snapshot()).unwrap(),
        original
    );
    let text = String::from_utf8(original).unwrap();
    for value in [
        "private-management-value",
        "SYNTHETIC_SECRET",
        "approval_token",
        "template_digest",
    ] {
        assert!(!text.contains(value));
    }
    assert!(
        f.requests()
            .iter()
            .all(|request| request.starts_with("GET "))
    );
}
#[tokio::test]
async fn expected_engine_and_raw_operator_or_managed_hash_mismatches_refuse() {
    for change in 0..3 {
        let mut c = configuration();
        match change {
            0 => c["engine_id"] = json!("different-engine"),
            1 => c["stacks"][0]["operator"]["sha256"] = json!("a".repeat(64)),
            _ => c["stacks"][0]["managed"]["sha256"] = json!("b".repeat(64)),
        }
        let inputs = inputs_with(c).unwrap();
        let f = fixture(config()).await;
        refusal(
            bind(inputs, inspect(&f).await.unwrap()),
            ErrorCode::Conflict,
        );
    }
}
#[tokio::test]
async fn full_declared_directory_order_service_and_project_identity_must_match() {
    for change in 0..4 {
        let mut rows = vec![selected("media", false), selected("transcoder", false)];
        for row in &mut rows {
            match change {
                0 => {
                    row["com.docker.compose.project.working_dir"] = json!("/opt/stacks/media-other")
                }
                1 => {
                    row["com.docker.compose.project.config_files"] = json!(
                        "/opt/stacks/media/limeos.override.json,/opt/stacks/media/compose.yaml"
                    )
                }
                2 => row["com.docker.compose.service"] = json!("unregistered"),
                _ => row["com.docker.compose.project"] = json!("different-project"),
            }
        }
        let f = fixture(Config {
            compose_labels: Some(rows),
            ..config()
        })
        .await;
        refusal(
            bind(
                inputs_with(configuration()).unwrap(),
                inspect(&f).await.unwrap(),
            ),
            ErrorCode::Conflict,
        );
    }
}
#[tokio::test]
async fn differently_named_project_cannot_share_or_nest_selected_paths() {
    for (directory, files) in [
        (
            "/opt/stacks/media/child",
            "/opt/stacks/media/child/compose.yaml",
        ),
        ("/opt/stacks", "/opt/stacks/compose.yaml"),
        ("/elsewhere", "/opt/stacks/media/compose.yaml"),
    ] {
        let f = fixture(Config {
            compose_labels: Some(vec![
                selected("media", false),
                labels("other", "service", directory, files, false),
            ]),
            ..config()
        })
        .await;
        refusal(
            bind(
                inputs_with(configuration()).unwrap(),
                inspect(&f).await.unwrap(),
            ),
            ErrorCode::Conflict,
        );
    }
    let f = fixture(Config {
        compose_labels: Some(vec![
            selected("media", false),
            labels(
                "other",
                "service",
                "/opt/stacks/media-extra",
                "/opt/stacks/media-extra/compose.yaml",
                false,
            ),
        ]),
        ..config()
    })
    .await;
    let b = bind(
        inputs_with(configuration()).unwrap(),
        inspect(&f).await.unwrap(),
    )
    .unwrap();
    b.revalidate(&|| false).await.unwrap();
    assert_eq!(b.owner.snapshot().member_indices, [0]);
}
#[tokio::test]
async fn registered_zero_members_and_all_stopped_members_keep_past_facts_without_effect_authority()
{
    for count in [0, 2] {
        let f = fixture(Config {
            count,
            running_count: Some(0),
            pid: Some(json!(0)),
            ..config()
        })
        .await;
        let b = bind(
            inputs_with(configuration()).unwrap(),
            inspect(&f).await.unwrap(),
        )
        .unwrap();
        b.revalidate(&|| false).await.unwrap();
        assert_eq!(b.owner.snapshot().member_indices.len(), count);
        assert!(
            b.owner
                .snapshot()
                .engine
                .containers
                .iter()
                .all(|m| !m.container.running)
        );
    }
}
#[tokio::test]
async fn all_operator_names_and_explicit_absent_managed_selection_bind_in_order() {
    for name in [
        "compose.yaml",
        "compose.yml",
        "docker-compose.yaml",
        "docker-compose.yml",
    ] {
        for present in [false, true] {
            let mut c = configuration();
            c["stacks"][0]["operator"]["name"] = json!(name);
            if !present {
                c["stacks"][0]["managed"] = json!({"state":"absent"});
            }
            let files = format!(
                "/opt/stacks/media/{name}{}",
                if present {
                    ",/opt/stacks/media/limeos.override.json"
                } else {
                    ""
                }
            );
            let f = fixture(Config {
                compose_labels: Some(vec![
                    labels(
                        "media",
                        "media",
                        "/opt/stacks/media",
                        &files,
                        false
                    );
                    2
                ]),
                ..config()
            })
            .await;
            let b = bind(inputs_with(c).unwrap(), inspect(&f).await.unwrap()).unwrap();
            b.revalidate(&|| false).await.unwrap();
            assert_eq!(
                b.owner.snapshot().files.files.len(),
                1 + usize::from(present)
            );
            assert_eq!(b.owner.snapshot().member_indices, [0, 1]);
        }
    }
}
#[tokio::test]
async fn different_genuine_file_root_cannot_reuse_catalog_metadata() {
    let mut inputs = inputs_with(configuration()).unwrap();
    let other = inputs_with(configuration()).unwrap();
    inputs.files = other.files;
    let f = fixture(config()).await;
    refusal(
        bind(inputs, inspect(&f).await.unwrap()),
        ErrorCode::Conflict,
    );
}
#[tokio::test]
async fn actual_catalog_file_or_engine_changes_poison_binding_and_cannot_restore_it() {
    for change in 0..3 {
        let f = fixture(config()).await;
        let b = bind(
            inputs_with(configuration()).unwrap(),
            inspect(&f).await.unwrap(),
        )
        .unwrap();
        b.revalidate(&|| false).await.unwrap();
        match change {
            0 => {
                let path = b.tree.path().join("config/catalog.json");
                let mut raw = fs::read(&path).unwrap();
                raw.push(b' ');
                fs::write(path, raw).unwrap();
            }
            1 => fs::write(
                b.tree.path().join("opt/stacks/media/compose.yaml"),
                b"changed",
            )
            .unwrap(),
            _ => f.set(|c| c.changed = Some(Change::Pid)),
        }
        assert!(b.revalidate(&|| false).await.is_err());
        let requests = f.requests().len();
        refusal(b.revalidate(&|| false).await, ErrorCode::Conflict);
        assert_eq!(f.requests().len(), requests);
        assert_eq!(b.owner.snapshot().stack_resource, "stack:media-stack");
    }
}
#[tokio::test]
async fn late_cancel_after_engine_checks_and_cancelled_entry_poison() {
    for late in [false, true] {
        let f = fixture(config()).await;
        let b = bind(
            inputs_with(configuration()).unwrap(),
            inspect(&f).await.unwrap(),
        )
        .unwrap();
        let baseline = f.requests().len();
        refusal(
            b.revalidate(&|| !late || f.requests().len() > baseline)
                .await,
            ErrorCode::Conflict,
        );
        refusal(b.revalidate(&|| false).await, ErrorCode::Conflict);
    }
}
#[tokio::test]
async fn drop_during_genuine_engine_revalidation_closes_connection_and_poisoned_retry_sends_no_http()
 {
    let f = fixture(config()).await;
    let b = bind(
        inputs_with(configuration()).unwrap(),
        inspect(&f).await.unwrap(),
    )
    .unwrap();
    f.set(|c| c.hold = true);
    let mut check = Box::pin(b.revalidate(&|| false));
    tokio::select! { result = &mut check => panic!("expected held request: {result:?}"), () = f.held.notified() => {} }
    drop(check);
    tokio::time::timeout(Duration::from_secs(1), f.eof.notified())
        .await
        .unwrap();
    f.set(|c| c.hold = false);
    let before = f.requests().len();
    refusal(b.revalidate(&|| false).await, ErrorCode::Conflict);
    assert_eq!(f.requests().len(), before);
}
#[tokio::test]
async fn original_binding_timeout_closes_held_http_and_permanently_invalidates() {
    let f = fixture(config()).await;
    let mut b = bind(
        inputs_with(configuration()).unwrap(),
        inspect(&f).await.unwrap(),
    )
    .unwrap();
    b.expire(Duration::from_millis(40));
    f.set(|c| c.hold = true);
    refusal(b.revalidate(&|| false).await, ErrorCode::Expired);
    tokio::time::timeout(Duration::from_secs(1), f.eof.notified())
        .await
        .unwrap();
    refusal(b.revalidate(&|| false).await, ErrorCode::Conflict);
}
#[tokio::test]
async fn concurrent_refusal_or_reentrant_poll_cannot_restore_first_success() {
    let f = fixture(config()).await;
    let b = bind(
        inputs_with(configuration()).unwrap(),
        inspect(&f).await.unwrap(),
    )
    .unwrap();
    let (first, second) = tokio::join!(b.revalidate(&|| false), b.revalidate(&|| false));
    refusal(first, ErrorCode::Conflict);
    refusal(second, ErrorCode::Conflict);
    let b = bind(
        inputs_with(configuration()).unwrap(),
        inspect(&f).await.unwrap(),
    )
    .unwrap();
    let called = AtomicBool::new(false);
    refusal(
        b.revalidate(&|| {
            if !called.swap(true, Ordering::SeqCst) {
                let mut nested = Box::pin(b.revalidate(&|| false));
                let mut context = std::task::Context::from_waker(std::task::Waker::noop());
                assert!(matches!(
                    std::future::Future::poll(nested.as_mut(), &mut context),
                    std::task::Poll::Ready(Err(Error(ErrorCode::Conflict)))
                ));
            }
            false
        })
        .await,
        ErrorCode::Conflict,
    );
}
#[tokio::test]
async fn original_wall_time_and_borrowed_report_bounds_do_not_renew() {
    let f = fixture(config()).await;
    let b = bind(
        inputs_with(configuration()).unwrap(),
        inspect(&f).await.unwrap(),
    )
    .unwrap();
    let before = b.owner.snapshot().observed_at;
    b.revalidate(&|| false).await.unwrap();
    assert_eq!(b.owner.snapshot().observed_at, before);
    let bytes = serde_json::to_vec(&b.owner.snapshot()).unwrap().len();
    compose_management::check_report(&b.owner.snapshot(), bytes).unwrap();
    refusal(
        compose_management::check_report(&b.owner.snapshot(), bytes - 1),
        ErrorCode::Unavailable,
    );
    let mut b = bind(
        inputs_with(configuration()).unwrap(),
        inspect(&f).await.unwrap(),
    )
    .unwrap();
    b.set_wall_time(crate::compose_files::now().unwrap() + 1);
    refusal(b.revalidate(&|| false).await, ErrorCode::Expired);
}
#[tokio::test]
async fn unwind_during_binding_validation_permanently_invalidates_without_http() {
    let f = fixture(config()).await;
    let b = bind(
        inputs_with(configuration()).unwrap(),
        inspect(&f).await.unwrap(),
    )
    .unwrap();
    let before = f.requests().len();
    let cancelled = || panic!("owned binding validation unwind");
    let mut check = Box::pin(b.revalidate(&cancelled));
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut context = std::task::Context::from_waker(std::task::Waker::noop());
            std::future::Future::poll(check.as_mut(), &mut context)
        }))
        .is_err()
    );
    drop(check);
    refusal(b.revalidate(&|| false).await, ErrorCode::Conflict);
    assert_eq!(f.requests().len(), before);
}
#[tokio::test]
async fn public_binding_nonroot_refusal_precedes_callback_target_and_canonical_engine_access() {
    assert!(!rustix::process::geteuid().is_root());
    let called = AtomicBool::new(false);
    refusal(
        compose_management::inspect_compose_binding(
            std::path::Path::new("/limeos-owned-missing-binding-catalog.json"),
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
