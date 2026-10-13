//! Capacity admission, not retained evidence or independent process supervision.
use limeos_executor_container::docker::process_evidence::EngineReadProfile;
use limeos_executor_storage::{
    StorageReadProfile, configuration::ConfigurationReadProfile, destinations::DestinationLimits,
    processes::ProcessReadProfile,
};
use rustix::process::{Resource, getrlimit};
use serde::Serialize;
use std::{
    fs::File,
    io::{self, Read, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[cfg(test)]
mod tests;
const MIB: usize = 1024 * 1024;
const REPORT: usize = 256 * 1024;
const WORKER_FDS: usize = 226;
const BASELINE_FDS: usize = 16;
const TRANSIENT_FDS: usize = 32;
const BASELINE_RSS: usize = 24 * MIB;
const PAYLOAD: usize = 24 * MIB;
const WORKER_RSS: usize = BASELINE_RSS + PAYLOAD;
const WORKER_THREADS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Busy,
    Unavailable,
    ResourceLimit,
    InvalidPhase,
    Poisoned,
}
pub type Result<T> = std::result::Result<T, Refusal>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Configuration,
    Engine,
    Processes,
    Physical,
    Destinations,
    Graph,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct OwnerReservation {
    phase: Phase,
    retained_fds: usize,
    modeled_payload_bytes: usize,
}
const OWNERS: [OwnerReservation; 6] = [
    OwnerReservation {
        phase: Phase::Configuration,
        retained_fds: 33,
        modeled_payload_bytes: 12 * MIB,
    },
    OwnerReservation {
        phase: Phase::Engine,
        retained_fds: 1,
        modeled_payload_bytes: 2 * MIB,
    },
    OwnerReservation {
        phase: Phase::Processes,
        retained_fds: 37,
        modeled_payload_bytes: 2 * MIB,
    },
    OwnerReservation {
        phase: Phase::Physical,
        retained_fds: 75,
        modeled_payload_bytes: 2 * MIB,
    },
    OwnerReservation {
        phase: Phase::Destinations,
        retained_fds: 32,
        modeled_payload_bytes: MIB,
    },
    OwnerReservation {
        phase: Phase::Graph,
        retained_fds: 0,
        modeled_payload_bytes: 2 * MIB,
    },
];

/// Serialize-only resource model; this establishes no host or effect authority.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct BudgetPlan {
    version: u16,
    owners: [OwnerReservation; 6],
    retained_fds: usize,
    baseline_fds: usize,
    transient_fds: usize,
    worker_soft_nofile: usize,
    baseline_rss_bytes: usize,
    modeled_payload_bytes: usize,
    sampled_rss_refusal_bytes: usize,
    report_bytes: usize,
    worker_threads: usize,
    service_nofile: usize,
    service_memory_bytes: usize,
    service_tasks: usize,
}
pub fn plan() -> BudgetPlan {
    BudgetPlan {
        version: 1,
        owners: OWNERS,
        retained_fds: OWNERS.iter().map(|o| o.retained_fds).sum(),
        baseline_fds: BASELINE_FDS,
        transient_fds: TRANSIENT_FDS,
        worker_soft_nofile: WORKER_FDS,
        baseline_rss_bytes: BASELINE_RSS,
        modeled_payload_bytes: PAYLOAD,
        sampled_rss_refusal_bytes: WORKER_RSS,
        report_bytes: REPORT,
        worker_threads: WORKER_THREADS,
        service_nofile: 256,
        service_memory_bytes: 64 * MIB,
        service_tasks: 16,
    }
}

