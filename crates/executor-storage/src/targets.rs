//! Root operator target preparation. This is separate from non-executable API previews.
use crate::*;
use limeos_domain::{StorageContract, StorageInventory, opaque_id};
use rusqlite::{Connection, OptionalExtension, params};
use rustix::{
    fd::OwnedFd,
    fs::{FlockOperation, Mode, OFlags},
};
use serde::{Deserialize, de::DeserializeOwned};
use std::{
    fs::File,
    io::Read,
    time::{SystemTime, UNIX_EPOCH},
};

const STATE: &str = "/var/lib/limeos/executors/storage";
const OPERATION: &str = "storage.prepare_targets";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryIdentity {
    pub major: u32,
    pub minor: u32,
    pub mount_id: u64,
    pub inode: u64,
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetEvidence {
    pub mountpoint: String,
    pub parent: DirectoryIdentity,
    pub existing: Option<DirectoryIdentity>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetPreparationPlan {
    pub version: u16,
    pub operation: String,
    pub action: String,
    pub expected: StorageInventory,
    pub contract: StorageContract,
    pub targets: Vec<TargetEvidence>,
    pub created_at: i64,
    pub expires_at: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreparationState {
    Prepared,
    Verified,
    PreconditionChanged,
    OutcomeUnknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetPreparationReceipt {
    pub plan: TargetPreparationPlan,
    pub digest: String,
    pub state: PreparationState,
    pub after: Vec<TargetEvidence>,
}
fn now() -> Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Failure::Unavailable)?
        .as_secs()
        .try_into()
        .map_err(|_| Failure::Unavailable)
}
fn encode<T: Serialize>(value: &T) -> Result<String> {
    let body = serde_json::to_string(value).map_err(|_| Failure::InvalidPlan)?;
    if body.len() > 128 * 1024 {
        return Err(Failure::InvalidPlan);
    }
    Ok(body)
}
fn root() -> Result<()> {
    if !rustix::process::geteuid().is_root() {
        return Err(Failure::Unavailable);
    }
    Ok(())
}
fn read_json<T: DeserializeOwned>(file: &Path) -> Result<T> {
    root()?;
    let fd = path::open(file, false, true)?;
    let s = path::stat(&fd)?;
    if s.stx_mode & 0o170000 != 0o100000 || s.stx_nlink != 1 || s.stx_size > 65536 {
        return Err(Failure::InvalidPlan);
    }
    let mut bytes = Vec::new();
    File::from(fd)
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::InvalidPlan)?;
    if bytes.len() > 65536 {
        return Err(Failure::InvalidPlan);
    }
    serde_json::from_slice(&bytes).map_err(|_| Failure::InvalidPlan)
}
pub fn read_contract(file: &Path) -> Result<StorageContract> {
    read_json(file)
}
pub fn read_target_plan(file: &Path) -> Result<TargetPreparationPlan> {
    read_json(file)
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
impl TargetPreparationPlan {
    fn validate(&self, at: i64) -> Result<()> {
        if self.version != 1
            || self.operation != OPERATION
            || !opaque_id(&self.action)
            || self.created_at < 0
            || self.expires_at <= self.created_at
            || self.expires_at > self.created_at.saturating_add(300)
            || at < self.created_at
            || at >= self.expires_at
            || self.targets.len() != self.contract.devices.len()
        {
            return Err(Failure::InvalidPlan);
        }
        self.contract.validate().map_err(|_| Failure::InvalidPlan)?;
        self.expected.validate().map_err(|_| Failure::InvalidPlan)?;
        Ok(())
    }
    fn resources(&self) -> Vec<String> {
        let mut resources = vec!["storage:configuration".into()];
        for d in &self.contract.devices {
            resources.push(format!("storage:uuid:{}", d.filesystem_uuid));
            resources.push(format!("storage:mount:{}", d.mountpoint));
        }
        resources.sort();
        resources.dedup();
        resources
    }
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
    plan.validate(created_at)?;
    if encode(&plan)?.len() > 65536 {
        return Err(Failure::InvalidPlan);
    }
    Ok(plan)
}

struct Journal {
    conn: Connection,
    _lock: OwnedFd,
    directory: OwnedFd,
}
fn durable(_: rusqlite::Error) -> Failure {
    Failure::StateNotDurable
}
fn private_file(fd: &OwnedFd, owner: u32) -> Result<()> {
    let s = path::stat(fd)?;
    if s.stx_mode & 0o170000 != 0o100000
        || s.stx_uid != owner
        || s.stx_nlink != 1
        || s.stx_mode & 0o077 != 0
        || s.stx_size > 16 * 1024 * 1024
    {
        return Err(Failure::StateNotDurable);
    }
    Ok(())
}
impl Journal {
    fn open() -> Result<Self> {
        Self::at(Path::new(STATE))
    }
    fn readonly() -> Result<Self> {
        root()?;
        let directory = path::open(Path::new(STATE), true, true)?;
        if path::stat(&directory)?.stx_mode & 0o077 != 0 {
            return Err(Failure::UnsafePath);
        }
        let lock = rustix::fs::openat(
            &directory,
            "targets.lock",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| Failure::StateNotDurable)?;
        private_file(&lock, 0)?;
        rustix::fs::flock(&lock, FlockOperation::NonBlockingLockShared)
            .map_err(|_| Failure::Conflict)?;
        for name in ["targets.sqlite", "targets.sqlite-wal", "targets.sqlite-shm"] {
            match rustix::fs::openat(
                &directory,
                name,
                OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                Ok(fd) => private_file(&fd, 0)?,
                Err(e) if e == rustix::io::Errno::NOENT && name != "targets.sqlite" => {}
                Err(_) => return Err(Failure::StateNotDurable),
            }
        }
        let conn = Connection::open_with_flags(
            format!("/proc/self/fd/{}/targets.sqlite", directory.as_raw_fd()),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(durable)?;
        conn.pragma_update(None, "trusted_schema", "OFF")
            .map_err(durable)?;
        if conn
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .map_err(durable)?
            != 1
        {
            return Err(Failure::StateNotDurable);
        }
        Ok(Self {
            conn,
            _lock: lock,
            directory,
        })
    }
    fn at(state: &Path) -> Result<Self> {
        root()?;
        let directory = path::open(state, true, true)?;
        if path::stat(&directory)?.stx_mode & 0o077 != 0 {
            return Err(Failure::UnsafePath);
        }
        Self::from_directory(directory, 0)
    }
    fn from_directory(directory: OwnedFd, owner: u32) -> Result<Self> {
        let lock = rustix::fs::openat(
            &directory,
            "targets.lock",
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|_| Failure::StateNotDurable)?;
        private_file(&lock, owner)?;
        rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive)
            .map_err(|_| Failure::Conflict)?;
        for name in [
            "targets.sqlite",
            "targets.sqlite-wal",
            "targets.sqlite-shm",
            "targets.sqlite-journal",
        ] {
            match rustix::fs::openat(
                &directory,
                name,
                OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                Ok(fd) => private_file(&fd, owner)?,
                Err(e) if e == rustix::io::Errno::NOENT => {}
                Err(_) => return Err(Failure::StateNotDurable),
            }
        }
        let file = rustix::fs::openat(
            &directory,
            "targets.sqlite",
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|_| Failure::StateNotDurable)?;
        private_file(&file, owner)?;
        let conn = Connection::open(format!(
            "/proc/self/fd/{}/targets.sqlite",
            directory.as_raw_fd()
        ))
        .map_err(durable)?;
        conn.busy_timeout(Duration::from_millis(200))
            .map_err(durable)?;
        if conn
            .pragma_query_value(None, "page_size", |r| r.get::<_, u32>(0))
            .map_err(durable)?
            != 4096
        {
            return Err(Failure::StateNotDurable);
        }
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON; PRAGMA max_page_count=4096; PRAGMA wal_autocheckpoint=128; PRAGMA journal_size_limit=1048576;").map_err(durable)?;
        match conn.pragma_query_value(None, "user_version", |r| r.get::<_,u32>(0)).map_err(durable)? {
            0 => conn.execute_batch("BEGIN IMMEDIATE; CREATE TABLE receipts(action TEXT PRIMARY KEY,digest TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','verified','precondition_changed','outcome_unknown')),body TEXT NOT NULL) STRICT; CREATE TABLE locks(resource TEXT PRIMARY KEY,action TEXT NOT NULL REFERENCES receipts(action)) STRICT; PRAGMA user_version=1; COMMIT;").map_err(durable)?,
            1 => {}, _ => return Err(Failure::StateNotDurable),
        }
        if conn
            .query_row("PRAGMA quick_check(1)", [], |r| r.get::<_, String>(0))
            .map_err(durable)?
            != "ok"
        {
            return Err(Failure::StateNotDurable);
        }
        conn.prepare("SELECT action,digest,state,body FROM receipts LIMIT 0")
            .map_err(durable)?;
        conn.prepare("SELECT resource,action FROM locks LIMIT 0")
            .map_err(durable)?;
        rustix::fs::fsync(&file).map_err(|_| Failure::StateNotDurable)?;
        // O_PATH cannot be fsynced. Reopen the retained directory descriptor.
        let sync = rustix::fs::openat(
            &directory,
            ".",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| Failure::StateNotDurable)?;
        rustix::fs::fsync(&sync).map_err(|_| Failure::StateNotDurable)?;
        Ok(Self {
            conn,
            _lock: lock,
            directory: sync,
        })
    }
    fn receipt(
        &self,
        action: &str,
        digest: Option<&str>,
    ) -> Result<Option<TargetPreparationReceipt>> {
        if !opaque_id(action) {
            return Err(Failure::InvalidPlan);
        }
        let row: Option<(String, String, String)> = self
            .conn
            .query_row(
                "SELECT digest,state,body FROM receipts WHERE action=?",
                [action],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(durable)?;
        let Some((stored, state, body)) = row else {
            return Ok(None);
        };
        if digest.is_some_and(|d| d != stored) {
            return Err(Failure::Conflict);
        }
        if body.len() > 128 * 1024 {
            return Err(Failure::StateNotDurable);
        }
        let receipt: TargetPreparationReceipt =
            serde_json::from_str(&body).map_err(|_| Failure::StateNotDurable)?;
        if receipt.plan.action != action
            || receipt.digest != stored
            || limeos_identity::digest(&encode(&receipt.plan)?) != stored
            || state != state_name(receipt.state)
        {
            return Err(Failure::StateNotDurable);
        }
        Ok(Some(receipt))
    }
    fn begin(&mut self, receipt: &TargetPreparationReceipt) -> Result<()> {
        let tx = self.conn.transaction().map_err(durable)?;
        if tx
            .query_row("SELECT count(*) FROM receipts", [], |r| r.get::<_, u32>(0))
            .map_err(durable)?
            >= 1024
        {
            return Err(Failure::StateNotDurable);
        }
        tx.execute(
            "INSERT INTO receipts VALUES(?,?,'prepared',?)",
            params![receipt.plan.action, receipt.digest, encode(receipt)?],
        )
        .map_err(durable)?;
        for resource in receipt.plan.resources() {
            tx.execute("INSERT INTO locks VALUES(?,?)", params![resource,receipt.plan.action]).map_err(|e| if matches!(e, rusqlite::Error::SqliteFailure(code, _) if code.code == rusqlite::ErrorCode::ConstraintViolation) { Failure::Conflict } else { durable(e) })?;
        }
        tx.commit().map_err(durable)?;
        rustix::fs::fsync(&self.directory).map_err(|_| Failure::StateNotDurable)
    }
    fn finish(&mut self, receipt: &TargetPreparationReceipt) -> Result<()> {
        let tx = self.conn.transaction().map_err(durable)?;
        if tx.execute("UPDATE receipts SET state=?,body=? WHERE action=? AND digest=? AND state IN ('prepared','outcome_unknown')", params![state_name(receipt.state),encode(receipt)?,receipt.plan.action,receipt.digest]).map_err(durable)? != 1 { return Err(Failure::Conflict); }
        if matches!(
            receipt.state,
            PreparationState::Verified | PreparationState::PreconditionChanged
        ) {
            tx.execute("DELETE FROM locks WHERE action=?", [&receipt.plan.action])
                .map_err(durable)?;
        }
        tx.commit().map_err(durable)
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
    receipt.plan.validate(receipt.plan.created_at)?;
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
pub async fn prepare_targets(plan: &TargetPreparationPlan) -> Result<TargetPreparationReceipt> {
    root()?;
    let mut journal = Journal::open()?;
    let digest = limeos_identity::digest(&encode(plan)?);
    if let Some(receipt) = journal.receipt(&plan.action, Some(&digest))? {
        return Ok(receipt);
    }
    plan.validate(now()?)?;
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
            create(target)?;
        }
        if inventory().await? != plan.expected {
            return Err(Failure::Conflict);
        }
        let after = evidence(&inspect(&plan.contract, &plan.expected)?);
        if after.iter().any(|t| t.existing.is_none()) {
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
fn create(target: &Target) -> Result<()> {
    if identity(&target.parent)? != target.evidence.parent {
        return Err(Failure::Conflict);
    }
    if target.evidence.existing.is_none() {
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
        identity(&fd)?;
        empty(&fd)?;
        rustix::fs::fsync(&fd).map_err(|_| Failure::StateNotDurable)?;
    }
    let parent = rustix::fs::openat(
        &target.parent,
        ".",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| Failure::Unavailable)?;
    rustix::fs::fsync(parent).map_err(|_| Failure::StateNotDurable)
}

#[cfg(test)]
mod tests;
