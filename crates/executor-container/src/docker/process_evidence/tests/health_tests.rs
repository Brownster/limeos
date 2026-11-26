use super::*;
use crate::docker::health_evidence::{EngineHealthEvidence, HealthStatus, RestartMode};
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
async fn inspect_with(f: &Fixture, profile: EngineReadProfile) -> Result<EngineHealthEvidence> {
    let inner = inspect_projection(
        &f.docker,
        f.policy,
        TimeSource::default(),
        Some(Admission::new(profile)),
        Projection::Health,
    )
    .await?;
    EngineHealthEvidence::from_storage(inner)
}
/// The production health read (`inspect_container_health`) against this
/// fixture's socket, under its own profile.
async fn inspect(f: &Fixture) -> Result<EngineHealthEvidence> {
    EngineHealthEvidence::from_storage(inspect_health(&f.docker, f.policy).await?)
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
async fn sixteen_seventeen_and_sixty_four_containers_of_any_state_are_read_whole() {
    // Stopped containers count toward the bound like running ones.
    for (count, running, mounts) in [(16, 9, 2), (17, 1, 0), (17, 17, 2), (64, 40, 4)] {
        let f = fixture(Config {
            count,
            running_count: Some(running),
            mounts,
            ..config()
        })
        .await;
        let owner = inspect(&f).await.unwrap();
        let snap = owner.snapshot();
        assert_eq!(snap.containers.len(), count, "{count}");
        assert_eq!(snap.truncated, 0, "{count}");
        assert!(snap.complete, "{count}");
        assert_eq!(owner.storage().running().len(), running, "{count}");
        assert_eq!(
            snap.containers.iter().filter(|c| !c.running).count(),
            count - running
        );
        owner.revalidate().await.unwrap();
    }
    // The health read uses no mounts, so the Engine-wide selected-mount
    // ceiling (256), which still bounds the stack inventory, does not refuse
    // it: 64 containers with 8 mounts each (512) answer.
    let f = fixture(Config {
        count: 64,
        running_count: Some(40),
        mounts: 8,
        ..config()
    })
    .await;
    let started = Instant::now();
    let owner = inspect(&f).await.unwrap();
    assert_eq!(owner.snapshot().containers.len(), 64);
    assert!(owner.snapshot().complete);
    assert!(
        owner
            .storage()
            .declarations()
            .containers
            .iter()
            .all(|c| c.mounts.is_empty())
    );
    owner.revalidate().await.unwrap();
    // What the reader's worker does: a read and its revalidation, four GETs
    // per container plus ten (`CONTAINER_HEALTH_ENGINE_BUDGET`'s cost).
    assert_eq!(f.requests().len(), 4 * 64 + 10);
    eprintln!(
        "64-container health read and revalidation against the fixture Engine: {:?}",
        started.elapsed()
    );
    // The storage evidence of the same Engine still refuses past the bound.
    assert!(matches!(
        f.docker
            .inspect_storage_processes_with_profile(f.policy, EngineReadProfile::EngineWideV1)
            .await,
        Err(Error(ErrorCode::Unavailable))
    ));
}

#[tokio::test]
async fn a_container_with_a_mount_shape_the_storage_reads_refuse_is_still_read() {
    // An image mount is unsupported storage evidence; health does not read it.
    let f = fixture(Config {
        count: 2,
        all_running: true,
        mount_type: Some((1, "image")),
        ..config()
    })
    .await;
    let owner = inspect(&f).await.unwrap();
    let snap = owner.snapshot();
    assert_eq!(snap.containers.len(), 2);
    assert!(snap.complete);
    assert!(
        snap.containers
            .iter()
            .all(|c| c.health == HealthStatus::Healthy)
    );
}

#[tokio::test]
async fn past_sixty_four_the_read_keeps_running_containers_and_counts_the_rest() {
    for (count, running) in [(65, 40), (65, 64), (200, 67)] {
        let f = fixture(Config {
            count,
            running_count: Some(running),
            ..config()
        })
        .await;
        let owner = inspect(&f).await.unwrap();
        let snap = owner.snapshot();
        assert_eq!(snap.containers.len(), 64, "{count}");
        assert_eq!(usize::from(snap.truncated), count - 64, "{count}");
        // Truncated containers are unknown, so the read is not complete,
        // though every member read has its signal.
        assert!(!snap.complete, "{count}");
        assert!(snap.containers.iter().all(|c| c.signal.is_some()));
        // Running containers first, then stopped ones by name (c000, ...).
        let mut expected: Vec<_> = (0..count)
            .filter(|i| *i < running)
            .chain((0..count).filter(|i| *i >= running))
            .take(64)
            .map(|i| format!("container:{i:064x}"))
            .collect();
        expected.sort();
        let read: Vec<_> = snap.containers.iter().map(|c| c.resource.clone()).collect();
        assert_eq!(read, expected, "{count}");
        assert_eq!(
            snap.containers.iter().filter(|c| c.running).count(),
            running.min(64)
        );
        owner.revalidate().await.unwrap();
    }
}

#[tokio::test]
async fn sixty_four_longest_named_checked_members_fit_the_joint_report() {
    // Every name at the Engine's 255-byte limit, every container checked, and
    // the Engine-wide mount share: the largest health read the profile admits.
    let mut selected = fields("always", check(), status("unhealthy"));
    selected["Name"] = json!(format!("/{}", "n".repeat(255)));
    let f = fixture(Config {
        count: 64,
        running_count: Some(64),
        mounts: 4,
        health_inspect: Some(selected),
        ..config()
    })
    .await;
    let owner = inspect(&f).await.unwrap();
    let snap = owner.snapshot();
    assert_eq!(snap.containers.len(), 64);
    assert!(snap.containers.iter().all(|c| {
        c.signal.as_ref().unwrap().status == SignalStatus::Failing && c.name.len() == 255
    }));
}

#[tokio::test]
async fn long_mount_paths_cannot_amplify_the_health_report() {
    // Mount paths once filled the joint storage-and-health report (48 KiB
    // under CombinedV1) and refused the read; the health read carries no
    // mounts, so even the smaller profile answers with every name at 255.
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
    let owner = inspect_with(&f, EngineReadProfile::CombinedV1)
        .await
        .unwrap();
    assert_eq!(owner.snapshot().containers.len(), 16);
}

#[tokio::test]
async fn production_root_policy_and_wrong_kernel_peer_refuse_before_any_http_request() {
    // The fixed public reader (`inspect_container_health`) reaches the host's
    // /run/docker.sock, so it is not called here; the same collection under
    // the root policy is, against this test's socket.
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

/// The Pi shape that rolled the 26 November migration back: 18 containers,
/// 14 running and 4 exited; three running with a passing check, two exited
/// whose check had failed, the rest unchecked; 54 mounts (the host has 51).
fn wybie() -> Config {
    let named = |index: usize, policy: &str| {
        json!({"Name": format!("/svc{index:02}"),
            "HostConfig": {"RestartPolicy": {"Name": policy}}})
    };
    let mut health = Vec::new();
    for index in 0..18 {
        let mut value = named(index, if index >= 16 { "no" } else { "unless-stopped" });
        if index < 3 {
            value["Config"] = json!({"Healthcheck": check()});
            value["State"] = json!({"Health": status("healthy")});
        } else if index == 14 || index == 15 {
            value["HostConfig"]["RestartPolicy"]["Name"] = json!("always");
            value["Config"] = json!({"Healthcheck": check()});
            value["State"] = json!({"Health": status("unhealthy")});
        }
        health.push((index, value));
    }
    Config {
        count: 18,
        running_count: Some(14),
        mounts: 3,
        health_inspect: Some(fields("unless-stopped", Value::Null, Value::Null)),
        health_by_index: health,
        ..config()
    }
}

#[tokio::test]
async fn a_wybie_shaped_engine_of_eighteen_answers_every_container() {
    let f = fixture(wybie()).await;
    let owner = inspect(&f).await.unwrap();
    let snap = owner.snapshot();
    assert_eq!(snap.containers.len(), 18);
    assert_eq!(snap.truncated, 0);
    assert!(snap.complete);
    assert_eq!(owner.storage().running().len(), 14);
    let by_index = |index: usize| {
        let resource = format!("container:{index:064x}");
        snap.containers
            .iter()
            .find(|c| c.resource == resource)
            .unwrap()
    };
    for index in 0..14 {
        let row = by_index(index);
        assert!(row.running, "{index}");
        assert_eq!(
            row.signal.as_ref().map(|s| s.status),
            Some(SignalStatus::Healthy),
            "{index}"
        );
    }
    assert_eq!(by_index(0).health, HealthStatus::Healthy);
    assert_eq!(by_index(3).health, HealthStatus::NotConfigured);
    for index in [14, 15] {
        let row = by_index(index);
        assert!(!row.running);
        assert_eq!(row.health, HealthStatus::Unhealthy);
        let signal = row.signal.as_ref().unwrap();
        assert_eq!(signal.status, SignalStatus::Failing);
        assert_eq!(signal.summary, format!("svc{index} is not running"));
    }
    for index in [16, 17] {
        // Stopped with restart `no`: listed, counted, and not alerted on.
        assert!(by_index(index).signal.is_none());
    }
    owner.revalidate().await.unwrap();
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
