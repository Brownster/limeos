use crate::{
    compose_management::catalog::{
        ManagedFileExpectation, ManagementCatalog, ManagementCatalogEvidence,
    },
    docker::compose_evidence::{ComposeOwnership, EngineComposeEvidence, EngineComposeSnapshot},
};
use limeos_domain::deployment::discovery::{
    self as model, CatalogFacts, Declaration, DiscoveryCohort, EngineFacts, FileStatus, Membership,
    ProjectIdentity, StackInventory,
};
use limeos_domain::{COMPOSE_MANAGED_FILE, ComposeFileName, Error, ErrorCode, Result};
use std::time::{SystemTime, UNIX_EPOCH};

/// Reuse genuine owners and their original lifetimes/ledgers. No reader is
/// launched here; the central observation worker supplies one shared collection.
/// A returned serializable report is display data, never retained authority.
pub async fn inventory(
    catalog: &ManagementCatalogEvidence,
    engine: &EngineComposeEvidence,
    cancelled: &dyn Fn() -> bool,
) -> Result<StackInventory> {
    if cancelled() {
        return Err(Error(ErrorCode::Conflict));
    }
    catalog.revalidate(cancelled)?;
    engine.revalidate().await?;
    let report = project(
        catalog.catalog(),
        catalog.snapshot().observed_at,
        engine.snapshot(),
        now()?,
    )?;
    catalog.revalidate(cancelled)?;
    if cancelled() {
        return Err(Error(ErrorCode::Conflict));
    }
    Ok(report)
}

/// Select display facts only after each original owner's final revalidation.
/// Source refusal discards that source, while cancellation refuses the cohort.
/// The returned value cannot reconstruct either retained owner.
pub async fn cohort(
    catalog: Option<&ManagementCatalogEvidence>,
    engine: Option<&EngineComposeEvidence>,
    cancelled: &dyn Fn() -> bool,
) -> Result<DiscoveryCohort> {
    if cancelled() {
        return Err(Error(ErrorCode::Conflict));
    }
    let catalog = catalog.filter(|c| c.revalidate(cancelled).is_ok());
    let engine = match engine {
        Some(e) if e.revalidate().await.is_ok() => Some(e),
        _ => None,
    };
    if cancelled() {
        return Err(Error(ErrorCode::Conflict));
    }
    let catalog = catalog.filter(|c| c.revalidate(cancelled).is_ok());
    let catalog = catalog
        .map(|c| catalog_facts(c.catalog(), c.snapshot().observed_at))
        .transpose()?;
    let engine = engine.map(|e| engine_facts(e.snapshot())).transpose()?;
    let report = DiscoveryCohort::new(catalog, engine)?;
    if cancelled() {
        return Err(Error(ErrorCode::Conflict));
    }
    Ok(report)
}

fn now() -> Result<i64> {
    let at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error(ErrorCode::Unavailable))?
        .as_secs();
    i64::try_from(at).map_err(|_| Error(ErrorCode::Unavailable))
}

fn project(
    catalog: &ManagementCatalog,
    catalog_at: i64,
    engine: &EngineComposeSnapshot,
    at: i64,
) -> Result<StackInventory> {
    DiscoveryCohort::new(
        Some(catalog_facts(catalog, catalog_at)?),
        Some(engine_facts(engine)?),
    )?
    .project(at)
}

fn catalog_facts(catalog: &ManagementCatalog, sampled_at: i64) -> Result<CatalogFacts> {
    catalog.validate()?;
    let mut declarations = Vec::new();
    for registration in &catalog.stacks {
        let selection = registration.selection()?;
        let filename = match registration.operator.name {
            ComposeFileName::ComposeYaml => "compose.yaml",
            ComposeFileName::ComposeYml => "compose.yml",
            ComposeFileName::DockerComposeYaml => "docker-compose.yaml",
            ComposeFileName::DockerComposeYml => "docker-compose.yml",
        };
        let mut config_files = vec![format!(
            "{}/{}",
            selection.working_directory().display(),
            filename
        )];
        if matches!(registration.managed, ManagedFileExpectation::Present { .. }) {
            config_files.push(format!(
                "{}/{}",
                selection.working_directory().display(),
                COMPOSE_MANAGED_FILE
            ));
        }
        declarations.push(Declaration {
            stack: registration.stack.clone(),
            project: registration.project.clone(),
            working_directory: registration.working_directory.clone(),
            config_files,
            services: registration.services.clone(),
            files: FileStatus::NotInspected,
        });
    }
    Ok(CatalogFacts {
        engine_id: catalog.engine_id.clone(),
        sampled_at,
        stacks: declarations,
    })
}

