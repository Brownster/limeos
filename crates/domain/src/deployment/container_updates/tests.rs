use super::*;
use crate::ContainerSnapshot;
use discovery::{Declaration, Membership};
use serde_json::{Value, json};

fn hash(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}
fn project(stack: &str) -> ProjectIdentity {
    ProjectIdentity {
        name: stack.into(),
        working_directory: format!("/opt/stacks/{stack}"),
        config_files: vec![format!("/opt/stacks/{stack}/compose.yaml")],
    }
}
fn declaration(stack: &str) -> Declaration {
    Declaration {
        stack: stack.into(),
        project: stack.into(),
        working_directory: project(stack).working_directory,
        config_files: project(stack).config_files,
        services: vec!["web".into()],
        files: FileStatus::NotInspected,
    }
}
fn spec(stack: &str) -> StackUpdateSpec {
    StackUpdateSpec {
        stack: stack.into(),
        project: project(stack),
        services: vec![ServiceTrack {
            service: "web".into(),
            source: ImageSource::Tag {
                repository: "example/web".into(),
                tag: "stable".into(),
            },
            platform: Platform::LinuxArm64V8,
            replicas: 1,
        }],
    }
}
fn image(stack: &str, config: char) -> ImageResolution {
    ImageResolution {
        engine_id: "engine".into(),
        stack: stack.into(),
        project: project(stack),
        service: "web".into(),
        source: spec(stack).services[0].source.clone(),
        platform: Platform::LinuxArm64V8,
        manifest_digest: hash('c'),
        config_digest: hash(config),
        sampled_at: 100,
    }
}
fn member(stack: &str, id: u64, config: char) -> Membership {
    Membership::Compose {
        container: ContainerSnapshot {
            resource: format!("container:{id:064x}"),
            image: hash(config),
            started_at: "2026-10-15T00:00:00Z".into(),
            running: true,
        },
        project: project(stack),
        service: "web".into(),
        one_off: false,
    }
}
fn snapshot(member: &mut Membership) -> &mut ContainerSnapshot {
    match member {
        Membership::Compose { container, .. } | Membership::Standalone { container } => container,
    }
}
#[allow(clippy::too_many_arguments)] // Keep independent input fixtures explicit.
fn read(
    config: Option<&UpdateConfig>,
    mode: RunMode,
    selected: &[String],
    specs: &[StackUpdateSpec],
    stacks: &[Declaration],
    members: &[Membership],
    images: &[ImageResolution],
    sampled_at: i64,
) -> Result<UpdatePlan> {
    plan_updates(UpdateInput {
        config,
        mode,
        selected,
        platform: Platform::LinuxArm64V8,
        specs,
        catalog: Some(CatalogInput {
            engine_id: "engine",
            sampled_at,
            stacks,
        }),
        engine: Some(EngineInput {
            engine_id: "engine",
            sampled_at,
            members,
        }),
        images,
        now: 101,
    })
}
fn plan(members: &[Membership], candidate: &ImageResolution) -> UpdatePlan {
    read(
        Some(&UpdateConfig::default()),
        RunMode::Manual,
        &["media".into()],
        &[spec("media")],
        &[declaration("media")],
        members,
        std::slice::from_ref(candidate),
        100,
    )
    .unwrap()
}
fn after(
    plan: &UpdatePlan,
    stacks: &[Declaration],
    members: &[Membership],
    at: i64,
) -> PostAssessment {
    assess_post_update(
        plan,
        PostInput {
            now: 110,
            not_before: 105,
            catalog: Some(CatalogInput {
                engine_id: "engine",
                sampled_at: at,
                stacks,
            }),
            engine: Some(EngineInput {
                engine_id: "engine",
                sampled_at: at,
                members,
            }),
        },
    )
    .unwrap()
}

