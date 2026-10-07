//! Retained running-process observations, never Docker or operation authority.
//!
//! Supplied declarations/PIDs need authenticated Engine acquisition before and
//! after this library. Namespace mount IDs are local observations, not host
//! filesystem/UUID proof. Only collection constructs an evidence owner.

use crate::{DeviceNumber, Failure, Result, host_view, mounts};
use limeos_domain::{
    CONTAINER_STORAGE_MAX_AGE_SECONDS, CONTAINER_STORAGE_MAX_CONTAINERS, ContainerSnapshot,
    ContainerStorageInventory,
};
use rustix::{
    event::{PollFd, PollFlags, Timespec},
    fd::OwnedFd,
    fs::{AtFlags, FileType, Mode, OFlags, ResolveFlags, StatxFlags},
    process::{Pid, PidfdFlags},
};
use serde::Serialize;
use std::{
    cell::Cell,
    collections::BTreeSet,
    fmt,
    fs::File,
    io::{self, Read, Write},
    os::fd::AsRawFd,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_PID: u32 = 1 << 22; // Linux's 64-bit PID_MAX_LIMIT, exclusive.
const RECORD_BYTES: usize = 16 * 1024;
const TABLE_BYTES: usize = 1024 * 1024;
const AGGREGATE_TABLE_BYTES: usize = 8 * TABLE_BYTES;
const TABLE_ROWS: usize = 4096;
const REPORT_BYTES: usize = 48 * 1024;
const DECLARATION_BYTES: usize = 64 * 1024;
const WORK: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RunningContainerProcessBinding {
    pub container: ContainerSnapshot,
    pub pid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContainerProcessNamespace {
    pub device: DeviceNumber,
    pub inode: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContainerProcessRoot {
    pub device: DeviceNumber,
    pub inode: u64,
    /// Namespace-local mount observation; never equate this with a host ID.
    pub mount_id: u64,
    pub mode: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContainerProcessMount {
    pub mount_id: u64,
    pub parent_mount_id: u64,
    pub device: DeviceNumber,
    pub filesystem_root: String,
    pub mountpoint: String,
    pub filesystem: String,
    pub writable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RunningContainerProcess {
    pub container: ContainerSnapshot,
    pub pid: u32,
    pub start_ticks: u64,
    /// Real, effective, saved and filesystem IDs as observed through host proc.
    pub uids: [u32; 4],
    pub gids: [u32; 4],
    pub namespace: ContainerProcessNamespace,
    pub root: ContainerProcessRoot,
    pub mount_table_digest: String,
    pub mount_table_bytes: usize,
    pub mount_count: usize,
}

/// Serialize-only observation. Importing a report cannot construct an owner.
///
/// ```compile_fail
/// use limeos_executor_storage::processes::RunningContainerProcessSnapshot;
/// let imported: RunningContainerProcessSnapshot = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RunningContainerProcessSnapshot {
    pub version: u16,
    pub declarations_digest: String,
    pub bindings_digest: String,
    pub host_boot_id: String,
    pub host_root_mount_id: u64,
    pub host_mount_table_digest: String,
    pub declarations_observed_at: i64,
    pub processes: Vec<RunningContainerProcess>,
}

/// Retained kernel handles, absent from serialized snapshots. No signals,
/// namespace entry, mount mutation or data write occurs during observation.
///
/// ```compile_fail
/// use limeos_executor_storage::processes::RunningContainerProcessEvidence;
/// let forged = RunningContainerProcessEvidence { inner: () };
/// ```
pub struct RunningContainerProcessEvidence {
    inner: Evidence<Linux>,
}
impl fmt::Debug for RunningContainerProcessEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunningContainerProcessEvidence")
            .field("snapshot", &self.inner.snapshot)
            .finish_non_exhaustive()
    }
}
impl RunningContainerProcessEvidence {
    pub fn snapshot(&self) -> &RunningContainerProcessSnapshot {
        &self.inner.snapshot
    }
    pub fn mounts(&self, resource: &str) -> Option<&[ContainerProcessMount]> {
        self.inner.mounts(resource)
    }
    /// Checks this owner's original lifetime; never refreshes Engine facts.
    pub fn revalidate(&self) -> Result<()> {
        if !rustix::process::geteuid().is_root() {
            self.inner.invalid.set(true);
            return Err(Failure::Unavailable);
        }
        self.inner.revalidate()
    }
}

pub fn inspect_running_container_processes(
    declarations: &ContainerStorageInventory,
    bindings: &[RunningContainerProcessBinding],
) -> Result<RunningContainerProcessEvidence> {
    let kernel = Linux;
    let lease = Lease::new(declarations, kernel.clock()?)?;
    let sorted = validate_bindings(declarations, bindings)?;
    if !rustix::process::geteuid().is_root() {
        return Err(Failure::Unavailable);
    }
    Ok(RunningContainerProcessEvidence {
        inner: collect(kernel, declarations, &sorted, lease)?,
    })
}

fn domain_failure(e: limeos_domain::Error) -> Failure {
    if e.0 == limeos_domain::ErrorCode::Conflict {
        Failure::Conflict
    } else {
        Failure::InvalidPlan
    }
}
fn validate_pid(pid: u32) -> Result<()> {
    if (2..MAX_PID).contains(&pid) {
        Ok(())
    } else {
        Err(Failure::InvalidPlan)
    }
}
fn validate_bindings<'a>(
    declarations: &ContainerStorageInventory,
    bindings: &'a [RunningContainerProcessBinding],
) -> Result<Vec<&'a RunningContainerProcessBinding>> {
    if bindings.len() > CONTAINER_STORAGE_MAX_CONTAINERS
        || bindings.len()
            != declarations
                .containers
                .iter()
                .filter(|c| c.container.running)
                .count()
    {
        return Err(Failure::InvalidPlan);
    }
    let mut resources = BTreeSet::new();
    let mut pids = BTreeSet::new();
    let mut sorted = Vec::new();
    sorted
        .try_reserve_exact(bindings.len())
        .map_err(|_| Failure::Unavailable)?;
    for binding in bindings {
        validate_pid(binding.pid)?;
        binding.container.validate().map_err(domain_failure)?;
        if !binding.container.running
            || !resources.insert(&binding.container.resource)
            || !pids.insert(binding.pid)
        {
            return Err(Failure::InvalidPlan);
        }
        let declared = declarations
            .containers
            .iter()
            .find(|c| c.container.resource == binding.container.resource)
            .ok_or(Failure::InvalidPlan)?;
        if !declared.container.running {
            return Err(Failure::InvalidPlan);
        }
        if declared.container != binding.container {
            return Err(Failure::Conflict);
        }
        sorted.push(binding);
    }
    sorted.sort_by_key(|b| &b.container.resource);
    Ok(sorted)
}

