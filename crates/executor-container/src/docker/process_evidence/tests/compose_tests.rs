use super::*;
use crate::docker::compose_evidence::{
    ComposeOwnership, EngineComposeEvidence, inspect_compose_membership,
};

fn labels(service: &str) -> Value {
    json!({"com.docker.compose.project":"media","com.docker.compose.service":service,
        "com.docker.compose.project.working_dir":"/opt/stacks/media",
        "com.docker.compose.project.config_files":"/opt/stacks/media/compose.yaml,/opt/stacks/media/override.yaml",
        "com.docker.compose.oneoff":"False","credential":"private-value"})
}
fn config() -> Config {
    Config {
        compose_labels: Some(vec![labels("media"), labels("media")]),
        ..Config::default()
    }
}
async fn inspect(fixture: &Fixture) -> Result<EngineComposeEvidence> {
    let inner = inspect_selected(
        &fixture.docker,
        fixture.policy,
        TimeSource::default(),
        Some(Admission::new(EngineReadProfile::CombinedV1)),
        true,
    )
    .await?;
    EngineComposeEvidence::from_storage(inner)
}
async fn timed(fixture: &Fixture, clock: TimeSource) -> Result<EngineComposeEvidence> {
    let inner = inspect_selected(
        &fixture.docker,
        fixture.policy,
        clock,
        Some(Admission::new(EngineReadProfile::CombinedV1)),
        true,
    )
    .await?;
    EngineComposeEvidence::from_storage(inner)
}

