use super::*;
use serde_json::{Value, json};

fn spec(policy: Policy) -> PackageSpec {
    PackageSpec {
        name: "alpha".into(),
        manager: Manager::Apt,
        policy,
        critical: false,
        disable_self_update: false,
        feature: None,
    }
}
fn present(version: &str) -> Installed {
    Installed::Present {
        version: version.into(),
        selection: Selection::Install,
    }
}
fn facts(installed: Installed, candidate: Option<&str>, versions: &[&str]) -> PackageFacts {
    PackageFacts {
        name: "alpha".into(),
        installed,
        repository: Repository::Known {
            candidate: candidate.map(Into::into),
            versions: versions.iter().map(|s| (*s).into()).collect(),
        },
    }
}
fn one(spec: &PackageSpec, fact: &PackageFacts) -> Assessment {
    plan_baseline(
        std::slice::from_ref(spec),
        std::slice::from_ref(fact),
        FeatureState::Unknown,
    )
    .unwrap()
    .summary()
    .rows[0]
        .clone()
}
fn pinned() -> PackageSpec {
    spec(Policy::Pinned {
        version: "2.1.207".into(),
    })
}

#[test]
fn frozen_policy_and_revision_regressions() {
    let fixtures: Vec<Value> = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/deployment/packages/frozen-policies.json"
    ))
    .unwrap();
    assert_eq!(fixtures.len(), 10);
    for case in fixtures {
        let mut declaration = spec(serde_json::from_value(case["policy"].clone()).unwrap());
        declaration.critical = case["critical"].as_bool().unwrap();
        let installed = match case["installed"].as_str() {
            Some(version) => Installed::Present {
                version: version.into(),
                selection: serde_json::from_value(case["selection"].clone()).unwrap(),
            },
            None => Installed::Absent {},
        };
        let observation = PackageFacts {
            name: "alpha".into(),
            installed,
            repository: Repository::Known {
                candidate: case["candidate"].as_str().map(Into::into),
                versions: serde_json::from_value(case["versions"].clone()).unwrap(),
            },
        };
        let row = serde_json::to_value(one(&declaration, &observation)).unwrap();
        for key in ["compliant", "reason", "status", "steps"] {
            assert_eq!(row[key], case[key], "{}: {key}", case["label"]);
        }
        assert_eq!(row["pending_update"], case["pending"], "{}", case["label"]);
    }
}

#[derive(Deserialize)]
struct Oracle {
    versions: Vec<String>,
    ordering: Vec<Vec<i8>>,
    oracle: String,
}
#[test]
fn debian_order_matches_all_recorded_dpkg_pairs() {
    let oracle: Oracle = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/deployment/packages/version-order.json"
    ))
    .unwrap();
    assert!(oracle.oracle.starts_with("Debian 'dpkg'"));
    assert_eq!(oracle.ordering.len(), oracle.versions.len());
    for (i, a) in oracle.versions.iter().enumerate() {
        assert_eq!(oracle.ordering[i].len(), oracle.versions.len());
        for (j, b) in oracle.versions.iter().enumerate() {
            let expected = oracle.ordering[i][j].cmp(&0);
            assert_eq!(compare_versions(a, b).unwrap(), expected, "{a} vs {b}");
        }
    }
}

#[test]
fn conservative_version_and_package_validation_refuses_unsafe_inputs() {
    for value in [
        "",
        "-1",
        "alpha",
        "1:",
        ":1",
        "a:1",
        "1:2:3",
        "1-",
        "1-1-",
        "1_2",
        "1 2",
        "1\n",
        "1;id",
        "1000000000:1",
        "é1",
    ] {
        assert!(!version_valid(value), "{value:?}");
        assert_eq!(
            compare_versions(value, "1"),
            Err(PackageError::InvalidInput)
        );
    }
    assert!(!version_valid(&"1".repeat(MAX_VERSION_BYTES + 1)));
    for name in [
        "", "a", "--help", "Alpha", "a:b", "a_b", "a;id", "a\n", "a/b",
    ] {
        assert!(!package_name(name), "{name:?}");
    }
    assert!(package_name("libc6-dev"));
    assert!(package_name("3dchess"));
}

