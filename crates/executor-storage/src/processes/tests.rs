use super::*;
use limeos_domain::ContainerStorageConsumer;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    process::{Child, Command},
    rc::Rc,
};

const TABLE: &str =
    include_str!("../../../../tests/fixtures/container-process-evidence/mountinfo.txt");
const STATUS: &str =
    include_str!("../../../../tests/fixtures/container-process-evidence/status.txt");

fn declaration(n: usize, observed_at: i64) -> ContainerStorageInventory {
    ContainerStorageInventory {
        version: 1,
        engine_id: "engine:local-fixture".into(),
        observed_at,
        containers: (0..n)
            .map(|i| ContainerStorageConsumer {
                container: ContainerSnapshot {
                    resource: format!("container:{i:064x}"),
                    image: format!("sha256:{:064x}", i + 1),
                    running: true,
                    started_at: "2026-10-07T12:00:00Z".into(),
                },
                mounts: vec![],
            })
            .collect(),
    }
}
fn bindings(d: &ContainerStorageInventory) -> Vec<RunningContainerProcessBinding> {
    d.containers
        .iter()
        .enumerate()
        .filter(|(_, c)| c.container.running)
        .map(|(i, c)| RunningContainerProcessBinding {
            container: c.container.clone(),
            pid: 200 + i as u32,
        })
        .collect()
}
fn root(inode: u64) -> ContainerProcessRoot {
    ContainerProcessRoot {
        device: DeviceNumber { major: 8, minor: 1 },
        inode,
        mount_id: 30,
        mode: 0o40755,
    }
}
fn ns(inode: u64) -> ContainerProcessNamespace {
    ContainerProcessNamespace {
        device: DeviceNumber { major: 0, minor: 4 },
        inode,
    }
}
#[derive(Clone)]
struct Process {
    generation: u64,
    live: bool,
    facts: ProcessFacts,
    root: ContainerProcessRoot,
    namespace: ContainerProcessNamespace,
    table: String,
}
impl Default for Process {
    fn default() -> Self {
        Self {
            generation: 1,
            live: true,
            facts: ProcessFacts {
                start: 123,
                uids: [1000; 4],
                gids: [1000; 4],
            },
            root: root(5),
            namespace: ns(50),
            table: TABLE.into(),
        }
    }
}
#[derive(Default)]
struct Counts {
    active: Cell<usize>,
    peak: Cell<usize>,
    limit: Cell<Option<usize>>,
}
struct Handle {
    counts: Rc<Counts>,
    pid: u32,
    generation: u64,
    root: ContainerProcessRoot,
    namespace: ContainerProcessNamespace,
}
impl Drop for Handle {
    fn drop(&mut self) {
        self.counts.active.set(self.counts.active.get() - 1);
    }
}
enum Change {
    Exit,
    Reuse,
    Start,
    Uid,
    Gid,
    Root,
    Namespace,
    Table(String),
    Wall(Duration),
    Mono(Duration),
    Host,
}
struct Hook {
    op: &'static str,
    nth: usize,
    change: Change,
}
struct State {
    processes: BTreeMap<u32, Process>,
    wall: Duration,
    mono: Duration,
    calls: BTreeMap<&'static str, usize>,
    hooks: Vec<Hook>,
    fault: Option<&'static str>,
    host_changed: bool,
}
#[derive(Clone)]
struct Fake {
    state: Rc<RefCell<State>>,
    counts: Rc<Counts>,
    base: Instant,
}
impl Fake {
    fn new(n: usize) -> Self {
        Self {
            state: Rc::new(RefCell::new(State {
                processes: (0..n)
                    .map(|i| (200 + i as u32, Process::default()))
                    .collect(),
                wall: Duration::from_secs(100),
                mono: Duration::ZERO,
                calls: BTreeMap::new(),
                hooks: vec![],
                fault: None,
                host_changed: false,
            })),
            counts: Rc::new(Counts::default()),
            base: Instant::now(),
        }
    }
    fn hook(&self, op: &'static str, nth: usize, change: Change) {
        self.state.borrow_mut().hooks.push(Hook { op, nth, change });
    }
    fn step(&self, op: &'static str, budget: &Budget<'_, Self>) -> Result<()> {
        budget.check()?;
        let mut s = self.state.borrow_mut();
        let call = s.calls.entry(op).or_default();
        *call += 1;
        let nth = *call;
        if let Some(i) = s.hooks.iter().position(|h| h.op == op && h.nth == nth) {
            let h = s.hooks.remove(i);
            match h.change {
                Change::Wall(w) => s.wall = w,
                Change::Mono(m) => s.mono = m,
                Change::Host => s.host_changed = true,
                change => {
                    let p = s.processes.get_mut(&200).unwrap();
                    match change {
                        Change::Exit => p.live = false,
                        Change::Reuse => {
                            p.generation += 1;
                            p.facts.start += 1;
                        }
                        Change::Start => p.facts.start += 1,
                        Change::Uid => p.facts.uids[3] += 1,
                        Change::Gid => p.facts.gids[0] += 1,
                        Change::Root => p.root.inode += 1,
                        Change::Namespace => p.namespace.inode += 1,
                        Change::Table(t) => p.table = t,
                        _ => unreachable!(),
                    }
                }
            }
        }
        if s.fault == Some(op) {
            return Err(Failure::Unavailable);
        }
        drop(s);
        budget.check()
    }
    fn handle(&self, pid: u32) -> Result<Handle> {
        let n = self.counts.active.get();
        if self.counts.limit.get().is_some_and(|limit| n >= limit) {
            return Err(Failure::Unavailable);
        }
        let p = self
            .state
            .borrow()
            .processes
            .get(&pid)
            .cloned()
            .unwrap_or_default();
        self.counts.active.set(n + 1);
        self.counts.peak.set(self.counts.peak.get().max(n + 1));
        Ok(Handle {
            counts: self.counts.clone(),
            pid,
            generation: p.generation,
            root: p.root,
            namespace: p.namespace,
        })
    }
    fn process(&self, h: &Handle) -> Result<Process> {
        let p = self
            .state
            .borrow()
            .processes
            .get(&h.pid)
            .cloned()
            .ok_or(Failure::Unavailable)?;
        if !p.live || h.generation != p.generation {
            Err(Failure::Conflict)
        } else {
            Ok(p)
        }
    }
    fn inspect(
        &self,
        d: &ContainerStorageInventory,
        b: &[RunningContainerProcessBinding],
    ) -> Result<Evidence<Self>> {
        let lease = Lease::new(d, self.clock()?)?;
        let sorted = validate_bindings(d, b)?;
        collect(self.clone(), d, &sorted, lease)
    }
}
impl Kernel for Fake {
    type Handle = Handle;
    fn clock(&self) -> Result<Time> {
        let s = self.state.borrow();
        Ok(Time {
            wall: s.wall,
            monotonic: self.base + s.mono,
        })
    }
    fn context(&self, b: &Budget<'_, Self>) -> Result<HostHandles<Handle>> {
        self.step("context", b)?;
        Ok(HostHandles {
            root: self.handle(0)?,
            proc_root: self.handle(0)?,
            own_proc: self.handle(0)?,
            fdinfo: self.handle(0)?,
            init_proc: self.handle(0)?,
        })
    }
    fn host(&self, _: &HostHandles<Handle>, b: &Budget<'_, Self>) -> Result<HostFacts> {
        self.step("host", b)?;
        Ok(HostFacts {
            boot: "11111111-1111-1111-1111-111111111111".into(),
            root_mount: 30,
            table_digest: limeos_identity::digest(TABLE),
            init_table_digest: limeos_identity::digest(TABLE),
            namespace: ns(50),
            root: root(if self.state.borrow().host_changed {
                9
            } else {
                1
            }),
            proc_root: root(2),
        })
    }
    fn pidfd(&self, pid: u32, b: &Budget<'_, Self>) -> Result<Handle> {
        self.step("pidfd", b)?;
        self.handle(pid)
    }
    fn live(
        &self,
        _: &HostHandles<Handle>,
        h: &Handle,
        pid: u32,
        b: &Budget<'_, Self>,
    ) -> Result<()> {
        self.step("live", b)?;
        if h.pid != pid {
            return Err(Failure::Conflict);
        }
        self.process(h).map(|_| ())
    }
    fn proc(&self, _: &HostHandles<Handle>, pid: u32, b: &Budget<'_, Self>) -> Result<Handle> {
        self.step("proc", b)?;
        self.handle(pid)
    }
    fn facts(&self, h: &Handle, _: u32, b: &Budget<'_, Self>) -> Result<ProcessFacts> {
        self.step("facts", b)?;
        Ok(self.process(h)?.facts)
    }
    fn namespace(&self, h: &Handle, b: &Budget<'_, Self>) -> Result<Handle> {
        self.step("namespace", b)?;
        self.process(h)?;
        self.handle(h.pid)
    }
    fn root(&self, h: &Handle, b: &Budget<'_, Self>) -> Result<Handle> {
        self.step("root", b)?;
        self.process(h)?;
        self.handle(h.pid)
    }
    fn directory_identity(&self, h: &Handle, b: &Budget<'_, Self>) -> Result<ContainerProcessRoot> {
        self.step("directory_identity", b)?;
        if h.pid != 0 {
            self.process(h)?;
        }
        Ok(h.root.clone())
    }
    fn namespace_identity(
        &self,
        h: &Handle,
        b: &Budget<'_, Self>,
    ) -> Result<ContainerProcessNamespace> {
        self.step("namespace_identity", b)?;
        Ok(h.namespace.clone())
    }
    fn table(&self, h: &Handle, b: &Budget<'_, Self>, limit: usize) -> Result<String> {
        self.step("table", b)?;
        let p = self.process(h)?;
        if p.table.len() > limit {
            return Err(Failure::Unavailable);
        }
        Ok(p.table)
    }
}

