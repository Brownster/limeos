//! Fixed-image pre-effect gate and genuine owned-descendant quiescence.
//! A lost guardian, incomplete ACK or uninterruptible helper retains uncertainty.
use crate::storage_mount_admission::{MountAdmission, now, uptime_ticks};
use limeos_domain::{
    Error, ErrorCode, MountEffectQuiescence, MountHelperIdentity, MountHelperOutcome, Result,
};
use limeos_executor_storage::effects::MountLaunchPermit;
use rustix::{
    fd::OwnedFd,
    fs::{MemfdFlags, Mode, OFlags, SealFlags},
    process::{self, Pid, PidfdFlags, Signal, WaitOptions},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read, Write},
    os::{
        fd::{AsFd, AsRawFd},
        unix::{
            fs::{FileExt, MetadataExt},
            process::{CommandExt, ExitStatusExt},
        },
    },
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const IMAGE: &str = "/usr/lib/limeos/limeos-executor";
const ROLE: &str = "storage-mount-guardian";
const READY: &[u8; 8] = b"LMM1RDY\n";
const TICK: Duration = Duration::from_millis(5);
const DEADLINE: Duration = Duration::from_secs(3);
const MAX_FRAME: usize = 2048;
const MAX_IMAGE: u64 = 8 * 1024 * 1024;

fn error() -> Error {
    Error(ErrorCode::Unavailable)
}

/// A wire receipt cannot construct this token. It owns only a past actual
/// termination fact, never a permission to launch or retry.
pub struct MountHelperQuiescence {
    facts: MountEffectQuiescence,
}
impl MountHelperQuiescence {
    pub fn wire_facts(&self) -> &MountEffectQuiescence {
        &self.facts
    }
}