#[test]
fn unknown_and_missing_observations_never_plan_install_or_removal() {
    for policy in [
        Policy::Present {},
        Policy::Absent {},
        Policy::Minimum {
            version: "1".into(),
        },
        pinned().policy,
    ] {
        let declaration = spec(policy);
        let row = one(
            &declaration,
            &facts(Installed::Unknown {}, Some("2"), &["2"]),
        );
        assert_eq!(row.status, Status::Unknown);
        assert_eq!(row.compliant, None);
        assert!(row.steps.is_empty());
        let plan = plan_baseline(&[declaration], &[], FeatureState::Unknown).unwrap();
        assert_eq!(plan.summary().rows[0].status, Status::Unknown);
        assert!(plan.summary().rows[0].facts.is_none());
    }
}

#[test]
fn known_absence_requires_an_exact_available_candidate() {
    let mut observed = facts(Installed::Absent {}, None, &[]);
    assert_eq!(
        one(&spec(Policy::Present {}), &observed).reason,
        Reason::CandidateUnavailable
    );
    observed.repository = Repository::Unknown {};
    let row = one(&spec(Policy::Present {}), &observed);
    assert_eq!(row.reason, Reason::UnknownRepository);
    assert!(row.steps.is_empty());
    let row = one(
        &spec(Policy::Present {}),
        &facts(Installed::Absent {}, Some("1.0-1"), &["1.0-1"]),
    );
    assert_eq!(
        row.steps,
        vec![Step::InstallExact {
            version: "1.0-1".into()
        }]
    );
}

#[test]
fn pinned_install_chooses_highest_offered_revision_not_unoffered_upstream_text() {
    let row = one(
        &pinned(),
        &facts(
            Installed::Absent {},
            Some("2.2"),
            &["2.1.207-2", "2.2", "2.1.207-10", "2.1.207-1"],
        ),
    );
    assert_eq!(
        row.steps,
        vec![
            Step::InstallExact {
                version: "2.1.207-10".into()
            },
            Step::Hold {
                version: "2.1.207-10".into()
            }
        ]
    );
    let row = one(
        &pinned(),
        &facts(Installed::Absent {}, Some("2.2"), &["2.2"]),
    );
    assert_eq!(row.reason, Reason::PinnedVersionUnavailable);
    assert!(row.steps.is_empty());
}

#[test]
fn minimum_uses_full_epoch_tilde_and_revision_order() {
    for (minimum, installed, compliant) in [
        ("1:1.0", "2.0", false),
        ("1.0", "1.0~rc1", false),
        ("1.0-2", "1.0-10", true),
    ] {
        let row = one(
            &spec(Policy::Minimum {
                version: minimum.into(),
            }),
            &facts(present(installed), Some("2:1.0"), &["2:1.0"]),
        );
        assert_eq!(row.compliant, Some(compliant));
    }
}

#[test]
fn below_minimum_candidate_does_not_make_an_unversioned_upgrade() {
    let row = one(
        &spec(Policy::Minimum {
            version: "24".into(),
        }),
        &facts(present("20"), Some("23"), &["23", "25"]),
    );
    assert_eq!(row.reason, Reason::CandidateUnavailable);
    assert!(row.steps.is_empty());
}

#[test]
fn pinned_downgrade_and_equivalent_spelling_require_intervention() {
    let row = one(
        &pinned(),
        &facts(present("2.2"), Some("2.2"), &["2.2", "2.1.207-1"]),
    );
    assert_eq!(row.reason, Reason::DowngradeNeedsReview);
    assert!(row.steps.is_empty());
    let declaration = spec(Policy::Pinned {
        version: "1".into(),
    });
    let row = one(&declaration, &facts(present("01"), Some("1"), &["1"]));
    assert_eq!(row.reason, Reason::EquivalentPinNeedsReview);
    assert!(row.steps.is_empty());
}

#[test]
fn holds_are_not_automatically_released_for_changes() {
    let held = Installed::Present {
        version: "1.0".into(),
        selection: Selection::Hold,
    };
    for policy in [
        Policy::Absent {},
        pinned().policy,
        Policy::Minimum {
            version: "2".into(),
        },
    ] {
        let row = one(
            &spec(policy),
            &facts(held.clone(), Some("2.1.207-1"), &["2.1.207-1"]),
        );
        assert_eq!(row.reason, Reason::HeldChangeNeedsReview);
        assert!(row.steps.is_empty());
    }
}

