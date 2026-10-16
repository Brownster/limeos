use super::*;

fn target() -> Target {
    Target {
        stack: "media".into(),
        engine_id: "engine-a".into(),
        service: "prowlarr".into(),
        project: ProjectIdentity {
            name: "media".into(),
            working_directory: "/opt/Media".into(),
            config_files: vec![
                "/opt/Media/compose.yaml".into(),
                "/opt/Media/managed.json".into(),
            ],
        },
        container: ContainerSnapshot {
            resource: format!("container:{}", "1".repeat(64)),
            image: format!("sha256:{}", "a".repeat(64)),
            started_at: "2026-10-16T01:02:03Z".into(),
            running: true,
        },
    }
}
fn requests() -> Vec<Request> {
    [App::Sonarr, App::Radarr, App::Lidarr]
        .into_iter()
        .map(|application| Request {
            application,
            service: application.implementation().to_ascii_lowercase(),
            categories: vec![
                application.root(),
                application.root() + 30,
                application.root() + 40,
            ],
            anime_categories: (application == App::Sonarr).then(|| vec![5070]),
            tags: vec![],
        })
        .collect()
}
fn services() -> Vec<String> {
    ["prowlarr", "sonarr", "radarr", "lidarr"]
        .map(String::from)
        .to_vec()
}
fn application(app: App, id: u32) -> Application {
    let r = requests()
        .into_iter()
        .find(|r| r.application == app)
        .unwrap();
    Application {
        id,
        name: app.implementation().into(),
        implementation: app.implementation().into(),
        settings: Some(Settings {
            sync_level: SyncLevel::FullSync,
            categories: r.categories,
            anime_categories: r.anime_categories,
            tags: r.tags,
        }),
    }
}
fn sample<'a>(
    target: &'a Target,
    declared_services: &'a [String],
    applications: &'a [Application],
) -> Observation<'a> {
    Observation {
        target,
        sampled_at: 100,
        complete: true,
        declared_services,
        applications,
    }
}
fn run(apps: &[Application]) -> Preview {
    let target = target();
    preview(
        101,
        &target,
        &requests(),
        Some(sample(&target, &services(), apps)),
    )
    .unwrap()
}
fn blocked(row: &Row) -> Conflict {
    match &row.decision {
        Decision::Blocked { reason, .. } => *reason,
        other => panic!("expected blocked, got {other:?}"),
    }
}

