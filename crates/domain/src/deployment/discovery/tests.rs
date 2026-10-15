use super::*;

fn declaration() -> Declaration {
    Declaration {
        stack: "media".into(),
        project: "media".into(),
        working_directory: "/opt/stacks/Media".into(),
        config_files: vec!["/opt/stacks/Media/compose.yaml".into()],
        services: vec!["web".into()],
        files: FileStatus::NotInspected,
    }
}
fn project() -> ProjectIdentity {
    ProjectIdentity {
        name: "media".into(),
        working_directory: "/opt/stacks/Media".into(),
        config_files: declaration().config_files,
    }
}
fn member(index: usize, service: &str, running: bool, one_off: bool) -> Membership {
    Membership::Compose {
        container: ContainerSnapshot {
            resource: format!("container:{index:064x}"),
            image: format!("sha256:{}", "a".repeat(64)),
            started_at: "2026-10-15T00:00:00Z".into(),
            running,
        },
        project: project(),
        service: service.into(),
        one_off,
    }
}
fn read(stacks: &[Declaration], members: &[Membership]) -> StackInventory {
    discover(
        101,
        Some(CatalogInput {
            engine_id: "engine",
            sampled_at: 100,
            stacks,
        }),
        Some(EngineInput {
            engine_id: "engine",
            sampled_at: 100,
            members,
        }),
    )
    .unwrap()
}

#[derive(Deserialize)]
struct Scenario {
    name: String,
    services: Vec<String>,
    members: Vec<(String, bool, bool)>,
    runtime: String,
    missing: u16,
    ordinary: u16,
    running: u16,
    one_offs: u16,
    #[serde(default)]
    drift: bool,
}

#[test]
fn frontend_wire_fixture_matches_actual_rust_serialization() {
    let actual = read(
        &[declaration()],
        &[
            member(0, "web", true, false),
            member(1, "web", false, false),
            member(2, "web", true, true),
        ],
    );
    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/deployment/discovery/wire.json"
    ))
    .unwrap();
    assert_eq!(serde_json::to_value(actual).unwrap(), expected);
}

#[test]
fn frozen_layout_and_replica_one_off_regressions() {
    let cases: Vec<Scenario> = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/deployment/discovery/scenarios.json"
    ))
    .unwrap();
    for case in cases {
        let declaration = Declaration {
            services: case.services,
            ..declaration()
        };
        let members: Vec<_> = case
            .members
            .iter()
            .enumerate()
            .map(|(i, (service, running, one_off))| member(i, service, *running, *one_off))
            .collect();
        let report = read(&[declaration], &members);
        let stack = &report.stacks[0];
        assert_eq!(
            serde_json::to_value(stack.runtime).unwrap(),
            case.runtime,
            "{}",
            case.name
        );
        assert_eq!(stack.missing_services, Some(case.missing), "{}", case.name);
        let counts = stack.counts.unwrap();
        assert_eq!(
            (counts.ordinary, counts.running, counts.one_offs),
            (case.ordinary, case.running, case.one_offs),
            "{}",
            case.name
        );
        assert_eq!(
            stack.issues.contains(&StackIssue::UndeclaredServices),
            case.drift,
            "{}",
            case.name
        );
        assert_eq!(
            stack
                .services
                .iter()
                .map(|s| s.members.len())
                .sum::<usize>(),
            members.len()
        );
    }
}

#[test]
fn unavailable_stale_future_and_wrong_engine_produce_unknown_not_zero() {
    let declarations = [declaration()];
    for engine in [
        None,
        Some(EngineInput {
            engine_id: "engine",
            sampled_at: 0,
            members: &[],
        }),
        Some(EngineInput {
            engine_id: "engine",
            sampled_at: 102,
            members: &[],
        }),
        Some(EngineInput {
            engine_id: "wrong",
            sampled_at: 100,
            members: &[],
        }),
    ] {
        let report = discover(
            101,
            Some(CatalogInput {
                engine_id: "engine",
                sampled_at: 100,
                stacks: &declarations,
            }),
            engine,
        )
        .unwrap();
        let stack = &report.stacks[0];
        assert_eq!(stack.runtime, RuntimeStatus::Unknown);
        assert_eq!(stack.counts, None);
        assert_eq!(stack.missing_services, None);
        assert_eq!(stack.services[0].runtime, RuntimeStatus::Unknown);
        assert_eq!(stack.services[0].counts, None);
    }
}

