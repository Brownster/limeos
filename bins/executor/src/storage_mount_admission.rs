//! Closed A1 admission. Reports and plan digests never construct this owner.
use crate::read_budget::{FlightGuard, SingleFlight};
use limeos_domain::{
    Error, ErrorCode, Result, StorageFilesystem, StorageMountCatalog, StorageMountEffectPlan,
    StorageMountInspection, StorageMountSelection, StorageMountedSnapshot, StorageRole,
};
use limeos_executor_container::docker::{
    Docker,
    process_evidence::{EnginePeerPolicy, EngineReadProfile, EngineStorageProcessEvidence},
};
use limeos_executor_storage::{
    StorageReadProfile,
    configuration::{
        ConfigurationPaths, ConfigurationReadProfile, inspect_storage_configuration_with_profile,
    },
    destinations::{DestinationLimits, inspect_container_destinations},
    inspect_container_dependencies_with_profile,
    mount_admission::{self, MountTarget},
    processes::{
        ProcessReadProfile, RunningContainerProcessBinding, RunningContainerProcessEvidence,
        inspect_running_container_processes_with_profile,
    },
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    cell::Cell,
    io::{self, Read, Write},
    path::Path,
    sync::OnceLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const WINDOW: Duration = Duration::from_secs(4);
const MAX_RUNNING: usize = 4;
const MAX_DESTINATIONS: usize = 16;
const FDS: usize = 226;
const TRANSIENT: usize = 32;
// Initial A1 limits, not full-host parity. Reservations include every owner live
// during revalidation. No service or worker ceiling is increased.
// baseline16 + configuration33 + Engine1 + processes21 + physical75 +
// destinations16 + target8 + guardian14 + host-consumers9 + transient32 = 225.
const RESERVATIONS: [usize; 7] = [33, 1, 21, 75, 16, 17, 14];
static FLIGHT: OnceLock<SingleFlight> = OnceLock::new();

fn error() -> Error {
    Error(ErrorCode::Unavailable)
}
fn storage(failure: limeos_executor_storage::Failure) -> Error {
    use limeos_executor_storage::Failure;
    Error(match failure {
        Failure::InvalidPlan => ErrorCode::InvalidInput,
        Failure::StateNotDurable => ErrorCode::StateNotDurable,
        Failure::Unavailable | Failure::TimedOut | Failure::NotReady => ErrorCode::Unavailable,
        _ => ErrorCode::Conflict,
    })
}
pub(crate) fn now() -> Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .ok_or_else(error)
}
// Central role owns the installed protected file convention. No request or
// environment variable selects these files.
fn paths() -> ConfigurationPaths<'static> {
    ConfigurationPaths {
        contract: Path::new("/etc/limeos/storage-contract.json"),
        pools: Path::new("/etc/limeos/pools.json"),
        protection: Path::new("/etc/limeos/protection.json"),
        shares: Path::new("/etc/limeos/shares.json"),
    }
}

struct Window {
    start: Instant,
    wall: Cell<i64>,
    poisoned: Cell<bool>,
}
impl Window {
    fn new() -> Result<Self> {
        Ok(Self {
            start: Instant::now(),
            wall: Cell::new(now()?),
            poisoned: Cell::new(false),
        })
    }
    fn check(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        self.observe(now(), self.start.elapsed(), cancelled())
    }
    fn observe(&self, wall: Result<i64>, elapsed: Duration, cancelled: bool) -> Result<()> {
        let wall = match wall {
            Ok(wall) => wall,
            Err(error) => {
                self.poisoned.set(true);
                return Err(error);
            }
        };
        if self.poisoned.get() || cancelled || elapsed >= WINDOW || wall < self.wall.get() {
            self.poisoned.set(true);
            return Err(Error(ErrorCode::Expired));
        }
        self.wall.set(wall);
        Ok(())
    }
}