#[test]
fn complete_sorted_membership_includes_no_mount_and_excludes_stopped() {
    let f = Fake::new(3);
    let mut d = declaration(3, 100);
    d.containers[2].container.running = false;
    let mut b = bindings(&d);
    b.reverse();
    let e = f.inspect(&d, &b).unwrap();
    assert_eq!(e.snapshot.processes.len(), 2);
    assert_eq!(e.snapshot.processes[0].pid, 200);
    let mounts = e.mounts(&b[1].container.resource).unwrap();
    assert_eq!(mounts[1].mountpoint, "/data files");
    assert!(!mounts[1].writable);
    assert!(e.mounts(&d.containers[2].container.resource).is_none());
    assert_eq!(
        e.snapshot.bindings_digest,
        f.inspect(&d, &bindings(&d))
            .unwrap()
            .snapshot
            .bindings_digest
    );
    let report = serde_json::to_string(&e.snapshot).unwrap();
    assert!(!report.contains("private"));
    assert!(!report.contains("pidfd"));
    assert!(!report.contains("authority"));
    assert_eq!(f.counts.active.get(), 13);
    drop(e);
    assert_eq!(f.counts.active.get(), 0);
}
#[test]
fn all_stopped_retains_host_observation_but_no_running_consumers() {
    let f = Fake::new(1);
    let mut d = declaration(1, 100);
    d.containers[0].container.running = false;
    let e = f.inspect(&d, &[]).unwrap();
    assert!(e.snapshot.processes.is_empty());
    assert_eq!(f.counts.active.get(), 5);
}
#[test]
fn rejects_all_membership_and_full_snapshot_disagreements_before_open() {
    let d = declaration(2, 100);
    let original = bindings(&d);
    let mut cases = vec![vec![], original[..1].to_vec()];
    let mut b = original.clone();
    b.push(original[0].clone());
    cases.push(b);
    let mut b = original.clone();
    b[1] = b[0].clone();
    cases.push(b);
    let mut b = original.clone();
    b[1].pid = b[0].pid;
    cases.push(b);
    for field in ["resource", "image", "started_at", "running"] {
        let mut b = original.clone();
        match field {
            "resource" => b[0].container.resource = format!("container:{:064x}", 999),
            "image" => b[0].container.image = format!("sha256:{:064x}", 999),
            "started_at" => b[0].container.started_at = "2026-10-07T12:00:01Z".into(),
            _ => b[0].container.running = false,
        }
        cases.push(b);
    }
    for pid in [0, 1, MAX_PID, u32::MAX] {
        let mut b = original.clone();
        b[0].pid = pid;
        cases.push(b);
    }
    for b in cases {
        let f = Fake::new(2);
        assert!(f.inspect(&d, &b).is_err());
        assert!(f.state.borrow().calls.is_empty());
        assert_eq!(f.counts.active.get(), 0);
    }
    let mut stopped = d;
    stopped.containers[0].container.running = false;
    assert!(Fake::new(2).inspect(&stopped, &original).is_err());
    let overflow = declaration(65, 100);
    assert!(
        Fake::new(65)
            .inspect(&overflow, &bindings(&overflow))
            .is_err()
    );
}
#[test]
fn original_declaration_digest_covers_engine_stopped_and_sources() {
    let f = Fake::new(1);
    let d = declaration(1, 100);
    let e = f.inspect(&d, &bindings(&d)).unwrap();
    let mut changed = d.clone();
    changed.engine_id = "different".into();
    assert_ne!(
        e.snapshot.declarations_digest,
        f.inspect(&changed, &bindings(&changed))
            .unwrap()
            .snapshot
            .declarations_digest
    );
    changed
        .containers
        .push(declaration(2, 100).containers[1].clone());
    changed.containers[1].container.running = false;
    assert_ne!(
        e.snapshot.declarations_digest,
        f.inspect(&changed, &bindings(&changed))
            .unwrap()
            .snapshot
            .declarations_digest
    );
}
#[test]
fn btrfs_root_stat_device_and_mount_table_device_remain_distinct_observations() {
    let f = Fake::new(1);
    let d = declaration(1, 100);
    {
        let mut state = f.state.borrow_mut();
        let p = state.processes.get_mut(&200).unwrap();
        p.table = include_str!(
            "../../../../tests/fixtures/container-process-evidence/btrfs-mountinfo.txt"
        )
        .into();
        p.root.device = DeviceNumber {
            major: 0,
            minor: 39,
        };
    }
    let e = f
        .inspect(&d, &bindings(&d))
        .expect("btrfs device aliases are observations, not physical proof");
    assert_eq!(e.snapshot.processes[0].root.device.minor, 39);
    assert_eq!(e.retained[0].mounts[0].device.minor, 35);
    assert_eq!(e.revalidate(), Ok(()));
}
#[test]
fn pid_reuse_between_pidfd_and_proc_open_and_exit_during_collection_refuse() {
    for change in [Change::Reuse, Change::Exit] {
        let f = Fake::new(1);
        f.hook("proc", 1, change);
        let d = declaration(1, 100);
        assert!(matches!(
            f.inspect(&d, &bindings(&d)),
            Err(Failure::Conflict)
        ));
        assert_eq!(f.counts.active.get(), 0);
    }
}
#[test]
fn credentials_start_root_and_namespace_changes_invalidate_and_latch() {
    for change in [
        Change::Start,
        Change::Uid,
        Change::Gid,
        Change::Root,
        Change::Namespace,
        Change::Exit,
        Change::Reuse,
        Change::Host,
    ] {
        let f = Fake::new(1);
        let d = declaration(1, 100);
        let e = f.inspect(&d, &bindings(&d)).unwrap();
        let next = f.state.borrow().calls["host"] + 1;
        f.hook("host", next, change);
        assert_eq!(e.revalidate(), Err(Failure::Conflict));
        *f.state.borrow_mut().processes.get_mut(&200).unwrap() = Process::default();
        f.state.borrow_mut().host_changed = false;
        assert_eq!(e.revalidate(), Err(Failure::Conflict));
        drop(e);
        assert_eq!(f.counts.active.get(), 0);
    }
}
#[test]
fn raw_table_binds_parent_propagation_options_superoptions_and_source() {
    let mutations = [
        TABLE.replace("30 20", "30 21"),
        TABLE.replace("shared:7", "shared:8"),
        TABLE.replace("relatime", "noatime"),
        TABLE.replace("/dev/sda1", "/dev/sdb1"),
        TABLE.replace("size=1024k", "size=2048k"),
    ];
    for table in mutations {
        let f = Fake::new(1);
        let d = declaration(1, 100);
        let e = f.inspect(&d, &bindings(&d)).unwrap();
        let next = f.state.borrow().calls["table"] + 2;
        f.hook("table", next, Change::Table(table));
        assert_eq!(e.revalidate(), Err(Failure::Conflict));
    }
}
#[test]
fn namespace_and_root_changes_between_initial_and_final_observation_refuse() {
    for change in [Change::Root, Change::Namespace, Change::Uid, Change::Gid] {
        let f = Fake::new(1);
        f.hook("table", 1, change);
        let d = declaration(1, 100);
        assert!(matches!(
            f.inspect(&d, &bindings(&d)),
            Err(Failure::Conflict)
        ));
        assert_eq!(f.counts.active.get(), 0);
    }
}
#[test]
fn missing_primitives_denials_and_descriptor_exhaustion_fail_without_partial_owner() {
    for op in [
        "context",
        "host",
        "pidfd",
        "live",
        "proc",
        "facts",
        "namespace",
        "root",
        "directory_identity",
        "namespace_identity",
        "table",
    ] {
        let f = Fake::new(2);
        f.state.borrow_mut().fault = Some(op);
        let d = declaration(2, 100);
        assert!(matches!(
            f.inspect(&d, &bindings(&d)),
            Err(Failure::Unavailable)
        ));
        assert_eq!(f.counts.active.get(), 0);
    }
    for limit in [0, 4, 5, 8, 12, 14] {
        let f = Fake::new(2);
        f.counts.limit.set(Some(limit));
        let d = declaration(2, 100);
        assert!(matches!(
            f.inspect(&d, &bindings(&d)),
            Err(Failure::Unavailable)
        ));
        assert_eq!(f.counts.active.get(), 0);
    }
}
#[test]
fn future_expired_and_mid_read_age_are_rejected() {
    for observed in [101, 95, 94] {
        let f = Fake::new(1);
        let d = declaration(1, observed);
        assert!(matches!(
            f.inspect(&d, &bindings(&d)),
            Err(Failure::Conflict)
        ));
    }
    let f = Fake::new(1);
    f.hook("table", 1, Change::Wall(Duration::from_secs(105)));
    let d = declaration(1, 100);
    assert!(matches!(
        f.inspect(&d, &bindings(&d)),
        Err(Failure::Conflict)
    ));
    assert_eq!(f.counts.active.get(), 0);
}
#[test]
fn wall_rollback_monotonic_expiry_and_repeated_revalidation_never_renew() {
    for change in [
        Change::Wall(Duration::from_secs(99)),
        Change::Mono(Duration::from_secs(5)),
    ] {
        let f = Fake::new(1);
        let d = declaration(1, 100);
        let e = f.inspect(&d, &bindings(&d)).unwrap();
        for _ in 0..3 {
            assert_eq!(e.revalidate(), Ok(()));
        }
        let next = f.state.borrow().calls["host"] + 1;
        f.hook("host", next, change);
        assert_eq!(e.revalidate(), Err(Failure::Conflict));
        assert_eq!(e.revalidate(), Err(Failure::Conflict));
    }
    let f = Fake::new(1);
    f.state.borrow_mut().wall = Duration::from_millis(104_900);
    let d = declaration(1, 100);
    let e = f.inspect(&d, &bindings(&d)).unwrap();
    f.state.borrow_mut().mono = Duration::from_millis(100);
    assert_eq!(e.revalidate(), Err(Failure::Conflict));
}
#[test]
fn cooperative_work_budget_refuses_and_original_lease_survives_no_failure() {
    let f = Fake::new(1);
    f.hook("table", 1, Change::Mono(WORK));
    let d = declaration(1, 100);
    assert!(matches!(
        f.inspect(&d, &bindings(&d)),
        Err(Failure::TimedOut)
    ));
    assert_eq!(f.counts.active.get(), 0);
}

