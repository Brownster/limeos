use crate::{
    compose_management::catalog::{
        ManagedFileExpectation, ManagementCatalog, ManagementCatalogEvidence,
    },
    docker::compose_evidence::{ComposeOwnership, EngineComposeEvidence, EngineComposeSnapshot},
};
use limeos_domain::deployment::discovery::{
    self as model, CatalogInput, Declaration, EngineInput, FileStatus, Membership, ProjectIdentity,
    StackInventory,
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
    if cancelled() {
        return Err(Error(ErrorCode::Conflict));
    }
    let at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error(ErrorCode::Unavailable))?
        .as_secs();
    let report = project(
        catalog.catalog(),
        catalog.snapshot().observed_at,
        engine.snapshot(),
        i64::try_from(at).map_err(|_| Error(ErrorCode::Unavailable))?,
    )?;
    catalog.revalidate(cancelled)?;
    if cancelled() {
        return Err(Error(ErrorCode::Conflict));
    }
    Ok(report)
}

fn project(
    catalog: &ManagementCatalog,
    catalog_at: i64,
    engine: &EngineComposeSnapshot,
    at: i64,
) -> Result<StackInventory> {
    catalog.validate()?;
    if engine.version != 1 || engine.containers.len() > model::MAX_MEMBERS {
        return Err(Error(ErrorCode::Unavailable));
    }
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
    model::discover(
        at,
        Some(CatalogInput {
            engine_id: &catalog.engine_id,
            sampled_at: catalog_at,
            stacks: &declarations,
        }),
        Some(EngineInput {
            engine_id: &engine.engine_id,
            sampled_at: engine.containers_observed_at,
            members: &members,
        }),
    )
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
}