#[test]
fn frozen_defaults_and_preset_spellings_survive() {
    let expected: Value = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/deployment/container-updates/config.json"
    ))
    .unwrap();
    assert_eq!(
        serde_json::to_value(UpdateConfig::default()).unwrap(),
        expected
    );
    for value in ["disabled", "daily_4am", "weekly_sunday_4am"] {
        let preset: SchedulePreset = serde_json::from_value(json!(value)).unwrap();
        assert_eq!(serde_json::to_value(preset).unwrap(), json!(value));
    }
}
#[test]
fn scheduled_disable_and_explicit_manual_evaluation_are_distinct() {
    let mut config = UpdateConfig::default();
    for (enabled, preset, expected) in [
        (false, SchedulePreset::Daily4am, Decision::ScheduleDisabled),
        (true, SchedulePreset::Disabled, Decision::ScheduleDisabled),
        (true, SchedulePreset::Daily4am, Decision::UpdateProposed),
        (
            true,
            SchedulePreset::WeeklySunday4am,
            Decision::UpdateProposed,
        ),
    ] {
        config.enabled = enabled;
        config.schedule_preset = preset;
        let result = read(
            Some(&config),
            RunMode::Scheduled,
            &["media".into()],
            &[spec("media")],
            &[declaration("media")],
            &[member("media", 1, 'a')],
            &[image("media", 'b')],
            100,
        )
        .unwrap();
        assert_eq!(result.summary().rows[0].decision, expected);
    }
    assert_eq!(
        plan(&[member("media", 1, 'a')], &image("media", 'b'))
            .summary()
            .rows[0]
            .decision,
        Decision::UpdateProposed
    );
}
#[test]
fn exclusions_and_notify_preference_survive_without_scope_expansion() {
    let config = UpdateConfig {
        excluded_stacks: vec!["media".into()],
        notify_on_update: false,
        ..UpdateConfig::default()
    };
    let result = read(
        Some(&config),
        RunMode::Manual,
        &["media".into()],
        &[spec("media"), spec("other")],
        &[declaration("media"), declaration("other")],
        &[member("media", 1, 'a'), member("other", 2, 'a')],
        &[image("media", 'b'), image("other", 'b')],
        100,
    )
    .unwrap();
    assert_eq!(result.summary().rows.len(), 1);
    assert_eq!(result.summary().rows[0].decision, Decision::Excluded);
    assert!(result.summary().rows[0].services.is_empty());
    assert!(!result.summary().config.unwrap().notify_on_update);
}
#[test]
fn unreadable_config_is_unknown_instead_of_defaulted_permission() {
    let result = read(
        None,
        RunMode::Manual,
        &["media".into()],
        &[spec("media")],
        &[declaration("media")],
        &[member("media", 1, 'a')],
        &[image("media", 'b')],
        100,
    )
    .unwrap();
    assert_eq!(result.summary().rows[0].issue, Some(Issue::ConfigUnknown));
    assert!(result.summary().rows[0].services.is_empty());
}
#[test]
fn config_identity_not_pull_output_or_manifest_digest_controls_recreation() {
    let cases: Vec<Value> = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/deployment/container-updates/image-cases.json"
    ))
    .unwrap();
    for case in cases {
        let mut candidate = image(
            "media",
            case["config"].as_str().unwrap().chars().next().unwrap(),
        );
        candidate.manifest_digest =
            hash(case["manifest"].as_str().unwrap().chars().next().unwrap());
        let result = plan(&[member("media", 1, 'a')], &candidate);
        // pull_text is deliberately fixture-only: the planner has no log input.
        assert_eq!(
            serde_json::to_value(result.summary().rows[0].decision).unwrap(),
            case["decision"],
            "{}",
            case["label"]
        );
    }
}
#[test]
fn all_replicas_are_compared_and_mixed_versions_need_recreation() {
    let mut declaration = spec("media");
    declaration.services[0].replicas = 2;
    let result = read(
        Some(&UpdateConfig::default()),
        RunMode::Manual,
        &["media".into()],
        &[declaration],
        &[self::declaration("media")],
        &[member("media", 1, 'b'), member("media", 2, 'a')],
        &[image("media", 'b')],
        100,
    )
    .unwrap();
    assert!(result.summary().rows[0].services[0].recreate);
    assert_eq!(result.summary().rows[0].services[0].replicas, 2);
}
#[test]
fn stopped_missing_and_one_off_only_services_never_autostart() {
    let mut stopped = member("media", 1, 'a');
    snapshot(&mut stopped).running = false;
    let mut one_off = member("media", 2, 'a');
    if let Membership::Compose { one_off, .. } = &mut one_off {
        *one_off = true;
    }
    for members in [vec![], vec![stopped], vec![one_off]] {
        let result = plan(&members, &image("media", 'b'));
        assert_eq!(result.summary().rows[0].decision, Decision::Blocked);
        assert!(result.summary().rows[0].services.is_empty());
    }
}
#[test]
fn active_one_off_blocks_and_stopped_one_off_does_not_satisfy_replicas() {
    let mut extra = member("media", 2, 'a');
    if let Membership::Compose { one_off, .. } = &mut extra {
        *one_off = true;
    }
    assert_eq!(
        plan(
            &[member("media", 1, 'a'), extra.clone()],
            &image("media", 'b')
        )
        .summary()
        .rows[0]
            .issue,
        Some(Issue::ActiveOneOff)
    );
    snapshot(&mut extra).running = false;
    assert_eq!(
        plan(&[extra.clone()], &image("media", 'b')).summary().rows[0].issue,
        Some(Issue::RuntimeNotRunning)
    );
    assert_eq!(
        plan(&[member("media", 1, 'a'), extra], &image("media", 'b'))
            .summary()
            .rows[0]
            .decision,
        Decision::UpdateProposed
    );
}
#[test]
fn replica_and_service_drift_block_the_whole_stack() {
    let result = plan(
        &[member("media", 1, 'a'), member("media", 2, 'a')],
        &image("media", 'b'),
    );
    assert_eq!(result.summary().rows[0].issue, Some(Issue::ReplicaDrift));
    let mut extra = member("media", 2, 'a');
    if let Membership::Compose { service, .. } = &mut extra {
        *service = "undeclared".into();
    }
    assert_eq!(
        plan(&[member("media", 1, 'a'), extra], &image("media", 'b'))
            .summary()
            .rows[0]
            .issue,
        Some(Issue::RegistrationDrift)
    );
}
#[test]
fn stale_future_and_missing_resolutions_remain_unknown() {
    for at in [0, 102] {
        let mut candidate = image("media", 'b');
        candidate.sampled_at = at;
        assert_eq!(
            plan(&[member("media", 1, 'a')], &candidate).summary().rows[0].issue,
            Some(Issue::ResolutionStale)
        );
    }
    let result = read(
        Some(&UpdateConfig::default()),
        RunMode::Manual,
        &["media".into()],
        &[spec("media")],
        &[declaration("media")],
        &[member("media", 1, 'a')],
        &[],
        100,
    )
    .unwrap();
    assert_eq!(
        result.summary().rows[0].issue,
        Some(Issue::ResolutionUnavailable)
    );
}
#[test]
fn engine_source_ordered_files_and_platform_mismatches_refuse() {
    for change in 0..5 {
        let mut candidate = image("media", 'b');
        match change {
            0 => candidate.engine_id = "other".into(),
            1 => candidate.project.working_directory = "/opt/stacks/Media".into(),
            2 => candidate
                .project
                .config_files
                .push("/opt/stacks/media/other.json".into()),
            3 => {
                candidate.source = ImageSource::Tag {
                    repository: "example/web".into(),
                    tag: "other".into(),
                }
            }
            _ => candidate.platform = Platform::LinuxAmd64,
        }
        let result = plan(&[member("media", 1, 'a')], &candidate);
        assert_eq!(result.summary().rows[0].decision, Decision::Blocked);
        assert!(result.summary().rows[0].services.is_empty());
    }
}
#[test]
fn stale_catalog_and_engine_cannot_become_fresh_from_a_new_plan_time() {
    for at in [0, 102] {
        let result = read(
            Some(&UpdateConfig::default()),
            RunMode::Manual,
            &["media".into()],
            &[spec("media")],
            &[declaration("media")],
            &[member("media", 1, 'a')],
            &[image("media", 'b')],
            at,
        )
        .unwrap();
        assert_eq!(result.summary().rows[0].decision, Decision::Unknown);
        assert_eq!(result.summary().before.engine.sampled_at, Some(at));
    }
}
#[test]
fn pinned_sources_cannot_move_and_arm64_tracks_cannot_select_amd64() {
    let mut track = spec("media");
    track.services[0].source = ImageSource::Digest {
        repository: "example/web".into(),
        manifest_digest: hash('d'),
    };
    let mut candidate = image("media", 'b');
    candidate.source = track.services[0].source.clone();
    let result = read(
        Some(&UpdateConfig::default()),
        RunMode::Manual,
        &["media".into()],
        &[track.clone()],
        &[declaration("media")],
        &[member("media", 1, 'a')],
        &[candidate],
        100,
    )
    .unwrap();
    assert_eq!(
        result.summary().rows[0].issue,
        Some(Issue::PinnedSourceMoved)
    );
    track.services[0].platform = Platform::LinuxAmd64;
    let result = read(
        Some(&UpdateConfig::default()),
        RunMode::Manual,
        &["media".into()],
        &[track],
        &[declaration("media")],
        &[member("media", 1, 'a')],
        &[image("media", 'b')],
        100,
    )
    .unwrap();
    assert_eq!(
        result.summary().rows[0].issue,
        Some(Issue::PlatformMismatch)
    );
}
#[test]
fn explicit_source_conflicts_and_unsupported_diagnostics_block() {
    for files in [
        FileStatus::Unavailable,
        FileStatus::Unsupported {
            reason: discovery::UnsupportedSource::Yaml,
        },
        FileStatus::Conflict {
            candidates: vec![
                crate::ComposeFileName::ComposeYaml,
                crate::ComposeFileName::ComposeYml,
            ],
        },
    ] {
        let mut stack = declaration("media");
        stack.files = files;
        let result = read(
            Some(&UpdateConfig::default()),
            RunMode::Manual,
            &["media".into()],
            &[spec("media")],
            &[stack],
            &[member("media", 1, 'a')],
            &[image("media", 'b')],
            100,
        )
        .unwrap();
        assert!(result.summary().rows[0].services.is_empty());
        assert_ne!(result.summary().rows[0].decision, Decision::UpdateProposed);
    }
}
#[test]
fn post_observation_requires_all_running_replicas_on_expected_config() {
    let mut track = spec("media");
    track.services[0].replicas = 2;
    let plan = read(
        Some(&UpdateConfig::default()),
        RunMode::Manual,
        &["media".into()],
        &[track],
        &[declaration("media")],
        &[member("media", 1, 'a'), member("media", 2, 'a')],
        &[image("media", 'b')],
        100,
    )
    .unwrap();
    let stacks = [declaration("media")];
    let report = after(
        &plan,
        &stacks,
        &[member("media", 3, 'b'), member("media", 4, 'b')],
        106,
    );
    assert_eq!(report.rows[0].status, ObservationStatus::ObservedConverged);
    let report = after(
        &plan,
        &stacks,
        &[member("media", 3, 'b'), member("media", 4, 'a')],
        106,
    );
    assert_eq!(report.rows[0].issue, Some(PostIssue::ImageMismatch));
    let mut stopped = member("media", 4, 'b');
    snapshot(&mut stopped).running = false;
    assert_eq!(
        after(&plan, &stacks, &[member("media", 3, 'b'), stopped], 106).rows[0].issue,
        Some(PostIssue::StoppedReplica)
    );
    assert_eq!(
        after(&plan, &stacks, &[member("media", 3, 'b')], 106).rows[0].issue,
        Some(PostIssue::MembershipDrift)
    );
}
#[test]
fn post_old_stale_missing_and_future_observations_are_unknown() {
    let plan = plan(&[member("media", 1, 'a')], &image("media", 'b'));
    for at in [0, 100, 104, 111] {
        let report = after(
            &plan,
            &[declaration("media")],
            &[member("media", 2, 'b')],
            at,
        );
        assert_eq!(report.rows[0].status, ObservationStatus::Unknown);
        assert_eq!(report.engine_sampled_at, Some(at));
    }
    let report = assess_post_update(
        &plan,
        PostInput {
            now: 110,
            not_before: 105,
            catalog: None,
            engine: None,
        },
    )
    .unwrap();
    assert_eq!(report.rows[0].status, ObservationStatus::Unknown);
}
#[test]
fn post_engine_identity_scope_and_one_off_changes_never_converge() {
    let mut extra = member("media", 8, 'a');
    if let Membership::Compose { one_off, .. } = &mut extra {
        *one_off = true;
    }
    snapshot(&mut extra).running = false;
    let plan = plan(
        &[member("media", 1, 'a'), extra.clone()],
        &image("media", 'b'),
    );
    let stacks = [declaration("media")];
    let members = [member("media", 2, 'b'), extra.clone()];
    let report = assess_post_update(
        &plan,
        PostInput {
            now: 110,
            not_before: 105,
            catalog: Some(CatalogInput {
                engine_id: "other",
                sampled_at: 106,
                stacks: &stacks,
            }),
            engine: Some(EngineInput {
                engine_id: "other",
                sampled_at: 106,
                members: &members,
            }),
        },
    )
    .unwrap();
    assert_eq!(report.rows[0].issue, Some(PostIssue::EngineChanged));
    assert_eq!(
        after(&plan, &stacks, &[member("media", 2, 'b')], 106).rows[0].issue,
        Some(PostIssue::OneOffChanged)
    );
    assert_eq!(
        after(&plan, &stacks, &members, 106).rows[0].status,
        ObservationStatus::ObservedConverged
    );
    let mut changed = declaration("media");
    changed
        .config_files
        .push("/opt/stacks/media/new.json".into());
    assert_eq!(
        after(&plan, &[changed], &members, 106).rows[0].status,
        ObservationStatus::Diverged
    );
}
#[test]
fn current_or_excluded_drafts_do_not_turn_into_update_receipts() {
    let plan = plan(&[member("media", 1, 'a')], &image("media", 'a'));
    assert_eq!(
        after(
            &plan,
            &[declaration("media")],
            &[member("media", 1, 'a')],
            106
        )
        .rows[0]
            .status,
        ObservationStatus::NotApplicable
    );
    assert!(
        assess_post_update(
            &plan,
            PostInput {
                now: 110,
                not_before: 99,
                catalog: None,
                engine: None
            }
        )
        .is_err()
    );
}
#[test]
fn strict_models_reject_unknown_fields_and_arbitrary_image_sources() {
    let mut config = serde_json::to_value(UpdateConfig::default()).unwrap();
    config["approved"] = json!(true);
    assert!(serde_json::from_value::<UpdateConfig>(config).is_err());
    assert!(
        serde_json::from_value::<ImageSource>(
            json!({"kind":"tag","repository":"example/web","tag":"stable","shell":"id"})
        )
        .is_err()
    );
    for repo in [
        "",
        "Example/web",
        "example//web",
        "../web",
        "https://example/web",
        "reg:0/web",
        "reg:999999/web",
        "example/web:tag",
    ] {
        assert!(!repository(repo), "{repo}");
    }
    assert!(repository("registry.local:5000/team/web"));
    let mut candidate = image("media", 'b');
    candidate.config_digest = "sha256:short".into();
    assert!(
        read(
            Some(&UpdateConfig::default()),
            RunMode::Manual,
            &["media".into()],
            &[spec("media")],
            &[declaration("media")],
            &[member("media", 1, 'a')],
            &[candidate],
            100
        )
        .is_err()
    );
}
#[test]
fn duplicate_selection_resolution_and_oversized_inputs_are_refused_even_if_excluded() {
    let candidate = image("media", 'b');
    for selected in [
        vec![],
        vec!["media".into(), "media".into()],
        (0..=MAX_STACKS).map(|i| format!("stack-{i}")).collect(),
    ] {
        assert!(
            read(
                Some(&UpdateConfig::default()),
                RunMode::Manual,
                &selected,
                &[spec("media")],
                &[declaration("media")],
                &[member("media", 1, 'a')],
                std::slice::from_ref(&candidate),
                100
            )
            .is_err()
        );
    }
    assert!(
        read(
            Some(&UpdateConfig::default()),
            RunMode::Manual,
            &["media".into()],
            &[spec("media")],
            &[declaration("media")],
            &[member("media", 1, 'a')],
            &[candidate.clone(), candidate],
            100
        )
        .is_err()
    );
    let config = UpdateConfig {
        excluded_stacks: vec!["media".into()],
        ..UpdateConfig::default()
    };
    let mut oversized = spec("media");
    oversized.services[0].source = ImageSource::Tag {
        repository: "a".repeat(193),
        tag: "stable".into(),
    };
    assert!(
        read(
            Some(&config),
            RunMode::Manual,
            &["media".into()],
            &[oversized],
            &[declaration("media")],
            &[],
            &[],
            100
        )
        .is_err()
    );
}
#[test]
fn summary_is_deterministic_redacted_and_keeps_every_execution_gate() {
    let a = plan(&[member("media", 1, 'a')], &image("media", 'b'));
    let b = plan(&[member("media", 1, 'a')], &image("media", 'b'));
    assert_eq!(
        serde_json::to_value(a.summary()).unwrap(),
        serde_json::to_value(b.summary()).unwrap()
    );
    assert_eq!(a.summary().unresolved.len(), 8);
    assert!(!format!("{a:?}").contains("example/web"));
    assert_eq!(a.summary().rows[0].services[0].resolution_sampled_at, 100);
}

