//! Unregistered process mechanics; trusted launch policy remains outside this API.
//! A guardian acknowledgment proves cleanup, never retained storage authority.
mod guardian;
use crate::read_budget::{FlightGuard, SingleFlight};
pub use guardian::{WorkerBootstrap, guardian, write_worker_refusal, write_worker_report};
use rustix::{fd::AsFd, fs::OFlags};
use serde::Deserialize;
use std::{
    io::{self, Read},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

#[cfg(test)]
mod tests;
const WINDOW: Duration = Duration::from_secs(4);
const TICK: Duration = Duration::from_millis(10);
const REPORT: usize = 256 * 1024;
const HEADER: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Busy,
    Unavailable,
    WorkerRefused,
    Deadline,
    Cancelled,
    InvalidOutput,
    WorkerExit,
    CleanupUnconfirmed,
}
pub type Result<T> = std::result::Result<T, Refusal>;

/// Complete bytes plus successful guardian exit/cleanup; no effect authority.
/// ```compile_fail
/// use limeos_executor::read_supervision::SupervisedReport;
/// let forged: SupervisedReport = serde_json::from_str("{}").unwrap();
/// ```
pub struct SupervisedReport(Vec<u8>);
impl SupervisedReport {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Default)]
pub struct Supervisor {
    flight: SingleFlight,
    poisoned: Arc<AtomicBool>,
}
pub struct Ticket {
    receiver: Receiver<Result<SupervisedReport>>,
    cancelled: Arc<AtomicBool>,
    started: Instant,
}
impl Supervisor {
    /// Trusted Rust-side guardian selection only. No runtime route calls this.
    /// The dedicated guardian must invoke `guardian` before any other work.
    pub fn start(&self, mut command: Command) -> Result<Ticket> {
        if self.poisoned.load(Ordering::Acquire) {
            return Err(Refusal::CleanupUnconfirmed);
        }
        let flight = self.flight.try_acquire().map_err(|_| Refusal::Busy)?;
        if self.poisoned.load(Ordering::Acquire) {
            return Err(Refusal::CleanupUnconfirmed);
        }
        let started = Instant::now();
        let cancelled = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::sync_channel(1);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let cancel = cancelled.clone();
        let poisoned = self.poisoned.clone();
        thread::Builder::new()
            .name("read-monitor".into())
            .spawn(move || monitor(command, flight, started, cancel, poisoned, sender))
            .map_err(|_| Refusal::Unavailable)?;
        Ok(Ticket {
            receiver,
            cancelled,
            started,
        })
    }
}
impl Ticket {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn wait(self) -> Result<SupervisedReport> {
        if self.started.elapsed() >= WINDOW {
            self.cancel();
            return Err(Refusal::Deadline);
        }
        if self.cancelled.load(Ordering::Acquire) {
            return Err(Refusal::Cancelled);
        }
        let remaining = WINDOW.saturating_sub(self.started.elapsed());
        match self.receiver.recv_timeout(remaining) {
            Ok(result) => {
                if self.started.elapsed() >= WINDOW {
                    self.cancel();
                    Err(Refusal::Deadline)
                } else if self.cancelled.load(Ordering::Acquire) {
                    Err(Refusal::Cancelled)
                } else {
                    result
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.cancel();
                Err(Refusal::Deadline)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(Refusal::CleanupUnconfirmed),
        }
    }
}

struct Admission {
    _flight: FlightGuard,
    poisoned: Arc<AtomicBool>,
    launched: bool,
    confirmed: bool,
}
impl Drop for Admission {
    fn drop(&mut self) {
        if self.launched && !self.confirmed {
            self.poisoned.store(true, Ordering::Release);
        }
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn monitor(
    mut command: Command,
    flight: FlightGuard,
    started: Instant,
    cancelled: Arc<AtomicBool>,
    poisoned: Arc<AtomicBool>,
    sender: SyncSender<Result<SupervisedReport>>,
) {
    let mut admission = Admission {
        _flight: flight,
        poisoned: poisoned.clone(),
        launched: false,
        confirmed: false,
    };
    let mut capture = match Capture::new() {
        Ok(capture) => capture,
        Err(error) => {
            let _ = sender.try_send(Err(error));
            return;
        }
    };
    if cancelled.load(Ordering::Acquire) || started.elapsed() >= WINDOW {
        let error = if started.elapsed() >= WINDOW {
            Refusal::Deadline
        } else {
            Refusal::Cancelled
        };
        let _ = sender.try_send(Err(error));
        return;
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            let _ = sender.try_send(Err(Refusal::Unavailable));
            return;
        }
    };
    admission.launched = true;
    let mut liveness = child.stdin.take();
    let mut output = child.stdout.take().expect("piped guardian output");
    let mut failure = nonblocking(&output).err();
    let mut transport_failed = failure.is_some();
    let mut sent = false;
    loop {
        if failure.is_none() {
            failure = if cancelled.load(Ordering::Acquire) {
                Some(Refusal::Cancelled)
            } else if started.elapsed() >= WINDOW {
                Some(Refusal::Deadline)
            } else {
                None
            };
        }
        if !transport_failed {
            if let Err(error) = capture.pump(&mut output) {
                transport_failed = true;
                failure.get_or_insert(error);
            }
        } else if nonblocking(&output).is_ok() {
            // Keep the acknowledgment pipe draining after a malformed transport.
            let mut discard = [0; 8192];
            for _ in 0..8 {
                if output.read(&mut discard).is_err() {
                    break;
                }
            }
        }
        if let Some(error) = failure {
            // EOF is cancellation and parent death for the independent guardian.
            drop(liveness.take());
            if !sent {
                let _ = sender.try_send(Err(error));
                sent = true;
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let frame = if !transport_failed && status.success() && nonblocking(&output).is_ok()
                {
                    capture
                        .pump(&mut output)
                        .and_then(|()| capture.finish(false))
                } else {
                    Err(Refusal::CleanupUnconfirmed)
                };
                // Only the dedicated guardian's complete frame AND exit confirm cleanup.
                if frame.is_err() {
                    poisoned.store(true, Ordering::Release);
                } else {
                    admission.confirmed = true;
                }
                let result = frame.and_then(|frame| frame.result());
                if !sent {
                    let _ = sender.try_send(if started.elapsed() >= WINDOW {
                        Err(Refusal::Deadline)
                    } else if cancelled.load(Ordering::Acquire) {
                        Err(Refusal::Cancelled)
                    } else if let Some(error) = failure {
                        Err(error)
                    } else {
                        result
                    });
                }
                return;
            }
            Err(_) => {
                poisoned.store(true, Ordering::Release);
                drop(liveness.take());
                if !sent {
                    let _ = sender.try_send(Err(Refusal::CleanupUnconfirmed));
                    sent = true;
                }
                // Preserve the permit while exit/reaping remains unknown.
            }
            Ok(None) => {}
        }
        thread::sleep(TICK);
    }
}

fn nonblocking(fd: impl AsFd) -> Result<()> {
    let flags = rustix::fs::fcntl_getfl(&fd).map_err(|_| Refusal::Unavailable)?;
    rustix::fs::fcntl_setfl(fd, flags | OFlags::NONBLOCK).map_err(|_| Refusal::Unavailable)
}
struct Capture {
    bytes: Vec<u8>,
    eof: bool,
}
enum Frame {
    Report(SupervisedReport),
    Refused(Refusal),
}
impl Frame {
    fn result(self) -> Result<SupervisedReport> {
        match self {
            Self::Report(report) => Ok(report),
            Self::Refused(error) => Err(error),
        }
    }
}
impl Capture {
    fn new() -> Result<Self> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(REPORT + HEADER + 1)
            .map_err(|_| Refusal::Unavailable)?;
        Ok(Self { bytes, eof: false })
    }
    fn pump(&mut self, input: &mut impl Read) -> Result<()> {
        let mut scratch = [0; 8192];
        for _ in 0..64 {
            match input.read(&mut scratch) {
                Ok(0) => {
                    self.eof = true;
                    return Ok(());
                }
                Ok(length) => {
                    if length > REPORT + HEADER - self.bytes.len() {
                        return Err(Refusal::InvalidOutput);
                    }
                    self.bytes.extend_from_slice(&scratch[..length]);
                    if self.bytes.len() >= HEADER {
                        let declared =
                            u32::from_be_bytes(self.bytes[1..5].try_into().unwrap()) as usize;
                        if declared > REPORT || self.bytes.len() > HEADER + declared {
                            return Err(Refusal::InvalidOutput);
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(_) => return Err(Refusal::Unavailable),
            }
        }
        Ok(())
    }
    fn finish(mut self, worker: bool) -> Result<Frame> {
        if !self.eof || self.bytes.len() < HEADER {
            return Err(Refusal::InvalidOutput);
        }
        let status = self.bytes[0];
        let length = u32::from_be_bytes(self.bytes[1..5].try_into().unwrap()) as usize;
        if length > REPORT || self.bytes.len() != HEADER + length {
            return Err(Refusal::InvalidOutput);
        }
        if status == 0 {
            if length == 0 {
                return Err(Refusal::InvalidOutput);
            }
            let mut decoder = serde_json::Deserializer::from_slice(&self.bytes[HEADER..]);
            serde::de::IgnoredAny::deserialize(&mut decoder).map_err(|_| Refusal::InvalidOutput)?;
            decoder.end().map_err(|_| Refusal::InvalidOutput)?;
            self.bytes.copy_within(HEADER.., 0);
            self.bytes.truncate(length);
            Ok(Frame::Report(SupervisedReport(self.bytes)))
        } else if length != 0 || (worker && status != 1) || !(1..=6).contains(&status) {
            Err(Refusal::InvalidOutput)
        } else {
            Ok(Frame::Refused(match status {
                1 => Refusal::WorkerRefused,
                2 => Refusal::Deadline,
                3 => Refusal::Cancelled,
                4 => Refusal::InvalidOutput,
                5 => Refusal::WorkerExit,
                _ => Refusal::Unavailable,
            }))
        }
    }
}
