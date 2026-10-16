use super::*;
use crate::{
    ContainerSnapshot,
    deployment::{discovery::ProjectIdentity, media::ServiceIdentity},
};

fn target(app: MediaService) -> Target {
    Target {
        identity: ServiceIdentity {
            stack: "media".into(),
            engine_id: "engine-a".into(),
            service: "arr".into(),
            application: app,
            project: ProjectIdentity {
                name: "media".into(),
                working_directory: "/opt/Media".into(),
                config_files: vec![
                    "/opt/Media/compose.yaml".into(),
                    "/opt/Media/managed.json".into(),
                ],
            },
        },
        container: ContainerSnapshot {
            resource: format!("container:{}", "1".repeat(64)),
            image: format!("sha256:{}", "a".repeat(64)),
            started_at: "2026-10-16T01:02:03Z".into(),
            running: true,
        },
    }
}
fn requests(app: MediaService) -> Vec<Request> {
    let category = match app {
        MediaService::Sonarr => "sonarr",
        MediaService::Radarr => "radarr",
        _ => "lidarr",
    };
    [Provider::Transmission, Provider::Sabnzbd]
        .into_iter()
        .map(|provider| Request {
            provider,
            service: provider.implementation().to_ascii_lowercase(),
            category: category.into(),
            remove_completed: true,
            remove_failed: true,
        })
        .collect()
}
fn declarations() -> Vec<String> {
    ["arr", "transmission", "sabnzbd"]
        .map(String::from)
        .to_vec()
}
fn client(app: MediaService, provider: Provider, id: u32) -> Client {
    Client {
        id,
        name: provider.implementation().into(),
        implementation: provider.implementation().into(),
        category_field: CategoryField::for_app(app).unwrap(),
        category: Category::Known(requests(app)[0].category.clone()),
        enabled: true,
        remove_completed: true,
        remove_failed: true,
    }
}
fn sample<'a>(owner: &'a Target, services: &'a [String], clients: &'a [Client]) -> Observation<'a> {
    Observation {
        target: owner,
        sampled_at: 100,
        complete: true,
        declared_services: services,
        clients,
    }
}
fn run(clients: &[Client]) -> Preview {
    let app = MediaService::Sonarr;
    let owner = target(app);
    preview(
        101,
        &owner,
        &requests(app),
        Some(sample(&owner, &declarations(), clients)),
    )
    .unwrap()
}
fn reason(row: &Row) -> Conflict {
    match &row.decision {
        Decision::Blocked { reason, .. } => *reason,
        other => panic!("expected blocked, got {other:?}"),
    }
}

