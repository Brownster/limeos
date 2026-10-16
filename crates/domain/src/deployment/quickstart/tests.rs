use super::*;
use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/deployment/quickstart/bundle-layout.json"
    ))
    .unwrap()
}
fn contract(index: usize) -> StorageContract {
    let fixtures: Value = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/storage-contracts.json"
    ))
    .unwrap();
    serde_json::from_value(fixtures["cases"][index]["contract"].clone()).unwrap()
}
fn target() -> Target {
    Target {
        stack: "family-media".into(),
        engine_id: "engine-a".into(),
        project: ProjectIdentity {
            name: "family-media".into(),
            working_directory: "/opt/Family Media".into(),
            config_files: vec![
                "/opt/Family Media/compose.yaml".into(),
                "/opt/Family Media/managed.json".into(),
            ],
        },
    }
}
fn options() -> Options {
    Options {
        use_vpn: true,
        timezone: "Europe/London".into(),
        requested_identity: None,
    }
}
fn sample<'a>(
    target: &'a Target,
    exists: bool,
    declared_services: &'a [String],
    installations: &'a [Installation],
) -> Observation<'a> {
    Observation {
        target,
        sampled_at: 100,
        complete: true,
        exists,
        declared_services,
        installations,
    }
}
fn installation(member: Member) -> Installation {
    Installation {
        key: InstallationKey {
            app: member.id().into(),
            stack: target().stack,
        },
        service: member.id().into(),
        ownership: Ownership::Managed,
    }
}
fn run(existing: bool, services: &[String], installations: &[Installation]) -> Preview {
    let target = target();
    prepare(
        101,
        &target,
        &contract(0),
        &options(),
        Some(sample(&target, existing, services, installations)),
    )
    .unwrap()
}
fn new() -> Preview {
    run(false, &[], &[])
}

