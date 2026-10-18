//! Retained, bounded host task/namespace observations for the A1 launch gate.
//! Any unreadable/changing task, unsupported mapping, or excess input refuses.
use super::*;
use rustix::process::{Pid, PidfdFlags};
use std::{collections::BTreeMap, io::Read, os::fd::AsRawFd};

const TASKS: usize = 256;
const NAMESPACES: usize = 8;
const FDS_PER_PROCESS: usize = 256;
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct Task {
    pid: u32,
    tid: u32,
    start: u64,
    namespace: (u64, u64),
}
struct Namespace {
    fd: OwnedFd,
    digest: String,
}
pub(super) struct HostConsumers {
    proc: OwnedFd,
    tasks: Vec<Task>,
    namespaces: BTreeMap<(u64, u64), Namespace>,
    digest: String,
    expires: Instant,
    poisoned: Cell<bool>,
}
impl HostConsumers {
    pub(super) fn capture(
        selected: &StorageDevice,
        physical: &[StorageBlockDevice],
        underlay: &DirectoryIdentity,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self> {
        let expires = Instant::now() + WINDOW;
        let proc = rustix::fs::open(
            "/proc",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|error| io_failure("scan.proc.open", error, Failure::Unavailable))?;
        if rustix::fs::fstatfs(&proc)
            .map_err(|error| io_failure("scan.proc.filesystem", error, Failure::Unavailable))?
            .f_type
            != rustix::fs::PROC_SUPER_MAGIC
        {
            return Err(refusal(
                "scan.proc.not_procfs",
                Failure::Unavailable,
                0,
                0,
                0,
            ));
        }
        let (tasks, namespaces, digest) = diagnosed(
            "scan.initial_collection",
            collect(&proc, selected, physical, underlay, expires, cancelled),
        )?;
        let owner = Self {
            proc,
            tasks,
            namespaces,
            digest,
            expires,
            poisoned: Cell::new(false),
        };
        diagnosed(
            "scan.initial_recheck",
            owner.revalidate(selected, physical, underlay, cancelled),
        )?;
        Ok(owner)
    }
    pub(super) fn digest(&self) -> &str {
        &self.digest
    }
    pub(super) fn revalidate(
        &self,
        selected: &StorageDevice,
        physical: &[StorageBlockDevice],
        underlay: &DirectoryIdentity,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<()> {
        if self.poisoned.get() {
            return Err(Failure::Conflict);
        }
        let result = (|| {
            let (tasks, namespaces, digest) = diagnosed(
                "scan.recheck.collection",
                collect(
                    &self.proc,
                    selected,
                    physical,
                    underlay,
                    self.expires,
                    cancelled,
                ),
            )?;
            if tasks != self.tasks
                || digest != self.digest
                || namespaces.len() != self.namespaces.len()
            {
                return Err(Failure::Conflict);
            }
            for (key, held) in &self.namespaces {
                let fresh = namespaces.get(key).ok_or(Failure::Conflict)?;
                let stat = rustix::fs::fstat(&held.fd).map_err(|error| {
                    io_failure("scan.recheck.namespace.stat", error, Failure::Unavailable)
                })?;
                if (stat.st_dev, stat.st_ino) != *key || fresh.digest != held.digest {
                    return Err(Failure::Conflict);
                }
            }
            Ok(())
        })();
        if result.is_err() {
            self.poisoned.set(true);
        }
        result
    }
}
fn poll(expires: Instant, cancelled: &dyn Fn() -> bool) -> Result<()> {
    if cancelled() || Instant::now() >= expires {
        Err(Failure::TimedOut)
    } else {
        Ok(())
    }
}
fn numeric(fd: &OwnedFd, maximum: usize) -> Result<Vec<u32>> {
    let mut ids = Vec::new();
    ids.try_reserve_exact(maximum)
        .map_err(|_| refusal("proc.list.reserve", Failure::Unavailable, 0, 0, 0))?;
    for entry in rustix::fs::Dir::read_from(fd)
        .map_err(|error| io_failure("proc.list.open", error, Failure::Unavailable))?
    {
        let entry =
            entry.map_err(|error| io_failure("proc.list.next", error, Failure::Unavailable))?;
        let name = entry.file_name().to_bytes();
        if name.is_empty() || !name.iter().all(u8::is_ascii_digit) {
            continue;
        }
        if ids.len() == maximum {
            return Err(refusal(
                "proc.list.limit",
                Failure::Unavailable,
                0,
                ids.len() + 1,
                maximum,
            ));
        }
        let id = std::str::from_utf8(name)
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .ok_or_else(|| refusal("proc.list.numeric", Failure::Unavailable, 0, 0, 0))?;
        ids.push(id);
    }
    ids.sort_unstable();
    if ids.windows(2).any(|p| p[0] == p[1]) {
        return Err(refusal(
            "proc.list.duplicate",
            Failure::Unavailable,
            0,
            0,
            0,
        ));
    }
    Ok(ids)
}
fn openat(fd: &OwnedFd, name: &str, flags: OFlags) -> Result<OwnedFd> {
    rustix::fs::openat(
        fd,
        name,
        flags | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(|error| io_failure("proc.openat", error, Failure::Unavailable))
}
fn read(fd: &OwnedFd, name: &str, limit: usize) -> Result<String> {
    let file = diagnosed("proc.read.open", openat(fd, name, OFlags::RDONLY))?;
    if rustix::fs::fstatfs(&file)
        .map_err(|error| io_failure("proc.read.filesystem", error, Failure::Unavailable))?
        .f_type
        != rustix::fs::PROC_SUPER_MAGIC
    {
        return Err(refusal(
            "proc.read.not_procfs",
            Failure::Unavailable,
            0,
            0,
            0,
        ));
    }
    let mut text = String::new();
    text.try_reserve_exact(limit + 1)
        .map_err(|_| refusal("proc.read.reserve", Failure::Unavailable, 0, 0, 0))?;
    File::from(file)
        .take(limit as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|error| {
            refusal(
                "proc.read.bytes",
                Failure::Unavailable,
                error.raw_os_error().unwrap_or(0),
                text.len(),
                limit,
            )
        })?;
    if text.len() > limit {
        return Err(refusal(
            "proc.read.limit",
            Failure::Unavailable,
            0,
            text.len(),
            limit,
        ));
    }
    Ok(text)
}
fn start(fd: &OwnedFd, tid: u32) -> Result<u64> {
    let text = diagnosed("task.stat.read", read(fd, "stat", 4096))?;
    let (pid, _) = text
        .split_once(' ')
        .ok_or_else(|| refusal("task.stat.pid_field", Failure::Unavailable, 0, 0, 0))?;
    if pid.parse::<u32>().ok() != Some(tid) {
        return Err(Failure::IdentityMismatch);
    }
    let end = text
        .rfind(')')
        .ok_or_else(|| refusal("task.stat.framing", Failure::Unavailable, 0, 0, 0))?;
    text[end + 1..]
        .split_ascii_whitespace()
        .nth(19)
        .and_then(|s| s.parse().ok())
        .filter(|n| *n > 0)
        .ok_or_else(|| refusal("task.stat.start_field", Failure::Unavailable, 0, 0, 0))
}
type Collection = (Vec<Task>, BTreeMap<(u64, u64), Namespace>, String);
fn collect(
    proc: &OwnedFd,
    selected: &StorageDevice,
    physical: &[StorageBlockDevice],
    underlay: &DirectoryIdentity,
    expires: Instant,
    cancelled: &dyn Fn() -> bool,
) -> Result<Collection> {
    diagnosed("scan.window.before", poll(expires, cancelled))?;
    let pids = diagnosed("scan.process_ids.before", numeric(proc, TASKS))?;
    let mut tasks = Vec::new();
    tasks
        .try_reserve_exact(TASKS)
        .map_err(|_| refusal("scan.tasks.reserve", Failure::Unavailable, 0, 0, 0))?;
    let mut namespaces = BTreeMap::new();
    for pid in &pids {
        diagnosed("scan.window.process", poll(expires, cancelled))?;
        let process = diagnosed(
            "scan.process.open",
            openat(proc, &pid.to_string(), OFlags::RDONLY | OFlags::DIRECTORY),
        )?;
        let pidfd = rustix::process::pidfd_open(
            Pid::from_raw(*pid as i32)
                .ok_or_else(|| refusal("scan.pid_input", Failure::Unavailable, 0, 0, 0))?,
            PidfdFlags::empty(),
        )
        .map_err(|error| io_failure("scan.pidfd_open", error, Failure::Unavailable))?;
        let leader_start = diagnosed("scan.leader_start.before", start(&process, *pid))?;
        if diagnosed("scan.kernel_classification", kernel_thread(&process))? {
            // Kernel tasks have no userspace mm, descriptor table, or process
            // mount namespace. Topology/swap/boot and helper-pending barriers
            // remain independent checks; no general kernel-consumer claim.
            let taskdir = diagnosed(
                "scan.kernel.task_directory",
                openat(&process, "task", OFlags::RDONLY | OFlags::DIRECTORY),
            )?;
            let tids = diagnosed("scan.kernel.tids.before", numeric(&taskdir, TASKS))?;
            for tid in &tids {
                if tasks.len() == TASKS {
                    return Err(refusal(
                        "scan.kernel.task_limit",
                        Failure::Unavailable,
                        0,
                        tasks.len() + 1,
                        TASKS,
                    ));
                }
                let task = diagnosed(
                    "scan.kernel.task.open",
                    openat(
                        &taskdir,
                        &tid.to_string(),
                        OFlags::RDONLY | OFlags::DIRECTORY,
                    ),
                )?;
                tasks.push(Task {
                    pid: *pid,
                    tid: *tid,
                    start: diagnosed("scan.kernel.task.start", start(&task, *tid))?,
                    namespace: (0, 0),
                });
            }
            if diagnosed("scan.kernel.tids.after", numeric(&taskdir, TASKS))? != tids
                || diagnosed("scan.kernel.leader_start.after", start(&process, *pid))?
                    != leader_start
                || !diagnosed("scan.kernel.classification.after", kernel_thread(&process))?
            {
                return Err(Failure::Conflict);
            }
            diagnosed("scan.kernel.pidfd", live_pidfd(&pidfd, *pid))?;
            continue;
        }
        if diagnosed(
            "scan.guardian_classification",
            fixed_guardian(&process, *pid),
        )? {
            diagnosed("scan.guardian.pidfd", live_pidfd(&pidfd, *pid))?;
            continue;
        }
        let taskdir = diagnosed(
            "scan.task_directory",
            openat(&process, "task", OFlags::RDONLY | OFlags::DIRECTORY),
        )?;
        let tids = diagnosed("scan.tids.before", numeric(&taskdir, TASKS))?;
        if tids.is_empty() || tasks.len() + tids.len() > TASKS {
            return Err(refusal(
                "scan.task_membership",
                Failure::Unavailable,
                0,
                tasks.len() + tids.len(),
                TASKS,
            ));
        }
        for tid in &tids {
            diagnosed("scan.window.task", poll(expires, cancelled))?;
            let task = diagnosed(
                "scan.task.open",
                openat(
                    &taskdir,
                    &tid.to_string(),
                    OFlags::RDONLY | OFlags::DIRECTORY,
                ),
            )?;
            let ticks = diagnosed("scan.task.start.before", start(&task, *tid))?;
            let nsdir = diagnosed(
                "scan.namespace.directory",
                openat(&task, "ns", OFlags::RDONLY | OFlags::DIRECTORY),
            )?;
            // Proc namespace magic links are intentionally followed only from
            // the already-open genuine proc task directory, never caller paths.
            let namespace = rustix::fs::openat(
                &nsdir,
                "mnt",
                OFlags::RDONLY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| io_failure("scan.namespace.open", error, Failure::Unavailable))?;
            let stat = rustix::fs::fstat(&namespace)
                .map_err(|error| io_failure("scan.namespace.stat", error, Failure::Unavailable))?;
            let key = (stat.st_dev, stat.st_ino);
            if !namespaces.contains_key(&key) {
                if namespaces.len() == NAMESPACES {
                    return Err(refusal(
                        "scan.namespace.limit",
                        Failure::Unavailable,
                        0,
                        namespaces.len() + 1,
                        NAMESPACES,
                    ));
                }
                let table =
                    diagnosed("scan.mount_table.read", read(&task, "mountinfo", 64 * 1024))?;
                let rows = diagnosed(
                    "scan.mount_table.parse",
                    crate::mounts::parse_with_profile(
                        &table,
                        crate::StorageReadProfile::CombinedV1,
                    ),
                )?;
                diagnosed(
                    "scan.mount_table.consumers",
                    check_mounts(&rows, selected, physical, underlay),
                )?;
                let digest = limeos_identity::digest(&table);
                namespaces.insert(
                    key,
                    Namespace {
                        fd: namespace,
                        digest,
                    },
                );
            }
            if *pid != std::process::id() {
                diagnosed(
                    "scan.cwd",
                    check_link(&task, "cwd", selected, physical, underlay),
                )?;
                diagnosed(
                    "scan.root",
                    check_link(&task, "root", selected, physical, underlay),
                )?;
                // Individual tasks may unshare CLONE_FILES. Inspect their
                // actual tables, not only the process leader's table.
                diagnosed(
                    "scan.fds",
                    check_fds(&task, selected, physical, underlay, expires, cancelled),
                )?;
                diagnosed(
                    "scan.maps",
                    check_maps(
                        &diagnosed("scan.maps.read", read(&task, "maps", 256 * 1024))?,
                        selected,
                        physical,
                    ),
                )?;
            }
            let after = diagnosed("scan.task.start.after", start(&task, *tid))?;
            let fresh = rustix::fs::openat(
                &nsdir,
                "mnt",
                OFlags::RDONLY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| io_failure("scan.namespace.reopen", error, Failure::Unavailable))?;
            let fresh = rustix::fs::fstat(&fresh).map_err(|error| {
                io_failure("scan.namespace.restat", error, Failure::Unavailable)
            })?;
            if after != ticks || (fresh.st_dev, fresh.st_ino) != key {
                return Err(Failure::Conflict);
            }
            tasks.push(Task {
                pid: *pid,
                tid: *tid,
                start: ticks,
                namespace: key,
            });
        }
        if diagnosed("scan.leader_start.after", start(&process, *pid))? != leader_start
            || diagnosed("scan.tids.after", numeric(&taskdir, TASKS))? != tids
        {
            return Err(Failure::Conflict);
        }
        // Holding a pidfd prevents interpreting a recycled PID as this owner.
        diagnosed("scan.pidfd", live_pidfd(&pidfd, *pid))?;
    }
    if diagnosed("scan.process_ids.after", numeric(proc, TASKS))? != pids {
        return Err(Failure::Conflict);
    }
    diagnosed("scan.window.after", poll(expires, cancelled))?;
    let ns: Vec<_> = namespaces
        .iter()
        .map(|(key, n)| (*key, &n.digest))
        .collect();
    let digest = diagnosed("scan.digest", inventory::digest(&(&tasks, ns)))?;
    Ok((tasks, namespaces, digest))
}
fn relevant(device: (u32, u32), physical: &[StorageBlockDevice]) -> bool {
    physical.iter().any(|d| (d.major, d.minor) == device)
}
fn beneath(path: &str, root: &str) -> bool {
    path == root || path.strip_prefix(root).is_some_and(|s| s.starts_with('/'))
}
fn check_mounts(
    rows: &[crate::mounts::Mount],
    selected: &StorageDevice,
    physical: &[StorageBlockDevice],
    underlay: &DirectoryIdentity,
) -> Result<()> {
    if rows.iter().any(|m| {
        relevant((m.device.major, m.device.minor), physical)
            || ((m.device.major, m.device.minor) == (underlay.major, underlay.minor)
                && m.root != "/"
                && (beneath(&m.root, &selected.mountpoint)
                    || beneath(&selected.mountpoint, &m.root)))
    }) {
        return Err(Failure::Conflict);
    }
    Ok(())
}
fn check_link(
    task: &OwnedFd,
    name: &str,
    selected: &StorageDevice,
    physical: &[StorageBlockDevice],
    underlay: &DirectoryIdentity,
) -> Result<()> {
    let fd = rustix::fs::openat(task, name, OFlags::PATH | OFlags::CLOEXEC, Mode::empty())
        .map_err(|error| io_failure("link.open", error, Failure::Unavailable))?;
    let stat = rustix::fs::fstat(&fd)
        .map_err(|error| io_failure("link.stat", error, Failure::Unavailable))?;
    let device = (
        rustix::fs::major(stat.st_dev),
        rustix::fs::minor(stat.st_dev),
    );
    if relevant(device, physical)
        || (stat.st_mode & 0o170000 == 0o060000
            && relevant(
                (
                    rustix::fs::major(stat.st_rdev),
                    rustix::fs::minor(stat.st_rdev),
                ),
                physical,
            ))
        || (device.0, device.1, stat.st_ino) == (underlay.major, underlay.minor, underlay.inode)
    {
        return Err(Failure::Conflict);
    }
    let link = rustix::fs::readlinkat(task, name, Vec::new())
        .map_err(|error| io_failure("link.readlink", error, Failure::Unavailable))?;
    let link = link
        .to_str()
        .map_err(|_| refusal("link.utf8", Failure::Unavailable, 0, 0, 0))?;
    if link.len() > 4096 || beneath(link, &selected.mountpoint) {
        return Err(Failure::Conflict);
    }
    Ok(())
}
fn check_fds(
    process: &OwnedFd,
    selected: &StorageDevice,
    physical: &[StorageBlockDevice],
    underlay: &DirectoryIdentity,
    expires: Instant,
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    let fds = diagnosed(
        "fds.directory",
        openat(process, "fd", OFlags::RDONLY | OFlags::DIRECTORY),
    )?;
    let ids = diagnosed("fds.ids.before", numeric(&fds, FDS_PER_PROCESS))?;
    for fd in &ids {
        diagnosed("fds.window", poll(expires, cancelled))?;
        diagnosed(
            "fds.link",
            check_link(&fds, &fd.to_string(), selected, physical, underlay),
        )?;
    }
    if diagnosed("fds.ids.after", numeric(&fds, FDS_PER_PROCESS))? != ids {
        return Err(Failure::Conflict);
    }
    Ok(())
}
fn check_maps(text: &str, selected: &StorageDevice, physical: &[StorageBlockDevice]) -> Result<()> {
    if !text.is_empty() && !text.ends_with('\n') {
        return Err(refusal("maps.truncated", Failure::Unavailable, 0, 0, 0));
    }
    let lines = text.lines().count();
    if lines > 4096 {
        return Err(refusal(
            "maps.line_limit",
            Failure::Unavailable,
            0,
            lines,
            4096,
        ));
    }
    for line in text.lines() {
        let mut fields = line.split_ascii_whitespace();
        let _address = fields
            .next()
            .ok_or_else(|| refusal("maps.address_field", Failure::Unavailable, 0, 0, 0))?;
        let _permissions = fields
            .next()
            .ok_or_else(|| refusal("maps.permissions_field", Failure::Unavailable, 0, 0, 0))?;
        let _offset = fields
            .next()
            .ok_or_else(|| refusal("maps.offset_field", Failure::Unavailable, 0, 0, 0))?;
        let device = fields
            .next()
            .and_then(|s| s.split_once(':'))
            .ok_or_else(|| refusal("maps.device_field", Failure::Unavailable, 0, 0, 0))?;
        let device = (
            u32::from_str_radix(device.0, 16)
                .map_err(|_| refusal("maps.device_major", Failure::Unavailable, 0, 0, 0))?,
            u32::from_str_radix(device.1, 16)
                .map_err(|_| refusal("maps.device_minor", Failure::Unavailable, 0, 0, 0))?,
        );
        let mut remaining = line;
        for _ in 0..5 {
            remaining = remaining.trim_start_matches(|c: char| c.is_ascii_whitespace());
            remaining = match remaining.find(|c: char| c.is_ascii_whitespace()) {
                Some(index) => &remaining[index..],
                None => "",
            };
        }
        let filename = remaining.trim_start_matches(|c: char| c.is_ascii_whitespace());
        if relevant(device, physical) || beneath(filename, &selected.mountpoint) {
            return Err(Failure::Conflict);
        }
    }
    Ok(())
}
fn live_pidfd(fd: &OwnedFd, pid: u32) -> Result<()> {
    let info = rustix::fs::open(
        "/proc/self/fdinfo",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(|error| io_failure("pidfd.fdinfo.open", error, Failure::Unavailable))?;
    let text = diagnosed(
        "pidfd.fdinfo.read",
        read(&info, &fd.as_raw_fd().to_string(), 4096),
    )?;
    let mut ids = text.lines().filter_map(|s| s.strip_prefix("Pid:"));
    if ids.next().and_then(|s| s.trim().parse::<u32>().ok()) != Some(pid) || ids.next().is_some() {
        return Err(Failure::Conflict);
    }
    use rustix::event::{PollFd, PollFlags, Timespec};
    let mut fds = [PollFd::new(fd, PollFlags::IN)];
    let zero = Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    match rustix::event::poll(&mut fds, Some(&zero)) {
        Ok(0) if fds[0].revents().is_empty() => Ok(()),
        Err(error) => Err(io_failure("pidfd.poll", error, Failure::Conflict)),
        _ => Err(refusal("pidfd.readiness", Failure::Conflict, 0, 0, 0)),
    }
}
fn kernel_thread(process: &OwnedFd) -> Result<bool> {
    let stat = diagnosed("kernel.stat.read", read(process, "stat", 4096))?;
    let end = stat
        .rfind(')')
        .ok_or_else(|| refusal("kernel.stat.framing", Failure::Unavailable, 0, 0, 0))?;
    let flags = stat[end + 1..]
        .split_ascii_whitespace()
        .nth(6)
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| refusal("kernel.flags", Failure::Unavailable, 0, 0, 0))?;
    let status = diagnosed("kernel.status.read", read(process, "status", 16384))?;
    let kernel = diagnosed("kernel.marker", kernel_marker(flags, &status))?;
    if kernel {
        let fds = diagnosed(
            "kernel.fds.directory",
            openat(process, "fd", OFlags::RDONLY | OFlags::DIRECTORY),
        )?;
        if !diagnosed("kernel.fds", numeric(&fds, FDS_PER_PROCESS))?.is_empty()
            || !diagnosed("kernel.maps", read(process, "maps", 1))?.is_empty()
        {
            return Err(Failure::Conflict);
        }
    }
    Ok(kernel)
}
fn kernel_marker(flags: u64, status: &str) -> Result<bool> {
    let mut field = status.lines().filter_map(|s| s.strip_prefix("Kthread:"));
    let kernel = match field.next().map(str::trim) {
        Some("1") => true,
        Some("0") => false,
        None => {
            return Err(refusal(
                "kernel.marker.missing",
                Failure::Unavailable,
                0,
                0,
                0,
            ));
        }
        _ => {
            return Err(refusal(
                "kernel.marker.invalid",
                Failure::Unavailable,
                0,
                0,
                0,
            ));
        }
    };
    if field.next().is_some() || kernel != (flags & 0x0020_0000 != 0) {
        return Err(Failure::Conflict);
    }
    Ok(kernel)
}
fn fixed_guardian(process: &OwnedFd, pid: u32) -> Result<bool> {
    let stat = diagnosed("guardian.stat.read", read(process, "stat", 4096))?;
    let end = stat
        .rfind(')')
        .ok_or_else(|| refusal("guardian.stat.framing", Failure::Unavailable, 0, 0, 0))?;
    let parent = stat[end + 1..]
        .split_ascii_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u32>().ok())
        .ok_or_else(|| refusal("guardian.parent_field", Failure::Unavailable, 0, 0, 0))?;
    if parent != std::process::id() {
        return Ok(false);
    }
    let command = diagnosed("guardian.command.read", read(process, "cmdline", 1024))?;
    if command != "/usr/lib/limeos/limeos-executor\0storage-mount-guardian\0" {
        return Ok(false);
    }
    // This exception is closed over the actual direct child, fixed code image,
    // root host namespace and cgroup. No caller PID/boolean allowlist exists.
    let image = rustix::fs::openat(
        process,
        "exe",
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| io_failure("guardian.image.open", error, Failure::Unavailable))?;
    let current = rustix::fs::open(
        "/proc/self/exe",
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| io_failure("guardian.current_image.open", error, Failure::Unavailable))?;
    if diagnosed("guardian.image.digest", code_digest(&image))?
        != diagnosed("guardian.current_image.digest", code_digest(&current))?
    {
        return Err(Failure::IdentityMismatch);
    }
    let nsdir = diagnosed(
        "guardian.namespace.directory",
        openat(process, "ns", OFlags::RDONLY | OFlags::DIRECTORY),
    )?;
    let ns = rustix::fs::openat(
        &nsdir,
        "mnt",
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| io_failure("guardian.namespace.open", error, Failure::Unavailable))?;
    let host = rustix::fs::open(
        "/proc/1/ns/mnt",
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| io_failure("guardian.host_namespace.open", error, Failure::Unavailable))?;
    let ns = rustix::fs::fstat(&ns)
        .map_err(|error| io_failure("guardian.namespace.stat", error, Failure::Unavailable))?;
    let host = rustix::fs::fstat(&host)
        .map_err(|error| io_failure("guardian.host_namespace.stat", error, Failure::Unavailable))?;
    if (ns.st_dev, ns.st_ino) != (host.st_dev, host.st_ino)
        || diagnosed("guardian.cgroup.read", read(process, "cgroup", 1024))?
            != diagnosed(
                "guardian.current_cgroup.read",
                crate::bounded_read(Path::new("/proc/self/cgroup"), 1024),
            )?
    {
        return Err(Failure::WrongNamespace);
    }
    let status = diagnosed("guardian.status.read", read(process, "status", 16384))?;
    let mut uid = status.lines().filter_map(|s| s.strip_prefix("Uid:"));
    let ids: Vec<_> = uid
        .next()
        .ok_or_else(|| refusal("guardian.uid_field", Failure::Unavailable, 0, 0, 0))?
        .split_ascii_whitespace()
        .collect();
    if ids != ["0", "0", "0", "0"] || uid.next().is_some() {
        return Err(refusal("guardian.uid", Failure::Unavailable, 0, 0, 0));
    }
    // The outer collector owns this PID's pidfd and verifies it remains live.
    if pid <= 1 {
        return Err(refusal("guardian.pid_range", Failure::Unavailable, 0, 0, 0));
    }
    Ok(true)
}
fn code_digest(fd: &OwnedFd) -> Result<String> {
    let s = path::stat(fd)?;
    if s.stx_mode & 0o170000 != 0o100000
        || s.stx_uid != 0
        || s.stx_mode & 0o022 != 0
        || s.stx_size == 0
        || s.stx_size > 8 * 1024 * 1024
    {
        return Err(Failure::UnsafePath);
    }
    let file = File::from(
        rustix::io::dup(fd)
            .map_err(|error| io_failure("guardian.image.dup", error, Failure::Unavailable))?,
    );
    let mut digest = Sha256::new();
    let mut buffer = [0; 8192];
    let mut offset = 0;
    while offset < s.stx_size {
        let n = (s.stx_size - offset).min(buffer.len() as u64) as usize;
        file.read_exact_at(&mut buffer[..n], offset)
            .map_err(|error| {
                refusal(
                    "guardian.image.read",
                    Failure::Unavailable,
                    error.raw_os_error().unwrap_or(0),
                    0,
                    0,
                )
            })?;
        digest.update(&buffer[..n]);
        offset += n as u64;
    }
    let after = path::stat(fd)?;
    if Object::from(s) != Object::from(after) {
        return Err(Failure::IdentityMismatch);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn selected() -> StorageDevice {
        StorageDevice {
            id: "data".into(),
            role: limeos_domain::StorageRole::Data,
            filesystem_uuid: "uuid".into(),
            filesystem: StorageFilesystem::Ext4,
            mountpoint: "/mnt/my data".into(),
            serial: None,
        }
    }
    #[test]
    fn mapped_file_on_selected_device_or_underlay_refuses_without_engine_declaration() {
        let physical = [StorageBlockDevice { major: 8, minor: 1 }];
        assert_eq!(
            check_maps(
                "1000-2000 r--p 0000 08:01 1 /unrelated\n",
                &selected(),
                &physical
            ),
            Err(Failure::Conflict)
        );
        assert_eq!(
            check_maps(
                "1000-2000 r--p 0000 00:21 1    /mnt/my data/file\n",
                &selected(),
                &physical
            ),
            Err(Failure::Conflict)
        );
        assert_eq!(
            check_maps(
                "1000-2000 r--p 0000 00:21 1 /unrelated\n",
                &selected(),
                &physical
            ),
            Ok(())
        );
    }
    #[test]
    fn foreign_namespace_selected_disk_and_underlay_bind_alias_refuse() {
        let physical = [StorageBlockDevice { major: 8, minor: 1 }];
        let underlay = DirectoryIdentity {
            major: 0,
            minor: 21,
            mount_id: 1,
            inode: 42,
            mode: 0o40755,
            uid: 0,
            gid: 0,
        };
        let rows = crate::mounts::parse(
            "1 1 0:21 / / rw - ext4 /dev/root rw\n2 1 8:1 / /other rw - ext4 /dev/sdb1 rw\n",
        )
        .unwrap();
        assert_eq!(
            check_mounts(&rows, &selected(), &physical, &underlay),
            Err(Failure::Conflict)
        );
        let rows = crate::mounts::parse(
            "1 1 0:21 / / rw - ext4 /dev/root rw\n2 1 0:21 /mnt /alias rw - ext4 /dev/root rw\n",
        )
        .unwrap();
        assert_eq!(
            check_mounts(&rows, &selected(), &physical, &underlay),
            Err(Failure::Conflict)
        );
    }
    #[test]
    fn kernel_name_text_cannot_substitute_for_kernel_task_flags() {
        assert!(kernel_marker(0, "Name:\tkworker\nKthread:\t1\n").is_err());
        assert!(kernel_marker(0x0020_0000, "Name:\tkworker\nKthread:\t0\n").is_err());
        assert!(kernel_marker(0x0020_0000, "Name:\tanything\n").is_err());
        assert_eq!(
            kernel_marker(0x0020_0000, "Name:\tanything\nKthread:\t1\n"),
            Ok(true)
        );
    }
}
