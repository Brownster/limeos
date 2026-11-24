use super::*;
use crate::docker::health_evidence::{
    EngineHealthEvidence, HealthStatus, RestartMode, inspect_container_health,
};
use limeos_domain::automation::SignalStatus;

fn fields(policy: &str, check: Value, status: Value) -> Value {
    json!({"Name":"/plex","HostConfig":{"RestartPolicy":{"Name":policy}},
        "State":{"Paused":false,"Restarting":false,"Dead":false,"Health":status},
        "Config":{"Healthcheck":check}})
}
fn check() -> Value {
    json!({"Test":["CMD-SHELL","private-command --token=SECRET"]})
}
fn status(value: &str) -> Value {
    json!({"Status":value,"Log":[{"Output":"SECRET private health log"}]})
}
fn config() -> Config {
    Config {
        count: 1,
        mounts: 0,
        health_inspect: Some(fields("always", check(), status("healthy"))),
        ..Default::default()
    }
}
async fn inspect(f: &Fixture) -> Result<EngineHealthEvidence> {
    let inner = inspect_projection(
        &f.docker,
        f.policy,
        TimeSource::default(),
        Some(Admission::new(EngineReadProfile::CombinedV1)),
        Projection::Health,
    )
    .await?;
    EngineHealthEvidence::from_storage(inner)
}

#[tokio::test]
async fn actual_double_inspection_retains_stable_resources_and_sanitized_frozen_signals() {
    let f = fixture(config()).await;
    let owner = inspect(&f).await.unwrap();
    let snap = owner.snapshot();
    assert_eq!(snap.engine_id, owner.storage().declarations().engine_id);
    assert_eq!(
        snap.containers_observed_at,
        owner.storage().declarations().observed_at
    );
    assert!(snap.complete);
    let row = &snap.containers[0];
    assert_eq!(row.resource, format!("container:{:064x}", 0));
    assert_eq!(row.signal.as_ref().unwrap().key, row.resource);
    assert_eq!(row.signal.as_ref().unwrap().summary, "plex is healthy");
    assert_eq!(row.health, HealthStatus::Healthy);
    assert_eq!(row.restart, RestartMode::Always);
    let report = serde_json::to_string(snap).unwrap();
    for private in [
        "SECRET",
        "private-command",
        "private health log",
        "Env",
        "credential",
        "Test",
        "Log",
    ] {
        assert!(!report.contains(private));
    }
    assert_eq!(f.requests().len(), 7);
    owner.revalidate().await.unwrap();
    owner.storage().revalidate().await.unwrap();
    assert!(f.requests().iter().all(|line| line.starts_with("GET ")));
}

#[tokio::test]
async fn restart_policy_health_and_stopped_matrix_preserves_monitoring_predicate() {
    for (policy, running, check, status, expected, complete) in [
        (
            "always",
            true,
            check(),
            status("healthy"),
            Some(SignalStatus::Healthy),
            true,
        ),
        (
            "unless-stopped",
            true,
            check(),
            status("unhealthy"),
            Some(SignalStatus::Failing),
            true,
        ),
        (
            "always",
            true,
            check(),
            status("starting"),
            Some(SignalStatus::Healthy),
            true,
        ),
        (
            "always",
            false,
            check(),
            status("healthy"),
            Some(SignalStatus::Failing),
            true,
        ),
        (
            "always",
            false,
            Value::Null,
            Value::Null,
            Some(SignalStatus::Failing),
            true,
        ),
        (
            "always",
            true,
            Value::Null,
            Value::Null,
            Some(SignalStatus::Healthy),
            true,
        ),
        (
            "always",
            true,
            json!({"Test":["NONE"]}),
            Value::Null,
            Some(SignalStatus::Healthy),
            true,
        ),
        (
            "always",
            true,
            check(),
            Value::Null,
            Some(SignalStatus::Unavailable),
            false,
        ),
        (
            "always",
            true,
            json!({}),
            Value::Null,
            Some(SignalStatus::Unavailable),
            false,
        ),
        (
            "always",
            true,
            check(),
            status("future-value"),
            Some(SignalStatus::Unavailable),
            false,
        ),
        ("no", false, Value::Null, Value::Null, None, true),
        ("", false, Value::Null, Value::Null, None, true),
        (
            "on-failure",
            false,
            check(),
            status("unhealthy"),
            None,
            true,
        ),
    ] {
        let f = fixture(Config {
            first_running: running,
            pid: Some(json!(if running { 1000 } else { 0 })),
            health_inspect: Some(fields(policy, check, status)),
            ..config()
        })
        .await;
        let owner = inspect(&f).await.unwrap();
        assert_eq!(
            owner.snapshot().containers[0]
                .signal
                .as_ref()
                .map(|s| s.status),
            expected,
            "{policy}"
        );
        assert_eq!(owner.snapshot().complete, complete);
    }
}