/// One pending worker per manager. Retain its guard until the supervisor reaps
/// all owned processes. Drop alone does not terminate a child.
#[derive(Default)]
pub struct SingleFlight {
    occupied: Arc<AtomicBool>,
}
pub struct FlightGuard {
    occupied: Arc<AtomicBool>,
}
impl SingleFlight {
    pub fn try_acquire(&self) -> Result<FlightGuard> {
        self.occupied
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| Refusal::Busy)?;
        Ok(FlightGuard {
            occupied: self.occupied.clone(),
        })
    }
}
impl Drop for FlightGuard {
    fn drop(&mut self) {
        self.occupied.store(false, Ordering::Release);
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
struct Sample {
    open_fds: usize,
    rss_bytes: usize,
    threads: usize,
    soft_nofile: Option<u64>,
}
impl Sample {
    fn read() -> Result<Self> {
        let (rss_bytes, threads) = status(&read_status()?)?;
        // Counts the observer directory too; conservative by one descriptor.
        let mut open_fds = 0;
        for entry in std::fs::read_dir("/proc/self/fd").map_err(|_| Refusal::Unavailable)? {
            let entry = entry.map_err(|_| Refusal::Unavailable)?;
            if !entry
                .file_name()
                .as_encoded_bytes()
                .iter()
                .all(u8::is_ascii_digit)
            {
                return Err(Refusal::Unavailable);
            }
            open_fds += 1;
            if open_fds > 256 {
                return Err(Refusal::ResourceLimit);
            }
        }
        Ok(Self {
            open_fds,
            rss_bytes,
            threads,
            soft_nofile: getrlimit(Resource::Nofile).current,
        })
    }
    fn common(self) -> Result<()> {
        if self.soft_nofile != Some(WORKER_FDS as u64)
            || self.threads == 0
            || self.threads > WORKER_THREADS
        {
            Err(Refusal::ResourceLimit)
        } else {
            Ok(())
        }
    }
}
fn read_status() -> Result<String> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(16 * 1024 + 1)
        .map_err(|_| Refusal::Unavailable)?;
    bytes.resize(16 * 1024 + 1, 0);
    let mut file = File::open("/proc/self/status").map_err(|_| Refusal::Unavailable)?;
    let mut length = 0;
    while length < bytes.len() {
        match file.read(&mut bytes[length..]) {
            Ok(0) => break,
            Ok(n) => length += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(Refusal::Unavailable),
        }
    }
    if length > 16 * 1024 {
        return Err(Refusal::Unavailable);
    }
    bytes.truncate(length);
    String::from_utf8(bytes).map_err(|_| Refusal::Unavailable)
}
fn status(text: &str) -> Result<(usize, usize)> {
    if text.len() > 16 * 1024 || !text.ends_with('\n') {
        return Err(Refusal::Unavailable);
    }
    let (mut rss, mut threads) = (None, None);
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("VmRSS:") {
            let mut fields = value.split_whitespace();
            let bytes = fields
                .next()
                .ok_or(Refusal::Unavailable)?
                .parse::<usize>()
                .map_err(|_| Refusal::Unavailable)?
                .checked_mul(1024)
                .ok_or(Refusal::Unavailable)?;
            if fields.next() != Some("kB")
                || fields.next().is_some()
                || rss.replace(bytes).is_some()
            {
                return Err(Refusal::Unavailable);
            }
        } else if let Some(value) = line.strip_prefix("Threads:") {
            if threads
                .replace(
                    value
                        .trim()
                        .parse::<usize>()
                        .map_err(|_| Refusal::Unavailable)?,
                )
                .is_some()
            {
                return Err(Refusal::Unavailable);
            }
        }
    }
    Ok((
        rss.ok_or(Refusal::Unavailable)?,
        threads.ok_or(Refusal::Unavailable)?,
    ))
}

