//! Retained protected file facts, never catalog, Compose semantics or effect authority.
use limeos_domain::{COMPOSE_MANAGED_FILE, ComposeFileName, Error, ErrorCode, Result};
use rustix::{
    fd::OwnedFd,
    fs::{AtFlags, Mode, OFlags, Statx, StatxFlags},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    io::{self, Write},
    path::Path,
    sync::atomic::{AtomicU8, AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(test)]
mod tests;
const FILE_BYTES: u64 = 64 * 1024;
const READ_BYTES: u64 = 1024 * 1024;
const REPORT_BYTES: usize = 16 * 1024;
const RETAINED_FDS: usize = 19;
const LIFETIME: Duration = Duration::from_secs(4);
const WORK: Duration = Duration::from_secs(2);
const BASES: [&str; 4] = [
    "compose.yaml",
    "compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
];
fn unavailable() -> Error {
    Error(ErrorCode::Unavailable)
}
fn conflict() -> Error {
    Error(ErrorCode::Conflict)
}
fn base_name(name: ComposeFileName) -> &'static str {
    match name {
        ComposeFileName::ComposeYaml => BASES[0],
        ComposeFileName::ComposeYml => BASES[1],
        ComposeFileName::DockerComposeYaml => BASES[2],
        ComposeFileName::DockerComposeYml => BASES[3],
    }
}