#[tokio::test]
async fn paused_restarting_and_dead_flags_cannot_report_running_healthy() {
    for flag in ["Paused", "Restarting", "Dead"] {
        let mut selected = fields("always", check(), status("healthy"));
        selected["State"][flag] = json!(true);
        let f = fixture(Config {
            health_inspect: Some(selected),
            ..config()
        })
        .await;
        let owner = inspect(&f).await.unwrap();
        let row = &owner.snapshot().containers[0];
        assert!(!row.running);
        assert_eq!(row.signal.as_ref().unwrap().status, SignalStatus::Failing);
        assert_eq!(row.signal.as_ref().unwrap().summary, "plex is not running");
    }
}

#[tokio::test]
async fn malformed_or_contradictory_selected_health_refuses_but_old_selection_still_works() {
    let base = fields("always", check(), status("healthy"));
    let cases = [
        ("/HostConfig", json!(null)),
        ("/HostConfig/RestartPolicy", json!([])),
        (
            "/HostConfig/RestartPolicy/Name",
            json!("future-private-policy"),
        ),
        ("/HostConfig/RestartPolicy/Name", json!(null)),
        ("/Name", json!("relative")),
        ("/Name", json!("/bad/name")),
        ("/Name", json!(format!("/{}", "x".repeat(256)))),
        ("/State/Paused", json!(null)),
        ("/State/Restarting", json!("false")),
        ("/State/Dead", json!(0)),
        ("/State/Health", json!([])),
        ("/State/Health/Status", json!(false)),
        ("/Config/Healthcheck", json!(null)),
        ("/Config/Healthcheck/Test", json!(["NONE", "unexpected"])),
        ("/Config/Healthcheck/Test", json!([false])),
        ("/Config/Healthcheck/Test", json!(["future-private-kind"])),
        ("/Config/Healthcheck/Test", json!("secret")),
    ];
    for (pointer, value) in cases {
        let mut selected = base.clone();
        *selected.pointer_mut(pointer).unwrap() = value;
        let f = fixture(Config {
            health_inspect: Some(selected),
            ..config()
        })
        .await;
        assert!(
            f.inspect_combined().await.is_ok(),
            "legacy selection {pointer}"
        );
        // Per container: unavailable this round, never healthy and never gone.
        let owner = inspect(&f).await.unwrap();
        let snap = owner.snapshot();
        let row = &snap.containers[0];
        assert_eq!(row.health, HealthStatus::Unavailable, "{pointer}");
        assert_eq!(
            row.signal.as_ref().unwrap().status,
            SignalStatus::Unavailable,
            "{pointer}"
        );
        assert!(!snap.complete, "{pointer}");
    }
}

#[tokio::test]
async fn selected_second_pass_changes_refuse_without_promoting_a_partial_snapshot() {
    for (pointer, value) in [
        ("/Name", json!("/changed")),
        ("/HostConfig/RestartPolicy/Name", json!("no")),
        ("/State/Health/Status", json!("unhealthy")),
        ("/State/Paused", json!(true)),
    ] {
        let mut after = fields("always", check(), status("healthy"));
        *after.pointer_mut(pointer).unwrap() = value;
        let f = fixture(Config {
            health_inspect_after: Some(after),
            ..config()
        })
        .await;
        // The changing container is reported unavailable with its first-pass
        // name; neither pass's health is promoted.
        let owner = inspect(&f).await.unwrap();
        let snap = owner.snapshot();
        assert_eq!(
            snap.containers[0].health,
            HealthStatus::Unavailable,
            "{pointer}"
        );
        assert_eq!(
            snap.containers[0].signal.as_ref().unwrap().summary,
            "plex observation unavailable"
        );
        assert!(!snap.complete);
    }
}

