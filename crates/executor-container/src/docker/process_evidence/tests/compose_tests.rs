use super::*;
use crate::docker::compose_evidence::{
    ComposeOwnership, EngineComposeEvidence, inspect_compose_membership,
};
use limeos_domain::deployment::discovery::{
    CatalogFacts, Declaration, DiscoveryCohort, FileStatus, OmissionReason, StackIssue,
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
async fn inspect_profile_for(
    fixture: &Fixture,
    profile: EngineReadProfile,
) -> Result<EngineComposeEvidence> {
    let inner = inspect_selected(
        &fixture.docker,
        fixture.policy,
        TimeSource::default(),
        Some(Admission::new(profile)),
        true,
    )
    .await?;
    EngineComposeEvidence::from_storage(inner)
}
/// The production stack-inventory profile.
async fn inspect(fixture: &Fixture) -> Result<EngineComposeEvidence> {
    inspect_profile_for(fixture, EngineReadProfile::EngineWideV1).await
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
async fn an_engine_over_the_bound_keeps_running_containers_first_and_counts_the_rest() {
    for (count, every) in [(65, 5), (200, 3)] {
        let mut members = vec![Value::Null; count];
        for slot in &mut members[60..65] {
            *slot = labels("media");
        }
        let fixture = fixture(Config {
            count,
            running_every: Some(every),
            compose_labels: Some(members),
            ..config()
        })
        .await;
        let owner = inspect(&fixture).await.unwrap();
        let snapshot = owner.snapshot();
        assert_eq!(snapshot.containers.len(), 64, "{count}");
        assert_eq!(usize::from(snapshot.truncated), count - 64);
        let running = (0..count).filter(|i| i % every == 0).count();
        assert_eq!(
            snapshot
                .containers
                .iter()
                .filter(|c| c.container.running)
                .count(),
            running.min(64)
        );
        // Stopped containers fill the rest by name: c001, c002, ...
        let mut expected: Vec<_> = (0..count)
            .filter(|i| i % every == 0)
            .chain((0..count).filter(|i| i % every != 0))
            .take(64)
            .map(|i| format!("container:{i:064x}"))
            .collect();
        expected.sort();
        let shown: Vec<_> = snapshot
            .containers
            .iter()
            .map(|c| c.container.resource.clone())
            .collect();
        assert_eq!(shown, expected);
        owner.revalidate().await.unwrap();
        let facts = crate::deployment::discovery::cohort(None, Some(&owner), &|| false)
            .await
            .unwrap();
        assert_eq!(
            facts.engine().unwrap().truncated,
            u16::try_from(count - 64).unwrap()
        );
        let view = facts.project(snapshot.containers_observed_at).unwrap();
        assert_eq!(
            view.truncated_containers,
            Some(u16::try_from(count - 64).unwrap())
        );
        assert_eq!(
            view.engine.state,
            limeos_domain::deployment::discovery::SourceState::Fresh
        );
    }
}
#[tokio::test]
async fn storage_evidence_over_the_bound_still_refuses_whole() {
    let fixture = fixture(Config {
        count: 65,
        running_every: Some(5),
        ..Config::default()
    })
    .await;
    assert!(
        fixture
            .docker
            .inspect_storage_processes_with_profile(fixture.policy, EngineReadProfile::EngineWideV1)
            .await
            .is_err()
    );
    assert!(fixture.inspect_combined().await.is_err());
}
#[tokio::test]
async fn public_nonroot_refusal_precedes_canonical_endpoint_access() {
    assert!(!crate::full_read_access());
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
        // Never merged: the whole project is left out as unsupported.
        let owner = inspect(&fixture).await.unwrap();
        let snapshot = owner.snapshot();
        assert!(snapshot.containers.is_empty());
        assert_eq!(snapshot.omitted.len(), 2);
        assert!(
            snapshot
                .omitted
                .iter()
                .all(|o| o.project.as_deref() == Some("media")
                    && o.reason == OmissionReason::Unsupported)
        );
        assert!(!snapshot.complete());
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
            compose_labels: Some(vec![malformed.clone(), Value::Null]),
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
        // The odd container is left out with a reason; the other is kept.
        let owner = inspect(&fixture).await.unwrap();
        let snapshot = owner.snapshot();
        assert_eq!(snapshot.containers.len(), 1);
        assert_eq!(
            snapshot.containers[0].container.resource,
            format!("container:{:064x}", 1)
        );
        assert_eq!(
            snapshot.omitted[0].resource,
            format!("container:{:064x}", 0)
        );
        assert_eq!(snapshot.omitted[0].reason, OmissionReason::Unsupported);
        assert_eq!(
            snapshot.omitted[0].project.as_deref(),
            malformed["com.docker.compose.project"].as_str()
        );
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
        // Per container: the changed one is reported changing, never promoted
        // with either pass's labels; the stable one is returned.
        let owner = inspect(&fixture).await.unwrap();
        let snapshot = owner.snapshot();
        assert_eq!(snapshot.containers.len(), 1);
        assert_eq!(
            snapshot.containers[0].container.resource,
            format!("container:{:064x}", 1)
        );
        assert_eq!(snapshot.omitted.len(), 1);
        assert_eq!(snapshot.omitted[0].reason, OmissionReason::Changing);
        assert_eq!(snapshot.omitted[0].project.as_deref(), Some("media"));
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
async fn combined_profile_count_running_and_selected_mount_ceilings_are_unchanged() {
    let fixture = fixture(Config {
        count: 16,
        running_count: Some(8),
        mounts: 2,
        ..config()
    })
    .await;
    let owner = inspect_profile_for(&fixture, EngineReadProfile::CombinedV1)
        .await
        .unwrap();
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
            inspect_profile_for(&fixture, EngineReadProfile::CombinedV1).await,
            Err(Error(ErrorCode::Unavailable))
        ));
    }
}
#[tokio::test]
async fn total_storage_and_compose_report_refuses_without_raising_the_original_ceiling() {
    let mut long = labels("media");
    // Every path at 512 bytes, in segments within the 255-byte limit.
    long["com.docker.compose.project.working_dir"] = json!(format!("/{0}/{0}", "d".repeat(255)));
    long["com.docker.compose.project.config_files"] = json!(
        (0..4)
            .map(|i| format!("/{}/{}{i}", "f".repeat(255), "f".repeat(254)))
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
        inspect_profile_for(&fixture, EngineReadProfile::CombinedV1).await,
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
    let owner = inspect_profile_for(&fixture, EngineReadProfile::CombinedV1)
        .await
        .unwrap();
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

#[tokio::test]
async fn engine_wide_profile_admits_exactly_sixty_four_containers_of_any_state() {
    // 64 containers, 40 running (more than CombinedV1's 8), mounts at the
    // 4-per-container share of the 256 Engine-wide selected-mount ceiling.
    let fixture = fixture(Config {
        count: 64,
        running_count: Some(40),
        mounts: 4,
        ..config()
    })
    .await;
    let owner = inspect(&fixture).await.unwrap();
    assert_eq!(owner.snapshot().containers.len(), 64);
    assert_eq!(owner.storage().running().len(), 40);
    owner.revalidate().await.unwrap();
    assert_eq!(owner.snapshot().truncated, 0);
    // One more is truncated, not refused; the mount ceiling still refuses.
    let over = super::fixture(Config {
        count: 65,
        ..config()
    })
    .await;
    assert_eq!(inspect(&over).await.unwrap().snapshot().truncated, 1);
    let fixture = super::fixture(Config {
        count: 64,
        mounts: 5,
        ..config()
    })
    .await;
    assert!(matches!(
        inspect(&fixture).await,
        Err(Error(ErrorCode::Unavailable))
    ));
    // The storage-dependency profile keeps its own, smaller ceilings.
    let narrow = super::fixture(Config {
        count: 17,
        ..config()
    })
    .await;
    assert!(
        inspect_profile_for(&narrow, EngineReadProfile::CombinedV1)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn engine_wide_profile_admits_the_wybie_shape_of_eighteen_containers() {
    // 18 containers, 14 running, 4 stopped: past CombinedV1's 16 and 8.
    let fixture = fixture(Config {
        count: 18,
        running_count: Some(14),
        mounts: 2,
        ..config()
    })
    .await;
    let owner = inspect(&fixture).await.unwrap();
    assert_eq!(owner.snapshot().containers.len(), 18);
    assert_eq!(owner.storage().running().len(), 14);
    owner.revalidate().await.unwrap();
    assert!(
        inspect_profile_for(&fixture, EngineReadProfile::CombinedV1)
            .await
            .is_err()
    );
}

fn stack_labels(project: &str, service: &str) -> Value {
    json!({"com.docker.compose.project":project,"com.docker.compose.service":service,
        "com.docker.compose.project.working_dir":format!("/opt/stacks/{project}"),
        "com.docker.compose.project.config_files":format!("/opt/stacks/{project}/compose.yaml"),
        "com.docker.compose.oneoff":"False"})
}
fn registered(projects: &[&str]) -> CatalogFacts {
    CatalogFacts {
        engine_id: "fixture-engine".into(),
        sampled_at: 0,
        stacks: projects
            .iter()
            .map(|p| Declaration {
                stack: (*p).into(),
                project: (*p).into(),
                working_directory: format!("/opt/stacks/{p}"),
                config_files: vec![format!("/opt/stacks/{p}/compose.yaml")],
                services: vec!["app".into(), "db".into()],
                files: FileStatus::NotInspected,
            })
            .collect(),
    }
}
/// The stack inventory a reader projects from this collection.
async fn stacks(
    owner: &EngineComposeEvidence,
    projects: &[&str],
) -> limeos_domain::deployment::discovery::StackInventory {
    let facts = crate::deployment::discovery::cohort(None, Some(owner), &|| false)
        .await
        .unwrap();
    let engine = facts.engine().unwrap().clone();
    let mut catalog = registered(projects);
    catalog.sampled_at = engine.sampled_at;
    DiscoveryCohort::new(Some(catalog), Some(engine))
        .unwrap()
        .project(owner.snapshot().containers_observed_at)
        .unwrap()
}
#[tokio::test]
async fn one_crash_looping_container_is_changing_and_the_other_stacks_are_returned() {
    // media: 0, 1; tools: 2 (restart: always, crash looping), 3; 4 standalone.
    let fixture = fixture(Config {
        count: 5,
        all_running: true,
        crash_looping: Some(2),
        compose_labels: Some(vec![
            stack_labels("media", "app"),
            stack_labels("media", "db"),
            stack_labels("tools", "app"),
            stack_labels("tools", "db"),
            Value::Null,
        ]),
        ..config()
    })
    .await;
    let owner = inspect(&fixture).await.unwrap();
    let snapshot = owner.snapshot();
    assert_eq!(snapshot.containers.len(), 4);
    assert_eq!(snapshot.omitted.len(), 1);
    assert_eq!(
        snapshot.omitted[0].resource,
        format!("container:{:064x}", 2)
    );
    assert_eq!(snapshot.omitted[0].reason, OmissionReason::Changing);
    assert_eq!(snapshot.omitted[0].project.as_deref(), Some("tools"));
    // Excluded from the storage facts too: nothing unstable is retained.
    assert!(
        owner
            .storage()
            .declarations()
            .containers
            .iter()
            .all(|c| c.container.resource != snapshot.omitted[0].resource)
    );
    let view = stacks(&owner, &["media", "tools"]).await;
    assert_eq!(
        view.engine.state,
        limeos_domain::deployment::discovery::SourceState::Fresh
    );
    let media = view.stacks.iter().find(|s| s.project == "media").unwrap();
    assert!(media.issues.is_empty());
    assert_eq!(media.counts.unwrap().running, 2);
    let tools = view.stacks.iter().find(|s| s.project == "tools").unwrap();
    // Its stack is incomplete: shown with the reason, never planned.
    assert_eq!(tools.issues, [StackIssue::OmittedMembers]);
    assert_eq!(view.omitted_containers.len(), 1);
    assert_eq!(view.standalone_containers, Some(1));
}
#[tokio::test]
async fn a_member_changing_after_collection_is_shown_changing_but_strict_revalidation_refuses() {
    let fixture = fixture(Config {
        count: 3,
        all_running: true,
        compose_labels: Some(vec![
            stack_labels("media", "app"),
            stack_labels("media", "db"),
            stack_labels("tools", "app"),
        ]),
        ..config()
    })
    .await;
    let owner = inspect(&fixture).await.unwrap();
    assert!(owner.snapshot().complete());
    fixture.set(|c| {
        c.compose_labels.as_mut().unwrap()[1]["com.docker.compose.service"] = json!("other");
    });
    let changed = owner.revalidate_display().await.unwrap();
    assert_eq!(changed.changed, [format!("container:{:064x}", 1)]);
    let view = stacks(&owner, &["media", "tools"]).await;
    let media = view.stacks.iter().find(|s| s.project == "media").unwrap();
    assert_eq!(media.issues, [StackIssue::OmittedMembers]);
    assert!(
        view.stacks
            .iter()
            .find(|s| s.project == "tools")
            .unwrap()
            .issues
            .is_empty()
    );
    // Evidence for an effect stays strict.
    let strict = inspect(&fixture).await.unwrap();
    fixture.set(|c| {
        c.compose_labels.as_mut().unwrap()[1]["com.docker.compose.service"] = json!("third");
    });
    assert!(matches!(
        strict.revalidate().await,
        Err(Error(ErrorCode::Conflict))
    ));
}
#[tokio::test]
async fn an_image_mount_and_a_five_file_label_mark_only_their_containers_unsupported() {
    let mut five = stack_labels("tools", "db");
    five["com.docker.compose.project.config_files"] = json!(
        (1..=5)
            .map(|i| format!("/opt/stacks/tools/{i}.yaml"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let fixture = fixture(Config {
        count: 5,
        all_running: true,
        mount_type: Some((1, "image")),
        compose_labels: Some(vec![
            stack_labels("media", "app"),
            stack_labels("media", "db"),
            stack_labels("tools", "app"),
            five,
            stack_labels("other", "app"),
        ]),
        ..config()
    })
    .await;
    let owner = inspect(&fixture).await.unwrap();
    let snapshot = owner.snapshot();
    assert_eq!(snapshot.containers.len(), 3);
    let omitted: Vec<_> = snapshot
        .omitted
        .iter()
        .map(|o| (o.resource.clone(), o.project.clone(), o.reason))
        .collect();
    assert_eq!(
        omitted,
        [
            (
                format!("container:{:064x}", 1),
                Some("media".into()),
                OmissionReason::Unsupported
            ),
            (
                format!("container:{:064x}", 3),
                Some("tools".into()),
                OmissionReason::Unsupported
            ),
        ]
    );
    let view = stacks(&owner, &["media", "tools", "other"]).await;
    let issues = |p: &str| {
        view.stacks
            .iter()
            .find(|s| s.project == p)
            .unwrap()
            .issues
            .clone()
    };
    // The five-file label decodes neither inspected nor listed: that container
    // could overlap any stack, so every stack is blocked, and still shown.
    assert_eq!(snapshot.unattributed, 1);
    assert_eq!(view.unattributed_containers, Some(1));
    assert_eq!(
        issues("media"),
        [StackIssue::OmittedMembers, StackIssue::IncompleteInventory]
    );
    assert_eq!(
        issues("tools"),
        [StackIssue::OmittedMembers, StackIssue::IncompleteInventory]
    );
    assert_eq!(issues("other"), [StackIssue::IncompleteInventory]);
    // Storage evidence refuses the unknown mount type whole, as before.
    assert!(fixture.inspect().await.is_err());
    assert!(fixture.docker.storage_inventory().await.is_err());
}

fn issues_of(
    view: &limeos_domain::deployment::discovery::StackInventory,
    p: &str,
) -> Vec<StackIssue> {
    view.stacks
        .iter()
        .find(|s| s.project == p)
        .unwrap()
        .issues
        .clone()
}
/// 65 containers: the first 64 run, the 65th is stopped and so truncated.
fn crowded(labels: Vec<(usize, Value)>) -> Config {
    let mut all = vec![Value::Null; 65];
    for (at, value) in labels {
        all[at] = value;
    }
    Config {
        count: 65,
        running_count: Some(64),
        mounts: 0,
        compose_labels: Some(all),
        ..config()
    }
}
#[tokio::test]
async fn a_hidden_project_sharing_the_stack_directory_blocks_the_stack() {
    let mut other = stack_labels("other", "app");
    other["com.docker.compose.project.working_dir"] = json!("/opt/stacks/media");
    // Hidden by truncation, and hidden as an unsupported container.
    let truncated = crowded(vec![
        (0, stack_labels("media", "app")),
        (1, stack_labels("media", "db")),
        (64, other.clone()),
    ]);
    let unsupported = Config {
        count: 3,
        all_running: true,
        mount_type: Some((2, "image")),
        compose_labels: Some(vec![
            stack_labels("media", "app"),
            stack_labels("media", "db"),
            other,
        ]),
        ..config()
    };
    for config in [truncated, unsupported] {
        let fixture = fixture(config).await;
        let owner = inspect(&fixture).await.unwrap();
        let snapshot = owner.snapshot();
        assert!(
            snapshot
                .containers
                .iter()
                .all(|m| !matches!(&m.ownership, ComposeOwnership::Compose { project, .. } if project.name == "other"))
        );
        assert_eq!(snapshot.hidden_projects.len(), 1);
        let view = stacks(&owner, &["media"]).await;
        // As if the Engine had been read whole: the overlap refuses planning.
        assert!(issues_of(&view, "media").contains(&StackIssue::OverlappingProject));
        assert_eq!(
            view.stacks[0].runtime,
            limeos_domain::deployment::discovery::RuntimeStatus::Conflict
        );
    }
}
#[tokio::test]
async fn a_truncated_member_is_attributed_to_its_stack() {
    let fixture = fixture(crowded(vec![
        (0, stack_labels("media", "app")),
        (64, stack_labels("media", "db")),
        (1, stack_labels("tools", "app")),
    ]))
    .await;
    let owner = inspect(&fixture).await.unwrap();
    let snapshot = owner.snapshot();
    assert_eq!(snapshot.truncated, 1);
    assert_eq!(snapshot.unattributed, 0);
    let view = stacks(&owner, &["media", "tools"]).await;
    assert_eq!(issues_of(&view, "media"), [StackIssue::OmittedMembers]);
    assert!(issues_of(&view, "tools").is_empty());
}
#[tokio::test]
async fn an_undecodable_listed_row_blocks_every_stack_but_the_read_still_returns_them() {
    for labels in [
        json!({"com.docker.compose.project":"media","com.docker.compose.project.working_dir":"relative",
            "com.docker.compose.project.config_files":"/opt/stacks/media/compose.yaml"}),
        json!({"com.docker.compose.project":"media"}),
        json!({"com.docker.compose.project":7}),
        json!({"unrelated":["not","text"]}),
        json!(["not", "a", "map"]),
    ] {
        let mut config = crowded(vec![
            (0, stack_labels("media", "app")),
            (1, stack_labels("media", "db")),
            (2, stack_labels("tools", "app")),
        ]);
        config.list_labels = Some(vec![(64, labels.clone())]);
        let fixture = fixture(config).await;
        let owner = inspect(&fixture).await.unwrap();
        assert_eq!(owner.snapshot().unattributed, 1, "{labels}");
        let view = stacks(&owner, &["media", "tools"]).await;
        assert_eq!(view.unattributed_containers, Some(1));
        assert_eq!(view.stacks.len(), 2);
        for project in ["media", "tools"] {
            assert_eq!(issues_of(&view, project), [StackIssue::IncompleteInventory]);
            let stack = view.stacks.iter().find(|s| s.project == project).unwrap();
            assert!(stack.counts.is_some(), "the known members are still shown");
        }
    }
}
#[tokio::test]
async fn a_container_removed_between_passes_is_changing_and_the_stacks_are_returned() {
    let mut run = stack_labels("media", "app");
    run["com.docker.compose.oneoff"] = json!("True");
    // media: 0, 1; tools: 2, 3; 4 is a `--rm` container, standalone or a media
    // one-off, removed before its second inspection or before its first.
    for (labels, from, media) in [
        (Value::Null, 2, Vec::new()),
        (Value::Null, 1, Vec::new()),
        (run.clone(), 2, vec![StackIssue::OmittedMembers]),
        (run, 1, vec![StackIssue::OmittedMembers]),
    ] {
        let config = Config {
            count: 5,
            all_running: true,
            mounts: 0,
            vanishing: Some((4, from)),
            compose_labels: Some(vec![
                stack_labels("media", "app"),
                stack_labels("media", "db"),
                stack_labels("tools", "app"),
                stack_labels("tools", "db"),
                labels,
            ]),
            ..config()
        };
        let engine = fixture(config.clone()).await;
        let owner = inspect(&engine).await.unwrap();
        let snapshot = owner.snapshot();
        assert_eq!(snapshot.containers.len(), 4, "{from}");
        assert_eq!(snapshot.omitted.len(), 1);
        assert_eq!(
            snapshot.omitted[0].resource,
            format!("container:{:064x}", 4)
        );
        assert_eq!(snapshot.omitted[0].reason, OmissionReason::Changing);
        assert_eq!(snapshot.unattributed, 0);
        let view = stacks(&owner, &["media", "tools"]).await;
        assert_eq!(issues_of(&view, "media"), media, "{from}");
        assert!(issues_of(&view, "tools").is_empty());
        // Evidence for an effect still refuses a container that disappeared.
        let strict = fixture(config).await;
        assert!(strict.inspect().await.is_err());
    }
}
#[tokio::test]
async fn a_project_that_appears_after_collection_blocks_the_stack_it_overlaps() {
    let mut other = stack_labels("other", "app");
    other["com.docker.compose.project.working_dir"] = json!("/opt/stacks/media");
    let mut undecodable = stack_labels("other", "app");
    undecodable["com.docker.compose.project.working_dir"] = json!("relative");
    // A container of an overlapping project starts between the original read
    // and its closing revalidation: shown, inspected, or only listed (beyond
    // the bound), or with labels that decode to no project at all.
    for (appearing, crowd, issue) in [
        (other.clone(), false, StackIssue::OverlappingProject),
        (other, true, StackIssue::OverlappingProject),
        (undecodable, false, StackIssue::IncompleteInventory),
    ] {
        let config = if crowd {
            crowded(vec![
                (0, stack_labels("media", "app")),
                (1, stack_labels("media", "db")),
            ])
        } else {
            Config {
                count: 2,
                all_running: true,
                mounts: 0,
                compose_labels: Some(vec![
                    stack_labels("media", "app"),
                    stack_labels("media", "db"),
                ]),
                ..config()
            }
        };
        let fixture = fixture(config).await;
        let owner = inspect(&fixture).await.unwrap();
        assert!(owner.snapshot().hidden_projects.is_empty());
        assert!(issues_of(&stacks(&owner, &["media"]).await, "media").is_empty());
        fixture.set(|c| {
            // Beyond 64, a stopped 66th container: listed and truncated only.
            c.compose_labels.as_mut().unwrap().push(appearing.clone());
            c.count += 1;
        });
        let view = stacks(&owner, &["media"]).await;
        assert!(issues_of(&view, "media").contains(&issue), "{appearing}");
        if issue == StackIssue::OverlappingProject {
            assert_eq!(
                view.stacks[0].runtime,
                limeos_domain::deployment::discovery::RuntimeStatus::Conflict
            );
        }
    }
}
#[tokio::test]
async fn a_path_segment_the_stack_model_refuses_degrades_one_container_not_the_inventory() {
    let long = format!("/opt/{}", "x".repeat(256));
    let mut other = stack_labels("other", "app");
    other["com.docker.compose.project.working_dir"] = json!(long);
    let mut files = stack_labels("other", "app");
    files["com.docker.compose.project.config_files"] = json!(format!("{long}/compose.yaml"));
    for labels in [other, files] {
        // Hidden by truncation, and inspected.
        let truncated = crowded(vec![
            (0, stack_labels("media", "app")),
            (1, stack_labels("media", "db")),
            (64, labels.clone()),
        ]);
        let inspected = Config {
            count: 3,
            all_running: true,
            mounts: 0,
            compose_labels: Some(vec![
                stack_labels("media", "app"),
                stack_labels("media", "db"),
                labels.clone(),
            ]),
            ..config()
        };
        for config in [truncated, inspected] {
            let fixture = fixture(config).await;
            let owner = inspect(&fixture).await.unwrap();
            assert_eq!(owner.snapshot().unattributed, 1, "{labels}");
            assert!(owner.snapshot().hidden_projects.is_empty());
            let view = stacks(&owner, &["media"]).await;
            assert_eq!(view.unattributed_containers, Some(1));
            assert_eq!(issues_of(&view, "media"), [StackIssue::IncompleteInventory]);
            assert_eq!(view.stacks[0].counts.unwrap().running, 2);
        }
    }
}
#[tokio::test]
async fn a_kept_container_removed_over_the_bound_is_changing_and_the_read_still_returns() {
    // 65 containers; a kept one is removed before its second inspection, so the
    // closing listing has 64 and the stopped 65th would now be kept.
    let mut config = crowded(vec![
        (0, stack_labels("media", "app")),
        (1, stack_labels("media", "db")),
        (64, stack_labels("tools", "app")),
    ]);
    config.vanishing = Some((5, 2));
    let engine = fixture(config).await;
    let owner = inspect(&engine).await.unwrap();
    let snapshot = owner.snapshot();
    assert_eq!(snapshot.containers.len(), 63);
    assert_eq!(
        snapshot
            .omitted
            .iter()
            .map(|o| (o.resource.clone(), o.reason))
            .collect::<Vec<_>>(),
        [(format!("container:{:064x}", 5), OmissionReason::Changing)]
    );
    // The 65th was never inspected: still truncated, its project still hidden.
    assert_eq!(snapshot.truncated, 1);
    assert_eq!(snapshot.hidden_projects.len(), 1);
    let view = stacks(&owner, &["media", "tools"]).await;
    assert!(issues_of(&view, "media").is_empty());
    assert_eq!(issues_of(&view, "tools"), [StackIssue::OmittedMembers]);
    // Any other change to the listed set still refuses the read.
    let mut config = crowded(Vec::new());
    config.changed = Some(Change::Membership);
    assert!(inspect(&fixture(config).await).await.is_err());
}