/// Explicit names only; no imported selection grants file or stack authority.
/// ```compile_fail
/// use limeos_executor_container::compose_files::ComposeFileSelection;
/// let imported: ComposeFileSelection = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Clone, Debug, Serialize)]
pub struct ComposeFileSelection {
    working_directory: String,
    operator: ComposeFileName,
    include_managed: bool,
}
impl ComposeFileSelection {
    pub fn new(
        working_directory: &Path,
        operator: ComposeFileName,
        include_managed: bool,
    ) -> Result<Self> {
        let text = working_directory
            .to_str()
            .ok_or(Error(ErrorCode::InvalidInput))?;
        if !text.starts_with('/')
            || text.len() > 512
            || text.chars().any(char::is_control)
            || text.contains(',')
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let depth = text[1..].split('/').count();
        if depth > 15
            || text[1..]
                .split('/')
                .any(|s| matches!(s, "" | "." | "..") || s.len() > 255)
            || BASES
                .iter()
                .chain([&COMPOSE_MANAGED_FILE])
                .any(|name| text.len() + 1 + name.len() > 512)
            || 2 + depth + 1 + usize::from(include_managed) > RETAINED_FDS
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        Ok(Self {
            working_directory: text.to_owned(),
            operator,
            include_managed,
        })
    }
    pub fn working_directory(&self) -> &Path {
        Path::new(&self.working_directory)
    }
    pub fn operator(&self) -> ComposeFileName {
        self.operator
    }
    pub fn includes_managed(&self) -> bool {
        self.include_managed
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ComposeObjectIdentity {
    pub major: u32,
    pub minor: u32,
    pub mount_id: u64,
    pub inode: u64,
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
}
impl From<&Statx> for ComposeObjectIdentity {
    fn from(s: &Statx) -> Self {
        Self {
            major: s.stx_dev_major,
            minor: s.stx_dev_minor,
            mount_id: s.stx_mnt_id,
            inode: s.stx_ino,
            mode: s.stx_mode,
            uid: s.stx_uid,
            gid: s.stx_gid,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ComposeFileIdentity {
    pub object: ComposeObjectIdentity,
    pub links: u32,
    pub size: u64,
    pub modified_seconds: i64,
    pub modified_nanoseconds: u32,
    pub changed_seconds: i64,
    pub changed_nanoseconds: u32,
}
impl From<&Statx> for ComposeFileIdentity {
    fn from(s: &Statx) -> Self {
        Self {
            object: s.into(),
            links: s.stx_nlink,
            size: s.stx_size,
            modified_seconds: s.stx_mtime.tv_sec,
            modified_nanoseconds: s.stx_mtime.tv_nsec,
            changed_seconds: s.stx_ctime.tv_sec,
            changed_nanoseconds: s.stx_ctime.tv_nsec,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ComposeMountView {
    pub device: u64,
    pub inode: u64,
}
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ComposeFileObservation {
    pub path: String,
    pub identity: ComposeFileIdentity,
    pub sha256: String,
}
/// Selected past metadata cannot reconstruct retained descriptors or original budgets.
/// ```compile_fail
/// use limeos_executor_container::compose_files::ComposeFileSnapshot;
/// let imported: ComposeFileSnapshot = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug, Serialize)]
pub struct ComposeFileSnapshot {
    pub version: u16,
    pub observed_at: i64,
    pub selection: ComposeFileSelection,
    pub mount_view: Option<ComposeMountView>,
    pub root: ComposeObjectIdentity,
    pub ancestors: Vec<ComposeObjectIdentity>,
    /// Ordered operator then managed file, when explicitly selected.
    pub files: Vec<ComposeFileObservation>,
    /// Only fixed recognized basenames; not absence of arbitrary includes or consumers.
    pub absent_names: Vec<String>,
}
struct View {
    fd: OwnedFd,
    identity: ComposeMountView,
}
struct Root {
    fd: OwnedFd,
    view: Option<View>,
}
struct Ancestor {
    name: String,
    fd: OwnedFd,
    identity: ComposeObjectIdentity,
}
struct File {
    name: &'static str,
    fd: OwnedFd,
    observation: ComposeFileObservation,
}

struct Budget {
    spent: AtomicU64,
    expires: Instant,
}
impl Budget {
    fn poll(&self, deadline: Instant, cancelled: &dyn Fn() -> bool) -> Result<()> {
        if cancelled() {
            return Err(conflict());
        }
        if Instant::now() >= self.expires || Instant::now() >= deadline {
            return Err(Error(ErrorCode::Expired));
        }
        Ok(())
    }
    fn charge(&self, bytes: usize) -> Result<()> {
        self.spent
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |spent| {
                spent
                    .checked_add(bytes as u64)
                    .filter(|total| *total <= READ_BYTES)
            })
            .map(|_| ())
            .map_err(|_| unavailable())
    }
}
struct Validation<'a> {
    state: &'a AtomicU8,
    completed: bool,
}
impl Validation<'_> {
    fn finish(mut self, result: Result<()>) -> Result<()> {
        self.completed = true;
        if result.is_ok()
            && self
                .state
                .compare_exchange(1, 0, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            Ok(())
        } else {
            self.state.store(2, Ordering::SeqCst);
            result.and(Err(conflict()))
        }
    }
}
impl Drop for Validation<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.state.store(2, Ordering::SeqCst);
        }
    }
}

/// Only protected collection constructs an owner. Every failed check poisons it.
/// ```compile_fail
/// use limeos_executor_container::compose_files::ComposeFileEvidence;
/// let imported: ComposeFileEvidence = serde_json::from_str("{}").unwrap();
/// ```
pub struct ComposeFileEvidence {
    root: Root,
    ancestors: Vec<Ancestor>,
    files: Vec<File>,
    snapshot: ComposeFileSnapshot,
    budget: Budget,
    state: AtomicU8,
    owner: u32,
}
impl ComposeFileEvidence {
    pub fn snapshot(&self) -> &ComposeFileSnapshot {
        &self.snapshot
    }
    pub fn retained_descriptor_count(&self) -> usize {
        1 + usize::from(self.root.view.is_some()) + self.ancestors.len() + self.files.len()
    }
    /// Fresh protected names and held bytes must still match the original observation.
    pub fn revalidate(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        self.checked(|| {
            let deadline = Instant::now() + WORK;
            self.budget.poll(deadline, cancelled)?;
            let fresh = host_root()?;
            self.validate_from(&fresh, now()?, deadline, cancelled)?;
            let final_root = host_root()?;
            self.check_root(&final_root)?;
            age(self.snapshot.observed_at, now()?)?;
            self.budget.poll(deadline, cancelled)
        })
    }
    fn checked(&self, operation: impl FnOnce() -> Result<()>) -> Result<()> {
        if self
            .state
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            self.state.store(2, Ordering::SeqCst);
            return Err(conflict());
        }
        let guard = Validation {
            state: &self.state,
            completed: false,
        };
        guard.finish(operation())
    }
    fn check_root(&self, fresh: &Root) -> Result<()> {
        if directory(&self.root.fd, self.owner)? != self.snapshot.root
            || directory(&fresh.fd, self.owner)? != self.snapshot.root
            || self.root.view.as_ref().map(|v| &v.identity) != self.snapshot.mount_view.as_ref()
            || fresh.view.as_ref().map(|v| &v.identity) != self.snapshot.mount_view.as_ref()
        {
            return Err(conflict());
        }
        if let Some(view) = &self.root.view
            && view_identity(&view.fd)? != view.identity
        {
            return Err(conflict());
        }
        Ok(())
    }
    fn validate_from(
        &self,
        fresh_root: &Root,
        at: i64,
        deadline: Instant,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<()> {
        self.budget.poll(deadline, cancelled)?;
        age(self.snapshot.observed_at, at)?;
        self.check_root(fresh_root)?;
        let mut fresh = None;
        for ancestor in &self.ancestors {
            self.budget.poll(deadline, cancelled)?;
            let fd = open_directory(fresh.as_ref().unwrap_or(&fresh_root.fd), &ancestor.name)?;
            if directory(&ancestor.fd, self.owner)? != ancestor.identity
                || directory(&fd, self.owner)? != ancestor.identity
            {
                return Err(conflict());
            }
            fresh = Some(fd);
        }
        let fresh_directory = fresh.as_ref().unwrap_or(&fresh_root.fd);
        let held_directory = self
            .ancestors
            .last()
            .map(|a| &a.fd)
            .unwrap_or(&self.root.fd);
        check_names(
            fresh_directory,
            &self.snapshot.selection,
            &self.budget,
            deadline,
            cancelled,
        )?;
        check_names(
            held_directory,
            &self.snapshot.selection,
            &self.budget,
            deadline,
            cancelled,
        )?;
        self.check_selected(fresh_directory, deadline, cancelled)?;
        self.check_selected(held_directory, deadline, cancelled)?;
        for file in &self.files {
            self.budget.poll(deadline, cancelled)?;
            let (_, observation) = read_file(
                fresh_directory,
                file.name,
                &self.snapshot.selection,
                self.owner,
                &self.budget,
                deadline,
                cancelled,
            )?;
            let (identity, digest) =
                hash_file(&file.fd, self.owner, &self.budget, deadline, cancelled)?;
            if observation != file.observation
                || identity != file.observation.identity
                || digest != file.observation.sha256
            {
                return Err(conflict());
            }
        }
        check_names(
            fresh_directory,
            &self.snapshot.selection,
            &self.budget,
            deadline,
            cancelled,
        )?;
        check_names(
            held_directory,
            &self.snapshot.selection,
            &self.budget,
            deadline,
            cancelled,
        )?;
        // Rewalk names after content reads; never let a detached directory qualify.
        let mut final_directory = None;
        for ancestor in &self.ancestors {
            self.budget.poll(deadline, cancelled)?;
            let fd = open_directory(
                final_directory.as_ref().unwrap_or(&fresh_root.fd),
                &ancestor.name,
            )?;
            if directory(&ancestor.fd, self.owner)? != ancestor.identity
                || directory(&fd, self.owner)? != ancestor.identity
            {
                return Err(conflict());
            }
            final_directory = Some(fd);
        }
        let final_directory = final_directory.as_ref().unwrap_or(&fresh_root.fd);
        check_names(
            final_directory,
            &self.snapshot.selection,
            &self.budget,
            deadline,
            cancelled,
        )?;
        self.check_selected(final_directory, deadline, cancelled)?;
        age(self.snapshot.observed_at, at)?;
        self.budget.poll(deadline, cancelled)
    }
    fn check_selected(
        &self,
        parent: &OwnedFd,
        deadline: Instant,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<()> {
        for file in &self.files {
            self.budget.poll(deadline, cancelled)?;
            let named = classified(parent, file.name)?.ok_or_else(conflict)?;
            if file_identity(&named, self.owner)? != file.observation.identity
                || file_identity(&file.fd, self.owner)? != file.observation.identity
            {
                return Err(conflict());
            }
        }
        Ok(())
    }
}

fn now() -> Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| conflict())?
        .as_secs()
        .try_into()
        .map_err(|_| conflict())
}
fn age(issued: i64, at: i64) -> Result<()> {
    if at
        .checked_sub(issued)
        .is_some_and(|age| (0..=4).contains(&age))
    {
        Ok(())
    } else {
        Err(Error(ErrorCode::Expired))
    }
}
fn stat(fd: &OwnedFd) -> Result<Statx> {
    let mask = StatxFlags::BASIC_STATS | StatxFlags::MNT_ID;
    let stat = rustix::fs::statx(fd, "", AtFlags::EMPTY_PATH | AtFlags::STATX_DONT_SYNC, mask)
        .map_err(|_| unavailable())?;
    if stat.stx_mask & mask.bits() != mask.bits() {
        return Err(unavailable());
    }
    Ok(stat)
}
fn directory(fd: &OwnedFd, owner: u32) -> Result<ComposeObjectIdentity> {
    let s = stat(fd)?;
    if s.stx_mode & 0o170000 != 0o040000
        || s.stx_uid != owner
        || s.stx_mode & 0o022 != 0
        || s.stx_nlink == 0
    {
        return Err(unavailable());
    }
    Ok((&s).into())
}
fn view_identity(fd: &OwnedFd) -> Result<ComposeMountView> {
    // Namespace references intentionally follow these fixed proc magic links.
    // Linux NSFS_MAGIC, include/uapi/linux/magic.h; no arbitrary path selector.
    if rustix::fs::fstatfs(fd).map_err(|_| unavailable())?.f_type != 0x6e736673 {
        return Err(unavailable());
    }
    let s = rustix::fs::fstat(fd).map_err(|_| unavailable())?;
    if s.st_ino == 0 {
        return Err(unavailable());
    }
    Ok(ComposeMountView {
        device: s.st_dev,
        inode: s.st_ino,
    })
}
fn view() -> Result<View> {
    let open = |name| {
        rustix::fs::open(name, OFlags::RDONLY | OFlags::CLOEXEC, Mode::empty())
            .map_err(|_| unavailable())
    };
    let current = open("/proc/self/ns/mnt")?;
    let host = open("/proc/1/ns/mnt")?;
    let identity = view_identity(&current)?;
    if identity != view_identity(&host)? {
        return Err(unavailable());
    }
    Ok(View {
        fd: current,
        identity,
    })
}
fn host_root() -> Result<Root> {
    if !rustix::process::geteuid().is_root() {
        return Err(Error(ErrorCode::Forbidden));
    }
    let original = view()?;
    let fd = rustix::fs::open(
        "/",
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| unavailable())?;
    directory(&fd, 0)?;
    if view()?.identity != original.identity {
        return Err(conflict());
    }
    Ok(Root {
        fd,
        view: Some(original),
    })
}
fn open_directory(parent: &OwnedFd, name: &str) -> Result<OwnedFd> {
    rustix::fs::openat(
        parent,
        name,
        OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| unavailable())
}
fn classified(parent: &OwnedFd, name: &str) -> Result<Option<OwnedFd>> {
    match rustix::fs::openat(
        parent,
        name,
        OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => Ok(Some(fd)),
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(_) => Err(unavailable()),
    }
}
fn check_names(
    parent: &OwnedFd,
    selection: &ComposeFileSelection,
    budget: &Budget,
    deadline: Instant,
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    for name in BASES.into_iter().chain([COMPOSE_MANAGED_FILE]) {
        budget.poll(deadline, cancelled)?;
        let expected = name == base_name(selection.operator)
            || (name == COMPOSE_MANAGED_FILE && selection.include_managed);
        if classified(parent, name)?.is_some() != expected {
            return Err(conflict());
        }
    }
    Ok(())
}
fn file_identity(fd: &OwnedFd, owner: u32) -> Result<ComposeFileIdentity> {
    let s = stat(fd)?;
    if s.stx_mode & 0o170000 != 0o100000
        || s.stx_uid != owner
        || s.stx_mode & 0o022 != 0
        || s.stx_nlink != 1
        || s.stx_size > FILE_BYTES
    {
        return Err(unavailable());
    }
    Ok((&s).into())
}
fn hash_file(
    fd: &OwnedFd,
    owner: u32,
    budget: &Budget,
    deadline: Instant,
    cancelled: &dyn Fn() -> bool,
) -> Result<(ComposeFileIdentity, String)> {
    budget.poll(deadline, cancelled)?;
    let before = file_identity(fd, owner)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 4096];
    let mut offset = 0u64;
    while offset < before.size {
        budget.poll(deadline, cancelled)?;
        let length = usize::try_from((before.size - offset).min(buffer.len() as u64))
            .map_err(|_| unavailable())?;
        budget.charge(length)?;
        let read =
            rustix::io::pread(fd, &mut buffer[..length], offset).map_err(|_| unavailable())?;
        if read == 0 {
            return Err(conflict());
        }
        digest.update(&buffer[..read]);
        offset += read as u64;
    }
    budget.poll(deadline, cancelled)?;
    budget.charge(1)?;
    if rustix::io::pread(fd, &mut buffer[..1], offset).map_err(|_| unavailable())? != 0
        || file_identity(fd, owner)? != before
    {
        return Err(conflict());
    }
    budget.poll(deadline, cancelled)?;
    Ok((before, hex::encode(digest.finalize())))
}
fn read_file(
    parent: &OwnedFd,
    name: &'static str,
    selection: &ComposeFileSelection,
    owner: u32,
    budget: &Budget,
    deadline: Instant,
    cancelled: &dyn Fn() -> bool,
) -> Result<(OwnedFd, ComposeFileObservation)> {
    budget.poll(deadline, cancelled)?;
    let path = classified(parent, name)?.ok_or_else(unavailable)?;
    let expected = file_identity(&path, owner)?;
    let fd = rustix::fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| unavailable())?;
    if file_identity(&fd, owner)? != expected {
        return Err(conflict());
    }
    let (identity, sha256) = hash_file(&fd, owner, budget, deadline, cancelled)?;
    if identity != expected || file_identity(&path, owner)? != expected {
        return Err(conflict());
    }
    Ok((
        fd,
        ComposeFileObservation {
            path: format!("{}/{name}", selection.working_directory),
            identity,
            sha256,
        },
    ))
}

