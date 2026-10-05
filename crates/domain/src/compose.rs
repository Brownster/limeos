//! Typed, non-executable Compose previews. Host effects require a later operation.
use crate::{Error, ErrorCode, Result, identifier, opaque_id};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use ts_rs::TS;

pub const COMPOSE_VERSION: u16 = 1;
pub const COMPOSE_MANAGED_FILE: &str = "limeos.override.json";

fn valid(ok: bool) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error(ErrorCode::InvalidInput))
    }
}
pub fn compose_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 48
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}
fn literal_path(value: &str) -> bool {
    value.starts_with('/')
        && value.len() <= 256
        && value != "/"
        && value
            .split('/')
            .skip(1)
            .all(|s| !s.is_empty() && s != "." && s != "..")
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/-_.".contains(&b))
}
fn pinned_image(value: &str) -> bool {
    let Some((name, digest)) = value.split_once("@sha256:") else {
        return false;
    };
    !name.is_empty()
        && name.len() <= 192
        && opaque_id(digest)
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"/._-:".contains(&b))
        && name
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ComposeSelection {
    pub stack: String,
    pub template: String,
}
impl ComposeSelection {
    pub fn validate(&self) -> Result<()> {
        valid(compose_name(&self.stack) && compose_name(&self.template))
    }
}
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
pub enum ComposeFileName {
    #[serde(rename = "compose.yaml")]
    ComposeYaml,
    #[serde(rename = "compose.yml")]
    ComposeYml,
    #[serde(rename = "docker-compose.yaml")]
    DockerComposeYaml,
    #[serde(rename = "docker-compose.yml")]
    DockerComposeYml,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ComposeFile {
    pub name: ComposeFileName,
    pub sha256: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ComposeUser {
    pub uid: u32,
    pub gid: u32,
}
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(rename_all = "snake_case")]
pub enum PortProtocol {
    Tcp,
    Udp,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ComposePort {
    pub host_ip: String,
    pub published: u16,
    pub target: u16,
    pub protocol: PortProtocol,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComposeMount {
    Volume {
        name: String,
        target: String,
        read_only: bool,
    },
    Bind {
        source: String,
        target: String,
        read_only: bool,
    },
}
impl ComposeMount {
    fn target(&self) -> &str {
        match self {
            Self::Volume { target, .. } | Self::Bind { target, .. } => target,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ComposeDevice {
    pub source: String,
    pub target: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "kebab-case")]
pub enum ComposeRestart {
    No,
    Always,
    OnFailure,
    UnlessStopped,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ComposeService {
    pub name: String,
    pub image: String,
    pub user: Option<ComposeUser>,
    pub restart: ComposeRestart,
    pub ports: Vec<ComposePort>,
    pub mounts: Vec<ComposeMount>,
    pub devices: Vec<ComposeDevice>,
    pub privileged: bool,
    pub host_network: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ComposeProject {
    pub services: Vec<ComposeService>,
}
impl ComposeProject {
    pub fn normalize(&mut self) -> Result<()> {
        valid(self.services.len() <= 16)?;
        let mut names = BTreeSet::new();
        let mut ports = Vec::new();
        for service in &mut self.services {
            valid(
                compose_name(&service.name)
                    && names.insert(service.name.clone())
                    && pinned_image(&service.image),
            )?;
            valid(
                service.ports.len() <= 8 && service.mounts.len() <= 8 && service.devices.len() <= 8,
            )?;
            valid(!service.host_network || service.ports.is_empty())?;
            let mut targets = BTreeSet::new();
            for mount in &service.mounts {
                valid(literal_path(mount.target()) && targets.insert(mount.target().to_owned()))?;
                match mount {
                    ComposeMount::Volume { name, .. } => valid(compose_name(name))?,
                    ComposeMount::Bind { source, .. } => valid(literal_path(source))?,
                }
            }
            for device in &service.devices {
                valid(
                    literal_path(&device.source)
                        && device.source.starts_with("/dev/")
                        && literal_path(&device.target)
                        && device.target.starts_with("/dev/")
                        && targets.insert(device.target.clone()),
                )?;
            }
            for port in &mut service.ports {
                let ip: std::net::IpAddr = port
                    .host_ip
                    .parse()
                    .map_err(|_| Error(ErrorCode::InvalidInput))?;
                valid(port.published != 0 && port.target != 0)?;
                // Conflicting wildcard and specific listeners also fail before planning.
                valid(!ports.iter().any(
                    |(other, published, protocol): &(std::net::IpAddr, u16, PortProtocol)| {
                        *published == port.published
                            && *protocol == port.protocol
                            && (ip == *other || ip.is_unspecified() || other.is_unspecified())
                    },
                ))?;
                ports.push((ip, port.published, port.protocol));
                port.host_ip = ip.to_string();
            }
            service.ports.sort_by(|a, b| {
                (&a.host_ip, a.published, a.target, a.protocol).cmp(&(
                    &b.host_ip,
                    b.published,
                    b.target,
                    b.protocol,
                ))
            });
            service.mounts.sort_by(|a, b| a.target().cmp(b.target()));
            service.devices.sort_by(|a, b| a.target.cmp(&b.target));
        }
        self.services.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(())
    }
    pub fn elevated(&self) -> Vec<ComposePrivilege> {
        let mut required = BTreeSet::new();
        for service in &self.services {
            if service.privileged {
                required.insert(ComposePrivilege::Privileged);
            }
            if service.host_network {
                required.insert(ComposePrivilege::HostNetwork);
            }
            if !service.devices.is_empty() {
                required.insert(ComposePrivilege::Device);
            }
            for mount in &service.mounts {
                if let ComposeMount::Bind { source, .. } = mount {
                    required.insert(ComposePrivilege::HostMount);
                    if matches!(source.as_str(), "/var/run/docker.sock" | "/run/docker.sock") {
                        required.insert(ComposePrivilege::DockerSocket);
                    }
                }
            }
        }
        required.into_iter().collect()
    }
}
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(rename_all = "snake_case")]
pub enum ComposePrivilege {
    Privileged,
    HostMount,
    Device,
    DockerSocket,
    HostNetwork,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ComposeTemplate {
    pub id: String,
    pub project: ComposeProject,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ComposeStack {
    pub id: String,
    pub operator_files: Vec<ComposeFile>,
    pub current: ComposeProject,
    pub templates: Vec<ComposeTemplate>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ComposeCatalog {
    pub version: u16,
    pub stacks: Vec<ComposeStack>,
}
impl ComposeCatalog {
    pub fn normalize(&mut self) -> Result<()> {
        valid(self.version == COMPOSE_VERSION && self.stacks.len() <= 8)?;
        let mut names = BTreeSet::new();
        for stack in &mut self.stacks {
            valid(compose_name(&stack.id) && names.insert(stack.id.clone()))?;
            // Multiple recognized Compose files require an operator to resolve ambiguity.
            valid(
                stack.operator_files.len() <= 1
                    && stack.operator_files.iter().all(|f| opaque_id(&f.sha256)),
            )?;
            stack.current.normalize()?;
            valid((1..=8).contains(&stack.templates.len()))?;
            let mut templates = BTreeSet::new();
            for template in &mut stack.templates {
                valid(compose_name(&template.id) && templates.insert(template.id.clone()))?;
                template.project.normalize()?;
                valid(!template.project.services.is_empty())?;
            }
            stack.templates.sort_by(|a, b| a.id.cmp(&b.id));
        }
        self.stacks.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(())
    }
    pub fn select(
        &self,
        selection: &ComposeSelection,
    ) -> Result<(&ComposeStack, &ComposeTemplate)> {
        selection.validate()?;
        let stack = self
            .stacks
            .iter()
            .find(|s| s.id == selection.stack)
            .ok_or(Error(ErrorCode::NotFound))?;
        let template = stack
            .templates
            .iter()
            .find(|t| t.id == selection.template)
            .ok_or(Error(ErrorCode::NotFound))?;
        Ok((stack, template))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ComposeServiceChange {
    pub name: String,
    pub before: Option<ComposeService>,
    pub after: Option<ComposeService>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ComposeImpact {
    pub services: Vec<ComposeServiceChange>,
    pub elevated: Vec<ComposePrivilege>,
}
impl ComposeImpact {
    pub fn between(before: &ComposeProject, after: &ComposeProject) -> Self {
        let names: BTreeSet<_> = before
            .services
            .iter()
            .chain(&after.services)
            .map(|s| &s.name)
            .collect();
        let services = names
            .into_iter()
            .filter_map(|name| {
                let old = before.services.iter().find(|s| &s.name == name);
                let new = after.services.iter().find(|s| &s.name == name);
                (old != new).then(|| ComposeServiceChange {
                    name: name.clone(),
                    before: old.cloned(),
                    after: new.cloned(),
                })
            })
            .collect();
        Self {
            services,
            elevated: after.elevated(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ComposePlan {
    pub id: String,
    pub version: u16,
    pub principal: String,
    #[ts(type = "number")]
    pub grant_revision: i64,
    pub catalog_digest: String,
    pub template_digest: String,
    pub selection: ComposeSelection,
    pub operator_files: Vec<ComposeFile>,
    pub managed_file: String,
    pub before: ComposeProject,
    pub desired: ComposeProject,
    pub impact: ComposeImpact,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub expires_at: i64,
}
impl ComposePlan {
    pub fn validate(&self, now: i64) -> Result<()> {
        self.selection.validate()?;
        valid(
            self.version == COMPOSE_VERSION
                && opaque_id(&self.id)
                && identifier(&self.principal)
                && self.grant_revision >= 1
                && opaque_id(&self.catalog_digest)
                && opaque_id(&self.template_digest)
                && self.managed_file == COMPOSE_MANAGED_FILE
                && self.created_at >= 0
                && self.expires_at
                    == self
                        .created_at
                        .checked_add(crate::PLAN_TTL_SECONDS)
                        .ok_or(Error(ErrorCode::InvalidInput))?,
        )?;
        if now < self.created_at || now >= self.expires_at {
            return Err(Error(ErrorCode::Expired));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct PlannedCompose {
    pub plan: ComposePlan,
    pub digest: String,
}

#[cfg(test)]
mod tests;