#[test]
fn broken_and_removal_selected_packages_never_become_compliant() {
    for (installed, reason) in [
        (Installed::Broken {}, Reason::BrokenPackage),
        (
            Installed::Present {
                version: "1".into(),
                selection: Selection::Purge,
            },
            Reason::SelectedForRemoval,
        ),
        (
            Installed::Present {
                version: "1".into(),
                selection: Selection::Deinstall,
            },
            Reason::SelectedForRemoval,
        ),
        (
            Installed::Present {
                version: "1".into(),
                selection: Selection::Unknown,
            },
            Reason::UnknownSelection,
        ),
    ] {
        let row = one(&spec(Policy::Present {}), &facts(installed, None, &[]));
        assert_eq!(row.reason, reason);
        assert_eq!(row.compliant, None);
        assert!(row.steps.is_empty());
    }
}

#[test]
fn feature_scope_fails_closed_without_removing_feature_packages() {
    let mut declaration = pinned();
    declaration.feature = Some(Feature::AiAgents);
    for state in [FeatureState::Disabled, FeatureState::Unknown] {
        let plan = plan_baseline(std::slice::from_ref(&declaration), &[], state).unwrap();
        assert_eq!(plan.summary().rows[0].status, Status::Inactive);
        assert!(plan.summary().rows[0].steps.is_empty());
    }
    let plan = plan_baseline(&[declaration], &[], FeatureState::Enabled).unwrap();
    assert_eq!(plan.summary().rows[0].status, Status::Unknown);
}

#[test]
fn unsupported_managers_and_self_updater_controls_are_explicitly_blocked() {
    for manager in [Manager::Pip, Manager::Npm, Manager::Claude] {
        let mut declaration = spec(Policy::Present {});
        declaration.manager = manager;
        let row = one(
            &declaration,
            &facts(Installed::Absent {}, Some("1"), &["1"]),
        );
        assert_eq!(row.reason, Reason::UnsupportedManager);
        assert!(row.steps.is_empty());
    }
    let mut declaration = pinned();
    declaration.disable_self_update = true;
    let row = one(
        &declaration,
        &facts(Installed::Absent {}, Some("2.1.207-1"), &["2.1.207-1"]),
    );
    assert_eq!(row.reason, Reason::UnsupportedSelfUpdateControl);
    assert!(row.steps.is_empty());
}

#[test]
fn pending_updates_distinguish_unknown_no_candidate_and_revision_only() {
    let mut declaration = spec(Policy::Present {});
    declaration.critical = true;
    let mut observed = facts(present("1"), None, &[]);
    assert_eq!(
        one(&declaration, &observed).pending_update,
        PendingUpdate::NoCandidate
    );
    observed.repository = Repository::Unknown {};
    assert_eq!(
        one(&declaration, &observed).pending_update,
        PendingUpdate::Unknown
    );
    assert_eq!(
        one(
            &declaration,
            &facts(Installed::Unknown {}, Some("2"), &["2"])
        )
        .pending_update,
        PendingUpdate::Unknown
    );
    assert_eq!(
        one(
            &declaration,
            &facts(present("1.0-1"), Some("1.0-2"), &["1.0-2"])
        )
        .pending_update,
        PendingUpdate::None
    );
    assert_eq!(
        one(&declaration, &facts(present("2"), Some("1"), &["1"])).pending_update,
        PendingUpdate::None
    );
}

#[test]
fn duplicate_foreign_and_inconsistent_repository_facts_are_refused() {
    let declaration = spec(Policy::Present {});
    let observed = facts(Installed::Absent {}, Some("2"), &["1"]);
    assert!(
        plan_baseline(
            std::slice::from_ref(&declaration),
            &[observed],
            FeatureState::Unknown
        )
        .is_err()
    );
    let observed = facts(Installed::Absent {}, Some("1"), &["1", "1"]);
    assert!(
        plan_baseline(
            std::slice::from_ref(&declaration),
            &[observed],
            FeatureState::Unknown
        )
        .is_err()
    );
    let mut observed = facts(Installed::Absent {}, None, &[]);
    observed.name = "foreign".into();
    assert!(
        plan_baseline(
            std::slice::from_ref(&declaration),
            &[observed],
            FeatureState::Unknown
        )
        .is_err()
    );
    assert!(
        plan_baseline(
            &[declaration.clone(), declaration.clone()],
            &[],
            FeatureState::Unknown
        )
        .is_err()
    );
    let observed = facts(Installed::Absent {}, None, &[]);
    assert!(
        plan_baseline(
            &[declaration],
            &[observed.clone(), observed],
            FeatureState::Unknown
        )
        .is_err()
    );
}