/// Closed retained authority; no serde, Clone, caller proof, or renewal method.
pub struct MountAdmission {
    target: MountTarget,
    engine: EngineStorageProcessEvidence,
    processes: RunningContainerProcessEvidence,
    inspection: StorageMountInspection,
    window: Window,
    capacity: Capacity,
    deadline_ticks: u64,
    _flight: FlightGuard,
}
impl MountAdmission {
    pub fn inspection(&self) -> &StorageMountInspection {
        &self.inspection
    }
    pub(crate) fn target(&self) -> &MountTarget {
        &self.target
    }
    pub async fn revalidate(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        let result = self.check(cancelled).await;
        if result.is_err() {
            self.window.poisoned.set(true);
        }
        result
    }
    async fn check(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        self.window.check(cancelled)?;
        self.capacity.check()?;
        self.engine.revalidate().await?;
        self.processes.revalidate().map_err(storage)?;
        self.target.revalidate(cancelled).await.map_err(storage)?;
        self.window.check(cancelled)?;
        let digest = consumers(&self.target, &self.engine, &self.processes, cancelled).await?;
        if digest != self.inspection.consumer_digest {
            return Err(Error(ErrorCode::Conflict));
        }
        self.engine.revalidate().await?;
        self.processes.revalidate().map_err(storage)?;
        self.target.revalidate(cancelled).await.map_err(storage)?;
        self.capacity.check()?;
        self.window.check(cancelled)
    }
    pub(crate) fn guardian_capacity(&self) -> Result<()> {
        self.capacity.check()
    }
    pub(crate) fn remaining_window(&self) -> Result<Duration> {
        self.window.check(&|| false)?;
        WINDOW
            .checked_sub(self.window.start.elapsed())
            .ok_or(Error(ErrorCode::Expired))
    }
    pub(crate) fn deadline_ticks(&self) -> u64 {
        self.deadline_ticks
    }
}

pub async fn catalog(cancelled: &dyn Fn() -> bool) -> Result<StorageMountCatalog> {
    let window = Window::new()?;
    window.check(cancelled)?;
    let configuration = inspect_storage_configuration_with_profile(
        paths(),
        ConfigurationReadProfile::CombinedV1,
        cancelled,
    )
    .map_err(storage)?;
    let mut targets = Vec::new();
    targets
        .try_reserve_exact(configuration.configuration().contract.devices.len())
        .map_err(|_| error())?;
    for device in &configuration.configuration().contract.devices {
        if device.role == StorageRole::Data && device.filesystem == StorageFilesystem::Ext4 {
            targets.push(device.clone());
        }
    }
    let catalog = StorageMountCatalog { targets };
    catalog.validate()?;
    configuration.revalidate(cancelled).map_err(storage)?;
    window.check(cancelled)?;
    Ok(catalog)
}

