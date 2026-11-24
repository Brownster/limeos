//! Root operator target preparation. This is separate from non-executable API previews.
use crate::*;
pub use limeos_domain::{
    DirectoryIdentity, PreparationState, TargetEvidence, TargetPreparationPlan,
    TargetPreparationReceipt,
};
use limeos_domain::{StorageContract, StorageInventory, StorageTargetSnapshot};
use rustix::{
    fd::OwnedFd,
    fs::{Mode, OFlags},
};
use serde::de::DeserializeOwned;
use std::{
    fs::File,
    io::Read,
    time::{SystemTime, UNIX_EPOCH},
};

const OPERATION: &str = "storage.prepare_targets";
const JOURNAL_SCHEMA: &str = "BEGIN IMMEDIATE; CREATE TABLE receipts(action TEXT PRIMARY KEY,digest TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','verified','precondition_changed','outcome_unknown')),body TEXT NOT NULL) STRICT; CREATE TABLE locks(resource TEXT PRIMARY KEY,action TEXT NOT NULL REFERENCES receipts(action)) STRICT; PRAGMA user_version=1; COMMIT;";
fn now() -> Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Failure::Unavailable)?
        .as_secs()
        .try_into()
        .map_err(|_| Failure::Unavailable)
}
/// A plan or receipt encoded, refused (`InvalidPlan`) past 128 KiB.
fn encode<T: Serialize>(value: &T) -> Result<String> {
    crate::journal::encoded_within(value, 128 * 1024)
}
fn root() -> Result<()> {
    if !rustix::process::geteuid().is_root() {
        return Err(Failure::Unavailable);
    }
    Ok(())
}
/// A root-owned JSON file read by a root service: [`read_root_owned_json`],
/// refused unless this process runs as root.
pub fn read_protected_json<T: DeserializeOwned>(file: &Path) -> Result<T> {
    root()?;
    read_root_owned_json(file).map_err(ProtectedRead::failure)
}
/// Why a protected JSON file was not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtectedRead {
    /// The path is not a plain absolute path of at most 512 bytes.
    InvalidPath,
    /// A component does not exist.
    Missing,
    /// A component is a symlink or of the wrong type, not owned by the
    /// expected owner (root), or group- or world-writable.
    UnsafePath,
    /// The kernel refused this process access to a component.
    PermissionDenied,
    /// Not a regular file with one link and at most 64 KiB, or not the
    /// expected JSON.
    Malformed,
    /// Out of descriptors or memory, interrupted, or an I/O error.
    Unavailable,
}
impl ProtectedRead {
    pub fn message(self) -> &'static str {
        match self {
            ProtectedRead::InvalidPath => "the path is not a plain absolute path",
            ProtectedRead::Missing => "the file does not exist",
            ProtectedRead::UnsafePath => {
                "a component of the path is a symlink or not a directory, not root-owned, or group- or world-writable"
            }
            ProtectedRead::PermissionDenied => {
                "permission denied: this process may not search or read the path"
            }
            ProtectedRead::Malformed => {
                "not a single-link regular file of at most 64 KiB holding the expected JSON"
            }
            ProtectedRead::Unavailable => {
                "the file could not be read now (out of descriptors or memory, interrupted, or an I/O error)"
            }
        }
    }
}
impl std::fmt::Display for ProtectedRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}
impl ProtectedRead {
    /// The storage CLI's and root roles' coarser failure for this refusal.
    pub fn failure(self) -> Failure {
        match self {
            ProtectedRead::InvalidPath | ProtectedRead::Malformed => Failure::InvalidPlan,
            ProtectedRead::Missing => Failure::NotReady,
            ProtectedRead::UnsafePath => Failure::UnsafePath,
            ProtectedRead::PermissionDenied | ProtectedRead::Unavailable => Failure::Unavailable,
        }
    }
}
/// A root-owned JSON file, for any reader. Every component from `/` is
/// opened without following symlinks and must be root-owned and not group-
/// or world-writable; the file must be a regular file with one link and at
/// most 64 KiB. The unprivileged executors and `limeosctl check-ceiling`
/// read ceilings through this.
pub fn read_root_owned_json<T: DeserializeOwned>(
    file: &Path,
) -> std::result::Result<T, ProtectedRead> {
    read_owned_json_beneath(Path::new("/"), file, 0)
}
/// [`read_root_owned_json`] walked from `anchor` with `owner` in place of
/// root, `file` written as an absolute path beneath it. Only tests pass
/// anything but `/` and 0: they cannot create root-owned files, so they walk
/// their own directory as their own UID through the same checks.
#[doc(hidden)]
pub fn read_owned_json_beneath<T: DeserializeOwned>(
    anchor: &Path,
    file: &Path,
    owner: u32,
) -> std::result::Result<T, ProtectedRead> {
    use path::WalkError;
    let fd = path::walk_beneath(anchor, file, owner, false, true).map_err(|e| match e {
        WalkError::InvalidName => ProtectedRead::InvalidPath,
        WalkError::Missing => ProtectedRead::Missing,
        WalkError::Unsafe => ProtectedRead::UnsafePath,
        WalkError::Denied => ProtectedRead::PermissionDenied,
        WalkError::Unavailable => ProtectedRead::Unavailable,
    })?;
    let s = path::stat(&fd).map_err(|_| ProtectedRead::Unavailable)?;
    if s.stx_mode & 0o170000 != 0o100000 || s.stx_nlink != 1 || s.stx_size > 65536 {
        return Err(ProtectedRead::Malformed);
    }
    let mut bytes = Vec::new();
    File::from(fd)
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| ProtectedRead::Unavailable)?;
    if bytes.len() > 65536 {
        return Err(ProtectedRead::Malformed);
    }
    serde_json::from_slice(&bytes).map_err(|_| ProtectedRead::Malformed)
}
pub fn read_contract(file: &Path) -> Result<StorageContract> {
    read_protected_json(file)
}
pub fn read_target_plan(file: &Path) -> Result<TargetPreparationPlan> {
    read_protected_json(file)
}
fn identity(fd: &OwnedFd) -> Result<DirectoryIdentity> {
    let s = path::stat(fd)?;
    if s.stx_mode & 0o170000 != 0o040000 || s.stx_uid != 0 || s.stx_mode & 0o022 != 0 {
        return Err(Failure::UnsafePath);
    }
    Ok(DirectoryIdentity {
        major: s.stx_dev_major,
        minor: s.stx_dev_minor,
        mount_id: s.stx_mnt_id,
        inode: s.stx_ino,
        mode: s.stx_mode,
        uid: s.stx_uid,
        gid: s.stx_gid,
    })
}
fn directory(parent: &OwnedFd, name: &str) -> Result<Option<OwnedFd>> {
    match rustix::fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => Ok(Some(fd)),
        Err(e) if e == rustix::io::Errno::NOENT => Ok(None),
        Err(_) => Err(Failure::UnsafePath),
    }
}
fn empty(fd: &OwnedFd) -> Result<()> {
    let mut dir = rustix::fs::Dir::read_from(fd).map_err(|_| Failure::Unavailable)?;
    for entry in &mut dir {
        let entry = entry.map_err(|_| Failure::Unavailable)?;
        if !matches!(entry.file_name().to_bytes(), b"." | b"..") {
            return Err(Failure::UnsafePath);
        }
    }
    Ok(())
}
struct Target {
    evidence: TargetEvidence,
    parent: OwnedFd,
    name: String,
    _existing: Option<OwnedFd>,
}
fn inspect(contract: &StorageContract, inventory: &StorageInventory) -> Result<Vec<Target>> {
    // Also rejects unsupported filesystems, boot/swap aliases and foreign fstab ownership.
    limeos_domain::storage_managed_fstab(contract, 10, inventory)
        .map_err(|_| Failure::InvalidPlan)?;
    let mounts = mounts::parse(&bounded_read(
        Path::new("/proc/self/mountinfo"),
        1024 * 1024,
    )?)?;
    let mut assignments = contract.devices.clone();
    assignments.sort_by(|a, b| a.mountpoint.cmp(&b.mountpoint));
    let mut result = Vec::new();
    for assignment in assignments {
        let target = Path::new(&assignment.mountpoint);
        // Refuse occupied paths, nested mounts and mounted selected devices. A
        // mounted view cannot prove the safety of the directory hidden beneath.
        let actual = inventory
            .devices
            .iter()
            .find(|d| d.filesystem_uuid == assignment.filesystem_uuid)
            .ok_or(Failure::IdentityMismatch)?;
        if !actual.mounts.is_empty()
            || mounts.iter().any(|m| {
                m.path == assignment.mountpoint
                    || m.path.starts_with(&(assignment.mountpoint.clone() + "/"))
            })
        {
            return Err(Failure::Conflict);
        }
        let parent_path = target.parent().ok_or(Failure::InvalidPlan)?;
        let parent = path::open(parent_path, true, true)?;
        let parent_identity = identity(&parent)?;
        if parent_identity.mount_id != inventory.host_root_mount_id {
            return Err(Failure::UnsafePath);
        }
        let name = target
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or(Failure::InvalidPlan)?
            .to_string();
        let existing = directory(&parent, &name)?;
        let existing_identity = existing.as_ref().map(identity).transpose()?;
        if let Some(fd) = &existing {
            if identity(fd)?.mount_id != parent_identity.mount_id {
                return Err(Failure::UnsafePath);
            }
            empty(fd)?;
        }
        result.push(Target {
            evidence: TargetEvidence {
                mountpoint: assignment.mountpoint,
                parent: parent_identity,
                existing: existing_identity,
            },
            parent,
            name,
            _existing: existing,
        });
    }
    Ok(result)
}
fn evidence(targets: &[Target]) -> Vec<TargetEvidence> {
    targets.iter().map(|t| t.evidence.clone()).collect()
}
pub async fn target_snapshot(contract: &StorageContract) -> Result<StorageTargetSnapshot> {
    root()?;
    let expected = inventory().await?;
    let targets = inspect(contract, &expected)?;
    let targets = evidence(&targets);
    if inventory().await? != expected || evidence(&inspect(contract, &expected)?) != targets {
        return Err(Failure::Conflict);
    }
    Ok(StorageTargetSnapshot {
        inventory: expected,
        targets,
    })
}
pub async fn target_plan(contract: &StorageContract) -> Result<TargetPreparationPlan> {
    root()?;
    let expected = inventory().await?;
    let targets = inspect(contract, &expected)?;
    if inventory().await? != expected
        || evidence(&inspect(contract, &expected)?) != evidence(&targets)
    {
        return Err(Failure::Conflict);
    }
    let created_at = now()?;
    let plan = TargetPreparationPlan {
        version: 1,
        operation: OPERATION.into(),
        action: limeos_identity::opaque().map_err(|_| Failure::Unavailable)?,
        expected,
        contract: contract.clone(),
        targets: evidence(&targets),
        created_at,
        expires_at: created_at + 300,
    };
    plan.validate(created_at)
        .map_err(|_| Failure::InvalidPlan)?;
    if encode(&plan)?.len() > 65536 {
        return Err(Failure::InvalidPlan);
    }
    Ok(plan)
}