#[derive(Clone, Copy)]
struct Time {
    wall: Duration,
    monotonic: Instant,
}
struct Lease {
    observed_at: i64,
    expires: Instant,
    highest_wall: Cell<Duration>,
}
impl Lease {
    fn new(declarations: &ContainerStorageInventory, now: Time) -> Result<Self> {
        declarations
            .validate(i64::try_from(now.wall.as_secs()).map_err(|_| Failure::Unavailable)?)
            .map_err(domain_failure)?;
        let end = Duration::from_secs(
            u64::try_from(declarations.observed_at).map_err(|_| Failure::InvalidPlan)?
                + CONTAINER_STORAGE_MAX_AGE_SECONDS as u64,
        );
        let remaining = end
            .checked_sub(now.wall)
            .filter(|d| !d.is_zero())
            .ok_or(Failure::Conflict)?;
        Ok(Self {
            observed_at: declarations.observed_at,
            expires: now
                .monotonic
                .checked_add(remaining)
                .ok_or(Failure::Unavailable)?,
            highest_wall: Cell::new(now.wall),
        })
    }
    fn check(&self, now: Time) -> Result<()> {
        let highest = self.highest_wall.get();
        self.highest_wall.set(highest.max(now.wall));
        let begin = Duration::from_secs(self.observed_at as u64);
        let end = begin + Duration::from_secs(CONTAINER_STORAGE_MAX_AGE_SECONDS as u64);
        if now.wall < highest
            || now.wall < begin
            || now.wall >= end
            || now.monotonic >= self.expires
        {
            Err(Failure::Conflict)
        } else {
            Ok(())
        }
    }
}

// Each complete pass admits at most 8 MiB, including duplicate namespaces.
// Repeated before/after observations do not enlarge retained parsed data.
struct Budget<'a, K: Kernel> {
    kernel: &'a K,
    lease: &'a Lease,
    deadline: Instant,
    table_bytes: [Cell<usize>; 3],
}
impl<'a, K: Kernel> Budget<'a, K> {
    fn new(kernel: &'a K, lease: &'a Lease) -> Result<Self> {
        let now = kernel.clock()?;
        lease.check(now)?;
        Ok(Self {
            kernel,
            lease,
            deadline: now.monotonic + WORK,
            table_bytes: std::array::from_fn(|_| Cell::new(0)),
        })
    }
    fn check(&self) -> Result<()> {
        let now = self.kernel.clock()?;
        self.lease.check(now)?;
        if now.monotonic >= self.deadline {
            Err(Failure::TimedOut)
        } else {
            Ok(())
        }
    }
    fn table(&self, bytes: usize, pass: usize) -> Result<()> {
        self.check()?;
        let total = self.table_bytes[pass]
            .get()
            .checked_add(bytes)
            .filter(|n| bytes <= TABLE_BYTES && *n <= AGGREGATE_TABLE_BYTES)
            .ok_or(Failure::Unavailable)?;
        self.table_bytes[pass].set(total);
        Ok(())
    }
    fn table_limit(&self, pass: usize) -> Result<usize> {
        self.check()?;
        Ok(TABLE_BYTES.min(AGGREGATE_TABLE_BYTES - self.table_bytes[pass].get()))
    }
}