#[test]
fn total_selected_services_are_bounded_before_discovery_or_draft_clones() {
    let mut first = spec("media");
    first.services = (0..MAX_SERVICES)
        .map(|i| ServiceTrack {
            service: format!("service-{i}"),
            ..first.services[0].clone()
        })
        .collect();
    let selected = ["media".into(), "other".into()];
    let specs = [first, spec("other")];
    let input = UpdateInput {
        config: None,
        mode: RunMode::Manual,
        selected: &selected,
        platform: Platform::LinuxArm64V8,
        specs: &specs,
        catalog: None,
        engine: None,
        images: &[],
        now: 101,
    };
    assert_eq!(admit(&input).unwrap_err().0, ErrorCode::Unavailable);
}

#[test]
fn aggregate_resolution_text_is_bounded_even_with_valid_individual_paths() {
    let mut track = spec("media");
    track.project.working_directory = format!("/{}/{}", "a".repeat(240), "b".repeat(240));
    track.project.config_files = (0..4)
        .map(|i| format!("/{}/{i}{}", "a".repeat(240), "b".repeat(240)))
        .collect();
    track.services = (0..MAX_SERVICES)
        .map(|i| ServiceTrack {
            service: format!("service-{i}"),
            ..track.services[0].clone()
        })
        .collect();
    let images: Vec<_> = track
        .services
        .iter()
        .map(|s| ImageResolution {
            service: s.service.clone(),
            project: track.project.clone(),
            ..image("media", 'b')
        })
        .collect();
    let selected = ["media".into()];
    let specs = [track];
    let input = UpdateInput {
        config: None,
        mode: RunMode::Manual,
        selected: &selected,
        platform: Platform::LinuxArm64V8,
        specs: &specs,
        catalog: None,
        engine: None,
        images: &images,
        now: 101,
    };
    assert_eq!(admit(&input).unwrap_err().0, ErrorCode::Unavailable);
}

#[test]
fn review_summary_contains_only_requested_stacks_not_unrelated_inventory() {
    let result = read(
        Some(&UpdateConfig::default()),
        RunMode::Manual,
        &["media".into()],
        &[spec("media"), spec("private-other")],
        &[declaration("media"), declaration("private-other")],
        &[member("media", 1, 'a'), member("private-other", 2, 'a')],
        &[image("media", 'b'), image("private-other", 'b')],
        100,
    )
    .unwrap();
    let summary = serde_json::to_string(&result.summary()).unwrap();
    assert_eq!(result.summary().before.stacks.len(), 1);
    assert!(!summary.contains("private-other"));
}