#[test]
fn catalog_failure_does_not_label_projects_unregistered() {
    let members = [member(0, "web", true, false)];
    for catalog in [
        None,
        Some(CatalogInput {
            engine_id: "engine",
            sampled_at: 0,
            stacks: &[],
        }),
    ] {
        let report = discover(
            101,
            catalog,
            Some(EngineInput {
                engine_id: "engine",
                sampled_at: 100,
                members: &members,
            }),
        )
        .unwrap();
        assert_eq!(
            report.other_projects[0].relation,
            RegistrationRelation::CatalogUnknown
        );
    }
    assert_eq!(
        read(&[], &members).other_projects[0].relation,
        RegistrationRelation::NotRegistered
    );
}

#[test]
fn exact_case_paths_file_order_and_project_identity_are_required() {
    let mut declared = declaration();
    declared
        .config_files
        .push("/opt/stacks/Media/limeos.override.json".into());
    for identity in [
        ProjectIdentity {
            working_directory: "/opt/stacks/media".into(),
            ..project()
        },
        ProjectIdentity {
            config_files: declared.config_files.iter().rev().cloned().collect(),
            ..project()
        },
        ProjectIdentity {
            config_files: vec!["/opt/stacks/Media/docker-compose.yml".into()],
            ..project()
        },
    ] {
        let mut runtime = member(0, "web", true, false);
        if let Membership::Compose { project, .. } = &mut runtime {
            *project = identity;
        }
        let report = read(&[declared.clone()], &[runtime]);
        assert_eq!(report.stacks[0].runtime, RuntimeStatus::Conflict);
        assert_eq!(report.stacks[0].counts, None);
        assert_eq!(
            report.other_projects[0].relation,
            RegistrationRelation::IdentityConflict
        );
    }
}

#[test]
fn foreign_overlap_blocks_counts_but_sibling_prefix_does_not() {
    for (directory, conflict) in [
        ("/opt/stacks/Media/sub", true),
        ("/opt/stacks/Media2", false),
    ] {
        let mut foreign = member(0, "web", true, false);
        if let Membership::Compose { project, .. } = &mut foreign {
            project.name = "other".into();
            project.working_directory = directory.into();
            project.config_files = vec![format!("{directory}/compose.yaml")];
        }
        let report = read(&[declaration()], &[foreign]);
        assert_eq!(
            report.stacks[0].runtime == RuntimeStatus::Conflict,
            conflict
        );
        assert_eq!(
            report.other_projects[0].relation,
            if conflict {
                RegistrationRelation::Overlapping
            } else {
                RegistrationRelation::NotRegistered
            }
        );
    }
}

#[test]
fn file_conflict_and_unsupported_diagnostics_survive_without_action_authority() {
    for files in [
        FileStatus::Conflict {
            candidates: vec![
                ComposeFileName::ComposeYaml,
                ComposeFileName::DockerComposeYml,
            ],
        },
        FileStatus::Unsupported {
            reason: UnsupportedSource::Environment,
        },
        FileStatus::Unavailable,
        FileStatus::NotInspected,
    ] {
        let report = read(
            &[Declaration {
                files: files.clone(),
                ..declaration()
            }],
            &[],
        );
        assert_eq!(report.stacks[0].files, files);
    }
}

#[test]
fn standalone_is_separate_and_output_is_deterministic() {
    let a = member(0, "web", true, false);
    let b = member(1, "web", false, true);
    let c = Membership::Standalone {
        container: member(2, "web", false, false).container().clone(),
    };
    assert_eq!(
        read(&[declaration()], &[a.clone(), b.clone(), c.clone()]),
        read(&[declaration()], &[c, b, a])
    );
    assert_eq!(
        read(
            &[],
            &[Membership::Standalone {
                container: member(3, "web", false, false).container().clone()
            }]
        )
        .standalone_containers,
        Some(1)
    );
}