#[tokio::test]
async fn one_health_flip_marks_only_that_container_unavailable() {
    let f = fixture(Config {
        count: 3,
        all_running: true,
        health_inspect: Some(fields("always", check(), status("starting"))),
        health_inspect_after: Some(fields("always", check(), status("healthy"))),
        health_after_only: Some(1),
        ..config()
    })
    .await;
    let owner = inspect(&f).await.unwrap();
    let snap = owner.snapshot();
    let health: Vec<_> = snap.containers.iter().map(|c| c.health).collect();
    assert_eq!(
        health,
        [
            HealthStatus::Starting,
            HealthStatus::Unavailable,
            HealthStatus::Starting
        ]
    );
    assert!(!snap.complete);
    // Revalidation for alerting tolerates it; the strict one does not.
    assert!(owner.revalidate_display().await.is_ok());
    assert!(matches!(
        owner.revalidate().await,
        Err(Error(ErrorCode::Conflict))
    ));
}

#[tokio::test]
async fn changed_selected_or_base_facts_permanently_poison_both_validation_entries() {
    for change in [
        None,
        Some(Change::Pid),
        Some(Change::Membership),
        Some(Change::Engine),
        Some(Change::Snapshot),
    ] {
        let f = fixture(config()).await;
        let owner = inspect(&f).await.unwrap();
        f.set(|c| {
            c.changed = change;
            if change.is_none() {
                c.health_inspect.as_mut().unwrap()["State"]["Health"]["Status"] =
                    json!("unhealthy");
            }
        });
        assert!(matches!(
            owner.storage().revalidate().await,
            Err(Error(ErrorCode::Conflict))
        ));
        f.set(|c| {
            c.changed = None;
            c.health_inspect = config().health_inspect;
        });
        let before = f.requests().len();
        assert!(matches!(
            owner.revalidate().await,
            Err(Error(ErrorCode::Conflict))
        ));
        assert_eq!(f.requests().len(), before);
    }
}

#[tokio::test]
async fn dropped_or_concurrent_validation_poison_cannot_be_bypassed_through_storage() {
    let f = fixture(config()).await;
    let owner = inspect(&f).await.unwrap();
    f.set(|c| c.hold = true);
    let mut future = Box::pin(owner.revalidate());
    tokio::select! {result=&mut future=>panic!("unexpected completion: {result:?}"),_=f.held.notified()=>{}}
    drop(future);
    tokio::time::timeout(Duration::from_secs(1), f.eof.notified())
        .await
        .unwrap();
    f.set(|c| c.hold = false);
    let before = f.requests().len();
    assert!(matches!(
        owner.storage().revalidate().await,
        Err(Error(ErrorCode::Conflict))
    ));
    assert_eq!(f.requests().len(), before);
    let f = fixture(config()).await;
    let owner = inspect(&f).await.unwrap();
    let (a, b) = tokio::join!(owner.revalidate(), owner.storage().revalidate());
    assert!(matches!(a, Err(Error(ErrorCode::Conflict))));
    assert!(matches!(b, Err(Error(ErrorCode::Conflict))));
}

#[tokio::test]
async fn closed_membership_running_and_mount_limits_are_not_raised() {
    let f = fixture(Config {
        count: 16,
        running_count: Some(8),
        mounts: 2,
        ..config()
    })
    .await;
    let owner = inspect(&f).await.unwrap();
    assert_eq!(owner.snapshot().containers.len(), 16);
    assert_eq!(owner.storage().running().len(), 8);
    assert!(owner.snapshot().containers[8..].iter().all(|c| !c.running));
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
            mounts: 3,
            running_count: Some(8),
            ..config()
        },
    ] {
        let f = fixture(config).await;
        assert!(matches!(
            inspect(&f).await,
            Err(Error(ErrorCode::Unavailable))
        ));
    }
}