pub struct GatedMountHelper {
    owned: Option<Owned>,
}
pub struct RunningMountHelper {
    owned: Option<Owned>,
}
struct Owned {
    child: Child,
    input: Option<ChildStdin>,
    output: ChildStdout,
    _pidfd: OwnedFd,
    _image: File,
    _admission: MountAdmission,
    identity: MountHelperIdentity,
    action: String,
    digest: String,
    bytes: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Go {
    action: String,
    digest: String,
    expires_at: i64,
    deadline_ticks: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ack {
    action: String,
    digest: String,
    outcome: MountHelperOutcome,
}

impl GatedMountHelper {
    /// READY means the fixed guardian owns all invocation descriptors and has
    /// established its subreaper. It has not started any mount-capable child.
    pub async fn prepare(
        admission: MountAdmission,
        action: &str,
        digest: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self> {
        if !limeos_domain::opaque_id(action) || !limeos_domain::opaque_id(digest) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        admission.revalidate(cancelled).await?;
        admission.guardian_capacity()?;
        let image = sealed_image()?;
        let image_digest = digest_file(&image)?;
        let invocation = admission.target().invocation().map_err(|_| error())?;
        let mut command = Command::new(format!(
            "/proc/{}/fd/{}",
            std::process::id(),
            image.as_raw_fd()
        ));
        command
            .arg0(IMAGE)
            .arg(ROLE)
            .env_clear()
            .env("LIMEOS_MOUNT_PARENT", std::process::id().to_string())
            .env(
                "LIMEOS_MOUNT_SOURCE",
                invocation.source_descriptor().as_raw_fd().to_string(),
            )
            .env(
                "LIMEOS_MOUNT_TARGET",
                invocation.target_descriptor().as_raw_fd().to_string(),
            )
            .env(
                "LIMEOS_MOUNT_PROGRAM",
                invocation.image_descriptor().as_raw_fd().to_string(),
            )
            .env(
                "LIMEOS_MOUNT_NAMESPACE",
                admission.target().namespace_inode().to_string(),
            )
            .env(
                "LIMEOS_MOUNT_PROGRAM_SHA",
                admission.target().image_digest(),
            )
            .env(
                "LIMEOS_MOUNT_GATE_DEADLINE",
                admission.deadline_ticks().to_string(),
            )
            .env("LIMEOS_MOUNT_ACTION", action)
            .env("LIMEOS_MOUNT_DIGEST", digest)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        admission.remaining_window()?;
        let mut child = command.spawn().map_err(|_| error())?;
        let input = child.stdin.take().ok_or_else(error)?;
        let output = child.stdout.take().ok_or_else(error)?;
        // Child exists but remains gated. Every later failure closes the gate
        // and preserves original descriptors until the guardian is truly reaped.
        let pid = Pid::from_raw(child.id() as i32).ok_or_else(error)?;
        let pidfd = match process::pidfd_open(pid, PidfdFlags::empty()) {
            Ok(fd) => fd,
            Err(_) => {
                abandon_unidentified(child, input, output, admission, image);
                return Err(error());
            }
        };
        let pending_identity = MountHelperIdentity {
            pid: child.id(),
            start_time_ticks: 1,
            mount_namespace_inode: admission.target().namespace_inode(),
            cgroup: "/".into(),
            image_digest,
        };
        let mut owned = Owned {
            child,
            input: Some(input),
            output,
            _pidfd: pidfd,
            _image: image,
            _admission: admission,
            identity: pending_identity,
            action: action.into(),
            digest: digest.into(),
            bytes: Vec::new(),
        };
        let result = async {
            nonblocking(&owned.output)?;
            while owned.bytes.len() < READY.len() {
                if cancelled() {
                    return Err(Error(ErrorCode::Expired));
                }
                owned._admission.remaining_window()?;
                pump(&mut owned.output, &mut owned.bytes, READY.len())?;
                if owned.child.try_wait().map_err(|_| error())?.is_some() {
                    return Err(error());
                }
                if owned.bytes.len() < READY.len() {
                    tokio::time::sleep(TICK).await;
                }
            }
            if owned.bytes.as_slice() != READY {
                return Err(error());
            }
            owned.bytes.clear();
            let identity = identity(owned.child.id(), &owned.identity.image_digest)?;
            if identity.mount_namespace_inode != owned._admission.target().namespace_inode() {
                return Err(error());
            }
            owned.identity = identity;
            owned._admission.guardian_capacity()?;
            Ok(())
        }
        .await;
        if let Err(err) = result {
            abandon(owned);
            return Err(err);
        }
        Ok(Self { owned: Some(owned) })
    }
    pub fn identity(&self) -> &MountHelperIdentity {
        &self.owned.as_ref().expect("owned gated helper").identity
    }
    pub async fn release(
        mut self,
        permit: MountLaunchPermit,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<RunningMountHelper> {
        let owned = self.owned.as_mut().expect("owned gated helper");
        permit
            .check_binding(&owned.action, &owned.digest, &owned.identity, now()?)
            .map_err(|_| Error(ErrorCode::Conflict))?;
        owned._admission.revalidate(cancelled).await?;
        if cancelled()
            || now()? >= permit.expires_at()
            || uptime_ticks()?.saturating_add(1) >= owned._admission.deadline_ticks()
        {
            return Err(Error(ErrorCode::Expired));
        }
        if identity(owned.identity.pid, &owned.identity.image_digest)? != owned.identity {
            return Err(Error(ErrorCode::Conflict));
        }
        let go = Go {
            action: owned.action.clone(),
            digest: owned.digest.clone(),
            expires_at: permit.expires_at(),
            deadline_ticks: owned._admission.deadline_ticks(),
        };
        let bytes = serde_json::to_vec(&go).map_err(|_| error())?;
        if bytes.len() > MAX_FRAME {
            return Err(error());
        }
        let input = owned.input.as_mut().ok_or_else(error)?;
        input
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .and_then(|()| input.write_all(&bytes))
            .and_then(|()| input.flush())
            .map_err(|_| error())?;
        Ok(RunningMountHelper {
            owned: self.owned.take(),
        })
    }
}
impl RunningMountHelper {
    pub async fn await_quiescence(
        mut self,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<MountHelperQuiescence> {
        let owned = self.owned.as_mut().expect("owned running helper");
        let started = Instant::now();
        loop {
            if cancelled() || started.elapsed() >= DEADLINE {
                owned.input.take();
            }
            pump(&mut owned.output, &mut owned.bytes, MAX_FRAME)?;
            if let Some(exit) = owned.child.try_wait().map_err(|_| error())? {
                pump(&mut owned.output, &mut owned.bytes, MAX_FRAME)?;
                let token = finish(owned, exit.success())?;
                self.owned.take();
                return Ok(token);
            }
            // Deadline is a refusal to continue the caller, not proof of exit.
            // Drop transfers all live ownership to a cleanup monitor.
            if started.elapsed() >= DEADLINE + Duration::from_millis(250) {
                return Err(error());
            }
            tokio::time::sleep(TICK).await;
        }
    }
}
impl Drop for GatedMountHelper {
    fn drop(&mut self) {
        if let Some(owned) = self.owned.take() {
            abandon(owned);
        }
    }
}
impl Drop for RunningMountHelper {
    fn drop(&mut self) {
        if let Some(owned) = self.owned.take() {
            abandon(owned);
        }
    }
}
fn finish(owned: &Owned, success: bool) -> Result<MountHelperQuiescence> {
    if !success || owned.bytes.len() < 4 {
        return Err(error());
    }
    let size = u32::from_be_bytes(owned.bytes[..4].try_into().map_err(|_| error())?) as usize;
    if size > MAX_FRAME - 4 || owned.bytes.len() != size + 4 {
        return Err(error());
    }
    let ack: Ack = serde_json::from_slice(&owned.bytes[4..]).map_err(|_| error())?;
    if ack.action != owned.action || ack.digest != owned.digest {
        return Err(error());
    }
    let facts = MountEffectQuiescence::Reaped {
        helper: owned.identity.clone(),
        outcome: ack.outcome,
    };
    facts.validate()?;
    Ok(MountHelperQuiescence { facts })
}
fn abandon(mut owned: Owned) {
    owned.input.take();
    // A failed monitor creation leaks the retained owner/flight rather than
    // silently disarming unknown work. No further operation can acquire it.
    let retained = Box::leak(Box::new(owned));
    let _ = thread::Builder::new()
        .name("mount-cleanup".into())
        .spawn(move || {
            loop {
                let _ = pump(&mut retained.output, &mut retained.bytes, MAX_FRAME);
                match retained.child.try_wait() {
                    Ok(Some(exit)) if finish(retained, exit.success()).is_ok() => break,
                    Ok(Some(_)) | Err(_) => loop {
                        thread::park();
                    },
                    Ok(None) => thread::sleep(TICK),
                }
            }
            // Safe Rust cannot reclaim a Box::leak allocation. Keep the flight held
            // after an abandoned request; only a normal authenticated ACK path frees
            // it. This conservative process-local refusal requires reconciliation.
        });
}
fn abandon_unidentified(
    child: Child,
    input: ChildStdin,
    output: ChildStdout,
    admission: MountAdmission,
    image: File,
) {
    drop(input);
    let retained = Box::leak(Box::new((child, output, admission, image)));
    let _ = thread::Builder::new()
        .name("mount-unidentified".into())
        .spawn(move || {
            loop {
                let _ = retained.0.try_wait();
                thread::park();
            }
        });
}
fn nonblocking(fd: &impl AsFd) -> Result<()> {
    let flags = rustix::fs::fcntl_getfl(fd).map_err(|_| error())?;
    rustix::fs::fcntl_setfl(fd, flags | OFlags::NONBLOCK).map_err(|_| error())
}
fn pump(reader: &mut impl Read, bytes: &mut Vec<u8>, limit: usize) -> Result<()> {
    let mut scratch = [0; 256];
    loop {
        match reader.read(&mut scratch) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                if n > limit.saturating_sub(bytes.len()) {
                    return Err(error());
                }
                bytes.extend_from_slice(&scratch[..n]);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(error()),
        }
    }
}

fn bounded(path: &str, limit: usize) -> Result<Vec<u8>> {
    let mut file = File::open(path).map_err(|_| error())?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(limit + 1).map_err(|_| error())?;
    Read::by_ref(&mut file)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error())?;
    if bytes.len() > limit {
        return Err(error());
    }
    Ok(bytes)
}
fn identity(pid: u32, expected_image: &str) -> Result<MountHelperIdentity> {
    let stat = bounded(&format!("/proc/{pid}/stat"), 4096)?;
    let stat = std::str::from_utf8(&stat).map_err(|_| error())?;
    let end = stat.rfind(')').ok_or_else(error)?;
    let fields: Vec<_> = stat[end + 1..].split_ascii_whitespace().collect();
    if fields.len() < 20 || fields[1].parse::<u32>().ok() != Some(std::process::id()) {
        return Err(error());
    }
    let start_time_ticks = fields[19].parse().map_err(|_| error())?;
    let ns = std::fs::metadata(format!("/proc/{pid}/ns/mnt")).map_err(|_| error())?;
    let host = std::fs::metadata("/proc/1/ns/mnt").map_err(|_| error())?;
    if (ns.dev(), ns.ino()) != (host.dev(), host.ino()) {
        return Err(error());
    }
    let image = File::open(format!("/proc/{pid}/exe")).map_err(|_| error())?;
    if digest_file(&image)? != expected_image {
        return Err(error());
    }
    let cgroup = bounded(&format!("/proc/{pid}/cgroup"), 1024)?;
    let text = std::str::from_utf8(&cgroup).map_err(|_| error())?;
    let mut lines = text.lines();
    let cgroup = lines
        .next()
        .and_then(|s| s.strip_prefix("0::"))
        .ok_or_else(error)?;
    if lines.next().is_some() || !canonical(cgroup) {
        return Err(error());
    }
    Ok(MountHelperIdentity {
        pid,
        start_time_ticks,
        mount_namespace_inode: ns.ino(),
        cgroup: cgroup.into(),
        image_digest: expected_image.into(),
    })
}
fn canonical(path: &str) -> bool {
    path == "/"
        || path.starts_with('/')
            && path.len() <= 512
            && !path.chars().any(char::is_control)
            && path[1..].split('/').all(|s| !matches!(s, "" | "." | ".."))
}
fn protected_image() -> Result<File> {
    let mut fd = rustix::fs::open(
        "/",
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| error())?;
    for (i, part) in IMAGE[1..].split('/').enumerate() {
        let flags = if i == 3 {
            OFlags::RDONLY
        } else {
            OFlags::PATH | OFlags::DIRECTORY
        };
        let stat = rustix::fs::fstat(&fd).map_err(|_| error())?;
        if stat.st_uid != 0 || stat.st_mode & 0o022 != 0 {
            return Err(error());
        }
        fd = rustix::fs::openat(
            &fd,
            part,
            flags | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| error())?;
    }
    let file = File::from(fd);
    let meta = file.metadata().map_err(|_| error())?;
    if !meta.is_file()
        || meta.uid() != 0
        || meta.mode() & 0o022 != 0
        || meta.nlink() != 1
        || meta.mode() & 0o111 == 0
        || !(64..=MAX_IMAGE).contains(&meta.len())
    {
        return Err(error());
    }
    Ok(file)
}
fn sealed_image() -> Result<File> {
    let source = protected_image()?;
    let before = source.metadata().map_err(|_| error())?;
    let mut header = [0; 4];
    source.read_exact_at(&mut header, 0).map_err(|_| error())?;
    if header != *b"\x7fELF" {
        return Err(error());
    }
    let mut output = File::from(
        rustix::fs::memfd_create(
            "limeos-mount-guardian",
            MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
        )
        .map_err(|_| error())?,
    );
    let mut offset = 0;
    let mut scratch = [0; 8192];
    while offset < before.len() {
        let wanted = (before.len() - offset).min(scratch.len() as u64) as usize;
        source
            .read_exact_at(&mut scratch[..wanted], offset)
            .map_err(|_| error())?;
        output.write_all(&scratch[..wanted]).map_err(|_| error())?;
        offset += wanted as u64;
    }
    let after = source.metadata().map_err(|_| error())?;
    if (
        before.dev(),
        before.ino(),
        before.len(),
        before.ctime(),
        before.ctime_nsec(),
        before.mtime(),
        before.mtime_nsec(),
    ) != (
        after.dev(),
        after.ino(),
        after.len(),
        after.ctime(),
        after.ctime_nsec(),
        after.mtime(),
        after.mtime_nsec(),
    ) {
        return Err(error());
    }
    rustix::fs::fchmod(&output, Mode::RUSR | Mode::XUSR).map_err(|_| error())?;
    let seals = SealFlags::WRITE | SealFlags::GROW | SealFlags::SHRINK | SealFlags::SEAL;
    rustix::fs::fcntl_add_seals(&output, seals).map_err(|_| error())?;
    if !rustix::fs::fcntl_get_seals(&output)
        .map_err(|_| error())?
        .contains(seals)
    {
        return Err(error());
    }
    Ok(output)
}
fn digest_file(file: &File) -> Result<String> {
    let size = file.metadata().map_err(|_| error())?.len();
    if size == 0 || size > MAX_IMAGE {
        return Err(error());
    }
    let mut hash = Sha256::new();
    let mut offset = 0;
    let mut scratch = [0; 8192];
    while offset < size {
        let n = (size - offset).min(scratch.len() as u64) as usize;
        file.read_exact_at(&mut scratch[..n], offset)
            .map_err(|_| error())?;
        hash.update(&scratch[..n]);
        offset += n as u64;
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Dedicated CLI role, called before Tokio or any other thread starts. Only
/// this fixed guardian may spawn the fixed --internal-only mount invocation.
pub fn guardian() -> Result<()> {
    if !process::geteuid().is_root() {
        return Err(error());
    }
    let status = bounded("/proc/self/status", 16384)?;
    let status = std::str::from_utf8(&status).map_err(|_| error())?;
    let mut capabilities = status.lines().filter_map(|s| s.strip_prefix("CapEff:"));
    let effective = capabilities
        .next()
        .and_then(|s| u64::from_str_radix(s.trim(), 16).ok())
        .ok_or_else(error)?;
    let required = (1_u64 << 2) | (1_u64 << 19) | (1_u64 << 21);
    if capabilities.next().is_some() || effective & required != required {
        return Err(error());
    }
    let mut threads = status.lines().filter_map(|s| s.strip_prefix("Threads:"));
    if threads.next().map(str::trim) != Some("1")
        || threads.next().is_some()
        || !children()?.is_empty()
    {
        return Err(error());
    }
    let stdin = io::stdin();
    if rustix::fs::FileType::from_raw_mode(rustix::fs::fstat(&stdin).map_err(|_| error())?.st_mode)
        != rustix::fs::FileType::Fifo
    {
        return Err(error());
    }
    let mut input = stdin.lock();
    nonblocking(&input)?;
    let parent = env_number("LIMEOS_MOUNT_PARENT")?;
    if process::getppid().map(|p| p.as_raw_nonzero().get() as u64) != Some(parent) {
        return Err(error());
    }
    let namespace = env_number("LIMEOS_MOUNT_NAMESPACE")?;
    host_namespace(namespace)?;
    let deadline = env_number("LIMEOS_MOUNT_GATE_DEADLINE")?;
    let action = std::env::var("LIMEOS_MOUNT_ACTION").map_err(|_| error())?;
    let digest = std::env::var("LIMEOS_MOUNT_DIGEST").map_err(|_| error())?;
    if !limeos_domain::opaque_id(&action) || !limeos_domain::opaque_id(&digest) {
        return Err(error());
    }
    let source = copy_parent_fd(parent, "LIMEOS_MOUNT_SOURCE", false)?;
    let target = copy_parent_fd(parent, "LIMEOS_MOUNT_TARGET", true)?;
    let program = copy_parent_fd(parent, "LIMEOS_MOUNT_PROGRAM", false)?;
    let source_stat = rustix::fs::fstat(&source).map_err(|_| error())?;
    let target_stat = rustix::fs::fstat(&target).map_err(|_| error())?;
    let program_stat = rustix::fs::fstat(&program).map_err(|_| error())?;
    if source_stat.st_mode & 0o170000 != 0o060000
        || target_stat.st_mode & 0o170000 != 0o040000
        || target_stat.st_uid != 0
        || target_stat.st_mode & 0o022 != 0
        || program_stat.st_mode & 0o170000 != 0o100000
        || program_stat.st_uid != 0
        || program_stat.st_mode & 0o022 != 0
    {
        return Err(error());
    }
    let program = File::from(program);
    if digest_file(&program)? != std::env::var("LIMEOS_MOUNT_PROGRAM_SHA").map_err(|_| error())? {
        return Err(error());
    }
    process::setsid().map_err(|_| error())?;
    process::set_child_subreaper(Some(Pid::INIT)).map_err(|_| error())?;
    if process::child_subreaper().map_err(|_| error())? != Some(Pid::INIT) {
        return Err(error());
    }
    io::stdout()
        .lock()
        .write_all(READY)
        .and_then(|()| io::stdout().lock().flush())
        .map_err(|_| error())?;
    let mut frame = Vec::new();
    let go = loop {
        if uptime_ticks()?.saturating_add(1) >= deadline {
            return Err(Error(ErrorCode::Expired));
        }
        let mut buffer = [0; 256];
        match input.read(&mut buffer) {
            Ok(0) => return Err(error()),
            Ok(n) => {
                if n > MAX_FRAME.saturating_sub(frame.len()) {
                    return Err(error());
                }
                frame.extend_from_slice(&buffer[..n]);
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(_) => return Err(error()),
        }
        if frame.len() >= 4 {
            let n = u32::from_be_bytes(frame[..4].try_into().map_err(|_| error())?) as usize;
            if n > MAX_FRAME - 4 || frame.len() > n + 4 {
                return Err(error());
            }
            if frame.len() == n + 4 {
                break serde_json::from_slice::<Go>(&frame[4..]).map_err(|_| error())?;
            }
        }
        thread::sleep(TICK);
    };
    check_go(&go, &action, &digest, deadline, now()?, uptime_ticks()?)?;
    if parent_gone(&mut input)? {
        return Err(Error(ErrorCode::Expired));
    }
    host_namespace(namespace)?;
    let me = std::process::id();
    let mut command = Command::new(format!("/proc/{me}/fd/{}", program.as_raw_fd()));
    command
        .arg0("/usr/bin/mount")
        .args([
            "--no-canonicalize",
            "--no-mtab",
            "--internal-only",
            "--types",
            "ext4",
            "--options",
            "rw,nosuid,nodev",
            "--source",
        ])
        .arg(format!("/proc/{me}/fd/{}", source.as_raw_fd()))
        .arg("--target")
        .arg(format!("/proc/{me}/fd/{}", target.as_raw_fd()))
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    // Check both the original collection gate and plan expiry at launch. The
    // helper may subsequently block in-kernel; expiry never proves its death.
    check_go(&go, &action, &digest, deadline, now()?, uptime_ticks()?)?;
    if parent_gone(&mut input)? {
        return Err(Error(ErrorCode::Expired));
    }
    let mut child = command.spawn().map_err(|_| error())?;
    let started = Instant::now();
    let outcome = loop {
        if let Some(exit) = child.try_wait().map_err(|_| error())? {
            break if let Some(code) = exit.code() {
                MountHelperOutcome::Exited { code }
            } else {
                MountHelperOutcome::Signaled {
                    signal: exit.signal().ok_or_else(error)?,
                }
            };
        }
        if started.elapsed() >= DEADLINE || parent_gone(&mut input).unwrap_or(true) {
            cleanup();
            let exit = child.wait().map_err(|_| error());
            // wait() may see ECHILD because cleanup reaped the child; without
            // its actual exit status no authenticated Reaped ACK is emitted.
            return exit.and(Err(error()));
        }
        thread::sleep(TICK);
    };
    cleanup();
    let ack = serde_json::to_vec(&Ack {
        action,
        digest,
        outcome,
    })
    .map_err(|_| error())?;
    if ack.len() > MAX_FRAME - 4 {
        return Err(error());
    }
    let mut output = io::stdout().lock();
    output
        .write_all(&(ack.len() as u32).to_be_bytes())
        .and_then(|()| output.write_all(&ack))
        .and_then(|()| output.flush())
        .map_err(|_| error())
}
fn env_number(key: &str) -> Result<u64> {
    let value = std::env::var(key).map_err(|_| error())?;
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(error());
    }
    value.parse().map_err(|_| error())
}
fn copy_parent_fd(parent: u64, key: &str, directory: bool) -> Result<OwnedFd> {
    let fd = env_number(key)?;
    if !(3..226).contains(&fd) {
        return Err(error());
    }
    rustix::fs::open(
        format!("/proc/{parent}/fd/{fd}"),
        OFlags::RDONLY
            | OFlags::CLOEXEC
            | if directory {
                OFlags::DIRECTORY
            } else {
                OFlags::empty()
            },
        Mode::empty(),
    )
    .map_err(|_| error())
}
fn host_namespace(expected: u64) -> Result<()> {
    let current = std::fs::metadata("/proc/self/ns/mnt").map_err(|_| error())?;
    let host = std::fs::metadata("/proc/1/ns/mnt").map_err(|_| error())?;
    let root = std::fs::metadata("/").map_err(|_| error())?;
    let host_root = std::fs::metadata("/proc/1/root").map_err(|_| error())?;
    if current.ino() != expected
        || (current.dev(), current.ino()) != (host.dev(), host.ino())
        || (root.dev(), root.ino()) != (host_root.dev(), host_root.ino())
    {
        return Err(error());
    }
    Ok(())
}
fn parent_gone(input: &mut impl Read) -> Result<bool> {
    let mut byte = [0];
    match input.read(&mut byte) {
        Ok(_) => Ok(true),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) =>
        {
            Ok(false)
        }
        Err(_) => Err(error()),
    }
}
fn children() -> Result<Vec<Pid>> {
    let text = bounded("/proc/thread-self/children", 4096)?;
    let text = std::str::from_utf8(&text).map_err(|_| error())?;
    let mut children = Vec::new();
    children.try_reserve_exact(256).map_err(|_| error())?;
    for raw in text.split_ascii_whitespace() {
        let pid = raw.parse().ok().and_then(Pid::from_raw).ok_or_else(error)?;
        if children.len() == 256
            || pid == Pid::INIT
            || pid == process::getpid()
            || children.contains(&pid)
        {
            return Err(error());
        }
        children.push(pid);
    }
    Ok(children)
}
fn own_child(pid: Pid) -> Result<bool> {
    let bytes = bounded(&format!("/proc/{}/stat", pid.as_raw_nonzero().get()), 4096)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| error())?;
    let end = text.rfind(')').ok_or_else(error)?;
    let parent = text[end + 1..]
        .split_ascii_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<i32>().ok())
        .ok_or_else(error)?;
    Ok(parent == process::getpid().as_raw_nonzero().get())
}
fn cleanup() {
    loop {
        if let Ok(children) = children() {
            for pid in children {
                if own_child(pid) == Ok(true) {
                    if let Ok(fd) = process::pidfd_open(pid, PidfdFlags::empty()) {
                        if own_child(pid) == Ok(true) {
                            let _ = process::pidfd_send_signal(&fd, Signal::KILL);
                        }
                    }
                }
            }
        }
        loop {
            match process::wait(WaitOptions::NOHANG) {
                Ok(Some(_)) => continue,
                Err(rustix::io::Errno::CHILD) => return,
                _ => break,
            }
        }
        thread::sleep(TICK);
    }
}

fn check_go(
    go: &Go,
    action: &str,
    digest: &str,
    deadline: u64,
    wall: i64,
    ticks: u64,
) -> Result<()> {
    if go.action != action
        || go.digest != digest
        || go.deadline_ticks != deadline
        || wall >= go.expires_at
        || ticks.saturating_add(1) >= deadline
    {
        return Err(Error(ErrorCode::Expired));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guardian_gate_binds_operation_and_both_original_deadlines() {
        let go = Go {
            action: "action".into(),
            digest: "digest".into(),
            expires_at: 200,
            deadline_ticks: 400,
        };
        assert!(check_go(&go, "action", "digest", 400, 199, 398).is_ok());
        assert!(check_go(&go, "different", "digest", 400, 199, 398).is_err());
        assert!(check_go(&go, "action", "different", 400, 199, 398).is_err());
        assert!(check_go(&go, "action", "digest", 401, 199, 398).is_err());
        assert!(check_go(&go, "action", "digest", 400, 200, 398).is_err());
        assert!(check_go(&go, "action", "digest", 400, 199, 399).is_err());
    }
}
