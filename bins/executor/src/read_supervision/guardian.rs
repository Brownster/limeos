use super::*;
use crate::read_collection::{self, CollectedReadReport};
use limeos_executor_storage::configuration::ConfigurationPaths;
use rustix::process::{self, Pid, PidfdFlags, Resource, Rlimit, Signal, WaitOptions};
use std::{fs::File, io::Write, os::unix::process::CommandExt};

/// Bootstrap only in a dedicated worker before adapters/runtime initialization.
pub struct WorkerBootstrap {
    guardian: Pid,
}
impl WorkerBootstrap {
    pub fn establish(guardian: Pid) -> Result<Self> {
        let valid = || {
            process::getppid() == Some(guardian)
                && process::getpgid(None).ok() == Some(process::getpid())
                && process::getsid(None).ok() == Some(guardian)
                && process::getrlimit(Resource::Nofile).current == Some(226)
        };
        if !valid() {
            return Err(Refusal::Unavailable);
        }
        process::set_parent_process_death_signal(Some(Signal::KILL))
            .map_err(|_| Refusal::Unavailable)?;
        if !valid() {
            return Err(Refusal::Unavailable);
        }
        Ok(Self { guardian })
    }
    /// No authentication substitutions: calls the existing genuine public entry.
    pub async fn collect(
        &self,
        paths: ConfigurationPaths<'_>,
    ) -> read_collection::Result<CollectedReadReport> {
        read_collection::collect(paths, &|| process::getppid() != Some(self.guardian)).await
    }
}

pub fn write_worker_report(body: &[u8]) -> Result<()> {
    write_frame(0, body)
}
pub fn write_worker_refusal() -> Result<()> {
    write_frame(1, &[])
}
fn write_frame(status: u8, body: &[u8]) -> Result<()> {
    if body.len() > REPORT {
        return Err(Refusal::InvalidOutput);
    }
    let mut header = [0; HEADER];
    header[0] = status;
    header[1..].copy_from_slice(&(body.len() as u32).to_be_bytes());
    let mut output = io::stdout().lock();
    output
        .write_all(&header)
        .and_then(|()| output.write_all(body))
        .and_then(|()| output.flush())
        .map_err(|_| Refusal::Unavailable)
}
fn code(error: Refusal) -> u8 {
    match error {
        Refusal::WorkerRefused => 1,
        Refusal::Deadline => 2,
        Refusal::Cancelled => 3,
        Refusal::InvalidOutput => 4,
        Refusal::WorkerExit => 5,
        _ => 6,
    }
}

/// Run only in a dedicated, single-threaded guardian process with piped stdin.
/// Session/subreaper/rlimit changes apply to this process, never Core/the manager.
/// The trusted worker selection must invoke `WorkerBootstrap` before collection.
pub fn guardian(mut worker: Command) -> Result<()> {
    // Refuse process-wide mutation in a runtime or ordinary caller context.
    let status = bounded_file("/proc/self/status", 16 * 1024)?;
    let status = std::str::from_utf8(&status).map_err(|_| Refusal::Unavailable)?;
    let mut threads = status
        .lines()
        .filter_map(|line| line.strip_prefix("Threads:"));
    if threads.next().map(str::trim) != Some("1") || threads.next().is_some() {
        return Err(Refusal::Unavailable);
    }
    let input = io::stdin();
    let stat = rustix::fs::fstat(&input).map_err(|_| Refusal::Unavailable)?;
    if rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::Fifo {
        return Err(Refusal::Unavailable);
    }
    let mut input = input.lock();
    nonblocking(&input)?;
    let mut capture = Capture::new()?;
    if !children()?.is_empty() {
        return Err(Refusal::Unavailable);
    }
    process::setsid().map_err(|_| Refusal::Unavailable)?;
    process::set_child_subreaper(Some(Pid::INIT)).map_err(|_| Refusal::Unavailable)?;
    if process::child_subreaper().map_err(|_| Refusal::Unavailable)? != Some(Pid::INIT) {
        return Err(Refusal::Unavailable);
    }
    let old = process::getrlimit(Resource::Nofile);
    if old.current.is_some_and(|limit| limit < 226) {
        return Err(Refusal::Unavailable);
    }
    process::setrlimit(
        Resource::Nofile,
        Rlimit {
            current: Some(226),
            maximum: old.maximum,
        },
    )
    .map_err(|_| Refusal::Unavailable)?;
    let started = Instant::now();
    // Ensure an already-dead manager cannot start a worker.
    if parent_gone(&mut input)? {
        return write_frame(code(Refusal::Cancelled), &[]);
    }
    worker
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .env(
            "LIMEOS_READ_GUARDIAN_PID",
            process::getpid().as_raw_nonzero().get().to_string(),
        );
    let mut child = worker.spawn().map_err(|_| Refusal::Unavailable)?;
    let mut output = child.stdout.take().expect("piped worker output");
    let mut result = nonblocking(&output)
        .map(|()| None)
        .unwrap_or_else(|error| Some(Err(error)));
    while result.is_none() {
        let error = if started.elapsed() >= WINDOW {
            Some(Refusal::Deadline)
        } else if parent_gone(&mut input).unwrap_or(true) {
            Some(Refusal::Cancelled)
        } else {
            capture.pump(&mut output).err()
        };
        if let Some(error) = error {
            result = Some(Err(error));
            break;
        }
        match child.try_wait() {
            Ok(Some(exit)) => {
                result = Some(if !exit.success() {
                    Err(Refusal::WorkerExit)
                } else {
                    Ok(())
                });
                break;
            }
            Err(_) => {
                result = Some(Err(Refusal::Unavailable));
                break;
            }
            Ok(None) => {}
        }
        thread::sleep(TICK);
    }
    // No acknowledgment before the entire owned descendant set has been reaped.
    cleanup();
    let result = result.expect("finished worker").and_then(|()| {
        capture.pump(&mut output)?;
        capture.finish(true)?.result()
    });
    let result = if started.elapsed() >= WINDOW {
        Err(Refusal::Deadline)
    } else if parent_gone(&mut input).unwrap_or(true) {
        Err(Refusal::Cancelled)
    } else {
        result
    };
    match result {
        Ok(report) => write_frame(0, report.as_bytes()),
        Err(error) => write_frame(code(error), &[]),
    }
}

