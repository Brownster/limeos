use limeos_contracts::VERSION;
use limeos_domain::{Error, ErrorCode, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Ceiling {
    pub version: u16,
    pub core_uid: u32,
    pub allow_health: bool,
    #[serde(default)]
    pub allow_host_read: bool,
    #[serde(default)]
    pub allow_storage_read: bool,
    #[serde(default)]
    pub allow_stack_read: bool,
    #[serde(default)]
    pub allow_container_health_read: bool,
    #[serde(default)]
    pub allow_container_read: bool,
    #[serde(default)]
    pub allow_restart: bool,
    #[serde(default)]
    pub allow_start: bool,
    #[serde(default)]
    pub allow_stop: bool,
    #[serde(default)]
    pub allow_container_logs: bool,
    /// Whole-stack start/stop/restart, still bounded by the per-action flags,
    /// `managed_stacks` and every target in `managed_containers`.
    #[serde(default)]
    pub allow_stack: bool,
    #[serde(default)]
    pub managed_containers: Vec<String>,
    /// Compose project names.
    #[serde(default)]
    pub managed_stacks: Vec<String>,
}
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    ContainerHealth {
        version: u16,
        #[schemars(length(min = 64, max = 64))]
        nonce: String,
    },
    StackInventory {
        version: u16,
    },
    StorageMountCatalog {
        version: u16,
    },
    InspectStorageMount {
        version: u16,
        selection: limeos_domain::StorageMountSelection,
    },
    ExecuteStorageMount {
        version: u16,
        plan: Box<limeos_domain::StorageMountEffectPlan>,
    },
    StorageMountReceipt {
        version: u16,
        action: String,
        digest: String,
    },
    /// Refine only an existing accepted, reaped effect after fresh verification.
    /// This request cannot launch an effect or renew execution authority.
    ReconcileStorageMount {
        version: u16,
        action: String,
        digest: String,
    },
    InspectMountedStorage {
        version: u16,
        plan: Box<limeos_domain::StorageMountEffectPlan>,
    },
    ContainerStorageInventory {
        version: u16,
    },
    InspectStorageTargets {
        version: u16,
        contract: limeos_domain::StorageContract,
    },
    PrepareStorageTargets {
        version: u16,
        plan: Box<limeos_domain::TargetPreparationPlan>,
    },
    StorageTargetReceipt {
        version: u16,
        action: String,
        digest: String,
    },
    StorageInventory {
        version: u16,
    },
    Health {
        version: u16,
    },
    /// The container executor's current receipt-store epoch (`Receipt::epoch`),
    /// answered without reading or locking the store. Core asks for it
    /// immediately before it claims a job for dispatch, and dispatches the
    /// job under it.
    ReceiptEpoch {
        version: u16,
    },
    Observe {
        version: u16,
        source: limeos_contracts::Source,
        active: bool,
    },
    Inspect {
        version: u16,
        resource: String,
    },
    ContainerLogs {
        version: u16,
        resource: String,
        options: limeos_domain::LogOptions,
    },
    Restart {
        version: u16,
        request: limeos_domain::RestartRequest,
    },
    ExecuteContainer {
        version: u16,
        request: limeos_domain::ContainerRequest,
    },
    /// Receipt lookup. With no receipt, the answer's `absence` says whether
    /// one can ever have been pruned for `plan`. The executor recomputes the
    /// plan's digest and refuses the request unless it equals `digest`, so the
    /// expiry it judges by is the approved plan's own.
    RestartReceipt {
        version: u16,
        action: String,
        digest: String,
        /// The plan exactly as core stored it.
        plan: limeos_domain::ContainerPlan,
        /// With no row, record a closure: a terminal refusal that takes the
        /// action ID, so no execute request for it can run afterwards,
        /// whatever the clock says. Only for a caller that has lost the answer
        /// to its execute request; a status read leaves it false and writes
        /// nothing. A `never_recorded` closure is answered with its receipt
        /// (`precondition_changed`), a `no_longer_held` one with that absence;
        /// either way the answer's `closed` is set.
        #[serde(default)]
        close_if_absent: bool,
    },
    VerifyRestart {
        version: u16,
        action: String,
        digest: String,
        after: limeos_domain::ContainerSnapshot,
    },
    /// Fresh Engine listing of one Compose project's ordinary members.
    InspectStack {
        version: u16,
        project: String,
    },
    ExecuteStack {
        version: u16,
        request: limeos_domain::StackRequest,
    },
    /// As `RestartReceipt`, for a whole-stack action.
    StackReceipt {
        version: u16,
        action: String,
        digest: String,
        plan: limeos_domain::StackPlan,
        /// As for `RestartReceipt`.
        #[serde(default)]
        close_if_absent: bool,
    },
    /// Fresh inspections of the plan's targets; never launches an effect.
    VerifyStack {
        version: u16,
        action: String,
        digest: String,
        after: Vec<limeos_domain::ContainerSnapshot>,
    },
    /// Operator resolution of a stuck action. The executor re-inspects the
    /// action's targets read-only and, only if the result still hashes to
    /// `evidence_digest`, moves its own rows to `resolved` so the resource
    /// frees. Idempotent by action. Never issues an Engine start, stop or
    /// restart. With no row it always records a closure, as a receipt lookup
    /// with `close_if_absent` does, and answers the absence with `closed`.
    ResolveAction {
        version: u16,
        kind: limeos_domain::ResolveKind,
        action: String,
        /// Digest of the stored plan, as for the receipt lookups.
        digest: String,
        evidence_digest: String,
        declared: limeos_domain::DeclaredOutcome,
        /// The resolving administrator's principal ID, recorded in the row.
        resolved_by: String,
        /// The plan exactly as core stored it: of `kind`, and hashing to
        /// `digest`, or the request is invalid. With no row, its own
        /// `expires_at` decides the answer's `absence`.
        plan: limeos_domain::ActionPlan,
        /// Core holds a record a `never_recorded` closure for the action
        /// would contradict: the receipt store changed while core held the
        /// job (`executor_store_changed`), or core recorded a result for it.
        /// Such a closure is then not evidence, and every declaration is left
        /// to the operator's fresh measurement; without it a `never_recorded`
        /// closure allows only what a `precondition_changed` receipt allows.
        #[serde(default)]
        disputed: bool,
    },
}
/// Storage reads the storage reader abandoned at their deadline and that are
/// still stuck (typically in the kernel, on a failing disk). Restarting the
/// reader would not free them; it refuses further inventories meanwhile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StrandedReads {
    pub count: u32,
    /// When the oldest of them was abandoned (Unix seconds).
    #[ts(type = "number")]
    pub since: i64,
}
/// Why the storage reader could not answer a container-health read it was
/// granted: a closed, non-secret code (no Engine text, name or path) sent
/// with `error` so an operator can act on "unavailable". The reader logs the
/// same reason in its journal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum HealthReadFailure {
    /// The reader itself was over its memory or descriptor budget before
    /// starting the health worker.
    ReaderBudget,
    /// Another protected read held the single worker slot.
    WorkerBusy,
    /// The worker did not finish within its 4 s window.
    WorkerDeadline,
    /// The worker held a capability and refused to read.
    WorkerCapability,
    /// The worker or its guardian could not start, exited abnormally or
    /// wrote a malformed report.
    WorkerFailed,
    /// A worker's cleanup was not confirmed earlier; protected reads stay
    /// refused until the reader restarts.
    WorkerCleanup,
    /// The worker was over its memory budget.
    WorkerMemory,
    /// The worker was over its descriptor, file-limit or thread budget.
    WorkerDescriptors,
    /// The Engine's socket or peer is not the root-owned Engine.
    EngineForbidden,
    /// The Engine read was refused: the Engine was unreachable or slow, an
    /// answer was over its byte bound, or the Engine is past a bound the read
    /// does not truncate (more than 1,024 listed or 256 selected mounts).
    EngineUnavailable,
    /// The Engine read ran out of its counted memory allowance.
    EngineOverloaded,
    /// The Engine changed while it was read; the next read may succeed.
    EngineChanged,
    /// The answer was over the cohort's member or byte bound.
    CohortBound,
}
impl HealthReadFailure {
    /// One line for the reader's journal and `limeosctl reader-status`.
    pub fn message(self) -> &'static str {
        match self {
            Self::ReaderBudget => {
                "the storage reader was over its memory or descriptor budget before starting the health worker"
            }
            Self::WorkerBusy => "another protected read held the health worker's slot",
            Self::WorkerDeadline => "the health worker did not finish within its 4 s window",
            Self::WorkerCapability => {
                "the health worker held a capability and refused to read (its guardian did not clear the ambient set)"
            }
            Self::WorkerFailed => {
                "the health worker could not start, exited abnormally or wrote a malformed report"
            }
            Self::WorkerCleanup => {
                "an earlier worker's cleanup was not confirmed; protected reads stay refused until the reader restarts"
            }
            Self::WorkerMemory => "the health worker was over its 20 MiB memory budget",
            Self::WorkerDescriptors => {
                "the health worker was over its descriptor, file-limit or thread budget"
            }
            Self::EngineForbidden => {
                "the Engine socket or its peer is not the root-owned Docker Engine"
            }
            Self::EngineUnavailable => {
                "the Engine read was refused: the Engine was unreachable or slow, an answer was over its byte bound, or the Engine has more than 1,024 containers or 256 selected mounts"
            }
            Self::EngineOverloaded => "the Engine read ran out of its counted memory allowance",
            Self::EngineChanged => "the Engine changed while it was read",
            Self::CohortBound => "the health answer was over its member or byte bound",
        }
    }
}
// A whole container-health cohort and the receipt around it fit one frame.
const _: () = assert!(
    limeos_domain::automation::CONTAINER_HEALTH_BYTES + 4 * 1024 <= limeos_contracts::FRAME_LIMIT
);
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_health: Option<limeos_domain::automation::ContainerHealthCohort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_cohort: Option<limeos_domain::deployment::discovery::DiscoveryCohort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_mount_catalog: Option<limeos_domain::StorageMountCatalog>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_mount_inspection: Option<limeos_domain::StorageMountInspection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_mount: Option<limeos_domain::StorageMountReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mounted_storage: Option<limeos_domain::StorageMountedSnapshot>,
    pub version: u16,
    pub ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observations: Option<limeos_contracts::ObservationBatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspection: Option<limeos_domain::ContainerSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart: Option<limeos_domain::RestartReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack: Option<limeos_domain::StackReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_members: Option<limeos_domain::StackSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<limeos_domain::ResolveReceipt>,
    /// Set by a receipt lookup or `ResolveAction` that has no receipt to act
    /// on: it found no row, or a `no_longer_held` closure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub absence: Option<limeos_domain::ReceiptAbsence>,
    /// Set by a receipt lookup or `ResolveAction` when the executor holds a
    /// closure for the action, recorded now or earlier: no execute request
    /// for it can run any more. Without it an absence proves nothing about an
    /// execute request still on its way. Sent only when set, like every
    /// other optional answer, so a refusal stays version, `ready` and error.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub closed: bool,
    /// Set by `ResolveAction` when a terminal receipt's recorded outcome
    /// contradicts the declaration; nothing changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution_refusal: Option<limeos_domain::ResolveRefusal>,
    /// The container executor's receipt-store epoch, set on every answer that
    /// read or wrote the receipt store (execute, lookup, verify, resolve) and
    /// on the answer to `ReceiptEpoch`. It
    /// changes whenever the store is reopened, so a store lost and recreated,
    /// or restored from an older copy, never answers under the epoch core
    /// dispatched under: core then holds every job it dispatched under the
    /// earlier epoch for an operator instead of accepting a `never_recorded`
    /// closure for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logs: Option<limeos_domain::ContainerLogs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorCode>,
    /// Set with `error: rate_limited`: when, by the executor's clock, every
    /// container the refused action would charge has room in its effect
    /// budget again (ADR 0013).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(as = "Option<f64>", optional)]
    pub budget_frees_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<limeos_domain::StorageInventory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_targets: Option<limeos_domain::StorageTargetSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_preparation: Option<limeos_domain::TargetPreparationReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_storage: Option<limeos_domain::ContainerStorageInventory>,
    /// Set on the storage reader's answer to `Health` while it has a stuck
    /// read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stranded_reads: Option<StrandedReads>,
    /// Set with `error` on the storage reader's refusal of a granted
    /// container-health read: why it could not answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_failure: Option<HealthReadFailure>,
}
/// Independent, root-owned authority for the one closed nonpersistent mount.
#[derive(Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageMountCeiling {
    pub version: u16,
    pub core_uid: u32,
    pub allow_mount: bool,
    pub managed_targets: Vec<ManagedStorageMount>,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ManagedStorageMount {
    pub target_id: String,
    pub filesystem_uuid: String,
    pub mountpoint: String,
}
impl StorageMountCeiling {
    pub fn configuration_valid(&self) -> bool {
        if self.managed_targets.len() > 32 {
            return false;
        }
        let targets = StorageTargetCeiling {
            version: self.version,
            core_uid: self.core_uid,
            allow_prepare_targets: false,
            managed_targets: self
                .managed_targets
                .iter()
                .map(|target| ManagedStorageTarget {
                    filesystem_uuid: target.filesystem_uuid.clone(),
                    mountpoint: target.mountpoint.clone(),
                })
                .collect(),
        };
        let mut ids = std::collections::BTreeSet::new();
        targets.configuration_valid()
            && self.managed_targets.iter().all(|target| {
                limeos_domain::StorageMountSelection {
                    target_id: target.target_id.clone(),
                }
                .validate()
                .is_ok()
                    && ids.insert(&target.target_id)
            })
    }
    pub fn authorize_peer(&self, uid: u32) -> Result<()> {
        if !self.configuration_valid() || uid != self.core_uid {
            return Err(Error(ErrorCode::Forbidden));
        }
        Ok(())
    }
    pub fn authorize_selection(
        &self,
        uid: u32,
        selection: &limeos_domain::StorageMountSelection,
    ) -> Result<&ManagedStorageMount> {
        self.authorize_peer(uid)?;
        selection.validate()?;
        self.managed_targets
            .iter()
            .find(|target| target.target_id == selection.target_id)
            .ok_or(Error(ErrorCode::Forbidden))
    }
    pub fn authorize_plan(
        &self,
        uid: u32,
        plan: &limeos_domain::StorageMountEffectPlan,
        at: i64,
    ) -> Result<()> {
        let target = self.authorize_selection(
            uid,
            &limeos_domain::StorageMountSelection {
                target_id: plan.device.id.clone(),
            },
        )?;
        if !self.allow_mount
            || target.filesystem_uuid != plan.device.filesystem_uuid
            || target.mountpoint != plan.device.mountpoint
        {
            return Err(Error(ErrorCode::Forbidden));
        }
        plan.validate(at)
    }
}