pub async fn inspect(
    selection: &StorageMountSelection,
    cancelled: &dyn Fn() -> bool,
) -> Result<MountAdmission> {
    selection.validate()?;
    let flight = FLIGHT
        .get_or_init(SingleFlight::default)
        .try_acquire()
        .map_err(|_| Error(ErrorCode::Conflict))?;
    let deadline_ticks = uptime_ticks()?.checked_add(399).ok_or_else(error)?;
    let window = Window::new()?;
    let mut capacity = Capacity::admit()?;
    let poll = || window.check(cancelled).is_err();
    window.check(cancelled)?;
    capacity.enter(0)?;
    let configuration = inspect_storage_configuration_with_profile(
        paths(),
        ConfigurationReadProfile::CombinedV1,
        &poll,
    )
    .map_err(storage)?;
    window.check(cancelled)?;
    capacity.enter(1)?;
    let engine = Docker::new("/run/docker.sock".into())
        .inspect_storage_processes_with_profile(
            EnginePeerPolicy::root(),
            EngineReadProfile::CombinedV1,
        )
        .await?;
    window.check(cancelled)?;
    if engine.running().len() > MAX_RUNNING
        || engine
            .declarations()
            .containers
            .iter()
            .map(|c| c.mounts.len())
            .sum::<usize>()
            > MAX_DESTINATIONS
    {
        return Err(Error(ErrorCode::StorageMountLimit));
    }
    capacity.enter(2)?;
    let mut bindings = Vec::new();
    bindings
        .try_reserve_exact(engine.running().len())
        .map_err(|_| error())?;
    for process in engine.running() {
        bindings.push(RunningContainerProcessBinding {
            container: process.container().clone(),
            pid: process.pid(),
        });
    }
    let processes = inspect_running_container_processes_with_profile(
        engine.declarations(),
        &bindings,
        ProcessReadProfile::CombinedV1,
    )
    .map_err(storage)?;
    window.check(cancelled)?;
    capacity.enter(3)?;
    let physical = inspect_container_dependencies_with_profile(
        &configuration.configuration().contract,
        engine.declarations(),
        StorageReadProfile::CombinedV1,
    )
    .await
    .map_err(storage)?;
    window.check(cancelled)?;
    // Reserve transient destination ownership before any pinning, then all
    // retained target descriptors and the later gated guardian before opening.
    capacity.enter(4)?;
    capacity.enter(5)?;
    capacity.enter(6)?;
    let target = mount_admission::inspect(configuration, physical, &selection.target_id, &poll)
        .await
        .map_err(storage)?;
    if target.selected().role != StorageRole::Data {
        return Err(Error(ErrorCode::Forbidden));
    }
    window.check(cancelled)?;
    let consumer_digest = consumers(&target, &engine, &processes, &poll).await?;
    let inspection = StorageMountInspection {
        target_id: selection.target_id.clone(),
        device: target.selected().clone(),
        expected: target.inventory().clone(),
        target: target.target().clone(),
        physical_devices: target.physical_devices().to_vec(),
        consumer_digest,
        observed_at: now()?,
    };
    inspection.validate(now()?)?;
    let admission = MountAdmission {
        target,
        engine,
        processes,
        inspection,
        window,
        capacity,
        deadline_ticks,
        _flight: flight,
    };
    admission.revalidate(cancelled).await?;
    Ok(admission)
}