fn stat(comm: &[u8], state: &str, start: &str) -> Vec<u8> {
    let mut bytes = b"200 (".to_vec();
    bytes.extend_from_slice(comm);
    bytes.extend_from_slice(format!(") {state}").as_bytes());
    for field in 4..=52 {
        bytes.extend_from_slice(b" ");
        bytes.extend_from_slice(if field == 22 { start.as_bytes() } else { b"0" });
    }
    bytes.push(b'\n');
    bytes
}
#[test]
fn stat_comm_is_opaque_even_with_spaces_parentheses_controls_and_invalid_utf8() {
    for comm in [b"private name".as_slice(), b"a ) ( x", b"x\n\t\x01\xff)"] {
        assert_eq!(parse_stat(&stat(comm, "S", "123"), 200), Ok(123));
    }
}
#[test]
fn stat_refuses_malformed_truncated_oversized_and_dead_records() {
    for bytes in [
        b"bad\n".to_vec(),
        stat(b"x", "?", "123"),
        stat(b"x", "S", "0"),
        stat(b"x", "S", "18446744073709551616"),
        stat(b"x", "S", "-1"),
        stat(b"x", "S", "123")[..50].to_vec(),
        vec![b'x'; RECORD_BYTES + 1],
    ] {
        assert_eq!(parse_stat(&bytes, 200), Err(Failure::Unavailable));
    }
    assert_eq!(
        parse_stat(&stat(b"x", "S", "123"), 201),
        Err(Failure::Unavailable)
    );
    for state in ["Z", "X", "x"] {
        assert_eq!(
            parse_stat(&stat(b"x", state, "123"), 200),
            Err(Failure::Conflict)
        );
    }
}
#[test]
fn status_parses_numeric_ids_and_refuses_duplicates_truncation_and_overflow() {
    assert_eq!(
        parse_status(STATUS, 200),
        Ok(([1000, 1001, 1002, 1003], [2000, 2001, 2002, 2003]))
    );
    for text in [
        STATUS.trim_end().into(),
        format!("{STATUS}Uid: 1 1 1 1\n"),
        STATUS.replace("1000", "4294967296"),
        STATUS.replace("Uid:", "Unknown:"),
        STATUS.replace("1003", "1003 1004"),
        "x".repeat(RECORD_BYTES + 1),
    ] {
        assert!(parse_status(&text, 200).is_err());
    }
    assert_eq!(parse_status(STATUS, 201), Err(Failure::Conflict));
}
#[test]
fn pidfd_fdinfo_requires_live_visible_unique_pid() {
    assert_eq!(pidfd_pid("pos: 0\nPid:\t200\nNSpid:\t200 5\n"), Ok(200));
    assert_eq!(pidfd_pid("Pid: -1\n"), Err(Failure::Conflict));
    for text in [
        "Pid: 0\n",
        "Pid: 1\n",
        "Pid: 4194304\n",
        "Pid: 200\nPid: 200\n",
        "Pid: nope\n",
        "NSpid: 200\n",
    ] {
        assert_eq!(pidfd_pid(text), Err(Failure::Unavailable));
    }
}
#[test]
fn mount_records_reject_ambiguous_malformed_oversized_and_whitespace_amplification() {
    assert_eq!(parse_mounts(TABLE, &|| Ok(())).unwrap().len(), 2);
    for text in [
        TABLE.trim_end().into(),
        TABLE.replace("30 20", "0 20"),
        TABLE.replace("31 30", "30 30"),
        TABLE.replace("8:1", "8:no"),
        TABLE.replace(" - ", " "),
        TABLE.replace("/data\\040files", "/data\\041files"),
        TABLE.replace("/data\\040files", "/data\tfiles"),
        TABLE.replace("rw,relatime", "rw,ro"),
        format!("30 20 8:1 / / rw {} - ext4 x rw\n", "optional ".repeat(33)),
        "x".repeat(TABLE_BYTES + 1),
    ] {
        assert!(parse_mounts(&text, &|| Ok(())).is_err());
    }
    assert_eq!(
        parse_mounts(TABLE, &|| Err(Failure::TimedOut)),
        Err(Failure::TimedOut)
    );
}
fn rows(count: usize, padding: usize) -> String {
    let mut table = String::new();
    for i in 0..count {
        table.push_str(&format!(
            "{} 20 8:1 / / rw - ext4 {} rw\n",
            30 + i,
            "x".repeat(padding)
        ));
    }
    table
}
#[test]
fn row_byte_aggregate_and_report_limits_refuse_without_clipping() {
    assert_eq!(
        parse_mounts(&rows(TABLE_ROWS, 1), &|| Ok(()))
            .unwrap()
            .len(),
        TABLE_ROWS
    );
    assert!(parse_mounts(&rows(TABLE_ROWS + 1, 1), &|| Ok(())).is_err());
    let f = Fake::new(9);
    let d = declaration(9, 100);
    let table = format!(
        "30 20 8:1 / / rw - ext4 {} rw\n",
        "x".repeat(TABLE_BYTES - 31)
    );
    assert!(table.len() <= TABLE_BYTES);
    for p in f.state.borrow_mut().processes.values_mut() {
        p.table = table.clone();
    }
    assert!(matches!(
        f.inspect(&d, &bindings(&d)),
        Err(Failure::Unavailable)
    ));
    assert_eq!(f.counts.active.get(), 0);
    assert_eq!(
        json_bytes(&"x".repeat(REPORT_BYTES), REPORT_BYTES),
        Err(Failure::Unavailable)
    );
    assert_eq!(
        json_bytes(&"x".repeat(REPORT_BYTES - 2), REPORT_BYTES)
            .unwrap()
            .len(),
        REPORT_BYTES
    );
    let f = Fake::new(1);
    f.state
        .borrow_mut()
        .processes
        .get_mut(&200)
        .unwrap()
        .root
        .mount_id = 999;
    let d = declaration(1, 100);
    assert!(matches!(
        f.inspect(&d, &bindings(&d)),
        Err(Failure::Unavailable)
    ));
}
#[test]
fn bounded_reader_checks_sentinel_interruption_io_and_mid_read_budget() {
    assert_eq!(
        read_bounded(
            &mut io::Cursor::new(vec![0; RECORD_BYTES]),
            RECORD_BYTES,
            &|| Ok(())
        )
        .unwrap()
        .len(),
        RECORD_BYTES
    );
    assert_eq!(
        read_bounded(
            &mut io::Cursor::new(vec![0; RECORD_BYTES + 1]),
            RECORD_BYTES,
            &|| Ok(())
        ),
        Err(Failure::Unavailable)
    );
    struct Interrupted(bool);
    impl Read for Interrupted {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            if self.0 {
                self.0 = false;
                Err(io::ErrorKind::Interrupted.into())
            } else {
                Ok(0)
            }
        }
    }
    assert!(
        read_bounded(&mut Interrupted(true), RECORD_BYTES, &|| Ok(()))
            .unwrap()
            .is_empty()
    );
    let checks = Cell::new(0);
    assert_eq!(
        read_bounded(
            &mut io::Cursor::new(vec![0; RECORD_BYTES]),
            RECORD_BYTES,
            &|| {
                checks.set(checks.get() + 1);
                if checks.get() == 2 {
                    Err(Failure::TimedOut)
                } else {
                    Ok(())
                }
            }
        ),
        Err(Failure::TimedOut)
    );
}