/// A fresh worker must have the launcher's closed soft limit before admission.
/// This library reads limits; only isolated test children lower their own limit.
/// No lifetime/currentness or successful owner collection is proved by a phase.
pub struct BudgetSession {
    baseline: Sample,
    next: usize,
    retained_fds: usize,
    payload: usize,
    poisoned: bool,
}
impl BudgetSession {
    pub fn admit_worker() -> Result<Self> {
        Self::admit(Sample::read()?)
    }
    fn admit(baseline: Sample) -> Result<Self> {
        baseline.common()?;
        if baseline.open_fds > BASELINE_FDS || baseline.rss_bytes > BASELINE_RSS {
            return Err(Refusal::ResourceLimit);
        }
        Ok(Self {
            baseline,
            next: 0,
            retained_fds: 0,
            payload: REPORT,
            poisoned: false,
        })
    }
    pub fn configuration_profile(&self) -> ConfigurationReadProfile {
        ConfigurationReadProfile::CombinedV1
    }
    pub fn engine_profile(&self) -> EngineReadProfile {
        EngineReadProfile::CombinedV1
    }
    pub fn process_profile(&self) -> ProcessReadProfile {
        ProcessReadProfile::CombinedV1
    }
    pub fn storage_profile(&self) -> StorageReadProfile {
        StorageReadProfile::CombinedV1
    }
    pub fn destination_limits(&self) -> DestinationLimits {
        DestinationLimits {
            max_destinations: 32,
            max_retained_fds: 32,
            max_report_bytes: 32 * 1024,
        }
    }
    pub fn enter(&mut self, phase: Phase) -> Result<PhaseGuard<'_>> {
        self.enter_with(phase, Sample::read)
    }
    fn enter_with(
        &mut self,
        phase: Phase,
        sample: impl FnOnce() -> Result<Sample>,
    ) -> Result<PhaseGuard<'_>> {
        if self.poisoned {
            return Err(Refusal::Poisoned);
        }
        let Some(owner) = OWNERS.get(self.next).filter(|o| o.phase == phase) else {
            self.poisoned = true;
            return Err(Refusal::InvalidPhase);
        };
        let retained = self.retained_fds + owner.retained_fds;
        let payload = self.payload + owner.modeled_payload_bytes;
        if retained + self.baseline.open_fds + TRANSIENT_FDS > WORKER_FDS || payload > PAYLOAD {
            self.poisoned = true;
            return Err(Refusal::ResourceLimit);
        }
        if let Err(e) = sample().and_then(|s| self.check(s)) {
            self.poisoned = true;
            return Err(e);
        }
        self.retained_fds = retained;
        self.payload = payload;
        Ok(PhaseGuard {
            session: self,
            committed: false,
        })
    }
    fn check(&self, sample: Sample) -> Result<()> {
        sample.common()?;
        if sample.open_fds > self.baseline.open_fds + self.retained_fds + TRANSIENT_FDS
            || sample.rss_bytes > WORKER_RSS
        {
            Err(Refusal::ResourceLimit)
        } else {
            Ok(())
        }
    }
    pub fn report(self, value: &impl Serialize) -> Result<ReadReportBytes> {
        self.report_with(value, Sample::read)
    }
    fn report_with(
        mut self,
        value: &impl Serialize,
        sample: impl FnOnce() -> Result<Sample>,
    ) -> Result<ReadReportBytes> {
        if self.poisoned {
            return Err(Refusal::Poisoned);
        }
        if self.next != OWNERS.len() {
            self.poisoned = true;
            return Err(Refusal::InvalidPhase);
        }
        let result = (|| {
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(REPORT)
                .map_err(|_| Refusal::Unavailable)?;
            if bytes.capacity() != REPORT {
                return Err(Refusal::ResourceLimit);
            }
            let mut writer = ReportWriter { bytes };
            serde_json::to_writer(&mut writer, value).map_err(|_| Refusal::ResourceLimit)?;
            self.check(sample()?)?;
            Ok(ReadReportBytes(writer.bytes))
        })();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
}

/// Borrowing a phase prevents overlapping phases or dropping its session.
/// ```compile_fail
/// use limeos_executor::read_budget::{BudgetSession,Phase};
/// let mut session=BudgetSession::admit_worker().unwrap();
/// let phase=session.enter(Phase::Configuration).unwrap();
/// drop(session);
/// phase.commit().unwrap();
/// ```
pub struct PhaseGuard<'a> {
    session: &'a mut BudgetSession,
    committed: bool,
}
impl PhaseGuard<'_> {
    pub fn commit(self) -> Result<()> {
        self.commit_with(Sample::read)
    }
    fn commit_with(mut self, sample: impl FnOnce() -> Result<Sample>) -> Result<()> {
        match sample().and_then(|s| self.session.check(s)) {
            Ok(()) => {
                self.session.next += 1;
                self.committed = true;
                Ok(())
            }
            Err(e) => {
                self.session.poisoned = true;
                Err(e)
            }
        }
    }
}
impl Drop for PhaseGuard<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.session.poisoned = true;
        }
    }
}

/// Capacity-checked bytes only. They cannot recreate a session or evidence.
/// Report serialization consumes the session, so it cannot reserve another body.
/// ```compile_fail
/// use limeos_executor::read_budget::BudgetSession;
/// let session=BudgetSession::admit_worker().unwrap();
/// let first=session.report(&0);
/// let second=session.report(&0);
/// ```
pub struct ReadReportBytes(Vec<u8>);
impl ReadReportBytes {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}
struct ReportWriter {
    bytes: Vec<u8>,
}
impl Write for ReportWriter {
    fn write(&mut self, value: &[u8]) -> io::Result<usize> {
        if value.len() > REPORT - self.bytes.len() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        self.bytes.extend_from_slice(value);
        Ok(value.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