fn parent_gone(input: &mut impl Read) -> Result<bool> {
    let mut byte = [0];
    match input.read(&mut byte) {
        Ok(_) => Ok(true), // EOF or any explicit cancellation byte.
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => Ok(false),
        Err(_) => Err(Refusal::Unavailable),
    }
}
fn bounded_file(path: &str, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(limit + 1)
        .map_err(|_| Refusal::Unavailable)?;
    bytes.resize(limit + 1, 0);
    let mut file = File::open(path).map_err(|_| Refusal::Unavailable)?;
    let mut length = 0;
    while length < bytes.len() {
        match file.read(&mut bytes[length..]) {
            Ok(0) => break,
            Ok(n) => length += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(Refusal::Unavailable),
        }
    }
    bytes.truncate(length);
    if length > limit {
        Err(Refusal::Unavailable)
    } else {
        Ok(bytes)
    }
}
fn children() -> Result<Vec<Pid>> {
    let bytes = bounded_file("/proc/thread-self/children", 4096)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| Refusal::Unavailable)?;
    let mut children = Vec::new();
    children
        .try_reserve_exact(256)
        .map_err(|_| Refusal::Unavailable)?;
    for raw in text.split_ascii_whitespace() {
        if children.len() == 256 || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(Refusal::Unavailable);
        }
        let pid = raw
            .parse()
            .ok()
            .and_then(Pid::from_raw)
            .ok_or(Refusal::Unavailable)?;
        if pid == Pid::INIT || pid == process::getpid() || children.contains(&pid) {
            return Err(Refusal::Unavailable);
        }
        children.push(pid);
    }
    Ok(children)
}
fn own_child(pid: Pid) -> Result<bool> {
    let bytes = bounded_file(&format!("/proc/{}/stat", pid.as_raw_nonzero().get()), 4096)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| Refusal::Unavailable)?;
    let end = text.rfind(')').ok_or(Refusal::Unavailable)?;
    let parent = text[end + 1..]
        .split_ascii_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<i32>().ok())
        .ok_or(Refusal::Unavailable)?;
    Ok(parent == process::getpid().as_raw_nonzero().get())
}
fn cleanup() {
    loop {
        if let Ok(children) = children() {
            for pid in children {
                if own_child(pid) != Ok(true) {
                    continue;
                }
                // Signal the pinned task, not a potentially reused numeric PID.
                if let Ok(fd) = process::pidfd_open(pid, PidfdFlags::empty()) {
                    if own_child(pid) == Ok(true) {
                        let _ = process::pidfd_send_signal(&fd, Signal::KILL);
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