#[test]
fn frozen_full_sync_intent_proposes_three_incomplete_candidates_without_connection_material() {
    let r = run(&[]);
    assert_eq!(r.outcome, Outcome::AdditionsProposed);
    assert_eq!(r.additions.len(), 3);
    assert_eq!(r.additions[0].config_contract, "SonarrSettings");
    assert_eq!(r.additions[1].config_contract, "RadarrSettings");
    assert_eq!(r.additions[2].config_contract, "LidarrSettings");
    assert!(
        r.additions
            .iter()
            .all(|a| a.sync_level == SyncLevel::FullSync)
    );
    assert_eq!(r.unresolved, GATES);
    assert_eq!(READ_PATH, "/api/v1/applications");
    let json = serde_json::to_string(&r).unwrap();
    for unsupported in [
        "baseUrl",
        "prowlarrUrl",
        "apiKey",
        "127.0.0.1",
        "config.xml",
    ] {
        assert!(!json.contains(unsupported));
    }
}
#[test]
fn native_metadata_rerun_has_no_additions_and_retains_every_gate() {
    let r = run(&[
        application(App::Sonarr, 11),
        application(App::Radarr, 12),
        application(App::Lidarr, 13),
    ]);
    assert_eq!(r.outcome, Outcome::NoAdditions);
    assert!(r.additions.is_empty());
    assert_eq!(r.rows[0].decision, Decision::MatchingMetadata { id: 11 });
    assert_eq!(r.unresolved, GATES);
}
#[test]
fn renamed_and_case_varied_implementation_does_not_duplicate_an_existing_registration() {
    let mut app = application(App::Sonarr, 11);
    app.implementation = "sOnArR".into();
    app.name = "My TV indexers".into();
    let r = run(&[app]);
    assert_eq!(r.rows[0].decision, Decision::MatchingMetadata { id: 11 });
    assert_eq!(r.additions.len(), 2);
}
#[test]
fn duplicate_implementations_block_even_when_only_one_has_canonical_name_or_complete_metadata() {
    let a = application(App::Sonarr, 12);
    let mut b = application(App::Sonarr, 11);
    b.name = "Other instance".into();
    b.settings = None;
    let r = run(&[a.clone(), b.clone()]);
    assert_eq!(
        r.rows[0].decision,
        Decision::Blocked {
            reason: Conflict::AmbiguousImplementation,
            ids: vec![11, 12]
        }
    );
    assert_eq!(r.outcome, Outcome::Blocked);
    assert!(r.additions.is_empty());
    assert_eq!(r, run(&[b, a]));
}
#[test]
fn unknown_selected_settings_block_instead_of_becoming_defaults() {
    let mut app = application(App::Sonarr, 11);
    app.settings = None;
    let r = run(&[app]);
    assert_eq!(blocked(&r.rows[0]), Conflict::IncompleteMetadata);
    assert_eq!(r.rows[1].decision, Decision::NeedsRegistration);
    assert!(r.additions.is_empty());
}
#[test]
fn disabled_add_only_categories_anime_and_tag_differences_block_without_operator_updates() {
    for field in 0..5 {
        let mut app = application(App::Sonarr, 11);
        let s = app.settings.as_mut().unwrap();
        match field {
            0 => s.sync_level = SyncLevel::Disabled,
            1 => s.sync_level = SyncLevel::AddOnly,
            2 => s.categories = vec![2000],
            3 => s.anime_categories = Some(vec![]),
            _ => s.tags = vec![7],
        }
        let r = run(&[app]);
        assert_eq!(blocked(&r.rows[0]), Conflict::PolicyDiffers);
        assert!(r.additions.is_empty());
    }
}
#[test]
fn frozen_tv_for_movies_or_music_requests_refuse_with_explicit_scope_error() {
    for index in [1, 2] {
        let mut request = requests();
        request[index].categories = vec![5000, 5030, 5040];
        assert_eq!(
            preview(101, &target(), &request, None).unwrap_err(),
            RegistrationError::CategoryScope
        );
    }
    let mut request = requests();
    request[0].anime_categories = Some(vec![3000]);
    assert_eq!(
        preview(101, &target(), &request, None).unwrap_err(),
        RegistrationError::CategoryScope
    );
}
#[test]
fn missing_peer_declarations_are_explicit_and_custom_service_names_preserve_case() {
    let target = target();
    let mut request = requests();
    request[0].service = "TV-App".into();
    let mut declared = services();
    declared[1] = "tv-app".into();
    let r = preview(
        101,
        &target,
        &request,
        Some(sample(&target, &declared, &[])),
    )
    .unwrap();
    assert_eq!(r.rows[0].decision, Decision::NotDeclared);
    assert_eq!(r.additions.len(), 2);
    declared[1] = "TV-App".into();
    assert_eq!(
        preview(
            101,
            &target,
            &request,
            Some(sample(&target, &declared, &[]))
        )
        .unwrap()
        .additions
        .len(),
        3
    );
    assert_eq!(
        preview(
            101,
            &target,
            &request,
            Some(sample(&target, &["prowlarr".into()], &[]))
        )
        .unwrap()
        .outcome,
        Outcome::NoAdditions
    );
}
#[test]
fn canonical_name_collision_with_another_provider_blocks_and_unrelated_rows_are_preserved() {
    let mut app = Application {
        id: 7,
        name: "SONARR".into(),
        implementation: "OtherApplication".into(),
        settings: None,
    };
    assert_eq!(
        blocked(&run(&[app.clone()]).rows[0]),
        Conflict::NameCollision
    );
    app.name = "Unrelated app".into();
    let original = app.clone();
    assert_eq!(run(std::slice::from_ref(&app)).additions.len(), 3);
    assert_eq!(app, original);
}
#[test]
fn set_order_and_request_order_do_not_change_the_preview() {
    let target = target();
    let mut request = requests();
    request[0].tags = vec![12, 7];
    let mut app = application(App::Sonarr, 11);
    app.settings.as_mut().unwrap().tags = vec![7, 12];
    let expected = preview(
        101,
        &target,
        &request,
        Some(sample(&target, &services(), &[app.clone()])),
    )
    .unwrap();
    request[0].categories.reverse();
    request[0].tags.reverse();
    request.reverse();
    app.settings.as_mut().unwrap().categories.reverse();
    assert_eq!(
        preview(
            101,
            &target,
            &request,
            Some(sample(&target, &services(), &[app]))
        )
        .unwrap(),
        expected
    );
    assert_eq!(expected.rows[0].request.tags, [7, 12]);
}
#[test]
fn unavailable_partial_and_missing_target_declaration_are_unknown_without_candidates() {
    let target = target();
    let r = preview(101, &target, &requests(), None).unwrap();
    assert_eq!(r.source, SourceState::Unavailable);
    assert_eq!(r.outcome, Outcome::Unknown);
    assert!(r.rows.iter().all(|r| r.decision == Decision::Unknown));
    let declared = services();
    let mut s = sample(&target, &declared, &[]);
    s.complete = false;
    assert_eq!(
        preview(101, &target, &requests(), Some(s)).unwrap().source,
        SourceState::Incomplete
    );
    let r = preview(
        101,
        &target,
        &requests(),
        Some(sample(&target, &["sonarr".into()], &[])),
    )
    .unwrap();
    assert_eq!(r.source, SourceState::Incomplete);
    assert!(r.additions.is_empty());
}
#[test]
fn original_timestamp_and_inclusive_age_boundary_are_kept() {
    let target = target();
    for (now, expected) in [
        (190, SourceState::Fresh),
        (191, SourceState::Stale),
        (99, SourceState::Future),
    ] {
        let r = preview(
            now,
            &target,
            &requests(),
            Some(sample(&target, &services(), &[])),
        )
        .unwrap();
        assert_eq!(r.source, expected);
        assert_eq!(r.sampled_at, Some(100));
        if expected != SourceState::Fresh {
            assert!(r.additions.is_empty());
        }
    }
}
#[test]
fn exact_scope_and_container_drift_cannot_reuse_an_observation() {
    let owner = target();
    for field in 0..9 {
        let mut other = owner.clone();
        match field {
            0 => other.stack = "other".into(),
            1 => other.engine_id = "other".into(),
            2 => other.service = "other".into(),
            3 => other.project.name = "other".into(),
            4 => other.project.working_directory = "/Other".into(),
            5 => other.project.config_files.reverse(),
            6 => other.container.image = format!("sha256:{}", "b".repeat(64)),
            7 => other.container.resource = format!("container:{}", "2".repeat(64)),
            _ => other.container.started_at = "2026-10-16T02:02:03Z".into(),
        }
        let r = preview(
            101,
            &owner,
            &requests(),
            Some(sample(&other, &services(), &[])),
        )
        .unwrap();
        assert_eq!(r.source, SourceState::IdentityMismatch);
        assert!(r.additions.is_empty());
    }
}
#[test]
fn stopped_invalid_clock_and_scope_syntax_refuse_or_remain_unknown() {
    let mut owner = target();
    owner.container.running = false;
    assert_eq!(
        preview(
            101,
            &owner,
            &requests(),
            Some(sample(&owner, &services(), &[]))
        )
        .unwrap()
        .source,
        SourceState::Stopped
    );
    owner.container.resource = "container:short".into();
    assert!(preview(101, &owner, &requests(), None).is_err());
    assert!(preview(-1, &target(), &requests(), None).is_err());
    for field in 0..5 {
        let mut owner = target();
        match field {
            0 => owner.stack = "../media".into(),
            1 => owner.engine_id = "x\ny".into(),
            2 => owner.project.name = "Media".into(),
            3 => owner
                .project
                .config_files
                .push(owner.project.config_files[0].clone()),
            _ => owner.project.working_directory = "/opt/../Media".into(),
        }
        assert_eq!(
            owner.validate().unwrap_err(),
            RegistrationError::InvalidInput
        );
    }
}
#[test]
fn duplicate_requests_services_ids_and_wrong_anime_shape_refuse() {
    for field in 0..6 {
        let mut request = requests();
        match field {
            0 => request[1] = request[0].clone(),
            1 => request[1].service = request[0].service.clone(),
            2 => request[0].service = "prowlarr".into(),
            3 => request[0].categories.push(5000),
            4 => request[0].anime_categories = None,
            _ => request[1].anime_categories = Some(vec![5070]),
        }
        assert_eq!(
            preview(101, &target(), &request, None).unwrap_err(),
            RegistrationError::InvalidInput
        );
    }
    let mut a = application(App::Sonarr, 11);
    a.settings.as_mut().unwrap().tags = vec![7, 7];
    assert!(
        preview(
            101,
            &target(),
            &requests(),
            Some(sample(&target(), &services(), &[a]))
        )
        .is_err()
    );
    assert!(preview(101, &target(), &[], None).is_err());
}
#[test]
fn observed_bounds_ids_text_and_duplicates_are_validated_before_projection() {
    for field in 0..5 {
        let mut app = application(App::Sonarr, 11);
        match field {
            0 => app.id = 0,
            1 => app.id = u32::MAX,
            2 => app.name = "é".repeat(65),
            3 => app.implementation = "x\ny".into(),
            _ => app.settings.as_mut().unwrap().categories.push(0),
        }
        assert!(
            preview(
                101,
                &target(),
                &requests(),
                Some(sample(&target(), &services(), &[app]))
            )
            .is_err()
        );
    }
    let a = application(App::Sonarr, 11);
    assert!(
        preview(
            101,
            &target(),
            &requests(),
            Some(sample(&target(), &services(), &[a.clone(), a]))
        )
        .is_err()
    );
    let mut request = requests();
    request[0].categories = (5000..5016).collect();
    assert!(preview(101, &target(), &request, None).is_ok());
    request[0].categories.push(5016);
    assert_eq!(
        preview(101, &target(), &request, None).unwrap_err(),
        RegistrationError::Limit
    );
}
#[test]
fn registration_and_declaration_limits_refuse_without_truncation() {
    let apps: Vec<_> = (1..=16).map(|id| application(App::Sonarr, id)).collect();
    assert_eq!(run(&apps).outcome, Outcome::Blocked);
    let mut apps = apps;
    apps.push(application(App::Sonarr, 17));
    assert_eq!(
        preview(
            101,
            &target(),
            &requests(),
            Some(sample(&target(), &services(), &apps))
        )
        .unwrap_err(),
        RegistrationError::Limit
    );
    let declared: Vec<_> = (0..17).map(|i| format!("service{i}")).collect();
    assert_eq!(
        preview(
            101,
            &target(),
            &requests(),
            Some(sample(&target(), &declared, &[]))
        )
        .unwrap_err(),
        RegistrationError::Limit
    );
    assert!(
        preview(
            101,
            &target(),
            &requests(),
            Some(sample(
                &target(),
                &["prowlarr".into(), "prowlarr".into()],
                &[]
            ))
        )
        .is_err()
    );
}
#[test]
fn public_preview_streaming_ceiling_refuses_oversized_mutations() {
    let mut r = run(&[]);
    r.rows[0].request.service = "x".repeat(MAX_REPORT_BYTES);
    assert_eq!(r.check_size().unwrap_err(), RegistrationError::Limit);
}