#[test]
fn duplicate_catalog_and_member_identity_are_refused() {
    assert!(
        discover(
            101,
            Some(CatalogInput {
                engine_id: "engine",
                sampled_at: 100,
                stacks: &[declaration(), declaration()]
            }),
            None
        )
        .is_err()
    );
    let same = member(0, "web", true, false);
    assert!(
        discover(
            101,
            None,
            Some(EngineInput {
                engine_id: "engine",
                sampled_at: 100,
                members: &[same.clone(), same]
            })
        )
        .is_err()
    );
    let mut other = member(1, "web", true, false);
    if let Membership::Compose { project, .. } = &mut other {
        project.working_directory.push_str("-changed");
    }
    assert!(
        discover(
            101,
            None,
            Some(EngineInput {
                engine_id: "engine",
                sampled_at: 100,
                members: &[member(0, "web", true, false), other]
            })
        )
        .is_err()
    );
}

#[test]
fn aggregate_text_budget_rejects_large_valid_fields() {
    fn long_path(index: usize) -> String {
        format!(
            "/d{index}/{}/{}/{}",
            "a".repeat(240),
            "b".repeat(240),
            "c".repeat(20)
        )
    }
    let declarations: Vec<_> = (0..MAX_STACKS)
        .map(|i| Declaration {
            stack: format!("stack-{i}"),
            project: format!("project-{i}"),
            working_directory: long_path(i),
            config_files: (0..4).map(|j| long_path(100 + j)).collect(),
            services: vec!["web".into()],
            files: FileStatus::NotInspected,
        })
        .collect();
    let members: Vec<_> = (0..MAX_MEMBERS)
        .map(|i| {
            let mut member = member(i, "web", true, false);
            if let Membership::Compose { project, .. } = &mut member {
                project.working_directory = long_path(999);
                project.config_files = (0..4).map(|j| long_path(100 + j)).collect();
            }
            member
        })
        .collect();
    assert_eq!(
        discover(
            101,
            Some(CatalogInput {
                engine_id: "engine",
                sampled_at: 100,
                stacks: &declarations
            }),
            Some(EngineInput {
                engine_id: "engine",
                sampled_at: 100,
                members: &members
            })
        )
        .unwrap_err()
        .0,
        ErrorCode::Unavailable
    );
}

#[test]
fn bounds_are_explicit_and_report_size_is_streamed() {
    let oversized = vec![declaration(); MAX_STACKS + 1];
    assert_eq!(
        discover(
            101,
            Some(CatalogInput {
                engine_id: "engine",
                sampled_at: 100,
                stacks: &oversized
            }),
            None
        )
        .unwrap_err()
        .0,
        ErrorCode::Unavailable
    );
    let members: Vec<_> = (0..=MAX_MEMBERS)
        .map(|i| member(i, "web", true, false))
        .collect();
    assert_eq!(
        discover(
            101,
            None,
            Some(EngineInput {
                engine_id: "engine",
                sampled_at: 100,
                members: &members
            })
        )
        .unwrap_err()
        .0,
        ErrorCode::Unavailable
    );
    let mut report = read(&[declaration()], &[]);
    report.stacks[0].working_directory = "x".repeat(MAX_REPORT_BYTES);
    assert_eq!(report.check_size().unwrap_err().0, ErrorCode::Unavailable);
}

#[test]
fn sample_time_boundary_is_inclusive_and_reads_preserve_sample_times() {
    let declarations = [declaration()];
    let report = discover(
        190,
        Some(CatalogInput {
            engine_id: "engine",
            sampled_at: 100,
            stacks: &declarations,
        }),
        Some(EngineInput {
            engine_id: "engine",
            sampled_at: 100,
            members: &[],
        }),
    )
    .unwrap();
    assert_eq!(report.engine.state, SourceState::Fresh);
    assert_eq!(report.engine.sampled_at, Some(100));
    let report = discover(
        191,
        Some(CatalogInput {
            engine_id: "engine",
            sampled_at: 100,
            stacks: &declarations,
        }),
        Some(EngineInput {
            engine_id: "engine",
            sampled_at: 100,
            members: &[],
        }),
    )
    .unwrap();
    assert_eq!(report.stacks[0].runtime, RuntimeStatus::Unknown);
}