#[test]
fn frozen_requests_propose_two_incomplete_additions_with_native_category_fields() {
    for (app, field, path) in [
        (MediaService::Sonarr, "tvCategory", "/api/v3/downloadclient"),
        (
            MediaService::Radarr,
            "movieCategory",
            "/api/v3/downloadclient",
        ),
        (
            MediaService::Lidarr,
            "musicCategory",
            "/api/v1/downloadclient",
        ),
    ] {
        let owner = target(app);
        let r = preview(
            101,
            &owner,
            &requests(app),
            Some(sample(&owner, &declarations(), &[])),
        )
        .unwrap();
        assert_eq!(r.outcome, Outcome::AdditionsProposed);
        assert_eq!(r.additions.len(), 2);
        assert_eq!(r.additions[0].category_field.wire_name(), field);
        assert_eq!(r.additions[0].config_contract, "TransmissionSettings");
        assert_eq!(r.additions[1].config_contract, "SabnzbdSettings");
        assert_eq!(read_path(app).unwrap(), path);
        assert_eq!(r.unresolved, GATES);
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("localhost") && !json.contains("127.0.0.1"));
    }
}
#[test]
fn native_matching_rerun_has_no_additions_but_keeps_all_gates() {
    let r = run(&[
        client(MediaService::Sonarr, Provider::Transmission, 11),
        client(MediaService::Sonarr, Provider::Sabnzbd, 12),
    ]);
    assert_eq!(r.outcome, Outcome::NoAdditions);
    assert_eq!(r.rows[0].decision, Decision::Present { id: 11 });
    assert!(r.additions.is_empty());
    assert_eq!(r.unresolved, GATES);
}
#[test]
fn implementation_matching_ignores_ascii_case_and_category_matching_preserves_case() {
    let mut c = client(MediaService::Sonarr, Provider::Transmission, 11);
    c.implementation = "tRaNsMiSsIoN".into();
    c.name = "operator client".into();
    assert_eq!(
        run(&[c.clone()]).rows[0].decision,
        Decision::Present { id: 11 }
    );
    c.category = Category::Known("SONARR".into());
    assert_eq!(run(&[c]).rows[0].decision, Decision::NeedsRegistration);
}
#[test]
fn missing_declarations_are_explicit_skips_and_custom_peer_names_are_exact() {
    let app = MediaService::Sonarr;
    let owner = target(app);
    let mut request = requests(app);
    request[0].service = "TorrentClient".into();
    let mut services = declarations();
    services[1] = "torrentclient".into();
    let r = preview(101, &owner, &request, Some(sample(&owner, &services, &[]))).unwrap();
    assert_eq!(r.rows[0].decision, Decision::NotDeclared);
    assert_eq!(r.additions.len(), 1);
    services[1] = "TorrentClient".into();
    assert_eq!(
        preview(101, &owner, &request, Some(sample(&owner, &services, &[])))
            .unwrap()
            .additions
            .len(),
        2
    );
    let services = vec!["arr".into()];
    assert_eq!(
        preview(101, &owner, &request, Some(sample(&owner, &services, &[])))
            .unwrap()
            .outcome,
        Outcome::NoAdditions
    );
}
#[test]
fn unknown_category_blocks_even_if_another_known_registration_matches() {
    let mut unknown = client(MediaService::Sonarr, Provider::Transmission, 12);
    unknown.category = Category::Unknown;
    let r = run(&[
        client(MediaService::Sonarr, Provider::Transmission, 11),
        unknown,
    ]);
    assert_eq!(reason(&r.rows[0]), Conflict::UnknownCategory);
    assert_eq!(r.outcome, Outcome::Blocked);
    assert_eq!(r.rows[1].decision, Decision::NeedsRegistration);
    assert!(r.additions.is_empty());
}
#[test]
fn disabled_or_differing_removal_policy_never_counts_as_configured_or_proposes_an_update() {
    for field in 0..3 {
        let mut c = client(MediaService::Sonarr, Provider::Transmission, 11);
        match field {
            0 => c.enabled = false,
            1 => c.remove_completed = false,
            _ => c.remove_failed = false,
        }
        let r = run(&[c]);
        assert_eq!(reason(&r.rows[0]), Conflict::PolicyDiffers);
        assert!(r.additions.is_empty());
    }
}
#[test]
fn duplicate_matches_block_with_sorted_ids_independent_of_response_order() {
    let a = client(MediaService::Sonarr, Provider::Transmission, 12);
    let b = client(MediaService::Sonarr, Provider::Transmission, 11);
    let r = run(&[a.clone(), b.clone()]);
    assert_eq!(
        r.rows[0].decision,
        Decision::Blocked {
            reason: Conflict::DuplicateMatch,
            ids: vec![11, 12]
        }
    );
    assert_eq!(r, run(&[b, a]));
}
#[test]
fn existing_name_collision_blocks_including_unrelated_implementations() {
    let mut c = client(MediaService::Sonarr, Provider::Transmission, 11);
    c.category = Category::Known("other-category".into());
    c.name = "TRANSMISSION".into();
    assert_eq!(reason(&run(&[c.clone()]).rows[0]), Conflict::NameCollision);
    c.implementation = "OtherProvider".into();
    c.category = Category::Unknown;
    assert_eq!(reason(&run(&[c]).rows[0]), Conflict::NameCollision);
}
#[test]
fn unrelated_registrations_are_preserved_and_do_not_block_an_addition() {
    let mut c = client(MediaService::Sonarr, Provider::Transmission, 11);
    c.category = Category::Known("other-category".into());
    c.name = "Operator download client".into();
    let snapshot = c.clone();
    assert_eq!(run(std::slice::from_ref(&c)).additions.len(), 2);
    assert_eq!(c, snapshot);
    c.implementation = "OtherProvider".into();
    c.category = Category::Unknown;
    assert_eq!(run(&[c]).additions.len(), 2);
}
#[test]
fn unavailable_incomplete_and_missing_target_declaration_never_become_empty_success() {
    let app = MediaService::Sonarr;
    let owner = target(app);
    let r = preview(101, &owner, &requests(app), None).unwrap();
    assert_eq!(r.source, SourceState::Unavailable);
    assert_eq!(r.outcome, Outcome::Unknown);
    assert!(r.rows.iter().all(|r| r.decision == Decision::Unknown));
    let services = declarations();
    let mut s = sample(&owner, &services, &[]);
    s.complete = false;
    assert_eq!(
        preview(101, &owner, &requests(app), Some(s))
            .unwrap()
            .source,
        SourceState::Incomplete
    );
    let services = vec!["transmission".into(), "sabnzbd".into()];
    let r = preview(
        101,
        &owner,
        &requests(app),
        Some(sample(&owner, &services, &[])),
    )
    .unwrap();
    assert_eq!(r.source, SourceState::Incomplete);
    assert!(r.additions.is_empty());
}
#[test]
fn original_oldest_timestamp_and_inclusive_age_boundary_are_kept() {
    let app = MediaService::Sonarr;
    let owner = target(app);
    let services = declarations();
    for (now, expected) in [
        (190, SourceState::Fresh),
        (191, SourceState::Stale),
        (99, SourceState::Future),
    ] {
        let r = preview(
            now,
            &owner,
            &requests(app),
            Some(sample(&owner, &services, &[])),
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
fn exact_scope_and_container_drift_cannot_reuse_a_client_observation() {
    let app = MediaService::Sonarr;
    let owner = target(app);
    for field in 0..10 {
        let mut other = owner.clone();
        match field {
            0 => other.identity.stack = "other".into(),
            1 => other.identity.engine_id = "other".into(),
            2 => other.identity.service = "other".into(),
            3 => other.identity.project.name = "other".into(),
            4 => other.identity.project.working_directory = "/Other".into(),
            5 => other.identity.project.config_files.reverse(),
            6 => other.container.image = format!("sha256:{}", "b".repeat(64)),
            7 => other.container.started_at = "2026-10-16T02:02:03Z".into(),
            8 => other.container.resource = format!("container:{}", "2".repeat(64)),
            _ => other.identity.application = MediaService::Radarr,
        }
        let r = preview(
            101,
            &owner,
            &requests(app),
            Some(sample(&other, &declarations(), &[])),
        )
        .unwrap();
        assert_eq!(r.source, SourceState::IdentityMismatch);
        assert!(r.additions.is_empty());
    }
}
#[test]
fn stopped_or_invalid_targets_and_cross_application_fields_are_refused() {
    let app = MediaService::Sonarr;
    let mut owner = target(app);
    owner.container.running = false;
    assert_eq!(
        preview(
            101,
            &owner,
            &requests(app),
            Some(sample(&owner, &declarations(), &[]))
        )
        .unwrap()
        .source,
        SourceState::Stopped
    );
    owner.container.resource = "container:short".into();
    assert_eq!(
        preview(101, &owner, &requests(app), None).unwrap_err(),
        RegistrationError::InvalidInput
    );
    let owner = target(app);
    let clients = [client(MediaService::Radarr, Provider::Transmission, 11)];
    assert_eq!(
        preview(
            101,
            &owner,
            &requests(app),
            Some(sample(&owner, &declarations(), &clients))
        )
        .unwrap_err(),
        RegistrationError::InvalidInput
    );
    assert_eq!(
        preview(-1, &owner, &requests(app), None).unwrap_err(),
        RegistrationError::InvalidInput
    );
    assert_eq!(
        read_path(MediaService::Jellyfin).unwrap_err(),
        RegistrationError::Unsupported
    );
}
#[test]
fn request_duplicates_empty_categories_and_unsupported_transmission_labels_refuse() {
    let app = MediaService::Sonarr;
    for field in 0..6 {
        let mut request = requests(app);
        match field {
            0 => request[1] = request[0].clone(),
            1 => request[1].service = request[0].service.clone(),
            2 => request[0].category.clear(),
            3 => request[0].category = "has space".into(),
            4 => request[0].service = "../client".into(),
            _ => request[1].category = " x ".into(),
        }
        assert_eq!(
            preview(101, &target(app), &request, None).unwrap_err(),
            RegistrationError::InvalidInput
        );
    }
    assert!(preview(101, &target(app), &[], None).is_err());
    let mut request = requests(app);
    request[0].category = ".TV-Sonarr".into();
    request[1].category = "é".repeat(64);
    assert!(preview(101, &target(app), &request, None).is_ok());
    request[1].category.push('x');
    assert!(preview(101, &target(app), &request, None).is_err());
}
#[test]
fn malformed_ids_names_categories_or_duplicate_ids_refuse() {
    let app = MediaService::Sonarr;
    let owner = target(app);
    for field in 0..6 {
        let mut c = client(app, Provider::Transmission, 11);
        match field {
            0 => c.id = 0,
            1 => c.id = u32::MAX,
            2 => c.name.clear(),
            3 => c.implementation = "x".repeat(129),
            4 => c.category = Category::Known("x\ny".into()),
            _ => c.category = Category::Known("é".repeat(65)),
        }
        assert!(
            preview(
                101,
                &owner,
                &requests(app),
                Some(sample(&owner, &declarations(), &[c]))
            )
            .is_err()
        );
    }
    let c = client(app, Provider::Transmission, 11);
    assert!(
        preview(
            101,
            &owner,
            &requests(app),
            Some(sample(&owner, &declarations(), &[c.clone(), c]))
        )
        .is_err()
    );
}
#[test]
fn inventory_limits_and_declared_service_ambiguity_refuse_without_truncation() {
    let app = MediaService::Sonarr;
    let owner = target(app);
    let clients: Vec<_> = (1..=16)
        .map(|id| client(app, Provider::Transmission, id))
        .collect();
    assert!(
        preview(
            101,
            &owner,
            &requests(app),
            Some(sample(&owner, &declarations(), &clients))
        )
        .is_ok()
    );
    let mut clients = clients;
    clients.push(client(app, Provider::Transmission, 17));
    assert_eq!(
        preview(
            101,
            &owner,
            &requests(app),
            Some(sample(&owner, &declarations(), &clients))
        )
        .unwrap_err(),
        RegistrationError::Limit
    );
    let services = vec!["arr".into(), "arr".into()];
    assert!(
        preview(
            101,
            &owner,
            &requests(app),
            Some(sample(&owner, &services, &[]))
        )
        .is_err()
    );
    let services: Vec<_> = (0..17).map(|i| format!("service{i}")).collect();
    assert_eq!(
        preview(
            101,
            &owner,
            &requests(app),
            Some(sample(&owner, &services, &[]))
        )
        .unwrap_err(),
        RegistrationError::Limit
    );
}
#[test]
fn request_and_response_order_do_not_change_the_preview() {
    let app = MediaService::Sonarr;
    let owner = target(app);
    let mut request = requests(app);
    let mut clients = vec![
        client(app, Provider::Transmission, 11),
        client(app, Provider::Sabnzbd, 12),
    ];
    let expected = run(&clients);
    clients.reverse();
    request.reverse();
    assert_eq!(
        preview(
            101,
            &owner,
            &request,
            Some(sample(&owner, &declarations(), &clients))
        )
        .unwrap(),
        expected
    );
}
#[test]
fn public_preview_streaming_limit_refuses_oversized_mutations() {
    let mut r = run(&[]);
    r.rows[0].request.category = "x".repeat(MAX_REPORT_BYTES);
    assert_eq!(r.check_size().unwrap_err(), RegistrationError::Limit);
}
