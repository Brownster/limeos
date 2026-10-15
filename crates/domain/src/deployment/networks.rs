//! Configured network references, not process namespace or VPN evidence.
use super::discovery::ProjectIdentity;
use crate::{Error, ErrorCode, Result};
use serde::Serialize;
use std::{collections::BTreeMap, io::Write};

pub const MAX_MEMBERS: usize = 16;
pub const MAX_INPUT_TEXT: usize = 24 * 1024;
pub const MAX_REPORT_BYTES: usize = 64 * 1024;
pub const MAX_AGE_SECONDS: i64 = 90;
const MAX_TIME: i64 = 253_402_300_799;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    Healthy,
    Unhealthy,
    Starting,
    NotConfigured,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ComposeMember {
    pub project: ProjectIdentity,
    pub service: String,
    pub one_off: bool,
}

/// Ordinary caller-supplied facts. These types cannot authorize effects.
#[derive(Clone, Debug)]
pub struct Node {
    /// Full lowercase Engine ID, without a resource prefix.
    pub id: String,
    pub name: String,
    /// True only for running, unpaused, non-restarting and non-dead containers.
    pub running: bool,
    pub health: Health,
    pub network_mode: Option<String>,
    pub compose: Option<ComposeMember>,
    /// Optional single dependency, used only to annotate an unresolved reference.
    pub dependency_hint: Option<String>,
}
impl Node {
    /// Single-row shape admission; cohort admission still checks duplicates/bounds.
    pub fn validate(&self) -> Result<()> {
        admit_nodes(std::slice::from_ref(self), 0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completeness {
    Complete,
    Partial,
}
pub struct NetworkInput<'a> {
    pub engine_id: &'a str,
    pub sampled_at: i64,
    pub completeness: Completeness,
    pub nodes: &'a [Node],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    Fresh,
    Stale,
    Future,
    Incomplete,
    EngineMismatch,
    Unavailable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    Resolved,
    MissingProvider,
    AmbiguousProvider,
    UnsupportedContainerPrefix,
    MissingProject,
    SelfReference,
    CyclicProvider,
    ChainedProvider,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    Container,
    Service,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContainerView {
    pub id: String,
    pub name: String,
    pub running: bool,
    pub health: Health,
    pub compose: Option<ComposeMember>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MemberView {
    pub container: ContainerView,
    pub reference_kind: ReferenceKind,
    pub target: String,
    pub state: LinkState,
    pub provider_id: Option<String>,
    /// Never substitutes for provider_id or changes state to Resolved.
    pub hinted_provider_id: Option<String>,
    pub dependency_hint: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupState {
    ObservedRunning,
    ProviderStopped,
    ProviderUnhealthy,
    ProviderStarting,
    HealthUnknown,
    ProviderNetworkUnknown,
    UnsupportedTopology,
    OrphanHints,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GroupView {
    pub provider: ContainerView,
    pub state: GroupState,
    pub member_ids: Vec<String>,
    pub hinted_orphan_ids: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NetworkInventory {
    pub version: u16,
    pub generated_at: i64,
    pub source: SourceState,
    pub sampled_at: Option<i64>,
    pub max_age_seconds: i64,
    /// Null for any incomplete/unusable cohort. Zero requires fresh complete data.
    pub observed_containers: Option<u16>,
    pub independent_containers: Option<u16>,
    pub unknown_mode_containers: Option<u16>,
    pub groups: Vec<GroupView>,
    pub members: Vec<MemberView>,
}

fn invalid() -> Error {
    Error(ErrorCode::InvalidInput)
}
fn limit() -> Error {
    Error(ErrorCode::Unavailable)
}
pub fn full_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn name(value: &str, max: usize, project: bool) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.bytes().all(|b| {
            (b.is_ascii_alphanumeric() && (!project || !b.is_ascii_uppercase()))
                || b"_-".contains(&b)
                || (!project && b == b'.')
        })
}
fn path(value: &str) -> bool {
    value.starts_with('/')
        && value.len() <= 512
        && !value.contains(',')
        && !value.chars().any(char::is_control)
        && value[1..].split('/').count() <= 16
        && value[1..]
            .split('/')
            .all(|part| !matches!(part, "" | "." | "..") && part.len() <= 255)
}
fn reference(mode: &str) -> Option<(ReferenceKind, &str)> {
    mode.strip_prefix("container:")
        .map(|s| (ReferenceKind::Container, s))
        .or_else(|| {
            mode.strip_prefix("service:")
                .map(|s| (ReferenceKind::Service, s))
        })
}
fn admit(expected_engine: &str, input: Option<&NetworkInput<'_>>) -> Result<()> {
    let engine_valid =
        |s: &str| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control);
    if !engine_valid(expected_engine) {
        return Err(invalid());
    }
    let Some(input) = input else { return Ok(()) };
    if !engine_valid(input.engine_id) || !(0..=MAX_TIME).contains(&input.sampled_at) {
        return Err(invalid());
    }
    admit_nodes(input.nodes, expected_engine.len() + input.engine_id.len())
}
fn admit_nodes(nodes: &[Node], mut text: usize) -> Result<()> {
    if nodes.len() > MAX_MEMBERS {
        return Err(limit());
    }
    let mut charge = |s: &str| -> Result<()> {
        text = text.checked_add(s.len()).ok_or_else(limit)?;
        if text > MAX_INPUT_TEXT {
            return Err(limit());
        }
        Ok(())
    };
    for (i, node) in nodes.iter().enumerate() {
        if !full_id(&node.id)
            || !name(&node.name, 128, false)
            || nodes[..i]
                .iter()
                .any(|p| p.id == node.id || p.name == node.name)
        {
            return Err(invalid());
        }
        charge(&node.id)?;
        charge(&node.name)?;
        if let Some(mode) = &node.network_mode {
            if mode.len() > 256
                || !reference(mode).map_or_else(
                    || name(mode, 128, false),
                    |(kind, target)| {
                        name(
                            target,
                            if kind == ReferenceKind::Service {
                                64
                            } else {
                                128
                            },
                            false,
                        )
                    },
                )
            {
                return Err(invalid());
            }
            charge(mode)?;
        }
        if let Some(hint) = &node.dependency_hint {
            if !name(hint, 64, false) {
                return Err(invalid());
            }
            charge(hint)?;
        }
        if let Some(compose) = &node.compose {
            let p = &compose.project;
            if !name(&p.name, 64, true)
                || !name(&compose.service, 64, false)
                || !path(&p.working_directory)
                || !(1..=4).contains(&p.config_files.len())
                || p.config_files
                    .iter()
                    .enumerate()
                    .any(|(i, f)| !path(f) || p.config_files[..i].contains(f))
            {
                return Err(invalid());
            }
            charge(&p.name)?;
            charge(&p.working_directory)?;
            charge(&compose.service)?;
            for file in &p.config_files {
                charge(file)?;
            }
        }
    }
    Ok(())
}
fn view(node: &Node) -> ContainerView {
    ContainerView {
        id: node.id.clone(),
        name: node.name.clone(),
        running: node.running,
        health: node.health,
        compose: node.compose.clone(),
    }
}
fn service_candidates(nodes: &[Node], member: &Node, service: &str) -> Vec<usize> {
    let Some(owner) = &member.compose else {
        return Vec::new();
    };
    nodes
        .iter()
        .enumerate()
        .filter_map(|(i, node)| {
            node.compose
                .as_ref()
                .filter(|c| !c.one_off && c.project == owner.project && c.service == service)
                .map(|_| i)
        })
        .collect()
}
fn resolve(
    nodes: &[Node],
    index: usize,
    kind: ReferenceKind,
    target: &str,
) -> (LinkState, Option<usize>) {
    let matches: Vec<_> = match kind {
        ReferenceKind::Container => nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| (n.id == target || n.name == target).then_some(i))
            .collect(),
        ReferenceKind::Service => {
            if nodes[index].compose.is_none() {
                return (LinkState::MissingProject, None);
            }
            service_candidates(nodes, &nodes[index], target)
        }
    };
    match matches.as_slice() {
        [i] if *i == index => (LinkState::SelfReference, Some(*i)),
        [i] => (LinkState::Resolved, Some(*i)),
        [] if kind == ReferenceKind::Container
            && (1..64).contains(&target.len())
            && target
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) =>
        {
            (LinkState::UnsupportedContainerPrefix, None)
        }
        [] => (LinkState::MissingProvider, None),
        _ => (LinkState::AmbiguousProvider, None),
    }
}
fn group_state(node: &Node, unsupported: bool, hinted: bool) -> GroupState {
    if unsupported {
        return GroupState::UnsupportedTopology;
    }
    if hinted {
        return GroupState::OrphanHints;
    }
    if node.network_mode.is_none() {
        return GroupState::ProviderNetworkUnknown;
    }
    if !node.running {
        return GroupState::ProviderStopped;
    }
    match node.health {
        Health::Unhealthy => GroupState::ProviderUnhealthy,
        Health::Starting => GroupState::ProviderStarting,
        Health::Unknown => GroupState::HealthUnknown,
        Health::Healthy | Health::NotConfigured => GroupState::ObservedRunning,
    }
}

/// No I/O, clock acquisition, mutation or authority. Uses original sample time.
pub fn observe(
    now: i64,
    expected_engine: &str,
    input: Option<NetworkInput<'_>>,
) -> Result<NetworkInventory> {
    if !(0..=MAX_TIME).contains(&now) {
        return Err(invalid());
    }
    admit(expected_engine, input.as_ref())?;
    let source = match input.as_ref() {
        None => SourceState::Unavailable,
        Some(i) if i.engine_id != expected_engine => SourceState::EngineMismatch,
        Some(i) if i.sampled_at > now => SourceState::Future,
        Some(i) if now - i.sampled_at > MAX_AGE_SECONDS => SourceState::Stale,
        Some(i) if i.completeness == Completeness::Partial => SourceState::Incomplete,
        Some(_) => SourceState::Fresh,
    };
    let mut report = NetworkInventory {
        version: 1,
        generated_at: now,
        source,
        sampled_at: input.as_ref().map(|i| i.sampled_at),
        max_age_seconds: MAX_AGE_SECONDS,
        observed_containers: None,
        independent_containers: None,
        unknown_mode_containers: None,
        groups: vec![],
        members: vec![],
    };
    if source != SourceState::Fresh {
        return Ok(report);
    }
    let nodes = input.unwrap().nodes;
    let references: Vec<_> = nodes
        .iter()
        .map(|n| n.network_mode.as_deref().and_then(reference))
        .collect();
    let links: Vec<_> = references
        .iter()
        .enumerate()
        .map(|(i, r)| r.map(|(kind, target)| resolve(nodes, i, kind, target)))
        .collect();
    let mut groups: BTreeMap<usize, (Vec<String>, Vec<String>, bool)> = BTreeMap::new();
    for (i, r) in references.iter().enumerate() {
        let Some((kind, target)) = r else { continue };
        let (mut state, provider) = links[i].unwrap();
        if state == LinkState::Resolved {
            let mut visited = [false; MAX_MEMBERS];
            let mut cursor = i;
            while let Some((_, Some(next))) = links[cursor] {
                visited[cursor] = true;
                if visited[next] {
                    state = LinkState::CyclicProvider;
                    break;
                }
                cursor = next;
            }
            if state == LinkState::Resolved && references[provider.unwrap()].is_some() {
                state = LinkState::ChainedProvider;
            }
        }
        let hinted = if state == LinkState::MissingProvider && *kind == ReferenceKind::Container {
            nodes[i].dependency_hint.as_ref().and_then(|hint| {
                let candidates = service_candidates(nodes, &nodes[i], hint);
                match candidates.as_slice() {
                    [index] if *index != i => Some(*index),
                    _ => None,
                }
            })
        } else {
            None
        };
        if let Some(provider) = provider {
            let group = groups.entry(provider).or_default();
            group.0.push(nodes[i].id.clone());
            group.2 |= state != LinkState::Resolved;
        }
        if let Some(hinted) = hinted {
            groups
                .entry(hinted)
                .or_default()
                .1
                .push(nodes[i].id.clone());
        }
        report.members.push(MemberView {
            container: view(&nodes[i]),
            reference_kind: *kind,
            target: (*target).into(),
            state,
            provider_id: provider.map(|p| nodes[p].id.clone()),
            hinted_provider_id: hinted.map(|p| nodes[p].id.clone()),
            dependency_hint: nodes[i].dependency_hint.clone(),
        });
    }
    for (provider, (mut members, mut hints, unsupported)) in groups {
        members.sort();
        hints.sort();
        report.groups.push(GroupView {
            provider: view(&nodes[provider]),
            state: group_state(&nodes[provider], unsupported, !hints.is_empty()),
            member_ids: members,
            hinted_orphan_ids: hints,
        });
    }
    report
        .groups
        .sort_by(|a, b| a.provider.id.cmp(&b.provider.id));
    report
        .members
        .sort_by(|a, b| a.container.id.cmp(&b.container.id));
    report.observed_containers = Some(nodes.len() as u16);
    report.independent_containers = Some(
        references
            .iter()
            .enumerate()
            .filter(|(i, r)| r.is_none() && nodes[*i].network_mode.is_some())
            .count() as u16,
    );
    report.unknown_mode_containers =
        Some(nodes.iter().filter(|n| n.network_mode.is_none()).count() as u16);
    report.check_size()?;
    Ok(report)
}
impl NetworkInventory {
    pub fn check_size(&self) -> Result<()> {
        struct Count(usize);
        impl Write for Count {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if bytes.len() > MAX_REPORT_BYTES - self.0 {
                    return Err(std::io::Error::other("network report bound"));
                }
                self.0 += bytes.len();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        serde_json::to_writer(&mut Count(0), self).map_err(|_| limit())
    }
}

#[cfg(test)]
mod tests;
