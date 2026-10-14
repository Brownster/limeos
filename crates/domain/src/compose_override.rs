//! Pure proposals for the ordinary JSON override supported by the typed P03 model.
//!
//! Inputs are declarations, not parsed operator files or retained host evidence.
use crate::{
    COMPOSE_MANAGED_FILE, ComposeImpact, ComposeMount, ComposePort, ComposeProject, ComposeRestart,
    ComposeService, Error, ErrorCode, Result,
};
use serde::Serialize;
use std::{collections::BTreeMap, io::Write};

const ARTIFACT_BYTES: usize = 64 * 1024;
const SUMMARY_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnresolvedComposeGate {
    SupportedSourceAdmission,
    RetainedFileBinding,
    EngineRuntimeBinding,
    PhysicalMountsAndConsumers,
    PrincipalAndFreshApproval,
    StackAndStorageLocks,
    AtomicWriteAndRecovery,
    ShutdownAndEffectTimeVerification,
}
const GATES: [UnresolvedComposeGate; 8] = [
    UnresolvedComposeGate::SupportedSourceAdmission,
    UnresolvedComposeGate::RetainedFileBinding,
    UnresolvedComposeGate::EngineRuntimeBinding,
    UnresolvedComposeGate::PhysicalMountsAndConsumers,
    UnresolvedComposeGate::PrincipalAndFreshApproval,
    UnresolvedComposeGate::StackAndStorageLocks,
    UnresolvedComposeGate::AtomicWriteAndRecovery,
    UnresolvedComposeGate::ShutdownAndEffectTimeVerification,
];

/// A reviewable proposal, with no approval, claim, shutdown receipt or effect.
///
/// ```compile_fail
/// use limeos_domain::compose_override::ManagedOverridePlan;
/// let imported: ManagedOverridePlan = serde_json::from_str("{}").unwrap();
/// ```
pub struct ManagedOverridePlan {
    operator: ComposeProject,
    before: ComposeProject,
    desired: ComposeProject,
    impact: ComposeImpact,
    shutdown_required: Vec<String>,
    artifact: Vec<u8>,
}

/// Supplied declarations remain past proposal facts, not execution authority.
///
/// ```compile_fail
/// use limeos_domain::compose_override::ManagedOverrideSummary;
/// let imported: ManagedOverrideSummary<'_> = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Serialize)]
pub struct ManagedOverrideSummary<'a> {
    pub version: u16,
    pub managed_file: &'static str,
    pub operator: &'a ComposeProject,
    pub before: &'a ComposeProject,
    pub desired: &'a ComposeProject,
    pub impact: &'a ComposeImpact,
    pub shutdown_required: &'a [String],
    pub artifact_bytes: usize,
    pub unresolved: &'a [UnresolvedComposeGate],
}

impl ManagedOverridePlan {
    pub fn artifact(&self) -> &[u8] {
        &self.artifact
    }

    pub fn summary(&self) -> ManagedOverrideSummary<'_> {
        ManagedOverrideSummary {
            version: 1,
            managed_file: COMPOSE_MANAGED_FILE,
            operator: &self.operator,
            before: &self.before,
            desired: &self.desired,
            impact: &self.impact,
            shutdown_required: &self.shutdown_required,
            artifact_bytes: self.artifact.len(),
            unresolved: &GATES,
        }
    }
}

/// Compile a complete replacement managed file against one unchanged operator model.
///
/// Both supplied effective projects must be representable against the operator.
/// This function neither decodes actual Compose files nor verifies live membership.
pub fn plan_managed_override(
    operator: &ComposeProject,
    current: &ComposeProject,
    desired: &ComposeProject,
) -> Result<ManagedOverridePlan> {
    for project in [operator, current, desired] {
        preflight(project)?;
    }
    let (mut operator, mut before, mut desired) =
        (operator.clone(), current.clone(), desired.clone());
    for project in [&mut operator, &mut before, &mut desired] {
        project.normalize()?;
    }
    // Validate the declared current combination too, without trusting it as a preimage.
    drop(render(&operator, &before)?);
    let artifact = render(&operator, &desired)?;
    let impact = ComposeImpact::between(&before, &desired);
    let shutdown_required = impact
        .services
        .iter()
        .filter(|change| change.after.is_none())
        .map(|change| change.name.clone())
        .collect();
    let plan = ManagedOverridePlan {
        operator,
        before,
        desired,
        impact,
        shutdown_required,
        artifact,
    };
    serde_json::to_writer(CountWriter(0), &plan.summary())
        .map_err(|_| Error(ErrorCode::InvalidInput))?;
    Ok(plan)
}