struct OwnedChild(Child);
impl OwnedChild {
    fn new() -> Self {
        Self(Command::new("/usr/bin/sleep").arg("20").spawn().unwrap())
    }
    fn stop(&mut self) {
        self.0.kill().unwrap();
        self.0.wait().unwrap();
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
// UID 1000 cannot open PID 1's namespace on the test host. Only this PRIVATE
// adapter substitutes host-namespace authentication. All child pidfd/proc/ns/
// root/table operations delegate unchanged to Linux; no production fallback.
#[derive(Default)]
struct LocalChildren {
    stop_on_proc: RefCell<Option<OwnedChild>>,
    peak_fds: Cell<usize>,
}
fn fd_count() -> usize {
    std::fs::read_dir("/proc/self/fd").unwrap().count()
}
impl LocalChildren {
    fn sample(&self) {
        self.peak_fds.set(self.peak_fds.get().max(fd_count()));
    }
}
macro_rules! delegate_linux {
    ($(fn $name:ident($($arg:ident: $ty:ty),*) -> $out:ty;)+) => {$ (
        fn $name(&self, $($arg: $ty,)* b: &Budget<'_, Self>) -> Result<$out> {
            self.sample();
            let local = Budget { kernel: &Linux, lease: b.lease, deadline: b.deadline,
                table_bytes: std::array::from_fn(|_| Cell::new(0)) };
            let result = Linux.$name($($arg,)* &local);
            if let Err(failure) = &result { eprintln!("local child phase {}: {failure:?}", stringify!($name)); }
            self.sample(); result
        }
    )+};
}
impl Kernel for LocalChildren {
    type Handle = OwnedFd;
    fn clock(&self) -> Result<Time> {
        Linux.clock()
    }
    delegate_linux! {
        fn context() -> HostHandles<OwnedFd>;
        fn pidfd(pid: u32) -> OwnedFd;
        fn live(context: &HostHandles<OwnedFd>, fd: &OwnedFd, pid: u32) -> ();
        fn facts(proc: &OwnedFd, pid: u32) -> ProcessFacts;
        fn namespace(proc: &OwnedFd) -> OwnedFd;
        fn root(proc: &OwnedFd) -> OwnedFd;
        fn directory_identity(fd: &OwnedFd) -> ContainerProcessRoot;
        fn namespace_identity(fd: &OwnedFd) -> ContainerProcessNamespace;
    }
    fn proc(
        &self,
        context: &HostHandles<OwnedFd>,
        pid: u32,
        b: &Budget<'_, Self>,
    ) -> Result<OwnedFd> {
        if let Some(mut child) = self.stop_on_proc.borrow_mut().take() {
            child.stop();
        }
        let local = Budget {
            kernel: &Linux,
            lease: b.lease,
            deadline: b.deadline,
            table_bytes: std::array::from_fn(|_| Cell::new(0)),
        };
        let result = Linux.proc(context, pid, &local);
        self.sample();
        result
    }
    fn table(&self, proc: &OwnedFd, b: &Budget<'_, Self>, limit: usize) -> Result<String> {
        b.check()?;
        read_proc(proc, "mountinfo", limit, &|| {
            self.sample();
            b.check()
        })
    }
    fn host(&self, context: &HostHandles<OwnedFd>, b: &Budget<'_, Self>) -> Result<HostFacts> {
        let root = self.directory_identity(&context.root, b)?;
        let proc_root = self.directory_identity(&context.proc_root, b)?;
        let raw = self.table(&context.own_proc, b, TABLE_BYTES)?;
        let namespace = self.namespace_identity(&self.namespace(&context.own_proc, b)?, b)?;
        let boot = read_proc(
            &context.proc_root,
            "sys/kernel/random/boot_id",
            RECORD_BYTES,
            &|| b.check(),
        )?;
        Ok(HostFacts {
            boot: boot.trim().into(),
            root_mount: root.mount_id,
            table_digest: limeos_identity::digest(&raw),
            init_table_digest: "unqualified-in-private-local-child-test".into(),
            namespace,
            root,
            proc_root,
        })
    }
}
fn real_evidence(child: &OwnedChild) -> Evidence<LocalChildren> {
    let kernel = LocalChildren::default();
    let now = kernel.clock().unwrap();
    let d = declaration(1, now.wall.as_secs() as i64);
    let mut b = bindings(&d);
    b[0].pid = child.0.id();
    let sorted = validate_bindings(&d, &b).unwrap();
    let lease = Lease::new(&d, now).unwrap();
    collect(kernel, &d, &sorted, lease).expect("real complete collection")
}
#[test]
fn real_owned_child_retains_live_kernel_identity_and_exit_invalidates() {
    let mut child = OwnedChild::new();
    let e = real_evidence(&child);
    assert_eq!(e.snapshot.processes[0].pid, child.0.id());
    assert!(e.snapshot.processes[0].start_ticks > 0);
    assert_eq!(e.revalidate(), Ok(()));
    child.stop();
    assert_eq!(e.revalidate(), Err(Failure::Conflict));
    assert_eq!(e.revalidate(), Err(Failure::Conflict));
}
#[test]
fn real_dead_pidfd_and_proc_handles_do_not_provide_live_evidence() {
    let mut child = OwnedChild::new();
    let kernel = Linux;
    let now = kernel.clock().unwrap();
    let d = declaration(1, now.wall.as_secs() as i64);
    let lease = Lease::new(&d, now).unwrap();
    let budget = Budget::new(&kernel, &lease).unwrap();
    let context = kernel.context(&budget).unwrap();
    let fd = kernel.pidfd(child.0.id(), &budget).unwrap();
    let proc = kernel.proc(&context, child.0.id(), &budget).unwrap();
    child.stop();
    assert_eq!(
        kernel.live(&context, &fd, child.0.id(), &budget),
        Err(Failure::Conflict)
    );
    assert!(kernel.facts(&proc, child.0.id(), &budget).is_err());
}
#[test]
fn real_owned_child_exit_between_pidfd_and_proc_open_refuses_collection() {
    let child = OwnedChild::new();
    let pid = child.0.id();
    let kernel = LocalChildren {
        stop_on_proc: RefCell::new(Some(child)),
        peak_fds: Cell::new(0),
    };
    let now = kernel.clock().unwrap();
    let d = declaration(1, now.wall.as_secs() as i64);
    let mut b = bindings(&d);
    b[0].pid = pid;
    let sorted = validate_bindings(&d, &b).unwrap();
    assert!(matches!(
        collect(kernel, &d, &sorted, Lease::new(&d, now).unwrap()),
        Err(Failure::Unavailable | Failure::Conflict)
    ));
}
#[test]
fn real_owner_drop_resource_measurement() {
    // Isolate FD counting from parallel test threads in an owned test process.
    const FLAG: &str = "LIMEOS_PROCESS_EVIDENCE_FD_PROBE";
    if std::env::var_os(FLAG).is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "processes::tests::real_owner_drop_resource_measurement",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(FLAG, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        println!("{}", String::from_utf8_lossy(&output.stdout));
        return;
    }
    let children: Vec<_> = (0..64).map(|_| OwnedChild::new()).collect();
    let baseline = fd_count();
    let kernel = LocalChildren::default();
    let now = kernel.clock().unwrap();
    let d = declaration(64, now.wall.as_secs() as i64);
    let mut b = bindings(&d);
    for (binding, child) in b.iter_mut().zip(&children) {
        binding.pid = child.0.id();
    }
    let sorted = validate_bindings(&d, &b).unwrap();
    let e = collect(kernel, &d, &sorted, Lease::new(&d, now).unwrap()).unwrap();
    assert_eq!(fd_count(), baseline + 261);
    assert_eq!(e.revalidate(), Ok(()));
    println!(
        "real_fd_measurement processes=64 baseline={baseline} retained={} peak={} host_namespace_authentication=private_substitution",
        fd_count() - baseline,
        e.kernel.peak_fds.get() - baseline
    );
    drop(e);
    assert_eq!(fd_count(), baseline);
    drop(children);
    assert_eq!(fd_count(), baseline);
}
#[test]
fn real_ordinary_descriptors_cannot_substitute_for_namespace_or_root() {
    let kernel = Linux;
    let now = kernel.clock().unwrap();
    let d = declaration(0, now.wall.as_secs() as i64);
    let lease = Lease::new(&d, now).unwrap();
    let budget = Budget::new(&kernel, &lease).unwrap();
    let fd = open_fixed("/", OFlags::PATH | OFlags::DIRECTORY).unwrap();
    assert_eq!(
        kernel.namespace_identity(&fd, &budget),
        Err(Failure::Unavailable)
    );
    let fd =
        rustix::fs::open("/dev/null", OFlags::RDONLY | OFlags::CLOEXEC, Mode::empty()).unwrap();
    assert_eq!(
        kernel.directory_identity(&fd, &budget),
        Err(Failure::Unavailable)
    );
}
#[test]
fn real_directory_link_count_is_validity_not_identity_and_detachment_refuses() {
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("owned-root");
    std::fs::create_dir(&path).unwrap();
    let fd = rustix::fs::open(
        &path,
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .unwrap();
    let kernel = Linux;
    let now = kernel.clock().unwrap();
    let d = declaration(0, now.wall.as_secs() as i64);
    let lease = Lease::new(&d, now).unwrap();
    let budget = Budget::new(&kernel, &lease).unwrap();
    let before = kernel.directory_identity(&fd, &budget).unwrap();
    let links = rustix::fs::fstat(&fd).unwrap().st_nlink;
    let subdirectory = path.join("owned-subdirectory");
    std::fs::create_dir(&subdirectory).unwrap();
    assert_ne!(rustix::fs::fstat(&fd).unwrap().st_nlink, links);
    assert_eq!(kernel.directory_identity(&fd, &budget).unwrap(), before);
    std::fs::remove_dir(subdirectory).unwrap();
    std::fs::remove_dir(path).unwrap();
    assert_eq!(
        kernel.directory_identity(&fd, &budget),
        Err(Failure::Unavailable)
    );
}
#[test]
fn maximum_numeric_credential_and_identity_facts_are_preserved() {
    let f = Fake::new(64);
    let d = declaration(64, 100);
    for p in f.state.borrow_mut().processes.values_mut() {
        p.facts.uids = [u32::MAX; 4];
        p.facts.gids = [u32::MAX; 4];
        p.facts.start = u64::MAX;
        p.namespace.inode = u64::MAX;
        p.namespace.device = DeviceNumber {
            major: u32::MAX,
            minor: u32::MAX,
        };
        p.root.inode = u64::MAX;
        p.root.device = DeviceNumber {
            major: u32::MAX,
            minor: u32::MAX,
        };
    }
    let e = f.inspect(&d, &bindings(&d)).unwrap();
    assert_eq!(e.snapshot.processes[0].uids, [u32::MAX; 4]);
    assert_eq!(e.snapshot.processes[0].start_ticks, u64::MAX);
    assert_eq!(e.snapshot.processes[0].namespace.inode, u64::MAX);
    let mut amplified = e.snapshot.clone();
    amplified.host_boot_id = "x".repeat(REPORT_BYTES);
    assert_eq!(
        json_bytes(&amplified, REPORT_BYTES),
        Err(Failure::Unavailable)
    );
    drop(e);
    assert_eq!(f.counts.active.get(), 0);
}
#[test]
fn unprivileged_public_collection_refuses_without_weak_metadata_fallback() {
    if !rustix::process::geteuid().is_root() {
        let child = OwnedChild::new();
        let d = declaration(1, Linux.clock().unwrap().wall.as_secs() as i64);
        let mut b = bindings(&d);
        b[0].pid = child.0.id();
        assert!(matches!(
            inspect_running_container_processes(&d, &b),
            Err(Failure::Unavailable)
        ));
    }
}
#[test]
fn maximum_admitted_retention_resource_measurement() {
    // Frozen private clock measures allocation limits, not a real two-second
    // performance qualification. Every process gets 4096 rows; aggregate 8 MiB.
    let f = Fake::new(64);
    let d = declaration(64, 100);
    let table = rows(TABLE_ROWS, 1);
    assert!(table.len() * 64 <= AGGREGATE_TABLE_BYTES);
    for p in f.state.borrow_mut().processes.values_mut() {
        p.table = table.clone();
    }
    let e = f.inspect(&d, &bindings(&d)).unwrap();
    assert_eq!(e.snapshot.processes.len(), 64);
    assert_eq!(f.counts.active.get(), 261);
    assert_eq!(f.counts.peak.get(), 264);
    let vector_bytes: usize = e
        .retained
        .iter()
        .map(|r| r.mounts.capacity() * size_of::<ContainerProcessMount>())
        .sum();
    let string_bytes: usize = e
        .retained
        .iter()
        .flat_map(|r| &r.mounts)
        .map(|m| m.filesystem_root.capacity() + m.mountpoint.capacity() + m.filesystem.capacity())
        .sum();
    let report = json_bytes(&e.snapshot, REPORT_BYTES).unwrap();
    let hwm = std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find(|l| l.starts_with("VmHWM:"))
        .unwrap()
        .to_owned();
    println!(
        "resource_measurement processes=64 rows={} raw_table_bytes={} mount_vector_bytes={vector_bytes} mount_string_bytes={string_bytes} report_bytes={} retained_descriptors=261 peak_modeled_descriptors={} {hwm}",
        64 * TABLE_ROWS,
        table.len() * 64,
        report.len(),
        f.counts.peak.get()
    );
    drop(e);
    assert_eq!(f.counts.active.get(), 0);
}