struct Count {
    bytes: usize,
    max: usize,
}
impl Write for Count {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .filter(|bytes| *bytes <= self.max)
            .ok_or_else(|| io::Error::other("Compose file report limit"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn check_report(snapshot: &ComposeFileSnapshot, max: usize) -> Result<()> {
    serde_json::to_writer(&mut Count { bytes: 0, max }, snapshot).map_err(|_| unavailable())
}
/// Unregistered fixed-bound read-only adapter. Files do not establish managed stacks.
pub fn inspect_compose_files(
    selection: &ComposeFileSelection,
    cancelled: &dyn Fn() -> bool,
) -> Result<ComposeFileEvidence> {
    // The real privilege check precedes namespace, root and target access.
    if !rustix::process::geteuid().is_root() {
        return Err(Error(ErrorCode::Forbidden));
    }
    let budget = Budget {
        spent: AtomicU64::new(0),
        expires: Instant::now() + LIFETIME,
    };
    let deadline = Instant::now() + WORK;
    let at = now()?;
    budget.poll(deadline, cancelled)?;
    let evidence = collect(host_root()?, selection, 0, at, budget, deadline, cancelled)?;
    evidence.revalidate(cancelled)?;
    Ok(evidence)
}
fn collect(
    root: Root,
    selection: &ComposeFileSelection,
    owner: u32,
    at: i64,
    budget: Budget,
    deadline: Instant,
    cancelled: &dyn Fn() -> bool,
) -> Result<ComposeFileEvidence> {
    budget.poll(deadline, cancelled)?;
    let root_identity = directory(&root.fd, owner)?;
    let depth = selection.working_directory[1..].split('/').count();
    let mut ancestors = Vec::new();
    ancestors
        .try_reserve_exact(depth)
        .map_err(|_| unavailable())?;
    for name in selection.working_directory[1..].split('/') {
        budget.poll(deadline, cancelled)?;
        let fd = open_directory(
            ancestors
                .last()
                .map(|a: &Ancestor| &a.fd)
                .unwrap_or(&root.fd),
            name,
        )?;
        let identity = directory(&fd, owner)?;
        ancestors.push(Ancestor {
            name: name.to_owned(),
            fd,
            identity,
        });
    }
    let directory = ancestors.last().map(|a| &a.fd).unwrap_or(&root.fd);
    check_names(directory, selection, &budget, deadline, cancelled)?;
    let mut files = Vec::new();
    files
        .try_reserve_exact(1 + usize::from(selection.include_managed))
        .map_err(|_| unavailable())?;
    for name in std::iter::once(base_name(selection.operator))
        .chain(selection.include_managed.then_some(COMPOSE_MANAGED_FILE))
    {
        let (fd, observation) = read_file(
            directory, name, selection, owner, &budget, deadline, cancelled,
        )?;
        files.push(File {
            name,
            fd,
            observation,
        });
    }
    check_names(directory, selection, &budget, deadline, cancelled)?;
    let snapshot = ComposeFileSnapshot {
        version: 1,
        observed_at: at,
        selection: selection.clone(),
        mount_view: root.view.as_ref().map(|v| v.identity.clone()),
        root: root_identity,
        ancestors: ancestors.iter().map(|a| a.identity.clone()).collect(),
        files: files
            .iter()
            .map(|f| ComposeFileObservation {
                path: f.observation.path.clone(),
                identity: f.observation.identity.clone(),
                sha256: f.observation.sha256.clone(),
            })
            .collect(),
        absent_names: BASES
            .into_iter()
            .chain([COMPOSE_MANAGED_FILE])
            .filter(|name| {
                *name != base_name(selection.operator)
                    && (*name != COMPOSE_MANAGED_FILE || !selection.include_managed)
            })
            .map(str::to_owned)
            .collect(),
    };
    check_report(&snapshot, REPORT_BYTES)?;
    let evidence = ComposeFileEvidence {
        root,
        ancestors,
        files,
        snapshot,
        budget,
        state: AtomicU8::new(0),
        owner,
    };
    if evidence.retained_descriptor_count() > RETAINED_FDS {
        return Err(unavailable());
    }
    evidence.budget.poll(deadline, cancelled)?;
    Ok(evidence)
}