struct JsonBuffer {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for JsonBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit - self.bytes.len() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn json_bytes(value: &impl Serialize, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(limit)
        .map_err(|_| Failure::Unavailable)?;
    let mut buffer = JsonBuffer { bytes, limit };
    serde_json::to_writer(&mut buffer, value).map_err(|_| Failure::Unavailable)?;
    Ok(buffer.bytes)
}
fn digest_json(value: &impl Serialize, limit: usize) -> Result<String> {
    let bytes = json_bytes(value, limit)?;
    Ok(limeos_identity::digest(
        std::str::from_utf8(&bytes).map_err(|_| Failure::Unavailable)?,
    ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HostFacts {
    boot: String,
    root_mount: u64,
    table_digest: String,
    init_table_digest: String,
    namespace: ContainerProcessNamespace,
    root: ContainerProcessRoot,
    proc_root: ContainerProcessRoot,
}
struct HostHandles<H> {
    root: H,
    proc_root: H,
    own_proc: H,
    fdinfo: H,
    init_proc: H,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct ProcessFacts {
    start: u64,
    uids: [u32; 4],
    gids: [u32; 4],
}
struct ProcessHandles<H> {
    pidfd: H,
    proc: H,
    namespace: H,
    root: H,
}
struct RetainedProcess<H> {
    observation: RunningContainerProcess,
    handles: ProcessHandles<H>,
    proc_identity: ContainerProcessRoot,
    mounts: Vec<ContainerProcessMount>,
}

// Entirely private interleaving boundary: no caller-selected proc root, handle
// import, clock, provider, or fixture is part of the production API.
trait Kernel: Sized {
    type Handle;
    fn clock(&self) -> Result<Time>;
    fn context(&self, budget: &Budget<'_, Self>) -> Result<HostHandles<Self::Handle>>;
    fn host(
        &self,
        context: &HostHandles<Self::Handle>,
        budget: &Budget<'_, Self>,
    ) -> Result<HostFacts>;
    fn pidfd(&self, pid: u32, budget: &Budget<'_, Self>) -> Result<Self::Handle>;
    fn live(
        &self,
        context: &HostHandles<Self::Handle>,
        fd: &Self::Handle,
        pid: u32,
        budget: &Budget<'_, Self>,
    ) -> Result<()>;
    fn proc(
        &self,
        context: &HostHandles<Self::Handle>,
        pid: u32,
        budget: &Budget<'_, Self>,
    ) -> Result<Self::Handle>;
    fn facts(
        &self,
        proc: &Self::Handle,
        pid: u32,
        budget: &Budget<'_, Self>,
    ) -> Result<ProcessFacts>;
    fn namespace(&self, proc: &Self::Handle, budget: &Budget<'_, Self>) -> Result<Self::Handle>;
    fn root(&self, proc: &Self::Handle, budget: &Budget<'_, Self>) -> Result<Self::Handle>;
    fn directory_identity(
        &self,
        fd: &Self::Handle,
        budget: &Budget<'_, Self>,
    ) -> Result<ContainerProcessRoot>;
    fn namespace_identity(
        &self,
        fd: &Self::Handle,
        budget: &Budget<'_, Self>,
    ) -> Result<ContainerProcessNamespace>;
    fn table(&self, proc: &Self::Handle, budget: &Budget<'_, Self>, limit: usize)
    -> Result<String>;
}

struct Evidence<K: Kernel> {
    kernel: K,
    context: HostHandles<K::Handle>,
    host: HostFacts,
    declarations: ContainerStorageInventory,
    lease: Lease,
    snapshot: RunningContainerProcessSnapshot,
    retained: Vec<RetainedProcess<K::Handle>>,
    invalid: Cell<bool>,
}
impl<K: Kernel> Evidence<K> {
    fn mounts(&self, resource: &str) -> Option<&[ContainerProcessMount]> {
        self.retained
            .iter()
            .find(|p| p.observation.container.resource == resource)
            .map(|p| p.mounts.as_slice())
    }
    fn revalidate(&self) -> Result<()> {
        if self.invalid.get() {
            return Err(Failure::Conflict);
        }
        let result = Budget::new(&self.kernel, &self.lease).and_then(|budget| self.check(&budget));
        if result.is_err() {
            self.invalid.set(true);
        }
        result
    }
    fn check(&self, budget: &Budget<'_, K>) -> Result<()> {
        budget.check()?;
        self.declarations
            .validate(
                i64::try_from(self.kernel.clock()?.wall.as_secs())
                    .map_err(|_| Failure::Unavailable)?,
            )
            .map_err(domain_failure)?;
        if self.kernel.host(&self.context, budget)? != self.host {
            return Err(Failure::Conflict);
        }
        for retained in &self.retained {
            check_process(&self.kernel, &self.context, retained, budget)?;
        }
        for retained in &self.retained {
            self.kernel.live(
                &self.context,
                &retained.handles.pidfd,
                retained.observation.pid,
                budget,
            )?;
        }
        if self.kernel.host(&self.context, budget)? != self.host {
            return Err(Failure::Conflict);
        }
        budget.check()
    }
}

fn collect<K: Kernel>(
    kernel: K,
    declarations: &ContainerStorageInventory,
    bindings: &[&RunningContainerProcessBinding],
    lease: Lease,
) -> Result<Evidence<K>> {
    let declaration_digest = digest_json(declarations, DECLARATION_BYTES)?;
    let binding_digest = digest_json(&bindings, DECLARATION_BYTES)?;
    let budget = Budget::new(&kernel, &lease)?;
    let context = kernel.context(&budget)?;
    let host = kernel.host(&context, &budget)?;
    let mut retained = Vec::new();
    retained
        .try_reserve_exact(bindings.len())
        .map_err(|_| Failure::Unavailable)?;
    for binding in bindings {
        budget.check()?;
        let pidfd = kernel.pidfd(binding.pid, &budget)?;
        kernel.live(&context, &pidfd, binding.pid, &budget)?;
        let proc = kernel.proc(&context, binding.pid, &budget)?;
        let proc_identity = kernel.directory_identity(&proc, &budget)?;
        let facts = kernel.facts(&proc, binding.pid, &budget)?;
        let namespace = kernel.namespace(&proc, &budget)?;
        let namespace_identity = kernel.namespace_identity(&namespace, &budget)?;
        let root = kernel.root(&proc, &budget)?;
        let root_identity = kernel.directory_identity(&root, &budget)?;
        let raw = kernel.table(&proc, &budget, budget.table_limit(0)?)?;
        budget.table(raw.len(), 0)?;
        let mounts = parse_mounts(&raw, &|| budget.check())?;
        // stat's device may be a Btrfs subvolume alias, distinct from the
        // mount table's superblock device. Preserve both facts for the later
        // filesystem/backing composer; neither authenticates physical storage.
        if !mounts.iter().any(|m| m.mount_id == root_identity.mount_id) {
            return Err(Failure::Unavailable);
        }
        kernel.live(&context, &pidfd, binding.pid, &budget)?;
        retained.push(RetainedProcess {
            observation: RunningContainerProcess {
                container: binding.container.clone(),
                pid: binding.pid,
                start_ticks: facts.start,
                uids: facts.uids,
                gids: facts.gids,
                namespace: namespace_identity,
                root: root_identity,
                mount_table_digest: limeos_identity::digest(&raw),
                mount_table_bytes: raw.len(),
                mount_count: mounts.len(),
            },
            handles: ProcessHandles {
                pidfd,
                proc,
                namespace,
                root,
            },
            proc_identity,
            mounts,
        });
    }
    let snapshot = RunningContainerProcessSnapshot {
        version: 1,
        declarations_digest: declaration_digest,
        bindings_digest: binding_digest,
        host_boot_id: host.boot.clone(),
        host_root_mount_id: host.root_mount,
        host_mount_table_digest: host.table_digest.clone(),
        declarations_observed_at: declarations.observed_at,
        processes: retained.iter().map(|p| p.observation.clone()).collect(),
    };
    json_bytes(&snapshot, REPORT_BYTES)?;
    let deadline = budget.deadline;
    let evidence = Evidence {
        kernel,
        context,
        host,
        declarations: declarations.clone(),
        lease,
        snapshot,
        retained,
        invalid: Cell::new(false),
    };
    let budget = Budget {
        kernel: &evidence.kernel,
        lease: &evidence.lease,
        deadline,
        table_bytes: std::array::from_fn(|_| Cell::new(0)),
    };
    evidence.check(&budget)?;
    Ok(evidence)
}

fn check_process<K: Kernel>(
    kernel: &K,
    context: &HostHandles<K::Handle>,
    retained: &RetainedProcess<K::Handle>,
    budget: &Budget<'_, K>,
) -> Result<()> {
    let old = &retained.observation;
    let handles = &retained.handles;
    kernel.live(context, &handles.pidfd, old.pid, budget)?;
    if kernel.directory_identity(&handles.proc, budget)? != retained.proc_identity
        || kernel.namespace_identity(&handles.namespace, budget)? != old.namespace
        || kernel.directory_identity(&handles.root, budget)? != old.root
    {
        return Err(Failure::Conflict);
    }
    let fresh_proc = kernel.proc(context, old.pid, budget)?;
    if kernel.directory_identity(&fresh_proc, budget)? != retained.proc_identity {
        return Err(Failure::Conflict);
    }
    let expected = ProcessFacts {
        start: old.start_ticks,
        uids: old.uids,
        gids: old.gids,
    };
    if kernel.facts(&handles.proc, old.pid, budget)? != expected {
        return Err(Failure::Conflict);
    }
    let first = kernel.table(&handles.proc, budget, budget.table_limit(1)?)?;
    budget.table(first.len(), 1)?;
    if limeos_identity::digest(&first) != old.mount_table_digest {
        return Err(Failure::Conflict);
    }
    let namespace = kernel.namespace(&fresh_proc, budget)?;
    let root = kernel.root(&fresh_proc, budget)?;
    if kernel.namespace_identity(&namespace, budget)? != old.namespace
        || kernel.directory_identity(&root, budget)? != old.root
    {
        return Err(Failure::Conflict);
    }
    let second = kernel.table(&fresh_proc, budget, budget.table_limit(2)?)?;
    budget.table(second.len(), 2)?;
    if limeos_identity::digest(&second) != old.mount_table_digest
        || kernel.facts(&fresh_proc, old.pid, budget)? != expected
    {
        return Err(Failure::Conflict);
    }
    kernel.live(context, &handles.pidfd, old.pid, budget)?;
    budget.check()
}

struct Linux;
impl Kernel for Linux {
    type Handle = OwnedFd;
    fn clock(&self) -> Result<Time> {
        Ok(Time {
            wall: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| Failure::Unavailable)?,
            monotonic: Instant::now(),
        })
    }
    fn context(&self, budget: &Budget<'_, Self>) -> Result<HostHandles<OwnedFd>> {
        budget.check()?;
        let root = open_fixed("/", OFlags::PATH | OFlags::DIRECTORY)?;
        protected(&root, false)?;
        let proc_root = open_fixed("/proc", OFlags::RDONLY | OFlags::DIRECTORY)?;
        protected(&proc_root, true)?;
        let mut name = [0u8; 32];
        let n = rustix::fs::readlinkat_raw(&proc_root, "self", &mut name)
            .map_err(|_| Failure::Unavailable)?;
        if parse_u64(&name[..n])? != u64::from(std::process::id()) {
            return Err(Failure::WrongNamespace);
        }
        let own_proc = open_proc_at(
            &proc_root,
            &std::process::id().to_string(),
            OFlags::RDONLY | OFlags::DIRECTORY,
        )?;
        let fdinfo = open_proc_at(&own_proc, "fdinfo", OFlags::RDONLY | OFlags::DIRECTORY)?;
        let init_proc = open_proc_at(&proc_root, "1", OFlags::RDONLY | OFlags::DIRECTORY)?;
        budget.check()?;
        Ok(HostHandles {
            root,
            proc_root,
            own_proc,
            fdinfo,
            init_proc,
        })
    }
    fn host(&self, context: &HostHandles<OwnedFd>, budget: &Budget<'_, Self>) -> Result<HostFacts> {
        budget.check()?;
        protected(&context.root, false)?;
        protected(&context.proc_root, true)?;
        let root = self.directory_identity(&context.root, budget)?;
        let proc_root = self.directory_identity(&context.proc_root, budget)?;
        // A retained descriptor alone would not observe a replaced / or /proc.
        if self.directory_identity(&open_fixed("/", OFlags::PATH | OFlags::DIRECTORY)?, budget)?
            != root
            || self.directory_identity(
                &open_fixed("/proc", OFlags::RDONLY | OFlags::DIRECTORY)?,
                budget,
            )? != proc_root
        {
            return Err(Failure::Conflict);
        }
        let current = read_proc(&context.own_proc, "mountinfo", TABLE_BYTES, &|| {
            budget.check()
        })?;
        let host = read_proc(&context.init_proc, "mountinfo", TABLE_BYTES, &|| {
            budget.check()
        })?;
        let current_root = host_root_row(&parse_mounts(&current, &|| budget.check())?)?;
        let host_root = host_root_row(&parse_mounts(&host, &|| budget.check())?)?;
        let namespace =
            self.namespace_identity(&self.namespace(&context.own_proc, budget)?, budget)?;
        let init_namespace =
            self.namespace_identity(&self.namespace(&context.init_proc, budget)?, budget)?;
        if namespace != init_namespace {
            return Err(Failure::WrongNamespace);
        }
        let root_mount = host_view(&[current_root], &[host_root])?;
        if root.mount_id != root_mount {
            return Err(Failure::WrongNamespace);
        }
        let boot = read_proc(
            &context.proc_root,
            "sys/kernel/random/boot_id",
            RECORD_BYTES,
            &|| budget.check(),
        )?;
        let boot = boot.trim();
        if boot.len() != 36
            || !boot.bytes().enumerate().all(|(i, b)| {
                if [8, 13, 18, 23].contains(&i) {
                    b == b'-'
                } else {
                    b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
                }
            })
        {
            return Err(Failure::Unavailable);
        }
        budget.check()?;
        Ok(HostFacts {
            boot: boot.into(),
            root_mount,
            table_digest: limeos_identity::digest(&current),
            init_table_digest: limeos_identity::digest(&host),
            namespace,
            root,
            proc_root,
        })
    }
    fn pidfd(&self, pid: u32, budget: &Budget<'_, Self>) -> Result<OwnedFd> {
        budget.check()?;
        validate_pid(pid)?;
        rustix::process::pidfd_open(
            Pid::from_raw(pid as i32).ok_or(Failure::InvalidPlan)?,
            PidfdFlags::NONBLOCK,
        )
        .map_err(|_| Failure::Unavailable)
    }
    fn live(
        &self,
        context: &HostHandles<OwnedFd>,
        fd: &OwnedFd,
        pid: u32,
        budget: &Budget<'_, Self>,
    ) -> Result<()> {
        let mut poll = [PollFd::new(fd, PollFlags::IN)];
        let zero = Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        loop {
            budget.check()?;
            match rustix::event::poll(&mut poll, Some(&zero)) {
                Err(e) if e == rustix::io::Errno::INTR => continue,
                Err(_) => return Err(Failure::Unavailable),
                Ok(0) if poll[0].revents().is_empty() => break,
                Ok(_) if poll[0].revents().intersects(PollFlags::IN | PollFlags::HUP) => {
                    return Err(Failure::Conflict);
                }
                Ok(_) => return Err(Failure::Unavailable),
            }
        }
        // fdinfo's Pid is in the PID namespace of the held proc filesystem.
        // It ties this live pidfd to the numeric directory opened below.
        let text = read_proc(
            &context.fdinfo,
            &fd.as_raw_fd().to_string(),
            RECORD_BYTES,
            &|| budget.check(),
        )?;
        if pidfd_pid(&text)? != pid {
            return Err(Failure::Conflict);
        }
        budget.check()
    }
    fn proc(
        &self,
        context: &HostHandles<OwnedFd>,
        pid: u32,
        budget: &Budget<'_, Self>,
    ) -> Result<OwnedFd> {
        budget.check()?;
        validate_pid(pid)?;
        open_proc_at(
            &context.proc_root,
            &pid.to_string(),
            OFlags::RDONLY | OFlags::DIRECTORY,
        )
    }
    fn facts(&self, proc: &OwnedFd, pid: u32, budget: &Budget<'_, Self>) -> Result<ProcessFacts> {
        let before = parse_stat(
            read_proc_bytes(proc, "stat", RECORD_BYTES, &|| budget.check())?.as_slice(),
            pid,
        )?;
        let status = parse_status(
            &read_proc(proc, "status", RECORD_BYTES, &|| budget.check())?,
            pid,
        )?;
        let after = parse_stat(
            read_proc_bytes(proc, "stat", RECORD_BYTES, &|| budget.check())?.as_slice(),
            pid,
        )?;
        if before != after {
            return Err(Failure::Conflict);
        }
        Ok(ProcessFacts {
            start: before,
            uids: status.0,
            gids: status.1,
        })
    }
    fn namespace(&self, proc: &OwnedFd, budget: &Budget<'_, Self>) -> Result<OwnedFd> {
        budget.check()?;
        // Deliberately follow only this fixed kernel magic link through a held
        // proc directory. User-selected links/paths never reach this boundary.
        rustix::fs::openat(
            proc,
            "ns/mnt",
            OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| Failure::Unavailable)
    }
    fn root(&self, proc: &OwnedFd, budget: &Budget<'_, Self>) -> Result<OwnedFd> {
        budget.check()?;
        rustix::fs::openat(
            proc,
            "root",
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| Failure::Unavailable)
    }
    fn directory_identity(
        &self,
        fd: &OwnedFd,
        budget: &Budget<'_, Self>,
    ) -> Result<ContainerProcessRoot> {
        budget.check()?;
        let mask = StatxFlags::TYPE
            | StatxFlags::MODE
            | StatxFlags::INO
            | StatxFlags::NLINK
            | StatxFlags::MNT_ID;
        let stat = rustix::fs::statx(fd, "", AtFlags::EMPTY_PATH | AtFlags::STATX_DONT_SYNC, mask)
            .map_err(|_| Failure::Unavailable)?;
        if stat.stx_mask & mask.bits() != mask.bits()
            || stat.stx_ino == 0
            || stat.stx_mnt_id == 0
            || stat.stx_nlink == 0
            || FileType::from_raw_mode(u32::from(stat.stx_mode)) != FileType::Directory
        {
            return Err(Failure::Unavailable);
        }
        budget.check()?;
        Ok(ContainerProcessRoot {
            device: DeviceNumber {
                major: stat.stx_dev_major,
                minor: stat.stx_dev_minor,
            },
            inode: stat.stx_ino,
            mount_id: stat.stx_mnt_id,
            mode: stat.stx_mode,
        })
    }
    fn namespace_identity(
        &self,
        fd: &OwnedFd,
        budget: &Budget<'_, Self>,
    ) -> Result<ContainerProcessNamespace> {
        budget.check()?;
        // nsfs is an internal kernel filesystem; a same-looking ordinary file
        // is not a namespace descriptor. Never substitute proc symlink text.
        const NSFS_MAGIC: i64 = 0x6e736673;
        let fs = rustix::fs::fstatfs(fd).map_err(|_| Failure::Unavailable)?;
        let stat = rustix::fs::fstat(fd).map_err(|_| Failure::Unavailable)?;
        if fs.f_type as i64 != NSFS_MAGIC
            || stat.st_ino == 0
            || stat.st_nlink == 0
            || FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
        {
            return Err(Failure::Unavailable);
        }
        budget.check()?;
        Ok(ContainerProcessNamespace {
            device: DeviceNumber {
                major: rustix::fs::major(stat.st_dev),
                minor: rustix::fs::minor(stat.st_dev),
            },
            inode: stat.st_ino,
        })
    }
    fn table(&self, proc: &OwnedFd, budget: &Budget<'_, Self>, limit: usize) -> Result<String> {
        read_proc(proc, "mountinfo", limit, &|| budget.check())
    }
}

fn open_fixed(path: &str, flags: OFlags) -> Result<OwnedFd> {
    rustix::fs::openat2(
        rustix::fs::CWD,
        path,
        flags | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|_| Failure::Unavailable)
}
fn protected(fd: &OwnedFd, proc: bool) -> Result<()> {
    let stat = rustix::fs::fstat(fd).map_err(|_| Failure::Unavailable)?;
    if stat.st_uid != 0
        || stat.st_mode & 0o022 != 0
        || FileType::from_raw_mode(stat.st_mode) != FileType::Directory
    {
        return Err(Failure::Unavailable);
    }
    if proc
        && rustix::fs::fstatfs(fd)
            .map_err(|_| Failure::Unavailable)?
            .f_type
            != rustix::fs::PROC_SUPER_MAGIC
    {
        return Err(Failure::Unavailable);
    }
    Ok(())
}
fn open_proc_at(parent: &OwnedFd, path: &str, flags: OFlags) -> Result<OwnedFd> {
    let fd = rustix::fs::openat2(
        parent,
        path,
        flags | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|_| Failure::Unavailable)?;
    if rustix::fs::fstatfs(&fd)
        .map_err(|_| Failure::Unavailable)?
        .f_type
        != rustix::fs::PROC_SUPER_MAGIC
    {
        return Err(Failure::Unavailable);
    }
    Ok(fd)
}
fn read_proc_bytes(
    parent: &OwnedFd,
    path: &str,
    limit: usize,
    check: &dyn Fn() -> Result<()>,
) -> Result<Vec<u8>> {
    check()?;
    let mut file = File::from(open_proc_at(parent, path, OFlags::RDONLY)?);
    read_bounded(&mut file, limit, check)
}
fn read_bounded(
    reader: &mut impl Read,
    limit: usize,
    check: &dyn Fn() -> Result<()>,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        check()?;
        let want = chunk
            .len()
            .min(limit.saturating_sub(output.len()).saturating_add(1));
        let n = match reader.read(&mut chunk[..want]) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            result => result.map_err(|_| Failure::Unavailable)?,
        };
        if n == 0 {
            check()?;
            return Ok(output);
        }
        if n > limit - output.len() {
            return Err(Failure::Unavailable);
        }
        output
            .try_reserve_exact(n)
            .map_err(|_| Failure::Unavailable)?;
        output.extend_from_slice(&chunk[..n]);
    }
}
fn read_proc(
    parent: &OwnedFd,
    path: &str,
    limit: usize,
    check: &dyn Fn() -> Result<()>,
) -> Result<String> {
    String::from_utf8(read_proc_bytes(parent, path, limit, check)?)
        .map_err(|_| Failure::Unavailable)
}
fn host_root_row(table: &[ContainerProcessMount]) -> Result<mounts::Mount> {
    let mut roots = table.iter().filter(|m| m.mountpoint == "/");
    let root = roots.next().ok_or(Failure::Unavailable)?;
    if roots.next().is_some() {
        return Err(Failure::Unavailable);
    }
    Ok(mounts::Mount {
        id: root.mount_id,
        device: root.device,
        root: root.filesystem_root.clone(),
        path: root.mountpoint.clone(),
        filesystem: root.filesystem.clone(),
        writable: root.writable,
    })
}
fn parse_u64(bytes: &[u8]) -> Result<u64> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return Err(Failure::Unavailable);
    }
    std::str::from_utf8(bytes)
        .map_err(|_| Failure::Unavailable)?
        .parse()
        .map_err(|_| Failure::Unavailable)
}
fn parse_stat(bytes: &[u8], pid: u32) -> Result<u64> {
    if bytes.len() > RECORD_BYTES || !bytes.ends_with(b"\n") {
        return Err(Failure::Unavailable);
    }
    let opening = bytes
        .iter()
        .position(|b| *b == b'(')
        .ok_or(Failure::Unavailable)?;
    let closing = bytes
        .iter()
        .rposition(|b| *b == b')')
        .ok_or(Failure::Unavailable)?;
    if opening == 0
        || closing < opening
        || bytes[opening - 1] != b' '
        || bytes.get(closing + 1) != Some(&b' ')
        || parse_u64(&bytes[..opening - 1])? != u64::from(pid)
    {
        return Err(Failure::Unavailable);
    }
    // comm is opaque bytes (including whitespace, parentheses and controls).
    // Only the final ')' is a delimiter; comm never enters output/errors.
    let mut fields = bytes[closing + 2..]
        .split(u8::is_ascii_whitespace)
        .filter(|s| !s.is_empty());
    let state = fields.next().ok_or(Failure::Unavailable)?;
    if matches!(state, b"Z" | b"X" | b"x") {
        return Err(Failure::Conflict);
    }
    if state.len() != 1 || !b"RSDTtWKP I".contains(&state[0]) {
        return Err(Failure::Unavailable);
    }
    let mut start = None;
    let mut count = 3;
    for field in fields {
        count += 1;
        if count > 128 || field.len() > 20 {
            return Err(Failure::Unavailable);
        }
        let number = field.strip_prefix(b"-").unwrap_or(field);
        parse_u64(number)?;
        if count == 22 {
            start = Some(parse_u64(field)?);
        }
    }
    if count < 52 {
        return Err(Failure::Unavailable);
    }
    start.filter(|n| *n != 0).ok_or(Failure::Unavailable)
}
fn parse_status(text: &str, pid: u32) -> Result<([u32; 4], [u32; 4])> {
    if text.len() > RECORD_BYTES || !text.ends_with('\n') {
        return Err(Failure::Unavailable);
    }
    let mut tgid = None;
    let mut reported_pid = None;
    let mut uids = None;
    let mut gids = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("Tgid:") {
            if tgid.replace(parse_u64(value.trim().as_bytes())?).is_some() {
                return Err(Failure::Unavailable);
            }
        } else if let Some(value) = line.strip_prefix("Pid:") {
            if reported_pid
                .replace(parse_u64(value.trim().as_bytes())?)
                .is_some()
            {
                return Err(Failure::Unavailable);
            }
        } else if let Some(value) = line.strip_prefix("Uid:") {
            if uids.replace(parse_ids(value)?).is_some() {
                return Err(Failure::Unavailable);
            }
        } else if let Some(value) = line.strip_prefix("Gid:") {
            if gids.replace(parse_ids(value)?).is_some() {
                return Err(Failure::Unavailable);
            }
        }
    }
    if tgid != Some(u64::from(pid)) || reported_pid != tgid {
        return Err(Failure::Conflict);
    }
    Ok((
        uids.ok_or(Failure::Unavailable)?,
        gids.ok_or(Failure::Unavailable)?,
    ))
}
fn parse_ids(value: &str) -> Result<[u32; 4]> {
    let mut values = value.split_ascii_whitespace();
    let mut ids = [0; 4];
    for id in &mut ids {
        *id = u32::try_from(parse_u64(
            values.next().ok_or(Failure::Unavailable)?.as_bytes(),
        )?)
        .map_err(|_| Failure::Unavailable)?;
    }
    if values.next().is_some() {
        return Err(Failure::Unavailable);
    }
    Ok(ids)
}
fn pidfd_pid(text: &str) -> Result<u32> {
    let mut values = text.lines().filter_map(|l| l.strip_prefix("Pid:"));
    let value = values.next().ok_or(Failure::Unavailable)?.trim();
    if value == "-1" {
        return Err(Failure::Conflict);
    }
    let pid = u32::try_from(parse_u64(value.as_bytes())?).map_err(|_| Failure::Unavailable)?;
    if values.next().is_some() || !(2..MAX_PID).contains(&pid) {
        return Err(Failure::Unavailable);
    }
    Ok(pid)
}

fn parse_mounts(text: &str, check: &dyn Fn() -> Result<()>) -> Result<Vec<ContainerProcessMount>> {
    if text.is_empty() || text.len() > TABLE_BYTES || !text.ends_with('\n') {
        return Err(Failure::Unavailable);
    }
    let count = text.lines().count();
    if count > TABLE_ROWS {
        return Err(Failure::Unavailable);
    }
    let mut parsed = Vec::new();
    parsed
        .try_reserve_exact(count)
        .map_err(|_| Failure::Unavailable)?;
    let mut ids = BTreeSet::new();
    for line in text.lines() {
        check()?;
        let (left, right) = line.split_once(" - ").ok_or(Failure::Unavailable)?;
        // Bounded fixed/optional token storage avoids whitespace amplification.
        let mut l = left.split(' ');
        let id = parse_u64(l.next().ok_or(Failure::Unavailable)?.as_bytes())?;
        let parent = parse_u64(l.next().ok_or(Failure::Unavailable)?.as_bytes())?;
        let device = DeviceNumber::parse(l.next().ok_or(Failure::Unavailable)?)?;
        let root = l.next().ok_or(Failure::Unavailable)?;
        let path = l.next().ok_or(Failure::Unavailable)?;
        let options = l.next().ok_or(Failure::Unavailable)?;
        if id == 0
            || parent == 0
            || !ids.insert(id)
            || !mount_path_token(root)
            || !mount_path_token(path)
            || !mount_token(options)
        {
            return Err(Failure::Unavailable);
        }
        let mut optional_count = 0;
        for optional in l {
            optional_count += 1;
            if optional_count > 32 || !mount_token(optional) {
                return Err(Failure::Unavailable);
            }
        }
        let mut r = right.split(' ');
        let filesystem = r.next().ok_or(Failure::Unavailable)?;
        let source = r.next().ok_or(Failure::Unavailable)?;
        let super_options = r.next().ok_or(Failure::Unavailable)?;
        if r.next().is_some()
            || !mount_token(filesystem)
            || !mount_token(source)
            || !mount_token(super_options)
            || !(options.split(',').any(|o| o == "rw") ^ options.split(',').any(|o| o == "ro"))
            || !(super_options.split(',').any(|o| o == "rw")
                ^ super_options.split(',').any(|o| o == "ro"))
        {
            return Err(Failure::Unavailable);
        }
        let root = mounts::unescape(root)?;
        let path = mounts::unescape(path)?;
        if !root.starts_with('/') || !path.starts_with('/') {
            return Err(Failure::Unavailable);
        }
        parsed.push(ContainerProcessMount {
            mount_id: id,
            parent_mount_id: parent,
            device,
            filesystem_root: root,
            mountpoint: path,
            filesystem: filesystem.into(),
            writable: options.split(',').any(|o| o == "rw")
                && super_options.split(',').any(|o| o == "rw"),
        });
    }
    parsed.sort_by_key(|m| m.mount_id);
    Ok(parsed)
}
fn mount_token(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_graphic())
}
fn mount_path_token(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b > 0x20 && b != 0x7f)
}

#[cfg(test)]
mod tests;