#[test]
fn field_count_and_aggregate_limits_precede_proposal_clones() {
    let specs: Vec<_> = (0..=MAX_PACKAGES)
        .map(|i| {
            let mut s = spec(Policy::Present {});
            s.name = format!("pkg-{i}");
            s
        })
        .collect();
    assert_eq!(
        plan_baseline(&specs, &[], FeatureState::Unknown).unwrap_err(),
        PackageError::Limit
    );
    let mut observed = facts(Installed::Absent {}, None, &[]);
    observed.repository = Repository::Known {
        candidate: None,
        versions: (0..=MAX_OFFERED).map(|i| i.to_string()).collect(),
    };
    assert_eq!(
        plan_baseline(
            &[spec(Policy::Present {})],
            &[observed],
            FeatureState::Unknown
        )
        .unwrap_err(),
        PackageError::Limit
    );
    let specs: Vec<_> = (0..MAX_PACKAGES)
        .map(|i| {
            let mut s = spec(Policy::Present {});
            s.name = format!("pkg-{i}");
            s
        })
        .collect();
    let observations: Vec<_> = specs
        .iter()
        .map(|s| PackageFacts {
            name: s.name.clone(),
            installed: present(&"1".repeat(128)),
            repository: Repository::Known {
                candidate: None,
                versions: (0..MAX_OFFERED)
                    .map(|i| format!("{i}{}", "1".repeat(127)))
                    .collect(),
            },
        })
        .collect();
    assert_eq!(
        plan_baseline(&specs, &observations, FeatureState::Unknown).unwrap_err(),
        PackageError::Limit
    );
}

#[test]
fn unknown_fields_and_request_approval_shortcuts_cannot_enter_models() {
    let mut value = serde_json::to_value(spec(Policy::Present {})).unwrap();
    value["approved"] = json!(true);
    assert!(serde_json::from_value::<PackageSpec>(value).is_err());
    assert!(serde_json::from_value::<Policy>(json!({"kind":"present","version":"1"})).is_err());
    assert!(
        serde_json::from_value::<Repository>(
            json!({"kind":"known","candidate":null,"versions":[],"trusted":true})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<Repository>(json!({"kind":"unknown","trusted":true})).is_err()
    );
    for kind in ["absent", "unknown", "broken"] {
        assert!(serde_json::from_value::<Installed>(json!({"kind":kind,"version":"1"})).is_err());
    }
}

#[test]
fn deterministic_summary_retains_all_unresolved_effect_gates() {
    let mut alpha = spec(Policy::Present {});
    let mut zulu = alpha.clone();
    zulu.name = "zulu".into();
    let a = plan_baseline(&[zulu.clone(), alpha.clone()], &[], FeatureState::Unknown).unwrap();
    let b = plan_baseline(&[alpha.clone(), zulu], &[], FeatureState::Unknown).unwrap();
    assert_eq!(
        serde_json::to_value(a.summary()).unwrap(),
        serde_json::to_value(b.summary()).unwrap()
    );
    assert_eq!(a.summary().unresolved.len(), 8);
    alpha.name = "private-package".into();
    let plan = plan_baseline(&[alpha], &[], FeatureState::Unknown).unwrap();
    assert!(!format!("{plan:?}").contains("private-package"));
}

#[test]
fn offered_version_order_cannot_change_the_exact_pin_or_review_summary() {
    let forward = facts(
        Installed::Absent {},
        Some("2.2"),
        &["2.1.207-01", "2.1.207-1", "2.2"],
    );
    let reversed = facts(
        Installed::Absent {},
        Some("2.2"),
        &["2.2", "2.1.207-1", "2.1.207-01"],
    );
    let a = one(&pinned(), &forward);
    let b = one(&pinned(), &reversed);
    assert_eq!(a, b);
    assert_eq!(
        a.steps[0],
        Step::InstallExact {
            version: "2.1.207-1".into()
        }
    );
}