#[tokio::test]
async fn complete_replica_oneoff_stopped_mountless_and_non_compose_membership_is_retained() {
    let mut oneoff = labels("media");
    oneoff["com.docker.compose.oneoff"] = json!("True");
    let fixture = fixture(Config {
        count: 4,
        running_count: Some(2),
        mounts: 0,
        compose_labels: Some(vec![
            labels("media"),
            labels("transcoder"),
            oneoff,
            Value::Null,
        ]),
        ..config()
    })
    .await;
    let owner = inspect(&fixture).await.unwrap();
    let snapshot = owner.snapshot();
    assert_eq!(snapshot.containers.len(), 4);
    assert_eq!(owner.storage().running().len(), 2);
    assert_eq!(snapshot.engine_id, owner.storage().declarations().engine_id);
    assert_eq!(
        snapshot.containers_observed_at,
        owner.storage().declarations().observed_at
    );
    for (selected, original) in snapshot
        .containers
        .iter()
        .zip(&owner.storage().declarations().containers)
    {
        assert_eq!(selected.container, original.container);
        assert!(original.mounts.is_empty());
    }
    assert!(matches!(
        snapshot.containers[2].ownership,
        ComposeOwnership::Compose { one_off: true, .. }
    ));
    assert!(matches!(
        snapshot.containers[3].ownership,
        ComposeOwnership::NonCompose
    ));
    let body = serde_json::to_string(snapshot).unwrap();
    for secret in ["private-value", "SECRET", "credential", "TOKEN"] {
        assert!(!body.contains(secret));
    }
    owner.revalidate().await.unwrap();
    owner.storage().revalidate().await.unwrap();
    assert!(fixture.requests().iter().all(|r| r.starts_with("GET ")));
}
#[tokio::test]
async fn public_nonroot_refusal_precedes_canonical_endpoint_access() {
    assert!(!rustix::process::geteuid().is_root());
    assert!(matches!(
        inspect_compose_membership().await,
        Err(Error(ErrorCode::Forbidden))
    ));
}
#[tokio::test]
async fn root_policy_and_wrong_kernel_peer_refuse_before_http_bytes() {
    let fixture = fixture(config()).await;
    assert!(matches!(
        inspect_compose(&fixture.docker, EnginePeerPolicy::root()).await,
        Err(Error(ErrorCode::Forbidden))
    ));
    assert!(fixture.requests().is_empty());
    let wrong_peer = EnginePeerPolicy {
        peer_uid: fixture.policy.peer_uid + 1,
        ..fixture.policy
    };
    assert!(matches!(
        inspect_compose(&fixture.docker, wrong_peer).await,
        Err(Error(ErrorCode::Forbidden))
    ));
    assert!(fixture.requests().is_empty());
}
#[tokio::test]
async fn ambiguous_project_name_cannot_merge_directories_or_file_order() {
    for (key, text) in [
        ("com.docker.compose.project.working_dir", "/opt/other"),
        (
            "com.docker.compose.project.config_files",
            "/opt/stacks/media/override.yaml,/opt/stacks/media/compose.yaml",
        ),
    ] {
        let mut other = labels("second");
        other[key] = json!(text);
        let fixture = fixture(Config {
            compose_labels: Some(vec![labels("media"), other]),
            ..config()
        })
        .await;
        assert!(matches!(
            inspect(&fixture).await,
            Err(Error(ErrorCode::Unavailable))
        ));
    }
}
#[tokio::test]
async fn malformed_partial_metadata_refuses_while_old_storage_interfaces_stay_unchanged() {
    for malformed in [
        json!({"com.docker.compose.project":"media"}),
        json!({"secret":1}),
        json!([]),
    ] {
        let fixture = fixture(Config {
            compose_labels: Some(vec![malformed, Value::Null]),
            ..config()
        })
        .await;
        let old = fixture.inspect_combined().await.unwrap();
        assert!(old.compose_snapshot().is_none());
        assert!(old.snapshot.declarations.containers.len() == 2);
        assert!(
            !serde_json::to_string(old.snapshot())
                .unwrap()
                .contains("com.docker.compose")
        );
        assert!(matches!(
            inspect(&fixture).await,
            Err(Error(ErrorCode::Unavailable))
        ));
    }
}
#[tokio::test]
async fn selected_label_changes_between_initial_inspection_passes_refuse() {
    for (key, text) in [
        ("com.docker.compose.project", "other"),
        ("com.docker.compose.service", "other"),
        ("com.docker.compose.oneoff", "True"),
        ("com.docker.compose.project.working_dir", "/opt/other"),
        (
            "com.docker.compose.project.config_files",
            "/opt/stacks/media/override.yaml,/opt/stacks/media/compose.yaml",
        ),
    ] {
        let mut changed = labels("media");
        changed[key] = json!(text);
        let fixture = fixture(Config {
            compose_labels_after: Some(vec![changed, labels("media")]),
            ..config()
        })
        .await;
        assert!(matches!(
            inspect(&fixture).await,
            Err(Error(ErrorCode::Conflict))
        ));
    }
}
#[tokio::test]
async fn label_or_base_fact_changes_on_reacquisition_poison_every_owner_entry() {
    for change in [
        None,
        Some(Change::Pid),
        Some(Change::Mount),
        Some(Change::Snapshot),
        Some(Change::Membership),
        Some(Change::Engine),
    ] {
        let fixture = fixture(config()).await;
        let owner = inspect(&fixture).await.unwrap();
        fixture.set(|c| {
            c.changed = change;
            if change.is_none() {
                c.compose_labels.as_mut().unwrap()[0]["com.docker.compose.service"] =
                    json!("changed");
            }
        });
        assert!(matches!(
            owner.revalidate().await,
            Err(Error(ErrorCode::Conflict))
        ));
        fixture.set(|c| {
            c.changed = None;
            c.compose_labels = Some(vec![labels("media"), labels("media")]);
        });
        let before = fixture.requests().len();
        assert!(matches!(
            owner.revalidate().await,
            Err(Error(ErrorCode::Conflict))
        ));
        assert!(matches!(
            owner.storage().revalidate().await,
            Err(Error(ErrorCode::Conflict))
        ));
        assert_eq!(fixture.requests().len(), before);
    }
}
#[tokio::test]
async fn original_wall_and_monotonic_expiry_cannot_be_renewed_after_failure() {
    for at in [99, 106] {
        let fixture = fixture(config()).await;
        let (wall, time) = clock(100);
        let owner = timed(&fixture, time).await.unwrap();
        wall.store(at, Ordering::Relaxed);
        assert!(matches!(
            owner.revalidate().await,
            Err(Error(ErrorCode::Conflict))
        ));
        wall.store(100, Ordering::Relaxed);
        let before = fixture.requests().len();
        assert!(owner.storage().revalidate().await.is_err());
        assert_eq!(fixture.requests().len(), before);
    }
    let fixture = fixture(config()).await;
    let (_, time) = clock(100);
    let owner = timed(&fixture, time).await.unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(5)).await;
    assert!(matches!(
        owner.revalidate().await,
        Err(Error(ErrorCode::Conflict))
    ));
    tokio::time::resume();
}
#[tokio::test]
async fn dropped_inflight_validation_closes_its_connection_and_permanently_poisons() {
    let fixture = fixture(config()).await;
    let owner = inspect(&fixture).await.unwrap();
    fixture.set(|c| c.hold = true);
    let mut first = Box::pin(owner.revalidate());
    tokio::select! {result=&mut first=>panic!("unexpected completion: {result:?}"),_=fixture.held.notified()=>{}}
    drop(first);
    tokio::time::timeout(Duration::from_secs(1), fixture.eof.notified())
        .await
        .unwrap();
    fixture.set(|c| c.hold = false);
    let before = fixture.requests().len();
    assert!(matches!(
        owner.storage().revalidate().await,
        Err(Error(ErrorCode::Conflict))
    ));
    assert_eq!(fixture.requests().len(), before);
}
#[tokio::test]
async fn simultaneous_validation_refuses_both_results_without_restoring_admission() {
    let fixture = fixture(config()).await;
    let owner = inspect(&fixture).await.unwrap();
    let (first, second) = tokio::join!(owner.revalidate(), owner.revalidate());
    assert!(matches!(first, Err(Error(ErrorCode::Conflict))));
    assert!(matches!(second, Err(Error(ErrorCode::Conflict))));
    let before = fixture.requests().len();
    assert!(owner.storage().revalidate().await.is_err());
    assert_eq!(fixture.requests().len(), before);
}
#[tokio::test]
async fn complete_count_running_and_selected_mount_ceilings_are_unchanged() {
    let fixture = fixture(Config {
        count: 16,
        running_count: Some(8),
        mounts: 2,
        ..config()
    })
    .await;
    let owner = inspect(&fixture).await.unwrap();
    assert_eq!(owner.snapshot().containers.len(), 16);
    for config in [
        Config {
            count: 17,
            ..config()
        },
        Config {
            count: 16,
            running_count: Some(9),
            ..config()
        },
        Config {
            count: 16,
            running_count: Some(8),
            mounts: 3,
            ..config()
        },
    ] {
        let fixture = super::fixture(config).await;
        assert!(matches!(
            inspect(&fixture).await,
            Err(Error(ErrorCode::Unavailable))
        ));
    }
}
#[tokio::test]
async fn total_storage_and_compose_report_refuses_without_raising_the_original_ceiling() {
    let mut long = labels("media");
    long["com.docker.compose.project.working_dir"] = json!(format!("/{}", "d".repeat(511)));
    long["com.docker.compose.project.config_files"] = json!(
        (0..4)
            .map(|i| format!("/{}{i}", "f".repeat(510)))
            .collect::<Vec<_>>()
            .join(",")
    );
    let fixture = fixture(Config {
        count: 16,
        running_count: Some(8),
        mounts: 2,
        long_paths: true,
        compose_labels: Some(vec![long; 16]),
        ..config()
    })
    .await;
    assert!(fixture.inspect_combined().await.is_ok());
    assert!(matches!(
        inspect(&fixture).await,
        Err(Error(ErrorCode::Unavailable))
    ));
}
#[tokio::test]
async fn lifetime_http_allowance_never_renews_and_payload_returns_after_owner_drop() {
    let fixture = fixture(Config {
        count: 16,
        running_count: Some(8),
        mounts: 0,
        padding: 16 * 1024,
        ..config()
    })
    .await;
    let owner = inspect(&fixture).await.unwrap();
    let admission = owner.storage().admission.clone().unwrap();
    let mut successes = 0;
    loop {
        match owner.revalidate().await {
            Ok(()) => {
                successes += 1;
                assert!(successes <= 5);
            }
            Err(error) => {
                assert_eq!(error.0, ErrorCode::Unavailable);
                break;
            }
        }
    }
    assert!(successes >= 1);
    let before = fixture.requests().len();
    assert!(owner.storage().revalidate().await.is_err());
    assert_eq!(fixture.requests().len(), before);
    drop(owner);
    assert_eq!(admission.remaining_live(), 2 * 1024 * 1024);
}