// Admission precedes all project clones and bounds caller-constructed typed input.
fn preflight(project: &ComposeProject) -> Result<()> {
    let bounded = project.services.len() <= 16
        && project.services.iter().all(|s| {
            s.name.len() <= 48
                && s.image.len() <= 264
                && s.ports.len() <= 8
                && s.mounts.len() <= 8
                && s.devices.len() <= 8
                && s.ports.iter().all(|p| p.host_ip.len() <= 45)
                && s.mounts.iter().all(|m| match m {
                    ComposeMount::Volume { name, target, .. } => {
                        name.len() <= 48 && target.len() <= 256
                    }
                    ComposeMount::Bind { source, target, .. } => {
                        source.len() <= 256 && target.len() <= 256
                    }
                })
                && s.devices
                    .iter()
                    .all(|d| d.source.len() <= 256 && d.target.len() <= 256)
        });
    if bounded {
        Ok(())
    } else {
        Err(Error(ErrorCode::InvalidInput))
    }
}

fn target(mount: &ComposeMount) -> &str {
    match mount {
        ComposeMount::Volume { target, .. } | ComposeMount::Bind { target, .. } => target,
    }
}

fn representable(base: &ComposeService, desired: &ComposeService) -> bool {
    // Omission/empty sequences cannot erase an inherited value in ordinary JSON.
    (base.user.is_none() || desired.user.is_some())
        && (!base.host_network || desired.host_network)
        && base.ports.iter().all(|p| desired.ports.contains(p))
        && base.devices.iter().all(|d| desired.devices.contains(d))
        && base.mounts.iter().all(|m| {
            desired.mounts.iter().any(|n| {
                target(m) == target(n) && std::mem::discriminant(m) == std::mem::discriminant(n)
            })
        })
}

#[derive(Serialize)]
struct Document<'a> {
    services: BTreeMap<&'a str, Service<'a>>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    volumes: BTreeMap<&'a str, Empty>,
}
#[derive(Serialize)]
struct Empty {}
#[derive(Serialize)]
struct Service<'a> {
    image: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    restart: ComposeRestart,
    privileged: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    network_mode: Option<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    ports: Vec<Port<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    volumes: Vec<Mount<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    devices: Vec<String>,
}
#[derive(Serialize)]
struct Port<'a> {
    host_ip: &'a str,
    published: String,
    target: u16,
    protocol: crate::PortProtocol,
}
impl<'a> From<&'a ComposePort> for Port<'a> {
    fn from(port: &'a ComposePort) -> Self {
        Self {
            host_ip: &port.host_ip,
            published: port.published.to_string(),
            target: port.target,
            protocol: port.protocol,
        }
    }
}
#[derive(Serialize)]
struct Mount<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    source: &'a str,
    target: &'a str,
    read_only: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    bind: Option<Bind>,
}
#[derive(Serialize)]
struct Bind {
    create_host_path: bool,
}

fn render(operator: &ComposeProject, desired: &ComposeProject) -> Result<Vec<u8>> {
    for base in &operator.services {
        let wanted = desired.services.iter().find(|s| s.name == base.name);
        if wanted.is_none_or(|s| !representable(base, s)) {
            return Err(Error(ErrorCode::Conflict));
        }
    }
    let mut document = Document {
        services: BTreeMap::new(),
        volumes: BTreeMap::new(),
    };
    for service in &desired.services {
        let base = operator.services.iter().find(|s| s.name == service.name);
        if base == Some(service) {
            continue;
        }
        let ports = service
            .ports
            .iter()
            .filter(|p| base.is_none_or(|b| !b.ports.contains(p)))
            .map(Port::from)
            .collect();
        let devices = service
            .devices
            .iter()
            .filter(|d| base.is_none_or(|b| !b.devices.contains(d)))
            .map(|d| format!("{}:{}:rwm", d.source, d.target))
            .collect();
        let mut mounts = Vec::new();
        for mount in &service.mounts {
            if base.is_some_and(|b| b.mounts.contains(mount)) {
                continue;
            }
            let rendered = match mount {
                ComposeMount::Volume {
                    name,
                    target,
                    read_only,
                } => {
                    document.volumes.insert(name.as_str(), Empty {});
                    Mount {
                        kind: "volume",
                        source: name,
                        target,
                        read_only: *read_only,
                        bind: None,
                    }
                }
                ComposeMount::Bind {
                    source,
                    target,
                    read_only,
                } => Mount {
                    kind: "bind",
                    source,
                    target,
                    read_only: *read_only,
                    bind: Some(Bind {
                        create_host_path: false,
                    }),
                },
            };
            mounts.push(rendered);
        }
        document.services.insert(
            &service.name,
            Service {
                image: &service.image,
                user: service.user.map(|u| format!("{}:{}", u.uid, u.gid)),
                restart: service.restart,
                privileged: service.privileged,
                network_mode: service.host_network.then_some("host"),
                ports,
                volumes: mounts,
                devices,
            },
        );
    }
    let mut writer = ArtifactWriter(Vec::new());
    serde_json::to_writer(&mut writer, &document).map_err(|_| Error(ErrorCode::InvalidInput))?;
    Ok(writer.0)
}

struct ArtifactWriter(Vec<u8>);
impl Write for ArtifactWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > ARTIFACT_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other("Compose artifact exceeds its bound"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
struct CountWriter(usize);
impl Write for CountWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > SUMMARY_BYTES.saturating_sub(self.0) {
            return Err(std::io::Error::other("Compose summary exceeds its bound"));
        }
        self.0 += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