#[tokio::test]
async fn joint_report_limit_refuses_amplification_without_clipping_the_inventory() {
    let mut selected = fields("always", check(), status("healthy"));
    selected["Name"] = json!(format!("/{}", "n".repeat(255)));
    let f = fixture(Config {
        count: 16,
        running_count: Some(8),
        mounts: 2,
        long_paths: true,
        health_inspect: Some(selected),
        ..config()
    })
    .await;
    assert!(f.inspect_combined().await.is_ok());
    assert!(matches!(
        inspect(&f).await,
        Err(Error(ErrorCode::Unavailable))
    ));
}

#[tokio::test]
async fn production_nonroot_and_wrong_kernel_peer_refuse_before_any_http_request() {
    assert!(!crate::full_read_access());
    assert!(matches!(
        inspect_container_health().await,
        Err(Error(ErrorCode::Forbidden))
    ));
    let f = fixture(config()).await;
    assert!(matches!(
        inspect_health(&f.docker, EnginePeerPolicy::root()).await,
        Err(Error(ErrorCode::Forbidden))
    ));
    let wrong = EnginePeerPolicy {
        peer_uid: f.policy.peer_uid + 1,
        ..f.policy
    };
    assert!(matches!(
        inspect_health(&f.docker, wrong).await,
        Err(Error(ErrorCode::Forbidden))
    ));
    assert!(f.requests().is_empty());
}

#[tokio::test]
async fn original_wall_and_monotonic_lifetimes_survive_fresh_comparison() {
    for replacement in [99, 106] {
        let f = fixture(config()).await;
        let (wall, time) = clock(100);
        let inner = inspect_projection(
            &f.docker,
            f.policy,
            time,
            Some(Admission::new(EngineReadProfile::CombinedV1)),
            Projection::Health,
        )
        .await
        .unwrap();
        let owner = EngineHealthEvidence::from_storage(inner).unwrap();
        owner.revalidate().await.unwrap();
        assert_eq!(owner.snapshot().containers_observed_at, 100);
        wall.store(replacement, Ordering::Relaxed);
        assert!(matches!(
            owner.revalidate().await,
            Err(Error(ErrorCode::Conflict))
        ));
    }
    let f = fixture(config()).await;
    let (_, time) = clock(100);
    let inner = inspect_projection(
        &f.docker,
        f.policy,
        time,
        Some(Admission::new(EngineReadProfile::CombinedV1)),
        Projection::Health,
    )
    .await
    .unwrap();
    let owner = EngineHealthEvidence::from_storage(inner).unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(5)).await;
    assert!(matches!(
        owner.revalidate().await,
        Err(Error(ErrorCode::Conflict))
    ));
    tokio::time::resume();
}

#[tokio::test]
async fn health_read_keeps_the_combined_profile_so_eighteen_containers_refuse() {
    // Deliberate: only the alert/health-policy automation reads this projection,
    // behind `allow_container_health_read` (off by default, outside the basic
    // cutover). An Engine past 16 containers or 8 running fails closed here;
    // stack and container reads use EngineWideV1 or Engine-direct calls instead.
    let f = fixture(Config {
        count: 18,
        running_count: Some(14),
        mounts: 0,
        ..config()
    })
    .await;
    assert!(matches!(
        inspect(&f).await,
        Err(Error(ErrorCode::Unavailable))
    ));
}
#[tokio::test]
async fn a_container_removed_during_the_read_is_reported_gone_not_unavailable() {
    // Two `restart: always` containers; the second is removed (`--rm`) before
    // its first or its second inspection. The first stays unknown-policy-safe.
    for from in [1, 2] {
        let f = fixture(Config {
            count: 2,
            all_running: true,
            vanishing: Some((1, from)),
            ..config()
        })
        .await;
        let owner = inspect(&f).await.unwrap();
        let snap = owner.snapshot();
        let gone = format!("container:{:064x}", 1);
        assert!(
            snap.containers.iter().all(|row| row.resource != gone),
            "{from}"
        );
        assert_eq!(snap.containers.len(), 1);
        assert!(snap.complete, "{from}");
        assert_eq!(snap.containers[0].health, HealthStatus::Healthy);
    }
}
