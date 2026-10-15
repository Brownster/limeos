//! Pure catalog proposals. No source ownership, approval, lock, receipt or effect.
use crate::{
    COMPOSE_MANAGED_FILE, ComposeImpact, ComposeMount, ComposeProject, ComposeService,
    compose_name,
    compose_override::{ManagedOverridePlan, UnresolvedComposeGate, plan_managed_override},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    io::Write,
};

pub const MAX_STACKS: usize = 8;
pub const MAX_SERVICES: usize = 16;
pub const MAX_ENTRIES: usize = 8;
pub const MAX_RESOURCES: usize = 16;
pub const MAX_TEXT_BYTES: usize = 64 * 1024;
pub const MAX_ARTIFACT_BYTES: usize = 64 * 1024;
pub const MAX_SUMMARY_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogErrorCode {
    InvalidInput,
    Limit,
    NotFound,
    AlreadyExists,
    ResourceConflict,
    MissingDependencies,
    DependentServices,
    InheritedService,
    InvalidReference,
    CyclicDependencies,
    Unsupported,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CatalogError {
    pub code: CatalogErrorCode,
    pub resource: Option<ResourceKey>,
    pub services: Vec<String>,
}
type Result<T> = std::result::Result<T, CatalogError>;
fn error(code: CatalogErrorCode) -> CatalogError {
    CatalogError {
        code,
        resource: None,
        services: vec![],
    }
}
fn check(condition: bool) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(error(CatalogErrorCode::InvalidInput))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationKey {
    pub app: String,
    pub stack: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InstallTarget {
    Existing { stack: String },
    New { stack: String },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrphanPolicy {
    Preserve,
    RemoveOrphans,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NetworkDefinition {
    Bridge {
        name: Option<String>,
        internal: bool,
    },
    External {
        name: Option<String>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VolumeDefinition {
    Named { name: Option<String> },
    External { name: Option<String> },
}
/// References only: inline contents and environment reads are outside this subset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DataDefinition {
    File { file: String },
    External { name: Option<String> },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resources {
    pub networks: BTreeMap<String, NetworkDefinition>,
    pub volumes: BTreeMap<String, VolumeDefinition>,
    pub configs: BTreeMap<String, DataDefinition>,
    pub secrets: BTreeMap<String, DataDefinition>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataGrant {
    pub source: String,
    pub target: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogService {
    pub base: ComposeService,
    /// Package/catalog metadata, not an implicit Compose startup-order edit.
    pub requires: Vec<String>,
    pub depends_on: Vec<String>,
    pub networks: Vec<String>,
    pub configs: Vec<DataGrant>,
    pub secrets: Vec<DataGrant>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub services: Vec<CatalogService>,
    pub resources: Resources,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppTemplate {
    pub app: String,
    pub service: CatalogService,
    pub resources: Resources,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StackDeclaration {
    pub stack: String,
    pub project: String,
    pub operator: Project,
    pub managed: Project,
    pub orphan_policy: OrphanPolicy,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceSection {
    Networks,
    Volumes,
    Configs,
    Secrets,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ResourceKey {
    pub section: ResourceSection,
    pub name: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogAction {
    InstallExisting,
    InstallNew,
    Remove,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogGate {
    ApprovedTemplateAndCompleteSource,
    CurrentDependencyMetadata,
    ConfigSecretFileOwners,
    ExternalResourceVerification,
    PreparedTargetAndAbsence,
    ExplicitOrphanReview,
    VerifiedSelectedServiceShutdown,
}
const GATES: &[CatalogGate] = &[
    CatalogGate::ApprovedTemplateAndCompleteSource,
    CatalogGate::CurrentDependencyMetadata,
    CatalogGate::ConfigSecretFileOwners,
    CatalogGate::ExternalResourceVerification,
    CatalogGate::PreparedTargetAndAbsence,
    CatalogGate::ExplicitOrphanReview,
    CatalogGate::VerifiedSelectedServiceShutdown,
];

/// Opaque draft only. No deserializer or stop-result shortcut enables a write.
pub struct CatalogPlan {
    key: InstallationKey,
    action: CatalogAction,
    project: String,
    orphan_policy: OrphanPolicy,
    operator: Project,
    before: Project,
    desired: Project,
    added: Vec<ResourceKey>,
    reused: Vec<ResourceKey>,
    base: ManagedOverridePlan,
    artifact: Vec<u8>,
}
impl fmt::Debug for CatalogPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CatalogPlan")
            .field("key", &self.key)
            .field("action", &self.action)
            .field("artifact_bytes", &self.artifact.len())
            .finish_non_exhaustive()
    }
}
#[derive(Serialize)]
pub struct CatalogSummary<'a> {
    pub version: u16,
    pub installation: &'a InstallationKey,
    pub action: CatalogAction,
    pub project: &'a str,
    pub orphan_policy: OrphanPolicy,
    pub operator: &'a Project,
    pub before_managed: &'a Project,
    pub desired_managed: &'a Project,
    pub added_resources: &'a [ResourceKey],
    pub reused_resources: &'a [ResourceKey],
    pub shutdown_required: &'a [String],
    /// Existing base-service projection only; excludes resource/grant metadata.
    /// Execution review must bind the complete model and artifact instead.
    pub base_service_impact: &'a ComposeImpact,
    pub managed_file: &'static str,
    pub artifact_bytes: usize,
    pub compose_unresolved: &'a [UnresolvedComposeGate],
    pub catalog_unresolved: &'static [CatalogGate],
}
impl CatalogPlan {
    pub fn artifact(&self) -> &[u8] {
        &self.artifact
    }
    pub fn summary(&self) -> CatalogSummary<'_> {
        let base = self.base.summary();
        CatalogSummary {
            version: 1,
            installation: &self.key,
            action: self.action,
            project: &self.project,
            orphan_policy: self.orphan_policy,
            operator: &self.operator,
            before_managed: &self.before,
            desired_managed: &self.desired,
            added_resources: &self.added,
            reused_resources: &self.reused,
            shutdown_required: base.shutdown_required,
            base_service_impact: base.impact,
            managed_file: COMPOSE_MANAGED_FILE,
            artifact_bytes: self.artifact.len(),
            compose_unresolved: base.unresolved,
            catalog_unresolved: GATES,
        }
    }
}

fn external_name(name: &Option<String>) -> bool {
    name.as_ref().is_none_or(|s| {
        !s.is_empty()
            && s.len() <= 128
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    })
}
fn literal(value: &str, relative: bool) -> bool {
    let body = value
        .strip_prefix('/')
        .or_else(|| relative.then_some(value))
        .unwrap_or("");
    let body = if relative {
        body.strip_prefix("./").unwrap_or(body)
    } else {
        body
    };
    !body.is_empty()
        && value.len() <= 512
        && !value.contains(['$', '{', '}', '\\'])
        && !value.chars().any(char::is_control)
        && body.split('/').count() <= 16
        && body
            .split('/')
            .all(|p| !matches!(p, "" | "." | "..") && p.len() <= 255)
}
fn charge(bytes: &mut usize, n: usize) -> Result<()> {
    *bytes = bytes
        .checked_add(n)
        .ok_or_else(|| error(CatalogErrorCode::Limit))?;
    if *bytes > MAX_TEXT_BYTES {
        return Err(error(CatalogErrorCode::Limit));
    }
    Ok(())
}
fn named<T>(map: &BTreeMap<String, T>, bytes: &mut usize) -> Result<()> {
    if map.len() > MAX_RESOURCES {
        return Err(error(CatalogErrorCode::Limit));
    }
    for key in map.keys() {
        check(compose_name(key))?;
        charge(bytes, key.len())?;
    }
    Ok(())
}
fn resources(resources: &Resources, bytes: &mut usize) -> Result<()> {
    named(&resources.networks, bytes)?;
    named(&resources.volumes, bytes)?;
    named(&resources.configs, bytes)?;
    named(&resources.secrets, bytes)?;
    for value in resources.networks.values() {
        let (NetworkDefinition::Bridge { name, .. } | NetworkDefinition::External { name }) = value;
        check(external_name(name))?;
        charge(bytes, name.as_ref().map_or(0, String::len))?;
    }
    for value in resources.volumes.values() {
        let (VolumeDefinition::Named { name } | VolumeDefinition::External { name }) = value;
        check(external_name(name))?;
        charge(bytes, name.as_ref().map_or(0, String::len))?;
    }
    for value in resources.configs.values().chain(resources.secrets.values()) {
        match value {
            DataDefinition::File { file } => {
                check(literal(file, true))?;
                charge(bytes, file.len())?;
            }
            DataDefinition::External { name } => {
                check(external_name(name))?;
                charge(bytes, name.as_ref().map_or(0, String::len))?;
            }
        }
    }
    Ok(())
}
fn names(names: &[String], bytes: &mut usize) -> Result<()> {
    if names.len() > MAX_ENTRIES {
        return Err(error(CatalogErrorCode::Limit));
    }
    for (i, name) in names.iter().enumerate() {
        check(compose_name(name) && !names[..i].contains(name))?;
        charge(bytes, name.len())?;
    }
    Ok(())
}
fn service(service: &CatalogService, bytes: &mut usize) -> Result<()> {
    let base = &service.base;
    if base.name.len() > 48
        || base.image.len() > 264
        || base.ports.len() > MAX_ENTRIES
        || base.mounts.len() > MAX_ENTRIES
        || base.devices.len() > MAX_ENTRIES
        || service.configs.len() > MAX_ENTRIES
        || service.secrets.len() > MAX_ENTRIES
    {
        return Err(error(CatalogErrorCode::Limit));
    }
    charge(bytes, base.name.len() + base.image.len())?;
    for port in &base.ports {
        check(port.host_ip.len() <= 45)?;
        charge(bytes, port.host_ip.len())?;
    }
    for mount in &base.mounts {
        let (source, target) = match mount {
            ComposeMount::Bind { source, target, .. } => (source, target),
            ComposeMount::Volume { name, target, .. } => (name, target),
        };
        check(source.len() <= 256 && target.len() <= 256)?;
        charge(bytes, source.len() + target.len())?;
    }
    for device in &base.devices {
        check(device.source.len() <= 256 && device.target.len() <= 256)?;
        charge(bytes, device.source.len() + device.target.len())?;
    }
    names(&service.requires, bytes)?;
    names(&service.depends_on, bytes)?;
    names(&service.networks, bytes)?;
    check(!base.host_network || service.networks.is_empty())?;
    for (grants, secret) in [(&service.configs, false), (&service.secrets, true)] {
        let mut targets = BTreeSet::new();
        for grant in grants {
            check(compose_name(&grant.source))?;
            charge(bytes, grant.source.len())?;
            if let Some(target) = &grant.target {
                check(literal(target, secret && !target.contains('/')))?;
                charge(bytes, target.len())?;
            }
            let target = grant_target(grant, secret);
            check(
                targets.insert(target.clone())
                    && !base.mounts.iter().any(|m| match m {
                        ComposeMount::Bind { target: t, .. }
                        | ComposeMount::Volume { target: t, .. } => *t == target,
                    })
                    && !base.devices.iter().any(|device| device.target == target),
            )?;
        }
    }
    let config_targets: BTreeSet<_> = service
        .configs
        .iter()
        .map(|g| grant_target(g, false))
        .collect();
    check(
        !service
            .secrets
            .iter()
            .any(|g| config_targets.contains(&grant_target(g, true))),
    )
}
fn grant_target(grant: &DataGrant, secret: bool) -> String {
    match &grant.target {
        Some(target) if target.starts_with('/') => target.clone(),
        Some(target) => format!("/run/secrets/{target}"),
        None if secret => format!("/run/secrets/{}", grant.source),
        None => format!("/{}", grant.source),
    }
}
fn project(project: &Project, bytes: &mut usize) -> Result<()> {
    if project.services.len() > MAX_SERVICES {
        return Err(error(CatalogErrorCode::Limit));
    }
    resources(&project.resources, bytes)?;
    for (i, value) in project.services.iter().enumerate() {
        check(
            !project.services[..i]
                .iter()
                .any(|s| s.base.name == value.base.name),
        )?;
        service(value, bytes)?;
    }
    Ok(())
}
fn select<'a>(stacks: &'a [StackDeclaration], name: &str) -> Result<&'a StackDeclaration> {
    check(compose_name(name))?;
    if stacks.len() > MAX_STACKS {
        return Err(error(CatalogErrorCode::Limit));
    }
    for (i, stack) in stacks.iter().enumerate() {
        check(
            compose_name(&stack.stack)
                && compose_name(&stack.project)
                && !stacks[..i]
                    .iter()
                    .any(|s| s.stack == stack.stack || s.project == stack.project),
        )?;
    }
    stacks
        .iter()
        .find(|s| s.stack == name)
        .ok_or_else(|| error(CatalogErrorCode::NotFound))
}
fn admit(stack: &StackDeclaration, template: Option<&AppTemplate>) -> Result<()> {
    check(compose_name(&stack.stack) && compose_name(&stack.project))?;
    let mut bytes = 0;
    charge(&mut bytes, stack.stack.len() + stack.project.len())?;
    project(&stack.operator, &mut bytes)?;
    project(&stack.managed, &mut bytes)?;
    if stack.operator.services.len()
        + stack.managed.services.len()
        + usize::from(template.is_some())
        > MAX_SERVICES
    {
        return Err(error(CatalogErrorCode::Limit));
    }
    if stack.managed.services.iter().any(|m| {
        stack
            .operator
            .services
            .iter()
            .any(|o| o.base.name == m.base.name)
    }) {
        return Err(error(CatalogErrorCode::InheritedService));
    }
    if let Some(template) = template {
        check(compose_name(&template.app) && template.app == template.service.base.name)?;
        charge(&mut bytes, template.app.len())?;
        service(&template.service, &mut bytes)?;
        resources(&template.resources, &mut bytes)?;
    }
    Ok(())
}
fn merge_section<T: Clone + Eq>(
    section: ResourceSection,
    operator: &BTreeMap<String, T>,
    managed: &mut BTreeMap<String, T>,
    incoming: &BTreeMap<String, T>,
    added: &mut Vec<ResourceKey>,
    reused: &mut Vec<ResourceKey>,
) -> Result<()> {
    for (name, value) in incoming {
        if operator.get(name).is_some_and(|old| old != value)
            || managed.get(name).is_some_and(|old| old != value)
        {
            return Err(CatalogError {
                code: CatalogErrorCode::ResourceConflict,
                resource: Some(ResourceKey {
                    section,
                    name: name.clone(),
                }),
                services: vec![],
            });
        }
        let key = ResourceKey {
            section,
            name: name.clone(),
        };
        if operator.contains_key(name) || managed.contains_key(name) {
            reused.push(key);
        } else {
            managed.insert(name.clone(), value.clone());
            added.push(key);
        }
    }
    if managed.len() > MAX_RESOURCES {
        return Err(error(CatalogErrorCode::Limit));
    }
    Ok(())
}
fn merge(
    operator: &Resources,
    managed: &mut Resources,
    incoming: &Resources,
    added: &mut Vec<ResourceKey>,
    reused: &mut Vec<ResourceKey>,
) -> Result<()> {
    merge_section(
        ResourceSection::Networks,
        &operator.networks,
        &mut managed.networks,
        &incoming.networks,
        added,
        reused,
    )?;
    merge_section(
        ResourceSection::Volumes,
        &operator.volumes,
        &mut managed.volumes,
        &incoming.volumes,
        added,
        reused,
    )?;
    merge_section(
        ResourceSection::Configs,
        &operator.configs,
        &mut managed.configs,
        &incoming.configs,
        added,
        reused,
    )?;
    merge_section(
        ResourceSection::Secrets,
        &operator.secrets,
        &mut managed.secrets,
        &incoming.secrets,
        added,
        reused,
    )
}
fn effective(operator: &Project, managed: &Project) -> Result<ComposeProject> {
    let mut base = ComposeProject {
        services: operator
            .services
            .iter()
            .chain(&managed.services)
            .map(|s| s.base.clone())
            .collect(),
    };
    base.normalize()
        .map_err(|_| error(CatalogErrorCode::Unsupported))?;
    Ok(base)
}
fn has<T>(name: &str, op: &BTreeMap<String, T>, managed: &BTreeMap<String, T>) -> bool {
    op.contains_key(name) || managed.contains_key(name)
}
fn references(operator: &Project, managed: &Project) -> Result<()> {
    // Existing overlapping resource declarations must agree too.
    fn compatible<T: Eq>(
        section: ResourceSection,
        operator: &BTreeMap<String, T>,
        managed: &BTreeMap<String, T>,
    ) -> Result<()> {
        for (name, value) in managed {
            if operator.get(name).is_some_and(|old| old != value) {
                return Err(CatalogError {
                    code: CatalogErrorCode::ResourceConflict,
                    resource: Some(ResourceKey {
                        section,
                        name: name.clone(),
                    }),
                    services: vec![],
                });
            }
        }
        Ok(())
    }
    compatible(
        ResourceSection::Networks,
        &operator.resources.networks,
        &managed.resources.networks,
    )?;
    compatible(
        ResourceSection::Volumes,
        &operator.resources.volumes,
        &managed.resources.volumes,
    )?;
    compatible(
        ResourceSection::Configs,
        &operator.resources.configs,
        &managed.resources.configs,
    )?;
    compatible(
        ResourceSection::Secrets,
        &operator.resources.secrets,
        &managed.resources.secrets,
    )?;
    let all: Vec<_> = operator.services.iter().chain(&managed.services).collect();
    if all.len() > MAX_SERVICES {
        return Err(error(CatalogErrorCode::Limit));
    }
    let mut missing = BTreeSet::new();
    for service in &all {
        for required in service.requires.iter().chain(&service.depends_on) {
            if !all.iter().any(|s| s.base.name == *required) {
                missing.insert(required.clone());
            }
        }
        let op = &operator.resources;
        let own = &managed.resources;
        if service.networks.iter().any(|n| !has(n, &op.networks, &own.networks))
            || service.configs.iter().any(|g| !has(&g.source, &op.configs, &own.configs))
            || service.secrets.iter().any(|g| !has(&g.source, &op.secrets, &own.secrets))
            || service.base.mounts.iter().any(|m| matches!(m, ComposeMount::Volume { name, .. } if !has(name, &op.volumes, &own.volumes))) {
            return Err(error(CatalogErrorCode::InvalidReference));
        }
    }
    if !missing.is_empty() {
        return Err(CatalogError {
            code: CatalogErrorCode::MissingDependencies,
            resource: None,
            services: missing.into_iter().collect(),
        });
    }
    fn visit(
        index: usize,
        all: &[&CatalogService],
        visiting: &mut BTreeSet<usize>,
        done: &mut BTreeSet<usize>,
    ) -> bool {
        if done.contains(&index) {
            return true;
        }
        if !visiting.insert(index) {
            return false;
        }
        for name in all[index].requires.iter().chain(&all[index].depends_on) {
            if let Some(next) = all.iter().position(|s| s.base.name == *name)
                && !visit(next, all, visiting, done)
            {
                return false;
            }
        }
        visiting.remove(&index);
        done.insert(index);
        true
    }
    let (mut visiting, mut done) = (BTreeSet::new(), BTreeSet::new());
    for index in 0..all.len() {
        if !visit(index, &all, &mut visiting, &mut done) {
            return Err(error(CatalogErrorCode::CyclicDependencies));
        }
    }
    Ok(())
}

pub fn plan_install(
    stacks: &[StackDeclaration],
    target: &InstallTarget,
    template: &AppTemplate,
    orphan_policy_for_new: OrphanPolicy,
) -> Result<CatalogPlan> {
    let new_stack;
    let (stack, action) = match target {
        InstallTarget::Existing { stack } => {
            (select(stacks, stack)?, CatalogAction::InstallExisting)
        }
        InstallTarget::New { stack } => {
            match select(stacks, stack) {
                Ok(_) => return Err(error(CatalogErrorCode::AlreadyExists)),
                Err(e) if e.code == CatalogErrorCode::NotFound => {}
                Err(e) => return Err(e),
            }
            if stacks.len() >= MAX_STACKS {
                return Err(error(CatalogErrorCode::Limit));
            }
            if stacks.iter().any(|s| s.project == *stack) {
                return Err(error(CatalogErrorCode::AlreadyExists));
            }
            new_stack = StackDeclaration {
                stack: stack.clone(),
                project: stack.clone(),
                operator: Project::default(),
                managed: Project::default(),
                orphan_policy: orphan_policy_for_new,
            };
            (&new_stack, CatalogAction::InstallNew)
        }
    };
    admit(stack, Some(template))?;
    if stack
        .operator
        .services
        .iter()
        .chain(&stack.managed.services)
        .any(|s| s.base.name == template.app)
    {
        return Err(error(CatalogErrorCode::AlreadyExists));
    }
    references(&stack.operator, &stack.managed)?;
    let mut desired = stack.managed.clone();
    let (mut added, mut reused) = (Vec::new(), Vec::new());
    merge(
        &stack.operator.resources,
        &mut desired.resources,
        &template.resources,
        &mut added,
        &mut reused,
    )?;
    desired.services.push(template.service.clone());
    finish(stack, &template.app, action, desired, added, reused)
}
pub fn plan_remove(stacks: &[StackDeclaration], key: &InstallationKey) -> Result<CatalogPlan> {
    check(compose_name(&key.app))?;
    let stack = select(stacks, &key.stack)?;
    admit(stack, None)?;
    if stack
        .operator
        .services
        .iter()
        .any(|s| s.base.name == key.app)
    {
        return Err(error(CatalogErrorCode::InheritedService));
    }
    if !stack
        .managed
        .services
        .iter()
        .any(|s| s.base.name == key.app)
    {
        return Err(error(CatalogErrorCode::NotFound));
    }
    references(&stack.operator, &stack.managed)?;
    let mut dependents: Vec<_> = stack
        .operator
        .services
        .iter()
        .chain(&stack.managed.services)
        .filter(|s| {
            s.base.name != key.app
                && s.requires
                    .iter()
                    .chain(&s.depends_on)
                    .any(|name| *name == key.app)
        })
        .map(|s| s.base.name.clone())
        .collect();
    dependents.sort();
    if !dependents.is_empty() {
        return Err(CatalogError {
            code: CatalogErrorCode::DependentServices,
            resource: None,
            services: dependents,
        });
    }
    let mut desired = stack.managed.clone();
    desired.services.retain(|s| s.base.name != key.app);
    // Never garbage-collect resource definitions as a side effect of app removal.
    finish(
        stack,
        &key.app,
        CatalogAction::Remove,
        desired,
        vec![],
        vec![],
    )
}
fn finish(
    stack: &StackDeclaration,
    app: &str,
    action: CatalogAction,
    mut desired: Project,
    added: Vec<ResourceKey>,
    reused: Vec<ResourceKey>,
) -> Result<CatalogPlan> {
    let mut text = 0;
    project(&desired, &mut text)?;
    references(&stack.operator, &desired)?;
    let operator_base = effective(&stack.operator, &Project::default())?;
    let before = effective(&stack.operator, &stack.managed)?;
    let after = effective(&stack.operator, &desired)?;
    let base = plan_managed_override(&operator_base, &before, &after)
        .map_err(|_| error(CatalogErrorCode::Unsupported))?;
    desired
        .services
        .sort_by(|a, b| a.base.name.cmp(&b.base.name));
    let artifact = render(&base, &desired)?;
    let plan = CatalogPlan {
        key: InstallationKey {
            app: app.into(),
            stack: stack.stack.clone(),
        },
        action,
        project: stack.project.clone(),
        orphan_policy: stack.orphan_policy,
        operator: stack.operator.clone(),
        before: stack.managed.clone(),
        desired,
        added,
        reused,
        base,
        artifact,
    };
    serde_json::to_writer(
        LimitWriter {
            bytes: vec![],
            max: MAX_SUMMARY_BYTES,
            retain: false,
        },
        &plan.summary(),
    )
    .map_err(|_| error(CatalogErrorCode::Limit))?;
    Ok(plan)
}

struct LimitWriter {
    bytes: Vec<u8>,
    max: usize,
    retain: bool,
}
impl Write for LimitWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.max {
            return Err(std::io::Error::other("catalog byte bound"));
        }
        self.max -= bytes.len();
        if self.retain {
            self.bytes
                .try_reserve(bytes.len())
                .map_err(|_| std::io::Error::other("catalog allocation"))?;
            self.bytes.extend_from_slice(bytes);
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn named_body(mut value: Map<String, Value>, name: &Option<String>) -> Value {
    if let Some(name) = name {
        value.insert("name".into(), json!(name));
    }
    Value::Object(value)
}
fn render(base: &ManagedOverridePlan, desired: &Project) -> Result<Vec<u8>> {
    let mut value: Value = serde_json::from_slice(base.artifact())
        .map_err(|_| error(CatalogErrorCode::Unsupported))?;
    let document = value
        .as_object_mut()
        .ok_or_else(|| error(CatalogErrorCode::Unsupported))?;
    // Replace the base compiler's implicit empty volume declarations with exact
    // typed definitions; inherited operator definitions stay in operator YAML.
    document.remove("volumes");
    let services = document
        .get_mut("services")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| error(CatalogErrorCode::Unsupported))?;
    for service in &desired.services {
        let target = services
            .get_mut(&service.base.name)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| error(CatalogErrorCode::Unsupported))?;
        if !service.depends_on.is_empty() {
            target.insert("depends_on".into(), json!(service.depends_on));
        }
        if !service.networks.is_empty() {
            target.insert("networks".into(), json!(service.networks));
        }
        for (key, grants) in [("configs", &service.configs), ("secrets", &service.secrets)] {
            if !grants.is_empty() {
                let entries: Vec<_> = grants
                    .iter()
                    .map(|grant| {
                        let mut body = Map::new();
                        body.insert("source".into(), json!(grant.source));
                        if let Some(target) = &grant.target {
                            body.insert("target".into(), json!(target));
                        }
                        Value::Object(body)
                    })
                    .collect();
                target.insert(key.into(), Value::Array(entries));
            }
        }
    }
    let networks: Map<_, _> = desired
        .resources
        .networks
        .iter()
        .map(|(key, resource)| {
            let (name, body) = match resource {
                NetworkDefinition::Bridge { name, internal } => {
                    (name, json!({"driver":"bridge", "internal":internal}))
                }
                NetworkDefinition::External { name } => (name, json!({"external":true})),
            };
            (
                key.clone(),
                named_body(body.as_object().unwrap().clone(), name),
            )
        })
        .collect();
    let volumes: Map<_, _> = desired
        .resources
        .volumes
        .iter()
        .map(|(key, resource)| {
            let (name, body) = match resource {
                VolumeDefinition::Named { name } => (name, Map::new()),
                VolumeDefinition::External { name } => {
                    (name, json!({"external":true}).as_object().unwrap().clone())
                }
            };
            (key.clone(), named_body(body, name))
        })
        .collect();
    for (key, map) in [("networks", networks), ("volumes", volumes)] {
        if !map.is_empty() {
            document.insert(key.into(), Value::Object(map));
        }
    }
    for (key, resources) in [
        ("configs", &desired.resources.configs),
        ("secrets", &desired.resources.secrets),
    ] {
        let entries: Map<_, _> = resources
            .iter()
            .map(|(key, resource)| {
                let value = match resource {
                    DataDefinition::File { file } => json!({"file":file}),
                    DataDefinition::External { name } => {
                        named_body(json!({"external":true}).as_object().unwrap().clone(), name)
                    }
                };
                (key.clone(), value)
            })
            .collect();
        if !entries.is_empty() {
            document.insert(key.into(), Value::Object(entries));
        }
    }
    let mut writer = LimitWriter {
        bytes: vec![],
        max: MAX_ARTIFACT_BYTES,
        retain: true,
    };
    serde_json::to_writer(&mut writer, &value).map_err(|_| error(CatalogErrorCode::Limit))?;
    Ok(writer.bytes)
}

#[cfg(test)]
mod tests;