fn engine_facts(engine: &EngineComposeSnapshot) -> Result<EngineFacts> {
    if engine.version != 1 || engine.containers.len() > model::MAX_ENGINE_CONTAINERS {
        return Err(Error(ErrorCode::Unavailable));
    }
    let members: Vec<_> = engine
        .containers
        .iter()
        .map(|member| match &member.ownership {
            ComposeOwnership::NonCompose => Membership::Standalone {
                container: member.container.clone(),
            },
            ComposeOwnership::Compose {
                project,
                service,
                one_off,
            } => Membership::Compose {
                container: member.container.clone(),
                project: ProjectIdentity {
                    name: project.name.clone(),
                    working_directory: project.working_directory.clone(),
                    config_files: project.config_files.clone(),
                },
                service: service.clone(),
                one_off: *one_off,
            },
        })
        .collect();
    Ok(EngineFacts {
        engine_id: engine.engine_id.clone(),
        sampled_at: engine.containers_observed_at,
        members,
        truncated: engine.truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose_management::catalog::{OperatorFingerprint, StackRegistration};
    use limeos_domain::ComposeFileName;

    fn catalog(managed: bool) -> ManagementCatalog {
        ManagementCatalog {
            version: 1,
            engine_id: "engine-a".into(),
            stacks: vec![StackRegistration {
                stack: "media".into(),
                project: "media".into(),
                working_directory: "/opt/stacks/Media".into(),
                operator: OperatorFingerprint {
                    name: ComposeFileName::DockerComposeYml,
                    sha256: "a".repeat(64),
                },
                managed: if managed {
                    ManagedFileExpectation::Present {
                        sha256: "b".repeat(64),
                    }
                } else {
                    ManagedFileExpectation::Absent {}
                },
                services: vec!["web".into()],
            }],
        }
    }
    #[test]
    fn projects_registered_empty_stacks_without_claiming_file_inspection() {
        let engine = EngineComposeSnapshot {
            version: 1,
            truncated: 0,
            engine_id: "engine-a".into(),
            containers_observed_at: 100,
            containers: vec![],
        };
        for managed in [false, true] {
            let report = project(&catalog(managed), 100, &engine, 101).unwrap();
            let stack = &report.stacks[0];
            assert_eq!(stack.runtime, model::RuntimeStatus::NoContainers);
            assert_eq!(stack.files, FileStatus::NotInspected);
            assert_eq!(
                stack.config_files[0],
                "/opt/stacks/Media/docker-compose.yml"
            );
            assert_eq!(stack.config_files.len(), if managed { 2 } else { 1 });
            if managed {
                assert_eq!(
                    stack.config_files[1],
                    "/opt/stacks/Media/limeos.override.json"
                );
            }
            assert_eq!(report.catalog.sampled_at, Some(100));
            assert_eq!(report.engine.sampled_at, Some(100));
        }
    }
    #[test]
    fn mismatched_engine_and_unsupported_versions_never_produce_empty_success() {
        let mut engine = EngineComposeSnapshot {
            version: 1,
            truncated: 0,
            engine_id: "other".into(),
            containers_observed_at: 100,
            containers: vec![],
        };
        let report = project(&catalog(false), 100, &engine, 101).unwrap();
        assert_eq!(report.engine_matches_catalog, Some(false));
        assert_eq!(report.stacks[0].runtime, model::RuntimeStatus::Unknown);
        assert_eq!(report.stacks[0].counts, None);
        engine.version = 2;
        assert!(project(&catalog(false), 100, &engine, 101).is_err());
    }
    fn snapshot(count: usize) -> EngineComposeSnapshot {
        use crate::docker::compose_evidence::{ComposeContainerMembership, ComposeProjectIdentity};
        EngineComposeSnapshot {
            version: 1,
            truncated: 0,
            engine_id: "engine-a".into(),
            containers_observed_at: 100,
            containers: (0..count)
                .map(|i| ComposeContainerMembership {
                    container: limeos_domain::ContainerSnapshot {
                        resource: format!("container:{i:064x}"),
                        image: format!("sha256:{}", "a".repeat(64)),
                        started_at: "2026-10-15T00:00:00Z".into(),
                        running: i % 2 == 0,
                    },
                    ownership: if i < 16 {
                        ComposeOwnership::Compose {
                            project: ComposeProjectIdentity {
                                name: "media".into(),
                                working_directory: "/opt/stacks/Media".into(),
                                config_files: vec!["/opt/stacks/Media/docker-compose.yml".into()],
                            },
                            service: format!("svc-{i}"),
                            one_off: false,
                        }
                    } else {
                        ComposeOwnership::NonCompose
                    },
                })
                .collect(),
        }
    }
    #[test]
    fn engine_wide_bound_admits_64_containers_and_refuses_65_whole() {
        let at_bound = engine_facts(&snapshot(model::MAX_ENGINE_CONTAINERS)).unwrap();
        assert_eq!(at_bound.members.len(), 64);
        let report = project(&catalog(false), 100, &snapshot(64), 101).unwrap();
        assert_eq!(report.engine.state, model::SourceState::Fresh);
        assert_eq!(report.standalone_containers, Some(48));
        // 16 stack members among 48 unrelated containers is plannable input.
        assert_eq!(report.stacks[0].counts.unwrap().ordinary, 16);
        assert_eq!(
            engine_facts(&snapshot(model::MAX_ENGINE_CONTAINERS + 1))
                .unwrap_err()
                .0,
            ErrorCode::Unavailable
        );
        assert!(project(&catalog(false), 100, &snapshot(65), 101).is_err());
    }
}