/// A separate root-owned ceiling for the dormant directory-preparation service.
/// Exact UUID/path pairs constrain the core even if its authority is compromised.
#[derive(Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StorageTargetCeiling {
    pub version: u16,
    pub core_uid: u32,
    pub allow_prepare_targets: bool,
    pub managed_targets: Vec<ManagedStorageTarget>,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ManagedStorageTarget {
    pub filesystem_uuid: String,
    pub mountpoint: String,
}
impl StorageTargetCeiling {
    pub fn configuration_valid(&self) -> bool {
        let mut pairs = std::collections::BTreeSet::new();
        let mut paths = std::collections::BTreeSet::new();
        self.version == VERSION
            && self.core_uid != 0
            && self.managed_targets.len() <= 32
            && self.managed_targets.iter().all(|t| {
                !t.filesystem_uuid.is_empty()
                    && t.filesystem_uuid.len() <= 128
                    && t.filesystem_uuid
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                    && t.mountpoint.starts_with("/mnt/")
                    && t.mountpoint.len() <= 512
                    && !t.mountpoint.chars().any(char::is_control)
                    && !t.mountpoint.contains('\\')
                    && t.mountpoint
                        .split('/')
                        .skip(1)
                        .all(|p| !p.is_empty() && p != "." && p != "..")
                    && pairs.insert((&t.filesystem_uuid, &t.mountpoint))
                    && paths.insert(&t.mountpoint)
            })
    }
    pub fn authorize_contract(
        &self,
        uid: u32,
        contract: &limeos_domain::StorageContract,
    ) -> Result<()> {
        if !self.configuration_valid() || uid != self.core_uid || !self.allow_prepare_targets {
            return Err(Error(ErrorCode::Forbidden));
        }
        contract.validate()?;
        if contract.devices.iter().any(|d| {
            !self
                .managed_targets
                .iter()
                .any(|t| t.filesystem_uuid == d.filesystem_uuid && t.mountpoint == d.mountpoint)
        }) {
            return Err(Error(ErrorCode::Forbidden));
        }
        Ok(())
    }
}
/// The executor role a shared ceiling is read by, fixed by argv at start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CeilingRole {
    /// `limeos-storaged`: host reads and health only.
    Host,
    /// `limeos-containerd`.
    Container,
    /// The shadow package's container executor (`read-only` in argv).
    ContainerReadOnly,
    /// `limeos-storage-reader`: storage, stack and container-health reads.
    StorageReader,
}
impl CeilingRole {
    /// The names `limeosctl check-ceiling` takes.
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "host" => CeilingRole::Host,
            "container" => CeilingRole::Container,
            "container-read-only" => CeilingRole::ContainerReadOnly,
            "storage" => CeilingRole::StorageReader,
            _ => return None,
        })
    }
}
/// Why a well-parsed ceiling is refused for a role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CeilingRefusal {
    /// `core_uid` is 0.
    RootCore,
    /// Fails [`Ceiling::configuration_valid`].
    Invalid,
    /// The storage reader's ceiling grants no storage, stack or
    /// container-health read.
    ReaderWithoutReads,
    /// The storage reader's read flags, on another role.
    ReaderFlagsElsewhere,
    /// Writes or logs on a role that never performs them.
    WritesOnReadOnlyRole,
}
impl CeilingRefusal {
    pub fn message(self) -> &'static str {
        match self {
            CeilingRefusal::RootCore => {
                "the ceiling names core_uid 0; core runs as its own unprivileged user, so root is never accepted"
            }
            CeilingRefusal::Invalid => {
                "the ceiling fails validation (version, list sizes or IDs, or a flag that needs another)"
            }
            CeilingRefusal::ReaderWithoutReads => {
                "the storage reader requires a dedicated read-only ceiling granting a storage read"
            }
            CeilingRefusal::ReaderFlagsElsewhere => {
                "storage, stack and container-health reads belong only to the storage reader"
            }
            CeilingRefusal::WritesOnReadOnlyRole => {
                "this executor role cannot mutate containers or read their logs"
            }
        }
    }
}
impl std::fmt::Display for CeilingRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}
impl Ceiling {
    /// Whether the ceiling is well formed. A `core_uid` of 0 is refused, as
    /// the storage ceilings refuse it: core never runs as root, and a root
    /// peer must not pass the executors' peer check.
    pub fn configuration_valid(&self) -> bool {
        self.version == VERSION
            && self.core_uid != 0
            && self.managed_containers.len() <= 64
            && self
                .managed_containers
                .iter()
                .all(|id| limeos_domain::opaque_id(id))
            && self.managed_stacks.len() <= 16
            && self
                .managed_stacks
                .iter()
                .all(|project| limeos_domain::stack_project(project))
            && (!self.allow_stack || (self.allow_restart || self.allow_start || self.allow_stop))
            && (!(self.can_write() || self.allow_container_logs) || self.allow_container_read)
            && (!(self.allow_storage_read
                || self.allow_stack_read
                || self.allow_container_health_read)
                || (!self.can_write()
                    && !self.allow_container_read
                    && !self.allow_host_read
                    && !self.allow_container_logs))
    }
    pub fn can_write(&self) -> bool {
        self.allow_restart || self.allow_start || self.allow_stop || self.allow_stack
    }
    /// Whether this ceiling's flags permit a current-version `request` of
    /// its kind, before any check of its fields; `None` for another
    /// version, for kinds whose arm in `validate` answers `Forbidden`
    /// itself (executes and logs, which also check the target), and for
    /// requests this ceiling never serves.
    fn grants(&self, request: &Request) -> Option<bool> {
        use limeos_contracts::Source;
        let (version, granted) = match request {
            Request::ContainerHealth { version, .. } => (version, self.allow_container_health_read),
            Request::StackInventory { version } => (version, self.allow_stack_read),
            Request::ContainerStorageInventory { version }
            | Request::Inspect { version, .. }
            | Request::InspectStack { version, .. } => (version, self.allow_container_read),
            Request::StorageInventory { version } => (version, self.allow_storage_read),
            Request::Health { version } => (version, self.allow_health),
            Request::ReceiptEpoch { version }
            | Request::RestartReceipt { version, .. }
            | Request::VerifyRestart { version, .. } => (version, self.can_write()),
            Request::Observe {
                version, source, ..
            } => (
                version,
                match source {
                    Source::Host => self.allow_host_read,
                    Source::Docker => self.allow_container_read,
                    _ => return None,
                },
            ),
            Request::StackReceipt { version, .. } | Request::VerifyStack { version, .. } => {
                (version, self.allow_stack)
            }
            Request::ResolveAction { version, kind, .. } => (
                version,
                match kind {
                    limeos_domain::ResolveKind::Container => self.can_write(),
                    limeos_domain::ResolveKind::Stack => self.allow_stack,
                },
            ),
            _ => return None,
        };
        (*version == VERSION).then_some(granted)
    }
    /// Whether this ceiling may be served by `role`: `core_uid` is not 0,
    /// the ceiling is well formed, and its flags suit the role. The
    /// executors run this on every read of their ceiling, and
    /// `limeosctl check-ceiling` runs the same check.
    pub fn admit(&self, role: CeilingRole) -> std::result::Result<(), CeilingRefusal> {
        if self.core_uid == 0 {
            return Err(CeilingRefusal::RootCore);
        }
        if !self.configuration_valid() {
            return Err(CeilingRefusal::Invalid);
        }
        let reader_flags =
            self.allow_storage_read || self.allow_stack_read || self.allow_container_health_read;
        match role {
            CeilingRole::StorageReader if !reader_flags => Err(CeilingRefusal::ReaderWithoutReads),
            CeilingRole::Host | CeilingRole::Container | CeilingRole::ContainerReadOnly
                if reader_flags =>
            {
                Err(CeilingRefusal::ReaderFlagsElsewhere)
            }
            CeilingRole::ContainerReadOnly if self.can_write() => {
                Err(CeilingRefusal::WritesOnReadOnlyRole)
            }
            CeilingRole::Host | CeilingRole::StorageReader
                if self.can_write() || self.allow_container_logs =>
            {
                Err(CeilingRefusal::WritesOnReadOnlyRole)
            }
            _ => Ok(()),
        }
    }
    pub fn validate(&self, peer_uid: u32, request: &Request) -> Result<Receipt> {
        if !self.configuration_valid() || peer_uid != self.core_uid {
            return Err(Error(ErrorCode::Forbidden));
        }
        // A current request this ceiling does not permit is the ceiling's
        // "no" (`Forbidden`), which core treats as final, not a malformed
        // request (`InvalidInput`, the fall-through below). With writes
        // revoked, an operator's resolve request gets a definite refusal
        // instead of being retried.
        if self.grants(request) == Some(false) {
            return Err(Error(ErrorCode::Forbidden));
        }
        match request {
            Request::ContainerHealth { version, nonce }
                if *version == VERSION
                    && self.allow_container_health_read
                    && limeos_domain::opaque_id(nonce) =>
            {
                Ok(Receipt::empty())
            }
            Request::StackInventory { version } if *version == VERSION && self.allow_stack_read => {
                Ok(Receipt::empty())
            }
            Request::ContainerStorageInventory { version }
                if *version == VERSION && self.allow_container_read =>
            {
                Ok(Receipt::empty())
            }
            Request::StorageInventory { version }
                if *version == VERSION && self.allow_storage_read =>
            {
                Ok(Receipt::empty())
            }
            Request::Health { version } if *version == VERSION && self.allow_health => {
                Ok(Receipt::empty())
            }
            // Only an executor with effect authority keeps a receipt store.
            Request::ReceiptEpoch { version } if *version == VERSION && self.can_write() => {
                Ok(Receipt::empty())
            }
            Request::Observe {
                version, source, ..
            } if *version == VERSION
                && ((*source == limeos_contracts::Source::Host && self.allow_host_read)
                    || (*source == limeos_contracts::Source::Docker
                        && self.allow_container_read)) =>
            {
                Ok(Receipt::empty())
            }
            Request::Inspect { version, resource }
                if *version == VERSION
                    && self.allow_container_read
                    && limeos_domain::opaque_id(
                        resource.strip_prefix("container:").unwrap_or_default(),
                    ) =>
            {
                Ok(Receipt::empty())
            }
            Request::Restart { version, request } if *version == VERSION => {
                if request.plan.operation != limeos_domain::ContainerAction::Restart {
                    return Err(Error(ErrorCode::InvalidInput));
                }
                self.authorize_container(
                    peer_uid,
                    &request.plan.expected.resource,
                    request.plan.operation,
                )?;
                Ok(Receipt::empty())
            }
            Request::ContainerLogs {
                version,
                resource,
                options,
            } if *version == VERSION => {
                options.validate()?;
                let id = resource.strip_prefix("container:").unwrap_or_default();
                if !self.allow_container_logs
                    || !limeos_domain::opaque_id(id)
                    || !self.managed_containers.iter().any(|allowed| allowed == id)
                {
                    return Err(Error(ErrorCode::Forbidden));
                }
                Ok(Receipt::empty())
            }
            Request::ExecuteContainer { version, request } if *version == VERSION => {
                self.authorize_container(
                    peer_uid,
                    &request.plan.expected.resource,
                    request.plan.operation,
                )?;
                Ok(Receipt::empty())
            }
            Request::InspectStack { version, project }
                if *version == VERSION
                    && self.allow_container_read
                    && limeos_domain::stack_project(project) =>
            {
                Ok(Receipt::empty())
            }
            Request::ExecuteStack { version, request } if *version == VERSION => {
                self.authorize_stack(peer_uid, &request.plan.expected, request.plan.operation)?;
                Ok(Receipt::empty())
            }
            Request::StackReceipt {
                version,
                action,
                digest,
                ..
            }
            | Request::VerifyStack {
                version,
                action,
                digest,
                ..
            } if *version == VERSION
                && self.allow_stack
                && limeos_domain::opaque_id(action)
                && limeos_domain::opaque_id(digest) =>
            {
                if let Request::VerifyStack { after, .. } = request
                    && after.len() > limeos_domain::MAX_STACK_MEMBERS
                {
                    return Err(Error(ErrorCode::InvalidInput));
                }
                // The carried plan is bounded like an ExecuteStack plan.
                if let Request::StackReceipt { plan, .. } = request {
                    plan.validate_shape()?;
                }
                Ok(Receipt::empty())
            }
            Request::ResolveAction {
                version,
                kind,
                action,
                digest,
                evidence_digest,
                resolved_by,
                plan,
                ..
            } if *version == VERSION
                && match kind {
                    limeos_domain::ResolveKind::Container => self.can_write(),
                    limeos_domain::ResolveKind::Stack => self.allow_stack,
                }
                && [action, digest, evidence_digest, resolved_by]
                    .into_iter()
                    .all(|v| limeos_domain::opaque_id(v)) =>
            {
                if plan.kind() != *kind {
                    return Err(Error(ErrorCode::InvalidInput));
                }
                plan.validate_shape()?;
                Ok(Receipt::empty())
            }
            Request::RestartReceipt {
                version,
                action,
                digest,
                ..
            }
            | Request::VerifyRestart {
                version,
                action,
                digest,
                ..
            } if *version == VERSION
                && self.can_write()
                && limeos_domain::opaque_id(action)
                && limeos_domain::opaque_id(digest) =>
            {
                if let Request::RestartReceipt { plan, .. } = request {
                    plan.validate_shape()?;
                }
                Ok(Receipt::empty())
            }
            _ => Err(Error(ErrorCode::InvalidInput)),
        }
    }
    /// Stack authority adds to, and never replaces, the container ceiling: the
    /// project must be listed and every target must be an allowed container.
    pub fn authorize_stack(
        &self,
        peer_uid: u32,
        expected: &limeos_domain::StackSnapshot,
        operation: limeos_domain::ContainerAction,
    ) -> Result<()> {
        if !self.configuration_valid() || peer_uid != self.core_uid || !self.allow_stack {
            return Err(Error(ErrorCode::Forbidden));
        }
        expected.validate_for(operation)?;
        let project = expected.project()?;
        for target in expected.targets(operation) {
            self.authorize_container(peer_uid, &target.resource, operation)?;
        }
        if !self.managed_stacks.iter().any(|allowed| allowed == project) {
            return Err(Error(ErrorCode::Forbidden));
        }
        Ok(())
    }
    pub fn authorize_container(
        &self,
        peer_uid: u32,
        resource: &str,
        operation: limeos_domain::ContainerAction,
    ) -> Result<()> {
        let id = resource.strip_prefix("container:").unwrap_or_default();
        if self.version != VERSION
            || self.core_uid == 0
            || peer_uid != self.core_uid
            || !match operation {
                limeos_domain::ContainerAction::Restart => self.allow_restart,
                limeos_domain::ContainerAction::Start => self.allow_start,
                limeos_domain::ContainerAction::Stop => self.allow_stop,
            }
            || !limeos_domain::opaque_id(id)
            || self.managed_containers.len() > 64
            || self
                .managed_containers
                .iter()
                .any(|id| !limeos_domain::opaque_id(id))
            || !self.managed_containers.iter().any(|allowed| allowed == id)
        {
            return Err(Error(ErrorCode::Forbidden));
        }
        Ok(())
    }
}
impl Receipt {
    pub fn empty() -> Self {
        Self {
            container_health: None,
            stack_cohort: None,
            storage_mount_catalog: None,
            storage_mount_inspection: None,
            storage_mount: None,
            mounted_storage: None,
            version: VERSION,
            ready: true,
            observations: None,
            inspection: None,
            restart: None,
            stack: None,
            stack_members: None,
            resolution: None,
            absence: None,
            closed: false,
            resolution_refusal: None,
            epoch: None,
            logs: None,
            error: None,
            budget_frees_at: None,
            storage: None,
            storage_targets: None,
            target_preparation: None,
            container_storage: None,
            stranded_reads: None,
            health_failure: None,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    /// A refusal (a peer that is not core, a malformed request) carries the
    /// version, `ready: false` and the error, and nothing else: no field an
    /// answer would set, not even a false `closed`.
    #[test]
    fn a_refusal_is_version_ready_and_error_only_and_closed_is_sent_only_when_set() {
        let mut refusal = Receipt::empty();
        refusal.ready = false;
        refusal.error = Some(ErrorCode::Forbidden);
        assert_eq!(
            serde_json::to_string(&refusal).unwrap(),
            r#"{"version":1,"ready":false,"error":"forbidden"}"#
        );
        let mut lookup = Receipt::empty();
        lookup.closed = true;
        let wire = serde_json::to_value(&lookup).unwrap();
        assert_eq!(wire["closed"], true);
        assert!(serde_json::from_value::<Receipt>(wire).unwrap().closed);
        // An answer without the field is not closed.
        let open: Receipt = serde_json::from_str(r#"{"version":1,"ready":true}"#).unwrap();
        assert!(!open.closed);
    }
    #[test]
    fn stack_cache_read_is_closed_and_independent_from_other_read_and_effect_roles() {
        let mut c: Ceiling = serde_json::from_str(
            r#"{"version":1,"core_uid":1001,"allow_health":true,"allow_stack_read":true}"#,
        )
        .unwrap();
        assert!(c.configuration_valid());
        assert!(!c.can_write());
        assert!(
            c.validate(1001, &Request::StackInventory { version: VERSION })
                .is_ok()
        );
        assert!(
            c.validate(1002, &Request::StackInventory { version: VERSION })
                .is_err()
        );
        assert!(
            c.validate(1001, &Request::StackInventory { version: 2 })
                .is_err()
        );
        assert!(
            c.validate(1001, &Request::StorageInventory { version: VERSION })
                .is_err()
        );
        for field in [
            "path",
            "command",
            "endpoint",
            "principal",
            "approval",
            "resources",
        ] {
            let mut wire = serde_json::json!({"operation":"stack_inventory","version":1});
            wire[field] = serde_json::json!("caller");
            assert!(serde_json::from_value::<Request>(wire).is_err());
        }
        c.allow_host_read = true;
        assert!(!c.configuration_valid());
        c.allow_host_read = false;
        c.allow_container_read = true;
        assert!(!c.configuration_valid());
    }

    #[test]
    fn health_capture_requires_dedicated_read_ceiling_nonce_and_current_kernel_peer() {
        let mut ceiling: Ceiling = serde_json::from_str(
            r#"{"version":1,"core_uid":1001,"allow_health":true,"allow_storage_read":true}"#,
        )
        .unwrap();
        let request = Request::ContainerHealth {
            version: VERSION,
            nonce: "a".repeat(64),
        };
        assert!(ceiling.validate(1001, &request).is_err());
        ceiling.allow_container_health_read = true;
        assert!(ceiling.validate(1001, &request).is_ok());
        assert!(!ceiling.can_write());
        assert!(ceiling.validate(1002, &request).is_err());
        for nonce in ["".into(), "short".into(), "A".repeat(64), "a".repeat(65)] {
            assert!(
                ceiling
                    .validate(
                        1001,
                        &Request::ContainerHealth {
                            version: VERSION,
                            nonce
                        }
                    )
                    .is_err()
            );
        }
        assert!(
            ceiling
                .validate(
                    1001,
                    &Request::ContainerHealth {
                        version: 2,
                        nonce: "a".repeat(64)
                    }
                )
                .is_err()
        );
        for flag in [0, 1, 2, 3] {
            let mut mixed = ceiling.clone();
            match flag {
                0 => mixed.allow_host_read = true,
                1 => mixed.allow_container_read = true,
                2 => mixed.allow_container_logs = true,
                _ => {
                    mixed.allow_container_read = true;
                    mixed.allow_restart = true;
                }
            }
            assert!(!mixed.configuration_valid());
        }
        for field in [
            "endpoint", "uid", "clock", "path", "profile", "deadline", "command", "resource",
        ] {
            let mut wire = serde_json::json!({"operation":"container_health","version":1,"nonce":"a".repeat(64)});
            wire[field] = serde_json::json!("forbidden selector");
            assert!(serde_json::from_value::<Request>(wire).is_err());
        }
    }
    #[test]
    fn container_storage_read_requires_the_container_ceiling_and_accepts_no_filters() {
        let mut ceiling: Ceiling = serde_json::from_str(
            r#"{"version":1,"core_uid":1001,"allow_health":true,"allow_container_read":true}"#,
        )
        .unwrap();
        let request = Request::ContainerStorageInventory { version: VERSION };
        assert!(ceiling.validate(1001, &request).is_ok());
        assert!(ceiling.validate(1002, &request).is_err());
        assert!(
            ceiling
                .validate(1001, &Request::ContainerStorageInventory { version: 2 })
                .is_err()
        );
        assert!(!ceiling.can_write());
        ceiling.allow_container_read = false;
        ceiling.allow_host_read = true;
        assert!(ceiling.validate(1001, &request).is_err());
        for field in [
            "resource",
            "paths",
            "principal",
            "managed_containers",
            "command",
        ] {
            let mut value =
                serde_json::json!({"operation":"container_storage_inventory","version":1});
            value[field] = serde_json::json!("caller supplied");
            assert!(serde_json::from_value::<Request>(value).is_err());
        }
    }
    #[test]
    fn target_preparation_ceiling_requires_exact_uuid_and_literal_path_and_core_uid() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/storage-contracts.json"
        ))
        .unwrap();
        let contract: limeos_domain::StorageContract =
            serde_json::from_value(fixture["cases"][0]["contract"].clone()).unwrap();
        let mut ceiling = StorageTargetCeiling {
            version: VERSION,
            core_uid: 104,
            allow_prepare_targets: false,
            managed_targets: contract
                .devices
                .iter()
                .map(|d| ManagedStorageTarget {
                    filesystem_uuid: d.filesystem_uuid.clone(),
                    mountpoint: d.mountpoint.clone(),
                })
                .collect(),
        };
        assert!(ceiling.configuration_valid());
        assert_eq!(
            ceiling.authorize_contract(104, &contract).unwrap_err().0,
            ErrorCode::Forbidden
        );
        ceiling.allow_prepare_targets = true;
        assert!(ceiling.authorize_contract(104, &contract).is_ok());
        assert!(ceiling.authorize_contract(0, &contract).is_err());
        assert!(ceiling.authorize_contract(105, &contract).is_err());
        ceiling.managed_targets[0].filesystem_uuid = "replacement".into();
        assert!(ceiling.authorize_contract(104, &contract).is_err());
        ceiling.managed_targets[0].filesystem_uuid = contract.devices[0].filesystem_uuid.clone();
        for path in [
            "/",
            "/mnt/Data",
            "/mnt/data/../other",
            "/mnt//data",
            "/mnt/data/",
            "/mnt/data\n",
        ] {
            ceiling.managed_targets[0].mountpoint = path.into();
            assert!(
                ceiling.authorize_contract(104, &contract).is_err(),
                "{path:?}"
            );
        }
        let read_only: Ceiling = serde_json::from_str(
            r#"{"version":1,"core_uid":104,"allow_health":true,"allow_storage_read":true}"#,
        )
        .unwrap();
        assert!(
            read_only
                .validate(
                    104,
                    &Request::InspectStorageTargets {
                        version: VERSION,
                        contract
                    }
                )
                .is_err()
        );
    }
    #[test]
    fn a_ceiling_naming_root_as_core_is_invalid_and_answers_nothing() {
        let root: Ceiling =
            serde_json::from_str(r#"{"version":1,"core_uid":0,"allow_health":true}"#).unwrap();
        assert!(!root.configuration_valid());
        assert_eq!(
            root.validate(0, &Request::Health { version: VERSION })
                .err(),
            Some(Error(ErrorCode::Forbidden))
        );
        let core = Ceiling {
            core_uid: 1001,
            ..root
        };
        assert!(core.configuration_valid());
        assert!(
            core.validate(1001, &Request::Health { version: VERSION })
                .is_ok()
        );
    }
    #[test]
    fn a_request_kind_the_ceiling_does_not_grant_is_forbidden_not_invalid() {
        let none: Ceiling =
            serde_json::from_str(r#"{"version":1,"core_uid":1001,"allow_health":false}"#).unwrap();
        let id = || "d".repeat(64);
        let forbidden = Some(Error(ErrorCode::Forbidden));
        for request in [
            Request::Health { version: VERSION },
            Request::ReceiptEpoch { version: VERSION },
            Request::StackInventory { version: VERSION },
            Request::StorageInventory { version: VERSION },
            Request::ContainerStorageInventory { version: VERSION },
            Request::Observe {
                version: VERSION,
                source: limeos_contracts::Source::Host,
                active: false,
            },
            Request::Observe {
                version: VERSION,
                source: limeos_contracts::Source::Docker,
                active: false,
            },
            Request::Inspect {
                version: VERSION,
                resource: format!("container:{}", id()),
            },
            Request::InspectStack {
                version: VERSION,
                project: "media".into(),
            },
            Request::ContainerHealth {
                version: VERSION,
                nonce: id(),
            },
            Request::VerifyRestart {
                version: VERSION,
                action: id(),
                digest: id(),
                after: container_plan().expected,
            },
        ] {
            assert_eq!(
                none.validate(1001, &request).err(),
                forbidden,
                "{}",
                serde_json::to_string(&request).unwrap()
            );
        }
        // Another protocol version is still malformed, granted or not.
        assert_eq!(
            none.validate(1001, &Request::Health { version: 99 }).err(),
            Some(Error(ErrorCode::InvalidInput))
        );
        let health = Ceiling {
            allow_health: true,
            ..none
        };
        assert!(
            health
                .validate(1001, &Request::Health { version: VERSION })
                .is_ok()
        );
        assert_eq!(
            health
                .validate(1001, &Request::Health { version: 99 })
                .err(),
            Some(Error(ErrorCode::InvalidInput))
        );
    }
    #[test]
    fn each_role_admits_only_a_ceiling_meant_for_it() {
        use CeilingRole::*;
        let parse = |json: &str| serde_json::from_str::<Ceiling>(json).unwrap();
        let writer = parse(
            r#"{"version":1,"core_uid":1001,"allow_health":true,"allow_container_read":true,"allow_container_logs":true,"allow_restart":true,"managed_containers":[]}"#,
        );
        assert_eq!(writer.admit(Container), Ok(()));
        assert_eq!(
            writer.admit(ContainerReadOnly),
            Err(CeilingRefusal::WritesOnReadOnlyRole)
        );
        assert_eq!(
            writer.admit(Host),
            Err(CeilingRefusal::WritesOnReadOnlyRole)
        );
        let logs = parse(
            r#"{"version":1,"core_uid":1001,"allow_health":true,"allow_container_read":true,"allow_container_logs":true}"#,
        );
        assert_eq!(logs.admit(ContainerReadOnly), Ok(()));
        assert_eq!(logs.admit(Host), Err(CeilingRefusal::WritesOnReadOnlyRole));
        let host =
            parse(r#"{"version":1,"core_uid":1001,"allow_health":true,"allow_host_read":true}"#);
        assert_eq!(host.admit(Host), Ok(()));
        assert_eq!(
            host.admit(StorageReader),
            Err(CeilingRefusal::ReaderWithoutReads)
        );
        let reader =
            parse(r#"{"version":1,"core_uid":1001,"allow_health":true,"allow_stack_read":true}"#);
        assert_eq!(reader.admit(StorageReader), Ok(()));
        for role in [Host, Container, ContainerReadOnly] {
            assert_eq!(
                reader.admit(role),
                Err(CeilingRefusal::ReaderFlagsElsewhere)
            );
        }
        let root = Ceiling {
            core_uid: 0,
            ..host.clone()
        };
        for role in [Host, Container, ContainerReadOnly, StorageReader] {
            assert_eq!(root.admit(role), Err(CeilingRefusal::RootCore));
        }
        let invalid = Ceiling { version: 2, ..host };
        assert_eq!(invalid.admit(Host), Err(CeilingRefusal::Invalid));
        for (name, role) in [
            ("host", Host),
            ("container", Container),
            ("container-read-only", ContainerReadOnly),
            ("storage", StorageReader),
        ] {
            assert_eq!(CeilingRole::parse(name), Some(role));
        }
        assert_eq!(CeilingRole::parse("storage-reader"), None);
    }
    #[test]
    fn storage_reader_has_an_independent_read_only_ceiling_and_no_path_or_effect_input() {
        let old: Ceiling =
            serde_json::from_str(r#"{"version":1,"core_uid":1001,"allow_health":true}"#).unwrap();
        let request = Request::StorageInventory { version: VERSION };
        assert!(old.validate(1001, &request).is_err());
        let mut reader = Ceiling {
            allow_storage_read: true,
            ..old
        };
        assert!(reader.validate(1001, &request).is_ok());
        assert!(!reader.can_write());
        assert!(reader.validate(1002, &request).is_err());
        reader.allow_host_read = true;
        assert!(!reader.configuration_valid());
        for extra in ["path", "fstab", "command", "options", "principal"] {
            let raw =
                format!(r#"{{"operation":"storage_inventory","version":1,"{extra}":"attack"}}"#);
            assert!(serde_json::from_str::<Request>(&raw).is_err());
        }
        assert!(
            serde_json::from_str::<Request>(r#"{"operation":"storage_mount","version":1}"#)
                .is_err()
        );
    }
    #[test]
    fn log_ceiling_is_independent_managed_and_capped() {
        let id = "a".repeat(64);
        let mut ceiling = Ceiling {
            version: VERSION,
            core_uid: 1001,
            allow_health: true,
            allow_host_read: false,
            allow_storage_read: false,
            allow_stack_read: false,
            allow_container_health_read: false,
            allow_container_read: true,
            allow_restart: false,
            allow_start: false,
            allow_stop: false,
            allow_container_logs: false,
            allow_stack: false,
            managed_containers: vec![id.clone()],
            managed_stacks: Vec::new(),
        };
        let request = Request::ContainerLogs {
            version: VERSION,
            resource: format!("container:{id}"),
            options: limeos_domain::LogOptions { tail: 100 },
        };
        assert!(ceiling.validate(1001, &request).is_err());
        ceiling.allow_container_logs = true;
        assert!(ceiling.validate(1001, &request).is_ok());
        assert!(!ceiling.can_write());
        assert!(ceiling.validate(1002, &request).is_err());
        for tail in [0, 201, u16::MAX] {
            assert!(
                ceiling
                    .validate(
                        1001,
                        &Request::ContainerLogs {
                            version: VERSION,
                            resource: format!("container:{id}"),
                            options: limeos_domain::LogOptions { tail }
                        }
                    )
                    .is_err()
            );
        }
        ceiling.managed_containers.clear();
        assert!(ceiling.validate(1001, &request).is_err());
    }
    #[test]
    fn peer_and_ceiling_are_independent_and_no_content_or_command_is_accepted() {
        let ceiling = Ceiling {
            version: VERSION,
            core_uid: 1001,
            allow_health: true,
            allow_host_read: false,
            allow_storage_read: false,
            allow_stack_read: false,
            allow_container_health_read: false,
            allow_container_read: false,
            allow_restart: false,
            allow_start: false,
            allow_stop: false,
            allow_container_logs: false,
            allow_stack: false,
            managed_containers: Vec::new(),
            managed_stacks: Vec::new(),
        };
        assert!(
            ceiling
                .validate(1002, &Request::Health { version: VERSION })
                .is_err()
        );
        assert!(
            ceiling
                .validate(
                    1001,
                    &Request::Observe {
                        version: VERSION,
                        source: limeos_contracts::Source::Host,
                        active: true
                    }
                )
                .is_err()
        );
        let enabled = Ceiling {
            allow_host_read: true,
            ..ceiling.clone()
        };
        assert!(
            enabled
                .validate(
                    1001,
                    &Request::Observe {
                        version: VERSION,
                        source: limeos_contracts::Source::Host,
                        active: false
                    }
                )
                .is_ok()
        );
        assert!(
            enabled
                .validate(
                    1001,
                    &Request::Observe {
                        version: VERSION,
                        source: limeos_contracts::Source::Docker,
                        active: false
                    }
                )
                .is_err()
        );
        assert!(
            enabled
                .validate(
                    1002,
                    &Request::Observe {
                        version: VERSION,
                        source: limeos_contracts::Source::Host,
                        active: false
                    }
                )
                .is_err()
        );
        assert!(
            ceiling
                .validate(1001, &Request::Health { version: 99 })
                .is_err()
        );
        for extra in ["content", "command", "path", "principal"] {
            let raw = format!("{{\"operation\":\"health\",\"version\":1,\"{extra}\":\"attack\"}}");
            assert!(serde_json::from_str::<Request>(&raw).is_err());
        }
    }
    /// Only an executor with effect authority, which keeps the receipt
    /// store, answers its epoch; health access alone does not reach it.
    #[test]
    fn the_receipt_epoch_is_answered_only_by_an_executor_with_a_receipt_store() {
        let reader = Ceiling {
            version: VERSION,
            core_uid: 1001,
            allow_health: true,
            allow_host_read: false,
            allow_storage_read: false,
            allow_stack_read: false,
            allow_container_health_read: false,
            allow_container_read: true,
            allow_restart: false,
            allow_start: false,
            allow_stop: false,
            allow_container_logs: false,
            allow_stack: false,
            managed_containers: vec!["a".repeat(64)],
            managed_stacks: Vec::new(),
        };
        let request = Request::ReceiptEpoch { version: VERSION };
        assert!(reader.validate(1001, &request).is_err());
        let writer = Ceiling {
            allow_health: false,
            allow_stop: true,
            ..reader.clone()
        };
        assert!(writer.configuration_valid());
        assert!(writer.validate(1001, &request).is_ok());
        assert!(writer.validate(1002, &request).is_err());
        assert!(
            writer
                .validate(1001, &Request::ReceiptEpoch { version: 99 })
                .is_err()
        );
        let raw = r#"{"operation":"receipt_epoch","version":1,"action":"x"}"#;
        assert!(serde_json::from_str::<Request>(raw).is_err());
    }
    fn stack_member(n: char, running: bool) -> limeos_domain::ContainerSnapshot {
        limeos_domain::ContainerSnapshot {
            resource: format!("container:{}", n.to_string().repeat(64)),
            image: format!("sha256:{}", "b".repeat(64)),
            started_at: "2026-10-05T07:00:00.000000000Z".into(),
            running,
        }
    }
    fn stack_plan(operation: limeos_domain::ContainerAction) -> limeos_domain::StackPlan {
        let expected = limeos_domain::StackSnapshot::new(
            "media",
            vec![
                stack_member('a', true),
                stack_member('b', false),
                stack_member('c', true),
            ],
        )
        .unwrap();
        limeos_domain::StackPlan {
            id: "f".repeat(64),
            version: limeos_domain::STACK_VERSION,
            operation,
            principal: "alice".into(),
            grant_revision: 1,
            catalog_stack: "registered".into(),
            expected,
            created_at: 100,
            expires_at: 400,
        }
    }
    fn container_plan() -> limeos_domain::ContainerPlan {
        limeos_domain::ContainerPlan {
            id: "f".repeat(64),
            version: limeos_domain::RESTART_VERSION,
            operation: limeos_domain::ContainerAction::Restart,
            principal: "alice".into(),
            grant_revision: 1,
            expected: stack_member('a', true),
            created_at: 100,
            expires_at: 400,
        }
    }
    fn stack_request(operation: limeos_domain::ContainerAction) -> Request {
        Request::ExecuteStack {
            version: VERSION,
            request: limeos_domain::StackRequest {
                action: "d".repeat(64),
                plan_digest: "e".repeat(64),
                plan: stack_plan(operation),
            },
        }
    }
    fn stack_ceiling() -> Ceiling {
        serde_json::from_value(serde_json::json!({
            "version": 1, "core_uid": 1001, "allow_health": true,
            "allow_container_read": true, "allow_restart": true, "allow_stop": true,
            "allow_stack": true,
            "managed_containers": ["a".repeat(64), "b".repeat(64), "c".repeat(64)],
            "managed_stacks": ["media"]
        }))
        .unwrap()
    }
    #[test]
    fn stack_authority_needs_flag_action_project_peer_and_every_target() {
        use limeos_domain::ContainerAction::*;
        let ceiling = stack_ceiling();
        assert!(ceiling.configuration_valid() && ceiling.can_write());
        assert!(ceiling.validate(1001, &stack_request(Restart)).is_ok());
        assert!(ceiling.validate(1001, &stack_request(Stop)).is_ok());
        assert!(ceiling.validate(1002, &stack_request(Restart)).is_err());
        // Start is not enabled even though the stack itself is.
        assert!(ceiling.validate(1001, &stack_request(Start)).is_err());
        let change = |f: &dyn Fn(&mut Ceiling)| {
            let mut c = stack_ceiling();
            f(&mut c);
            c
        };
        for denied in [
            change(&|c| c.allow_stack = false),
            change(&|c| c.managed_stacks = vec!["other".into()]),
            change(&|c| c.managed_stacks.clear()),
            change(&|c| c.allow_restart = false),
            // A running target missing from the container allowlist.
            change(&|c| c.managed_containers.retain(|id| !id.starts_with('c'))),
        ] {
            assert!(denied.validate(1001, &stack_request(Restart)).is_err());
        }
        // Start acts only on the stopped member, so only that ID is required.
        let mut start = stack_ceiling();
        start.allow_start = true;
        start.managed_containers = vec!["b".repeat(64)];
        assert!(start.validate(1001, &stack_request(Start)).is_ok());
        // An unmanaged non-target member does not block, an unmanaged target does.
        let mut other = start.clone();
        other.managed_containers = vec!["a".repeat(64)];
        assert!(other.validate(1001, &stack_request(Start)).is_err());
    }
    #[test]
    fn stack_configuration_is_closed_and_shadow_visible_as_a_write_capability() {
        let mut c = stack_ceiling();
        c.allow_restart = false;
        c.allow_stop = false;
        assert!(!c.configuration_valid());
        for bad in ["Media", "../x", "", "a b", &"a".repeat(65)] {
            let mut c = stack_ceiling();
            c.managed_stacks = vec![bad.into()];
            assert!(!c.configuration_valid(), "{bad:?}");
        }
        let mut many = stack_ceiling();
        many.managed_stacks = (0..17).map(|i| format!("s{i}")).collect();
        assert!(!many.configuration_valid());
        // The read-only shadow refuses a ceiling for which can_write() is true.
        let mut flag_only: Ceiling = serde_json::from_str(
            r#"{"version":1,"core_uid":1001,"allow_health":true,"allow_container_read":true}"#,
        )
        .unwrap();
        assert!(!flag_only.can_write());
        flag_only.allow_stack = true;
        assert!(flag_only.can_write());
        // Defaults keep older policy files read-only for stacks.
        assert!(
            !serde_json::from_str::<Ceiling>(r#"{"version":1,"core_uid":1,"allow_health":true}"#)
                .unwrap()
                .allow_stack
        );
    }
    #[test]
    fn resolve_action_needs_the_matching_write_authority_and_closed_fields() {
        use limeos_domain::ResolveKind::{Container, Stack};
        let plan_of = |kind| match kind {
            Container => limeos_domain::ActionPlan::Container(container_plan()),
            Stack => limeos_domain::ActionPlan::Stack(stack_plan(
                limeos_domain::ContainerAction::Restart,
            )),
        };
        let resolve_with = |kind, plan| Request::ResolveAction {
            version: VERSION,
            kind,
            action: "d".repeat(64),
            digest: "e".repeat(64),
            evidence_digest: "f".repeat(64),
            declared: limeos_domain::DeclaredOutcome::NotDone,
            resolved_by: "a".repeat(64),
            plan,
            disputed: false,
        };
        let resolve = |kind| resolve_with(kind, plan_of(kind));
        let ceiling = stack_ceiling();
        assert!(ceiling.validate(1001, &resolve(Stack)).is_ok());
        assert!(ceiling.validate(1001, &resolve(Container)).is_ok());
        assert!(ceiling.validate(1002, &resolve(Stack)).is_err());
        let mut no_stack = stack_ceiling();
        no_stack.allow_stack = false;
        // Not permitted by the ceiling as read: a final "no", not a
        // malformed request, so core withdraws the resolve request.
        assert_eq!(
            no_stack.validate(1001, &resolve(Stack)).err(),
            Some(Error(ErrorCode::Forbidden))
        );
        assert!(no_stack.validate(1001, &resolve(Container)).is_ok());
        let mut read_only = stack_ceiling();
        read_only.allow_restart = false;
        read_only.allow_stop = false;
        read_only.allow_start = false;
        read_only.allow_stack = false;
        assert_eq!(
            read_only.validate(1001, &resolve(Container)).err(),
            Some(Error(ErrorCode::Forbidden))
        );
        let mut wire = serde_json::to_value(resolve(Stack)).unwrap();
        assert_eq!(wire["operation"], "resolve_action");
        wire["force"] = serde_json::json!(true);
        assert!(serde_json::from_value::<Request>(wire).is_err());
        // A request without the dispute flag is undisputed.
        let mut plain = serde_json::to_value(resolve(Stack)).unwrap();
        plain.as_object_mut().unwrap().remove("disputed");
        assert!(matches!(
            serde_json::from_value::<Request>(plain).unwrap(),
            Request::ResolveAction {
                disputed: false,
                ..
            }
        ));
        let Request::ResolveAction {
            version,
            kind,
            declared,
            ..
        } = resolve(Stack)
        else {
            unreachable!()
        };
        let bad = Request::ResolveAction {
            version,
            kind,
            action: "../x".into(),
            digest: "e".repeat(64),
            evidence_digest: "f".repeat(64),
            declared,
            resolved_by: "a".repeat(64),
            plan: plan_of(Stack),
            disputed: false,
        };
        assert_eq!(
            ceiling.validate(1001, &bad).err(),
            Some(Error(ErrorCode::InvalidInput))
        );
        // The carried plan must be of the named kind and well formed.
        assert!(
            ceiling
                .validate(1001, &resolve_with(Stack, plan_of(Container)))
                .is_err()
        );
        let mut malformed = stack_plan(limeos_domain::ContainerAction::Restart);
        malformed.expires_at = malformed.created_at + limeos_domain::PLAN_TTL_SECONDS + 1;
        assert!(
            ceiling
                .validate(
                    1001,
                    &resolve_with(Stack, limeos_domain::ActionPlan::Stack(malformed))
                )
                .is_err()
        );
        // An expired plan is still a valid question.
        let mut old = container_plan();
        old.created_at = 0;
        old.expires_at = 1;
        assert!(
            ceiling
                .validate(
                    1001,
                    &resolve_with(Container, limeos_domain::ActionPlan::Container(old))
                )
                .is_ok()
        );
    }
    #[test]
    fn stack_reads_receipts_and_verification_are_independent_bounded_and_closed() {
        let ceiling = stack_ceiling();
        let receipt = |version| Request::StackReceipt {
            version,
            action: "d".repeat(64),
            digest: "e".repeat(64),
            plan: stack_plan(limeos_domain::ContainerAction::Restart),
            close_if_absent: false,
        };
        assert!(ceiling.validate(1001, &receipt(VERSION)).is_ok());
        // The carried plan is bounded and well formed, as for ExecuteStack.
        let mut malformed = stack_plan(limeos_domain::ContainerAction::Restart);
        malformed.expected.members = vec![stack_member('a', true); 17];
        let oversized = Request::StackReceipt {
            version: VERSION,
            action: "d".repeat(64),
            digest: "e".repeat(64),
            plan: malformed,
            close_if_absent: true,
        };
        assert!(ceiling.validate(1001, &oversized).is_err());
        let mut bad_plan = container_plan();
        bad_plan.expires_at = bad_plan.created_at;
        let restart = |plan| Request::RestartReceipt {
            version: VERSION,
            action: "d".repeat(64),
            digest: "e".repeat(64),
            plan,
            close_if_absent: true,
        };
        assert!(ceiling.validate(1001, &restart(container_plan())).is_ok());
        assert!(ceiling.validate(1001, &restart(bad_plan)).is_err());
        let mut wire = serde_json::to_value(receipt(VERSION)).unwrap();
        wire["expires_at"] = serde_json::json!(400);
        assert!(serde_json::from_value::<Request>(wire).is_err());
        // Closing is explicit: a lookup that does not ask is a status read.
        let mut wire = serde_json::to_value(receipt(VERSION)).unwrap();
        wire.as_object_mut().unwrap().remove("close_if_absent");
        assert!(matches!(
            serde_json::from_value::<Request>(wire.clone()).unwrap(),
            Request::StackReceipt {
                close_if_absent: false,
                ..
            }
        ));
        wire["close_if_absent"] = serde_json::json!(true);
        assert!(matches!(
            serde_json::from_value::<Request>(wire).unwrap(),
            Request::StackReceipt {
                close_if_absent: true,
                ..
            }
        ));
        assert!(ceiling.validate(1001, &receipt(2)).is_err());
        assert!(ceiling.validate(1002, &receipt(VERSION)).is_err());
        let mut off = stack_ceiling();
        off.allow_stack = false;
        assert!(off.validate(1001, &receipt(VERSION)).is_err());
        let verify = |n: usize| Request::VerifyStack {
            version: VERSION,
            action: "d".repeat(64),
            digest: "e".repeat(64),
            after: vec![stack_member('a', true); n],
        };
        assert!(ceiling.validate(1001, &verify(2)).is_ok());
        assert!(ceiling.validate(1001, &verify(17)).is_err());
        let inspect = |project: &str| Request::InspectStack {
            version: VERSION,
            project: project.into(),
        };
        assert!(ceiling.validate(1001, &inspect("media")).is_ok());
        assert!(ceiling.validate(1001, &inspect("../media")).is_err());
        let mut no_read = stack_ceiling();
        no_read.allow_container_read = false;
        no_read.allow_restart = false;
        no_read.allow_stop = false;
        no_read.allow_stack = false;
        assert!(no_read.validate(1001, &inspect("media")).is_err());
        let mut wire =
            serde_json::to_value(stack_request(limeos_domain::ContainerAction::Stop)).unwrap();
        assert_eq!(wire["operation"], "execute_stack");
        wire["request"]["compose_file"] = serde_json::json!("/x");
        assert!(serde_json::from_value::<Request>(wire).is_err());
    }
}