fn overlaps(a: &str, b: &str) -> bool {
    fn beneath(a: &str, b: &str) -> bool {
        b == "/" || a == b || a.strip_prefix(b).is_some_and(|tail| tail.starts_with('/'))
    }
    beneath(a, b) || beneath(b, a)
}
async fn consumers(
    target: &MountTarget,
    engine: &EngineStorageProcessEvidence,
    processes: &RunningContainerProcessEvidence,
    cancelled: &dyn Fn() -> bool,
) -> Result<String> {
    let config = target.configuration();
    let graph = limeos_domain::storage_dependency_graph::configured_storage_dependency_graph(
        &config.contract,
        &config.pools,
        &config.protection,
        engine.declarations(),
        &config.shares,
        now()?,
    )
    .map_err(|_| Error(ErrorCode::Conflict))?;
    let row = graph
        .devices
        .iter()
        .find(|d| d.storage.expected == *target.selected())
        .ok_or_else(error)?;
    if !graph.unmapped_shares.is_empty()
        || !row.storage.pools.is_empty()
        || !row.storage.protection.is_empty()
        || !row.storage.containers.is_empty()
        || !row.shares.is_empty()
    {
        return Err(Error(ErrorCode::Conflict));
    }
    let physical = target.physical_evidence().snapshot();
    let row = physical
        .dependencies
        .iter()
        .find(|d| d.device_id == target.selected().id)
        .ok_or_else(error)?;
    if !row.consumers.is_empty() {
        return Err(Error(ErrorCode::Conflict));
    }
    // Stopped declarations also have genuine resolved host source handles.
    // Refuse aliases, ancestors and descendants of the prepared underlay.
    if target.source_snapshot().sources.iter().any(|s| {
        overlaps(&s.resolved_path, &target.selected().mountpoint)
            || target.target().existing.as_ref().is_some_and(|d| {
                d.inode == s.identity.inode
                    && d.major == s.identity.device.major
                    && d.minor == s.identity.device.minor
            })
    }) {
        return Err(Error(ErrorCode::Conflict));
    }
    for process in &processes.snapshot().processes {
        let mounts = processes
            .mounts(&process.container.resource)
            .ok_or_else(error)?;
        if mounts.iter().any(|m| {
            target
                .physical_devices()
                .iter()
                .any(|d| (d.major, d.minor) == (m.device.major, m.device.minor))
        }) {
            return Err(Error(ErrorCode::Conflict));
        }
    }
    let destinations = inspect_container_destinations(
        target.physical_evidence(),
        processes,
        DestinationLimits {
            max_destinations: MAX_DESTINATIONS as u32,
            max_retained_fds: MAX_DESTINATIONS as u32,
            max_report_bytes: 32 * 1024,
        },
        cancelled,
    )
    .await
    .map_err(storage)?;
    destinations.revalidate(cancelled).await.map_err(storage)?;
    let config_snapshot = target.configuration_snapshot();
    let process_snapshot = processes.snapshot();
    let sources = target.source_snapshot();
    // Original acquisition times/digests of timed declarations stay in the
    // genuine owners, not the stable plan identity. Actual facts remain bound.
    hash(&(
        (
            &config_snapshot.host_root,
            &config_snapshot.files,
            &config_snapshot.configuration_digest,
        ),
        (
            &engine.declarations().engine_id,
            &engine.declarations().containers,
            engine.running(),
        ),
        (
            &process_snapshot.host_boot_id,
            &process_snapshot.host_root_mount_id,
            &process_snapshot.host_mount_table_digest,
            &process_snapshot.processes,
        ),
        (
            &physical.dependencies,
            &physical.host_root_mount_id,
            &physical.mounts_digest,
        ),
        (&sources.sources, &sources.mounts),
        &destinations.snapshot().destinations,
        (&graph.devices, &graph.unmapped_shares),
        target.host_consumer_digest(),
    ))
}