#[test]
fn frozen_bundle_order_is_preserved_for_custom_stack_and_single_new_destination() {
    let fixture = fixture();
    let r = new();
    assert_eq!(
        serde_json::to_value(r.members.iter().map(|m| m.member).collect::<Vec<_>>()).unwrap(),
        fixture["members"]
    );
    assert_eq!(
        r.install_candidates
            .iter()
            .map(|i| i.app.as_str())
            .collect::<Vec<_>>(),
        MEMBERS.map(Member::id)
    );
    assert!(
        r.install_candidates
            .iter()
            .all(|i| i.stack == "family-media")
    );
    assert_eq!(
        r.catalog_target,
        Some(InstallTarget::New {
            stack: "family-media".into()
        })
    );
    assert_eq!(r.outcome, Outcome::PreparationProposed);
    assert_eq!(r.unresolved, GATES);
}
#[test]
fn frozen_library_and_download_directories_derive_from_each_storage_profile() {
    let fixture = fixture();
    for index in 0..3 {
        let c = contract(index);
        let target = target();
        let r = prepare(
            101,
            &target,
            &c,
            &options(),
            Some(sample(&target, false, &[], &[])),
        )
        .unwrap();
        assert_eq!(r.directories.len(), 16);
        for (i, kind) in fixture["libraries"].as_array().unwrap().iter().enumerate() {
            assert_eq!(r.directories[i].kind, DirectoryKind::Library);
            assert_eq!(
                r.directories[i].host,
                format!("{}/{}", c.locations.media_host, kind.as_str().unwrap())
            );
        }
        assert_eq!(
            r.directories[6].host,
            format!("{}/incomplete", c.locations.downloads_host)
        );
        for (i, category) in fixture["download_categories"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            assert_eq!(
                r.directories[i + 7].host,
                format!(
                    "{}/complete/{}",
                    c.locations.downloads_host,
                    category.as_str().unwrap()
                )
            );
        }
        assert_eq!(
            r.profile.unwrap().layout.config_root,
            c.locations.application_config_host
        );
    }
}
#[test]
fn managed_identity_wins_over_conflicting_request_without_mutating_contract() {
    let target = target();
    let mut c = contract(0);
    c.media_identity = StorageMediaIdentity {
        uid: 1200,
        gid: 1300,
    };
    let original = c.clone();
    let mut opts = options();
    opts.requested_identity = Some(StorageMediaIdentity {
        uid: 9999,
        gid: 9999,
    });
    let r = prepare(
        101,
        &target,
        &c,
        &opts,
        Some(sample(&target, false, &[], &[])),
    )
    .unwrap();
    assert_eq!(r.media_identity, c.media_identity);
    assert_eq!(r.profile.unwrap().media_identity, c.media_identity);
    assert_eq!(r.issues, [Issue::ManagedIdentityOverridesRequest]);
    assert_eq!(c, original);
    opts.requested_identity = Some(c.media_identity.clone());
    assert!(
        prepare(101, &target, &c, &opts, None)
            .unwrap()
            .issues
            .is_empty()
    );
}
#[test]
fn vpn_off_refuses_before_creating_a_preparation() {
    let mut opts = options();
    opts.use_vpn = false;
    assert_eq!(
        prepare(101, &target(), &contract(0), &opts, None).unwrap_err(),
        PreparationError::UnsupportedWithoutVpn
    );
}
#[test]
fn existing_stack_with_existing_vpn_keeps_existing_destination() {
    let r = run(true, &["vpn".into()], &[installation(Member::Vpn)]);
    assert_eq!(r.members[0].decision, MemberDecision::AlreadyDeclared);
    assert_eq!(r.install_candidates.len(), 7);
    assert_eq!(
        r.catalog_target,
        Some(InstallTarget::Existing {
            stack: target().stack
        })
    );
    assert_eq!(r.install_candidates[0].app, "transmission");
}
#[test]
fn empty_registered_stack_is_distinct_from_absent_stack() {
    assert_eq!(
        run(true, &[], &[]).catalog_target,
        Some(InstallTarget::Existing {
            stack: target().stack
        })
    );
    assert_eq!(
        new().catalog_target,
        Some(InstallTarget::New {
            stack: target().stack
        })
    );
}
#[test]
fn all_declared_members_need_no_installs_but_still_require_workflow_verification() {
    let declared = MEMBERS.map(|m| m.id().to_string());
    let installed = MEMBERS.map(installation);
    let r = run(true, &declared, &installed);
    assert!(r.install_candidates.is_empty());
    assert!(
        r.members
            .iter()
            .all(|m| m.decision == MemberDecision::AlreadyDeclared)
    );
    assert_eq!(r.workflow.len(), 10);
    assert_eq!(r.unresolved, GATES);
    assert!(r.profile.is_some());
    assert_eq!(r.outcome, Outcome::PreparationProposed);
}
#[test]
fn unregistered_same_name_service_blocks_instead_of_interpreting_an_already_installed_message() {
    let r = run(true, &["vpn".into()], &[]);
    assert_eq!(
        r.members[0].decision,
        MemberDecision::Blocked {
            reason: Conflict::NameCollision
        }
    );
    assert_eq!(r.outcome, Outcome::Blocked);
    assert!(r.install_candidates.is_empty());
    assert!(r.profile.is_none());
    assert!(r.catalog_target.is_none());
}
#[test]
fn operator_and_unknown_ownership_are_not_adopted() {
    for ownership in [Ownership::Operator, Ownership::Unknown] {
        let mut entry = installation(Member::Vpn);
        entry.ownership = ownership;
        let r = run(true, &["vpn".into()], &[entry]);
        assert_eq!(
            r.members[0].decision,
            MemberDecision::Blocked {
                reason: Conflict::UnownedInstallation
            }
        );
        assert!(r.install_candidates.is_empty());
    }
}
#[test]
fn duplicate_installations_and_custom_service_mappings_need_review() {
    let a = installation(Member::Vpn);
    let mut b = a.clone();
    b.service = "custom-vpn".into();
    let r = run(
        true,
        &["vpn".into(), "custom-vpn".into()],
        &[a.clone(), b.clone()],
    );
    assert_eq!(
        r.members[0].decision,
        MemberDecision::Blocked {
            reason: Conflict::AmbiguousInstallation
        }
    );
    assert_eq!(
        r,
        run(true, &["vpn".into(), "custom-vpn".into()], &[b.clone(), a])
    );
    let r = run(true, &["custom-vpn".into()], &[b]);
    assert_eq!(
        r.members[0].decision,
        MemberDecision::Blocked {
            reason: Conflict::NoncanonicalService
        }
    );
    assert!(r.profile.is_none());
}
#[test]
fn unrelated_installed_apps_are_preserved_and_scoped_to_the_selected_stack() {
    let entry = Installation {
        key: InstallationKey {
            app: "operator-app".into(),
            stack: target().stack,
        },
        service: "operator-service".into(),
        ownership: Ownership::Operator,
    };
    let original = entry.clone();
    assert_eq!(
        run(
            true,
            &["operator-service".into()],
            std::slice::from_ref(&entry)
        )
        .install_candidates
        .len(),
        8
    );
    assert_eq!(entry, original);
    let mut other = installation(Member::Vpn);
    other.key.stack = "different-stack".into();
    assert_eq!(
        prepare(
            101,
            &target(),
            &contract(0),
            &options(),
            Some(sample(&target(), true, &["vpn".into()], &[other]))
        )
        .unwrap_err(),
        PreparationError::InvalidInput
    );
}
#[test]
fn missing_and_partial_catalog_reads_cannot_become_new_or_existing_targets() {
    let target = target();
    let r = prepare(101, &target, &contract(0), &options(), None).unwrap();
    assert_eq!(r.source, SourceState::Unavailable);
    assert!(r.catalog_target.is_none());
    assert!(r.profile.is_none());
    assert!(r.install_candidates.is_empty());
    assert!(
        r.members
            .iter()
            .all(|m| m.decision == MemberDecision::Unknown)
    );
    let mut s = sample(&target, true, &[], &[]);
    s.complete = false;
    let r = prepare(101, &target, &contract(0), &options(), Some(s)).unwrap();
    assert_eq!(r.source, SourceState::Incomplete);
    assert_eq!(r.outcome, Outcome::Unknown);
    assert!(r.install_candidates.is_empty());
}
#[test]
fn original_age_boundary_and_future_samples_remain_explicit() {
    let target = target();
    for (now, expected) in [
        (190, SourceState::Fresh),
        (191, SourceState::Stale),
        (99, SourceState::Future),
    ] {
        let r = prepare(
            now,
            &target,
            &contract(0),
            &options(),
            Some(sample(&target, false, &[], &[])),
        )
        .unwrap();
        assert_eq!(r.source, expected);
        assert_eq!(r.sampled_at, Some(100));
        if expected != SourceState::Fresh {
            assert!(r.install_candidates.is_empty());
            assert!(r.profile.is_none());
        }
    }
}
#[test]
fn full_stack_engine_and_ordered_project_identity_must_match() {
    let owner = target();
    for field in 0..6 {
        let mut other = owner.clone();
        match field {
            0 => other.stack = "other".into(),
            1 => other.engine_id = "other".into(),
            2 => other.project.name = "other".into(),
            3 => other.project.working_directory = "/Other".into(),
            4 => other.project.config_files.reverse(),
            _ => other.project.config_files[0] = "/Other/compose.yaml".into(),
        }
        let r = prepare(
            101,
            &owner,
            &contract(0),
            &options(),
            Some(sample(&other, false, &[], &[])),
        )
        .unwrap();
        assert_eq!(r.source, SourceState::IdentityMismatch);
        assert!(r.profile.is_none());
        assert!(r.catalog_target.is_none());
    }
}
#[test]
fn workflow_has_quiescence_and_independent_verification_barriers() {
    let r = new();
    let expected = fixture();
    assert_eq!(
        serde_json::to_value(r.workflow.iter().map(|s| s.stage).collect::<Vec<_>>()).unwrap(),
        expected["workflow"]
    );
    assert_eq!(r.workflow[0].requires, None);
    for pair in r.workflow.windows(2) {
        assert_eq!(pair[1].requires, Some(pair[0].stage));
    }
    assert_eq!(
        r.workflow[4].requires,
        Some(Stage::ConfigureQuiescentDownloadClients)
    );
    assert_eq!(r.workflow[6].requires, Some(Stage::VerifyStartup));
    assert_eq!(
        r.workflow[8].requires,
        Some(Stage::VerifyLoadedConfiguration)
    );
    assert_eq!(r.workflow[9].requires, Some(Stage::SaveProfile));
}
#[test]
fn profile_v2_is_selected_intent_without_values_bag_secrets_or_completion_marker() {
    let r = new();
    let p = r.profile.unwrap();
    assert_eq!(p.version, 2);
    assert_eq!(p.bundle_id, fixture()["bundle_id"]);
    let value = serde_json::to_value(&p).unwrap();
    let obj = value.as_object().unwrap();
    let keys: std::collections::BTreeSet<_> = obj.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "version",
            "bundle_id",
            "stack_name",
            "use_vpn",
            "timezone",
            "media_identity",
            "layout",
            "members"
        ]
        .into_iter()
        .collect()
    );
    assert!(!obj.contains_key("values"));
    assert!(!obj.contains_key("done"));
    assert!(!obj.contains_key("complete"));
    let text = serde_json::to_string(&p).unwrap();
    for field in ["VPN_ENV_FILE", "apiKey", "password", "token", "config.xml"] {
        assert!(!text.contains(field));
    }
}
#[test]
fn literal_case_and_spaces_survive_derived_paths_and_profile_roots() {
    let target = target();
    let mut c = contract(0);
    c.locations.media_host = "/mnt/storage/Family Media".into();
    c.locations.application_config_host = "/var/lib/Family Apps".into();
    let r = prepare(
        101,
        &target,
        &c,
        &options(),
        Some(sample(&target, false, &[], &[])),
    )
    .unwrap();
    assert_eq!(r.directories[1].host, "/mnt/storage/Family Media/tv");
    assert_eq!(
        r.profile.unwrap().layout.config_root,
        "/var/lib/Family Apps"
    );
}
#[test]
fn overlapping_or_template_roots_and_generated_path_overflow_refuse() {
    let mut c = contract(0);
    c.locations.application_config_host = format!("{}/apps", c.locations.media_host);
    assert!(c.validate().is_ok());
    assert_eq!(
        prepare(101, &target(), &c, &options(), None).unwrap_err(),
        PreparationError::OverlappingRoots
    );
    let mut c = contract(0);
    c.locations.application_config_host = "/var/lib/${APP}".into();
    assert_eq!(
        prepare(101, &target(), &c, &options(), None).unwrap_err(),
        PreparationError::InvalidInput
    );
    let mut c = contract(0);
    c.locations.media_host = format!("/mnt/storage/{}/{}", "a".repeat(240), "b".repeat(250));
    assert!(c.validate().is_ok());
    assert!(c.locations.media_host.len() <= 512);
    assert_eq!(
        prepare(101, &target(), &c, &options(), None).unwrap_err(),
        PreparationError::InvalidInput
    );
}
#[test]
fn invalid_numeric_identity_timezone_clock_and_target_refuse() {
    let mut c = contract(0);
    c.media_identity.uid = 0;
    assert!(prepare(101, &target(), &c, &options(), None).is_err());
    let mut opts = options();
    opts.requested_identity = Some(StorageMediaIdentity { uid: 0, gid: 1000 });
    assert!(prepare(101, &target(), &contract(0), &opts, None).is_err());
    for zone in [
        "",
        "Europe/../London",
        "/etc/localtime",
        "Europe/London\n",
        "${ZONE}",
        "a//b",
    ] {
        opts.timezone = zone.into();
        opts.requested_identity = None;
        assert!(prepare(101, &target(), &contract(0), &opts, None).is_err());
    }
    for zone in ["UTC", "Etc/GMT+1", "America/Argentina/Buenos_Aires"] {
        opts.timezone = zone.into();
        assert!(prepare(101, &target(), &contract(0), &opts, None).is_ok());
    }
    assert!(prepare(-1, &target(), &contract(0), &options(), None).is_err());
    let mut owner = target();
    owner.project.name = "INVALID".into();
    assert!(prepare(101, &owner, &contract(0), &options(), None).is_err());
}
#[test]
fn incoherent_absence_dangling_mappings_and_duplicate_services_refuse() {
    for (exists, declared, entries) in [
        (false, vec!["vpn".into()], vec![]),
        (true, vec![], vec![installation(Member::Vpn)]),
        (true, vec!["vpn".into(), "vpn".into()], vec![]),
        (
            true,
            vec!["vpn".into()],
            vec![installation(Member::Vpn), installation(Member::Vpn)],
        ),
    ] {
        assert!(
            prepare(
                101,
                &target(),
                &contract(0),
                &options(),
                Some(sample(&target(), exists, &declared, &entries))
            )
            .is_err()
        );
    }
}
#[test]
fn source_record_limits_refuse_without_truncating_other_apps() {
    let declared: Vec<_> = (0..16).map(|i| format!("service{i}")).collect();
    let entries: Vec<_> = (0..16)
        .map(|i| Installation {
            key: InstallationKey {
                app: format!("app{i}"),
                stack: target().stack,
            },
            service: format!("service{i}"),
            ownership: Ownership::Managed,
        })
        .collect();
    let result = run(true, &declared, &entries);
    assert_eq!(result.outcome, Outcome::Blocked);
    assert!(result.issues.contains(&Issue::ServiceCapacityExceeded));
    let mut declared = declared;
    declared.push("service16".into());
    assert_eq!(
        prepare(
            101,
            &target(),
            &contract(0),
            &options(),
            Some(sample(&target(), true, &declared, &entries))
        )
        .unwrap_err(),
        PreparationError::Limit
    );
    let mut entries = entries;
    entries.push(entries[0].clone());
    assert_eq!(
        prepare(
            101,
            &target(),
            &contract(0),
            &options(),
            Some(sample(&target(), true, &declared[..16], &entries))
        )
        .unwrap_err(),
        PreparationError::Limit
    );
}
#[test]
fn effective_service_capacity_counts_other_apps_before_proposing_bundle_additions() {
    let declared: Vec<_> = (0..8).map(|i| format!("other{i}")).collect();
    let r = run(true, &declared, &[]);
    assert_eq!(r.install_candidates.len(), 8);
    let mut declared = declared;
    declared.push("other8".into());
    let r = run(true, &declared, &[]);
    assert_eq!(r.outcome, Outcome::Blocked);
    assert!(r.install_candidates.is_empty());
    assert!(r.profile.is_none());
    assert!(r.issues.contains(&Issue::ServiceCapacityExceeded));
    let mut declared: Vec<_> = (0..8).map(|i| format!("other{i}")).collect();
    declared.push("vpn".into());
    let r = run(true, &declared, &[installation(Member::Vpn)]);
    assert_eq!(r.install_candidates.len(), 7);
    assert!(!r.issues.contains(&Issue::ServiceCapacityExceeded));
}
#[test]
fn public_preview_streaming_ceiling_refuses_oversized_mutations() {
    let mut r = new();
    r.profile.as_mut().unwrap().timezone = "x".repeat(MAX_REPORT_BYTES);
    assert_eq!(r.check_size().unwrap_err(), PreparationError::Limit);
}