/// The target-preparation journal (`targets.sqlite`): the shared storage
/// receipt journal (`journal`), sharing its directory and process lock with
/// the mounts journal.
type Journal = crate::journal::Journal<TargetPreparationReceipt>;
impl crate::journal::Receipt for TargetPreparationReceipt {
    const DATABASE: &'static str = "targets.sqlite";
    const SCHEMA: &'static str = JOURNAL_SCHEMA;
    const BODY_BYTES: usize = 128 * 1024;
    const UNSAFE_DIRECTORY: Failure = Failure::UnsafePath;
    fn action(&self) -> &str {
        &self.plan.action
    }
    fn digest(&self) -> &str {
        &self.digest
    }
    fn state_name(&self) -> &'static str {
        state_name(self.state)
    }
    /// Prepared and unknown receipts keep every claim; only a verified or
    /// no-effect outcome releases them.
    fn holds_claims(&self) -> bool {
        matches!(
            self.state,
            PreparationState::Prepared | PreparationState::OutcomeUnknown
        )
    }
    fn claims(&self) -> Vec<String> {
        self.plan.resources()
    }
    fn plan_digest(&self) -> Result<String> {
        Ok(limeos_identity::digest(&encode(&self.plan)?))
    }
    fn admit(&self) -> bool {
        self.plan.validate(self.plan.created_at).is_ok()
    }
    fn same_subject(&self, next: &Self) -> bool {
        self.plan == next.plan && self.digest == next.digest
    }
}
impl Journal {
    fn receipt(
        &self,
        action: &str,
        digest: Option<&str>,
    ) -> Result<Option<TargetPreparationReceipt>> {
        self.stored(action, digest)
    }
    fn begin(&mut self, receipt: &TargetPreparationReceipt) -> Result<()> {
        self.insert(receipt)
    }
    /// Any change from a prepared or unknown receipt; a verified or no-effect
    /// outcome releases its claims in the same transaction.
    fn finish(&mut self, receipt: &TargetPreparationReceipt) -> Result<()> {
        let before = self
            .receipt(&receipt.plan.action, Some(&receipt.digest))?
            .ok_or(Failure::Conflict)?;
        self.replace(&before, receipt, |_, _| true)
    }
}
fn state_name(state: PreparationState) -> &'static str {
    match state {
        PreparationState::Prepared => "prepared",
        PreparationState::Verified => "verified",
        PreparationState::PreconditionChanged => "precondition_changed",
        PreparationState::OutcomeUnknown => "outcome_unknown",
    }
}
pub fn target_receipt(action: &str) -> Result<TargetPreparationReceipt> {
    Journal::readonly()?
        .receipt(action, None)?
        .ok_or(Failure::NotReady)
}
/// Explicit root reconciliation observes postconditions and changes receipts
/// only. It never repeats mkdir or removes an uncertain directory.
pub async fn reconcile_targets(action: &str) -> Result<TargetPreparationReceipt> {
    root()?;
    let mut journal = Journal::open()?;
    let mut receipt = journal.receipt(action, None)?.ok_or(Failure::NotReady)?;
    if !matches!(
        receipt.state,
        PreparationState::Prepared | PreparationState::OutcomeUnknown
    ) {
        return Ok(receipt);
    }
    receipt
        .plan
        .validate(receipt.plan.created_at)
        .map_err(|_| Failure::InvalidPlan)?;
    if inventory().await? != receipt.plan.expected {
        return Err(Failure::Conflict);
    }
    let current = evidence(&inspect(&receipt.plan.contract, &receipt.plan.expected)?);
    if current.len() != receipt.plan.targets.len()
        || current.iter().zip(&receipt.plan.targets).any(|(a, b)| {
            a.mountpoint != b.mountpoint
                || a.parent != b.parent
                || b.existing
                    .as_ref()
                    .is_some_and(|e| a.existing.as_ref() != Some(e))
        })
    {
        return Err(Failure::Conflict);
    }
    let resolved = if current == receipt.plan.targets {
        PreparationState::PreconditionChanged
    } else if current.iter().zip(&receipt.plan.targets).all(|(a, b)| {
        a.existing.as_ref().is_some_and(|e| {
            b.existing.is_some()
                || (e.uid == 0
                    && e.gid == 0
                    && e.mode & 0o7777 == 0o755
                    && e.mount_id == a.parent.mount_id)
        })
    }) {
        PreparationState::Verified
    } else {
        return Err(Failure::Conflict);
    };
    if inventory().await? != receipt.plan.expected
        || evidence(&inspect(&receipt.plan.contract, &receipt.plan.expected)?) != current
    {
        return Err(Failure::Conflict);
    }
    receipt.state = resolved;
    receipt.after = current;
    journal.finish(&receipt)?;
    Ok(receipt)
}
/// Explicit root review can resolve core death before IPC delivery. Absence of
/// a receipt alone is insufficient: the full original fresh target/device
/// evidence must still match. This never repeats creation and has no RPC path.
pub async fn reconcile_target_plan(
    plan: &TargetPreparationPlan,
) -> Result<TargetPreparationReceipt> {
    root()?;
    plan.validate(plan.created_at)
        .map_err(|_| Failure::InvalidPlan)?;
    let mut journal = Journal::open()?;
    let digest = limeos_identity::digest(&encode(plan)?);
    if journal.receipt(&plan.action, Some(&digest))?.is_some() {
        drop(journal);
        return reconcile_targets(&plan.action).await;
    }
    let before = target_snapshot(&plan.contract).await?;
    if before.inventory != plan.expected || before.targets != plan.targets {
        return Err(Failure::Conflict);
    }
    let mut receipt = TargetPreparationReceipt {
        plan: plan.clone(),
        digest,
        state: PreparationState::Prepared,
        after: vec![],
    };
    journal.begin(&receipt)?;
    // A race/death here retains the newly recorded barrier for explicit review.
    if target_snapshot(&plan.contract).await? != before {
        return Err(Failure::Conflict);
    }
    receipt.state = PreparationState::PreconditionChanged;
    receipt.after = before.targets;
    journal.finish(&receipt)?;
    Ok(receipt)
}
pub async fn prepare_targets(plan: &TargetPreparationPlan) -> Result<TargetPreparationReceipt> {
    root()?;
    let mut journal = Journal::open()?;
    let digest = limeos_identity::digest(&encode(plan)?);
    if let Some(receipt) = journal.receipt(&plan.action, Some(&digest))? {
        return Ok(receipt);
    }
    plan.validate(now()?).map_err(|_| Failure::InvalidPlan)?;
    if inventory().await? != plan.expected {
        return Err(Failure::Conflict);
    }
    let targets = inspect(&plan.contract, &plan.expected)?;
    if evidence(&targets) != plan.targets {
        return Err(Failure::Conflict);
    }
    let mut receipt = TargetPreparationReceipt {
        plan: plan.clone(),
        digest,
        state: PreparationState::Prepared,
        after: vec![],
    };
    journal.begin(&receipt)?;
    let result = async {
        let mut created_targets = Vec::new();
        // A durable prepared receipt exists before even the first directory.
        for target in &targets {
            if now()? >= plan.expires_at || inventory().await? != plan.expected {
                return Err(Failure::Conflict);
            }
            let fresh = inspect(&plan.contract, &plan.expected)?;
            let selected = fresh
                .iter()
                .find(|t| {
                    t.name == target.name && t.evidence.mountpoint == target.evidence.mountpoint
                })
                .ok_or(Failure::Conflict)?;
            if selected.evidence != target.evidence {
                return Err(Failure::Conflict);
            }
            let created = create(target)?;
            let mut prepared = target.evidence.clone();
            prepared.existing = Some(created);
            created_targets.push(prepared);
        }
        if inventory().await? != plan.expected {
            return Err(Failure::Conflict);
        }
        let after = evidence(&inspect(&plan.contract, &plan.expected)?);
        if after != created_targets {
            return Err(Failure::Conflict);
        }
        Ok(after)
    }
    .await;
    match result {
        Ok(after) => {
            receipt.after = after;
            receipt.state = PreparationState::Verified;
        }
        Err(_) => {
            receipt.state = PreparationState::OutcomeUnknown;
        }
    }
    // Any failure after durable dispatch keeps locks. Neither retries nor
    // receipt lookup repeats mkdir, deletes paths or releases uncertain claims.
    journal.finish(&receipt)?;
    Ok(receipt)
}
fn create(target: &Target) -> Result<DirectoryIdentity> {
    if identity(&target.parent)? != target.evidence.parent {
        return Err(Failure::Conflict);
    }
    let created = if target.evidence.existing.is_none() {
        rustix::fs::mkdirat(
            &target.parent,
            target.name.as_str(),
            Mode::from_raw_mode(0o755),
        )
        .map_err(|_| Failure::Conflict)?;
        let fd = directory(&target.parent, &target.name)?.ok_or(Failure::Conflict)?;
        rustix::fs::fchown(
            &fd,
            Some(rustix::fs::Uid::ROOT),
            Some(rustix::fs::Gid::ROOT),
        )
        .map_err(|_| Failure::Unavailable)?;
        rustix::fs::fchmod(&fd, Mode::from_raw_mode(0o755)).map_err(|_| Failure::Unavailable)?;
        let created = identity(&fd)?;
        empty(&fd)?;
        rustix::fs::fsync(&fd).map_err(|_| Failure::StateNotDurable)?;
        created
    } else {
        let fd = directory(&target.parent, &target.name)?.ok_or(Failure::Conflict)?;
        let existing = identity(&fd)?;
        if Some(&existing) != target.evidence.existing.as_ref() {
            return Err(Failure::Conflict);
        }
        empty(&fd)?;
        existing
    };
    let parent = rustix::fs::openat(
        &target.parent,
        ".",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| Failure::Unavailable)?;
    rustix::fs::fsync(parent).map_err(|_| Failure::StateNotDurable)?;
    Ok(created)
}

#[cfg(test)]
mod tests;