fn hash(value: &impl Serialize) -> Result<String> {
    struct Writer {
        digest: Sha256,
        used: usize,
    }
    impl Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > 256 * 1024 - self.used {
                return Err(io::ErrorKind::OutOfMemory.into());
            }
            self.digest.update(bytes);
            self.used += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Writer {
        digest: Sha256::new(),
        used: 0,
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| error())?;
    Ok(writer
        .digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

struct Capacity {
    baseline: usize,
    retained: usize,
    next: usize,
}
impl Capacity {
    fn admit() -> Result<Self> {
        let baseline = sample()?;
        if baseline > 16 || 16 + RESERVATIONS.iter().sum::<usize>() + TRANSIENT > FDS {
            return Err(error());
        }
        Ok(Self {
            baseline,
            retained: 0,
            next: 0,
        })
    }
    fn enter(&mut self, phase: usize) -> Result<()> {
        if self.next != phase {
            return Err(error());
        }
        self.retained += RESERVATIONS[phase];
        self.next += 1;
        if self.baseline + self.retained + TRANSIENT > FDS {
            return Err(error());
        }
        self.check()
    }
    fn check(&self) -> Result<()> {
        if sample()? > self.baseline + self.retained + TRANSIENT {
            Err(error())
        } else {
            Ok(())
        }
    }
}
fn sample() -> Result<usize> {
    let soft = rustix::process::getrlimit(rustix::process::Resource::Nofile)
        .current
        .ok_or_else(error)?;
    if !(226..=256).contains(&soft) {
        return Err(error());
    }
    let mut file = std::fs::File::open("/proc/self/status").map_err(|_| error())?;
    let mut status = String::new();
    Read::by_ref(&mut file)
        .take(16385)
        .read_to_string(&mut status)
        .map_err(|_| error())?;
    if status.len() > 16384 || !status.ends_with('\n') {
        return Err(error());
    }
    let field = |name: &str| -> Result<usize> {
        let mut rows = status.lines().filter_map(|s| s.strip_prefix(name));
        let value = rows
            .next()
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse().ok())
            .ok_or_else(error)?;
        if rows.next().is_some() {
            return Err(error());
        }
        Ok(value)
    };
    if field("VmRSS:")? > 48 * 1024 || !(1..=4).contains(&field("Threads:")?) {
        return Err(error());
    }
    let mut count = 0;
    for entry in std::fs::read_dir("/proc/self/fd").map_err(|_| error())? {
        let entry = entry.map_err(|_| error())?;
        if !entry
            .file_name()
            .as_encoded_bytes()
            .iter()
            .all(u8::is_ascii_digit)
        {
            return Err(error());
        }
        count += 1;
        if count > FDS - TRANSIENT {
            return Err(error());
        }
    }
    Ok(count)
}
/// Proc uptime is quantized down to centiseconds. The gate deadline subtracts
/// a tick and checks another tick conservatively; suspend only shortens admission.
pub(crate) fn uptime_ticks() -> Result<u64> {
    let mut file = std::fs::File::open("/proc/uptime").map_err(|_| error())?;
    let mut text = String::new();
    Read::by_ref(&mut file)
        .take(129)
        .read_to_string(&mut text)
        .map_err(|_| error())?;
    if text.len() > 128 {
        return Err(error());
    }
    let (seconds, fraction) = text
        .split_ascii_whitespace()
        .next()
        .and_then(|s| s.split_once('.'))
        .ok_or_else(error)?;
    if fraction.len() != 2 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return Err(error());
    }
    seconds
        .parse::<u64>()
        .ok()
        .and_then(|s| s.checked_mul(100))
        .and_then(|s| fraction.parse::<u64>().ok().and_then(|f| s.checked_add(f)))
        .ok_or_else(error)
}

/// Independent post-effect collection cannot refresh execution admission.
pub async fn inspect_mounted(
    plan: &StorageMountEffectPlan,
    cancelled: &dyn Fn() -> bool,
) -> Result<StorageMountedSnapshot> {
    plan.validate(plan.created_at)?;
    let configuration = inspect_storage_configuration_with_profile(
        paths(),
        ConfigurationReadProfile::CombinedV1,
        cancelled,
    )
    .map_err(storage)?;
    mount_admission::inspect_mounted(configuration, plan, cancelled)
        .await
        .map_err(storage)
}
pub async fn inspect_unmounted(
    plan: &StorageMountEffectPlan,
    cancelled: &dyn Fn() -> bool,
) -> Result<StorageMountInspection> {
    plan.validate(plan.created_at)?;
    let admission = inspect(
        &StorageMountSelection {
            target_id: plan.device.id.clone(),
        },
        cancelled,
    )
    .await?;
    plan.check_reconciliation(admission.inspection(), now()?)?;
    Ok(admission.inspection().clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn window() -> Window {
        Window {
            start: Instant::now(),
            wall: Cell::new(100),
            poisoned: Cell::new(false),
        }
    }
    #[test]
    fn cancellation_cannot_be_cleared_to_reuse_original_admission_window() {
        let window = window();
        assert!(window.observe(Ok(100), Duration::ZERO, true).is_err());
        assert!(window.observe(Ok(100), Duration::ZERO, false).is_err());
    }
    #[test]
    fn expiry_rollback_and_clock_failure_permanently_poison_the_window() {
        for failure in 0..3 {
            let window = window();
            let result = match failure {
                0 => window.observe(Ok(100), WINDOW, false),
                1 => window.observe(Ok(99), Duration::ZERO, false),
                _ => window.observe(Err(error()), Duration::ZERO, false),
            };
            assert!(result.is_err());
            assert!(window.observe(Ok(101), Duration::ZERO, false).is_err());
        }
    }
}
