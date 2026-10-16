//! Bounded APT baseline proposals, never executable package-manager parameters.
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, collections::BTreeSet, fmt, io::Write};

pub const MAX_PACKAGES: usize = 32;
pub const MAX_OFFERED: usize = 8;
pub const MAX_NAME_BYTES: usize = 128;
pub const MAX_VERSION_BYTES: usize = 128;
pub const MAX_TEXT_BYTES: usize = 32 * 1024;
pub const MAX_REPORT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageError {
    InvalidInput,
    Limit,
}
type Result<T> = std::result::Result<T, PackageError>;

pub fn package_name(name: &str) -> bool {
    (2..=MAX_NAME_BYTES).contains(&name.len())
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"+.-".contains(&b))
}
pub fn version_valid(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_VERSION_BYTES {
        return false;
    }
    let body = if let Some((epoch, body)) = value.split_once(':') {
        if epoch.is_empty()
            || !epoch.bytes().all(|b| b.is_ascii_digit())
            || epoch.trim_start_matches('0').len() > 9
        {
            return false;
        }
        body
    } else {
        value
    };
    let (upstream, revision) = body
        .rsplit_once('-')
        .map_or((body, None), |(a, b)| (a, Some(b)));
    upstream.as_bytes().first().is_some_and(u8::is_ascii_digit)
        && upstream
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".+-~".contains(&b))
        && revision.is_none_or(|r| {
            !r.is_empty()
                && r.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".+~".contains(&b))
        })
}
fn parts(value: &str) -> (&str, &str, &str) {
    let (epoch, body) = value.split_once(':').unwrap_or(("0", value));
    let (upstream, revision) = body.rsplit_once('-').unwrap_or((body, "0"));
    (epoch, upstream, revision)
}
fn numeric(a: &[u8], b: &[u8]) -> Ordering {
    let a = &a[a.iter().take_while(|b| **b == b'0').count()..];
    let b = &b[b.iter().take_while(|b| **b == b'0').count()..];
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}
fn order(value: Option<u8>) -> i16 {
    match value {
        Some(b'~') => -1,
        None | Some(b'0'..=b'9') => 0,
        Some(b) if b.is_ascii_alphabetic() => i16::from(b),
        Some(b) => i16::from(b) + 256,
    }
}
fn component(mut a: &[u8], mut b: &[u8]) -> Ordering {
    while !a.is_empty() || !b.is_empty() {
        while a.first().is_some_and(|c| !c.is_ascii_digit())
            || b.first().is_some_and(|c| !c.is_ascii_digit())
        {
            let cmp = order(a.first().copied()).cmp(&order(b.first().copied()));
            if cmp != Ordering::Equal {
                return cmp;
            }
            a = a.get(1..).unwrap_or_default();
            b = b.get(1..).unwrap_or_default();
        }
        let na = a.iter().take_while(|c| c.is_ascii_digit()).count();
        let nb = b.iter().take_while(|c| c.is_ascii_digit()).count();
        let cmp = numeric(&a[..na], &b[..nb]);
        if cmp != Ordering::Equal {
            return cmp;
        }
        a = &a[na..];
        b = &b[nb..];
    }
    Ordering::Equal
}
fn compare_valid(a: &str, b: &str) -> Ordering {
    let (ae, au, ar) = parts(a);
    let (be, bu, br) = parts(b);
    numeric(ae.as_bytes(), be.as_bytes())
        .then_with(|| component(au.as_bytes(), bu.as_bytes()))
        .then_with(|| component(ar.as_bytes(), br.as_bytes()))
}
pub fn compare_versions(a: &str, b: &str) -> Result<Ordering> {
    if !version_valid(a) || !version_valid(b) {
        return Err(PackageError::InvalidInput);
    }
    Ok(compare_valid(a, b))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Manager {
    Apt,
    Pip,
    Npm,
    Claude,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Policy {
    Present {},
    Absent {},
    Minimum { version: String },
    Pinned { version: String },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    AiAgents,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureState {
    Enabled,
    Disabled,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageSpec {
    pub name: String,
    pub manager: Manager,
    pub policy: Policy,
    pub critical: bool,
    pub disable_self_update: bool,
    pub feature: Option<Feature>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Selection {
    Install,
    Hold,
    Deinstall,
    Purge,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Installed {
    Unknown {},
    Absent {},
    Broken {},
    Present {
        version: String,
        selection: Selection,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Repository {
    Unknown {},
    Known {
        candidate: Option<String>,
        versions: Vec<String>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageFacts {
    pub name: String,
    pub installed: Installed,
    pub repository: Repository,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Compliant,
    Drift,
    Unknown,
    Blocked,
    Inactive,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Satisfied,
    NotInstalled,
    BelowMinimum,
    PinMismatch,
    MustBeAbsent,
    HoldMissing,
    UnknownInstalled,
    UnknownRepository,
    UnknownSelection,
    BrokenPackage,
    SelectedForRemoval,
    CandidateUnavailable,
    PinnedVersionUnavailable,
    HeldChangeNeedsReview,
    DowngradeNeedsReview,
    EquivalentPinNeedsReview,
    UnsupportedManager,
    UnsupportedSelfUpdateControl,
    FeatureDisabled,
    FeatureUnknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Step {
    InstallExact {
        version: String,
    },
    Remove,
    /// After verifying installation of exactly this version, not an unchecked batch.
    Hold {
        version: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PendingUpdate {
    NotApplicable,
    Unknown,
    NoCandidate,
    None,
    Available { version: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Assessment {
    pub spec: PackageSpec,
    pub facts: Option<PackageFacts>,
    pub status: Status,
    /// Version/presence policy only; holds and unavailable controls can still block.
    pub compliant: Option<bool>,
    pub reason: Reason,
    pub steps: Vec<Step>,
    pub pending_update: PendingUpdate,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageGate {
    TrustedManifestAndFeatureLifecycle,
    CompleteCurrentDpkgRepositoryFacts,
    SignedAptOriginAndExactArtifact,
    ResolvedAptSimulationAndDependencyClaims,
    FreshExecutableApproval,
    StateSpaceAndRecoveryPrerequisites,
    DatabaseMigrationCompatibility,
    DurableJobVerificationAndRecovery,
}
const GATES: &[PackageGate] = &[
    PackageGate::TrustedManifestAndFeatureLifecycle,
    PackageGate::CompleteCurrentDpkgRepositoryFacts,
    PackageGate::SignedAptOriginAndExactArtifact,
    PackageGate::ResolvedAptSimulationAndDependencyClaims,
    PackageGate::FreshExecutableApproval,
    PackageGate::StateSpaceAndRecoveryPrerequisites,
    PackageGate::DatabaseMigrationCompatibility,
    PackageGate::DurableJobVerificationAndRecovery,
];
pub struct PackagePlan {
    rows: Vec<Assessment>,
}
impl fmt::Debug for PackagePlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PackagePlan")
            .field("packages", &self.rows.len())
            .finish_non_exhaustive()
    }
}
#[derive(Serialize)]
pub struct PackageSummary<'a> {
    pub version: u16,
    pub rows: &'a [Assessment],
    pub unresolved: &'static [PackageGate],
}
impl PackagePlan {
    pub fn summary(&self) -> PackageSummary<'_> {
        PackageSummary {
            version: 1,
            rows: &self.rows,
            unresolved: GATES,
        }
    }
}
fn charge(text: &mut usize, n: usize) -> Result<()> {
    *text = text.checked_add(n).ok_or(PackageError::Limit)?;
    if *text > MAX_TEXT_BYTES {
        return Err(PackageError::Limit);
    }
    Ok(())
}
fn name(value: &str, text: &mut usize) -> Result<()> {
    if !package_name(value) {
        return Err(PackageError::InvalidInput);
    }
    charge(text, value.len())
}
fn version(value: &str, text: &mut usize) -> Result<()> {
    if !version_valid(value) {
        return Err(PackageError::InvalidInput);
    }
    charge(text, value.len())
}
fn admit(specs: &[PackageSpec], facts: &[PackageFacts]) -> Result<()> {
    if specs.is_empty() || specs.len() > MAX_PACKAGES || facts.len() > MAX_PACKAGES {
        return Err(PackageError::Limit);
    }
    let mut text = 0;
    let mut names = BTreeSet::new();
    for spec in specs {
        name(&spec.name, &mut text)?;
        if !names.insert(&spec.name) {
            return Err(PackageError::InvalidInput);
        }
        if let Policy::Minimum { version: v } | Policy::Pinned { version: v } = &spec.policy {
            version(v, &mut text)?;
        }
    }
    let mut observed = BTreeSet::new();
    for fact in facts {
        name(&fact.name, &mut text)?;
        if !names.contains(&fact.name) || !observed.insert(&fact.name) {
            return Err(PackageError::InvalidInput);
        }
        if let Installed::Present { version: v, .. } = &fact.installed {
            version(v, &mut text)?;
        }
        if let Repository::Known {
            candidate,
            versions,
        } = &fact.repository
        {
            if versions.len() > MAX_OFFERED {
                return Err(PackageError::Limit);
            }
            let mut unique = BTreeSet::new();
            for v in versions {
                version(v, &mut text)?;
                if !unique.insert(v) {
                    return Err(PackageError::InvalidInput);
                }
            }
            if let Some(v) = candidate {
                version(v, &mut text)?;
                if !unique.contains(v) {
                    return Err(PackageError::InvalidInput);
                }
            }
        }
    }
    Ok(())
}
fn pending(spec: &PackageSpec, fact: Option<&PackageFacts>) -> PendingUpdate {
    if !spec.critical && !matches!(spec.policy, Policy::Pinned { .. }) {
        return PendingUpdate::NotApplicable;
    }
    let Some(fact) = fact else {
        return PendingUpdate::Unknown;
    };
    let Repository::Known { candidate, .. } = &fact.repository else {
        return PendingUpdate::Unknown;
    };
    let Some(candidate) = candidate else {
        return PendingUpdate::NoCandidate;
    };
    match &fact.installed {
        Installed::Unknown {} | Installed::Broken {} => PendingUpdate::Unknown,
        Installed::Absent {} => PendingUpdate::Available {
            version: candidate.clone(),
        },
        Installed::Present { version, .. } => {
            if parts(version).1 == parts(candidate).1
                || compare_valid(version, candidate) != Ordering::Less
            {
                PendingUpdate::None
            } else {
                PendingUpdate::Available {
                    version: candidate.clone(),
                }
            }
        }
    }
}
fn block(row: &mut Assessment, status: Status, reason: Reason) {
    row.status = status;
    row.reason = reason;
    row.steps.clear();
}
fn assess_one(
    spec: &PackageSpec,
    fact: Option<&PackageFacts>,
    feature: FeatureState,
) -> Assessment {
    let mut row = Assessment {
        spec: spec.clone(),
        facts: fact.cloned(),
        status: Status::Unknown,
        compliant: None,
        reason: Reason::UnknownInstalled,
        steps: vec![],
        pending_update: PendingUpdate::NotApplicable,
    };
    if let Some(PackageFacts {
        repository: Repository::Known { versions, .. },
        ..
    }) = &mut row.facts
    {
        versions.sort_by(|a, b| compare_valid(a, b).then_with(|| a.cmp(b)));
    }
    if spec.feature.is_some() && feature != FeatureState::Enabled {
        block(
            &mut row,
            Status::Inactive,
            if feature == FeatureState::Disabled {
                Reason::FeatureDisabled
            } else {
                Reason::FeatureUnknown
            },
        );
        return row;
    }
    if spec.manager != Manager::Apt {
        block(&mut row, Status::Blocked, Reason::UnsupportedManager);
        return row;
    }
    row.pending_update = pending(spec, fact);
    if spec.disable_self_update {
        block(
            &mut row,
            Status::Blocked,
            Reason::UnsupportedSelfUpdateControl,
        );
        return row;
    }
    let Some(fact) = fact else {
        return row;
    };
    let (installed, held) = match &fact.installed {
        Installed::Unknown {} => return row,
        Installed::Broken {} => {
            block(&mut row, Status::Blocked, Reason::BrokenPackage);
            return row;
        }
        Installed::Absent {} => (None, false),
        Installed::Present { version, selection } => {
            match selection {
                Selection::Unknown => {
                    block(&mut row, Status::Unknown, Reason::UnknownSelection);
                    return row;
                }
                Selection::Deinstall | Selection::Purge => {
                    block(&mut row, Status::Blocked, Reason::SelectedForRemoval);
                    return row;
                }
                _ => {}
            }
            (Some(version.as_str()), *selection == Selection::Hold)
        }
    };
    let compliant = match &spec.policy {
        Policy::Absent {} => installed.is_none(),
        Policy::Present {} => installed.is_some(),
        Policy::Minimum { version } => {
            installed.is_some_and(|old| compare_valid(old, version) != Ordering::Less)
        }
        Policy::Pinned { version } => installed.is_some_and(|old| parts(old).1 == parts(version).1),
    };
    row.compliant = Some(compliant);
    row.status = if compliant {
        Status::Compliant
    } else {
        Status::Drift
    };
    row.reason = if compliant {
        Reason::Satisfied
    } else {
        match spec.policy {
            Policy::Absent {} => Reason::MustBeAbsent,
            _ if installed.is_none() => Reason::NotInstalled,
            Policy::Minimum { .. } => Reason::BelowMinimum,
            _ => Reason::PinMismatch,
        }
    };
    if compliant {
        if matches!(spec.policy, Policy::Pinned { .. }) && !held {
            row.status = Status::Drift;
            row.reason = Reason::HoldMissing;
            row.steps.push(Step::Hold {
                version: installed.unwrap().into(),
            });
        }
        return row;
    }
    if held {
        block(&mut row, Status::Blocked, Reason::HeldChangeNeedsReview);
        return row;
    }
    if matches!(spec.policy, Policy::Absent {}) {
        row.steps.push(Step::Remove);
        return row;
    }
    let Repository::Known {
        candidate,
        versions,
    } = &fact.repository
    else {
        block(&mut row, Status::Unknown, Reason::UnknownRepository);
        return row;
    };
    let target = match &spec.policy {
        Policy::Pinned { version } => versions
            .iter()
            .filter(|v| parts(v).1 == parts(version).1)
            .max_by(|a, b| compare_valid(a, b).then_with(|| a.cmp(b)))
            .map(String::as_str),
        Policy::Minimum { version } => candidate
            .as_deref()
            .filter(|v| compare_valid(v, version) != Ordering::Less),
        _ => candidate.as_deref(),
    };
    let Some(target) = target else {
        block(
            &mut row,
            Status::Blocked,
            if matches!(spec.policy, Policy::Pinned { .. }) {
                Reason::PinnedVersionUnavailable
            } else {
                Reason::CandidateUnavailable
            },
        );
        return row;
    };
    if let Some(old) = installed {
        match compare_valid(target, old) {
            Ordering::Less => {
                block(&mut row, Status::Blocked, Reason::DowngradeNeedsReview);
                return row;
            }
            Ordering::Equal => {
                block(&mut row, Status::Blocked, Reason::EquivalentPinNeedsReview);
                return row;
            }
            Ordering::Greater => {}
        }
    }
    row.steps.push(Step::InstallExact {
        version: target.into(),
    });
    if matches!(spec.policy, Policy::Pinned { .. }) {
        row.steps.push(Step::Hold {
            version: target.into(),
        });
    }
    row
}
struct Count(usize);
impl Write for Count {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_REPORT_BYTES.saturating_sub(self.0) {
            return Err(std::io::Error::other("package report bound"));
        }
        self.0 += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub fn plan_baseline(
    specs: &[PackageSpec],
    facts: &[PackageFacts],
    ai_agents: FeatureState,
) -> Result<PackagePlan> {
    admit(specs, facts)?;
    let mut rows: Vec<_> = specs
        .iter()
        .map(|spec| assess_one(spec, facts.iter().find(|f| f.name == spec.name), ai_agents))
        .collect();
    rows.sort_by(|a, b| a.spec.name.cmp(&b.spec.name));
    let plan = PackagePlan { rows };
    serde_json::to_writer(Count(0), &plan.summary()).map_err(|_| PackageError::Limit)?;
    Ok(plan)
}

#[cfg(test)]
mod tests;
