#![cfg(feature = "read-worker-probe")]
use limeos_executor::read_supervision::{Refusal, Supervisor, Ticket};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{
    path::PathBuf,
    process::Command,
    thread,
    time::{Duration, Instant},
};

fn command(mode: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_limeos-read-worker-probe"));
    command.arg("guardian").env("LIMEOS_TEST_WORKER_MODE", mode);
    command
}
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "read-supervision-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn ids(&self) -> PathBuf {
        self.0.join("ids")
    }
    fn wait_for(&self, role: &str) {
        let started = Instant::now();
        while !std::fs::read_to_string(self.ids())
            .unwrap_or_default()
            .lines()
            .any(|line| line.starts_with(&format!("{role} ")))
        {
            assert!(started.elapsed() < Duration::from_secs(3), "missing {role}");
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn gone(&self) {
        let text = std::fs::read_to_string(self.ids()).unwrap();
        for line in text.lines() {
            let pid = line.split_whitespace().nth(1).unwrap();
            assert!(
                !PathBuf::from(format!("/proc/{pid}")).exists(),
                "unreaped fixture: {line}; stat={:?}",
                std::fs::read_to_string(format!("/proc/{pid}/stat"))
            );
        }
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn next(manager: &Supervisor) -> Ticket {
    let started = Instant::now();
    loop {
        match manager.start(command("ok")) {
            Ok(ticket) => return ticket,
            Err(Refusal::Busy) => {
                assert!(started.elapsed() < Duration::from_secs(3));
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("admission: {error:?}"),
        }
    }
}
#[test]
fn complete_report_requires_successful_cleanup_and_preserves_parent_limit() {
    let before = rustix::process::getrlimit(rustix::process::Resource::Nofile);
    let manager = Supervisor::default();
    let result = manager.start(command("ok")).unwrap().wait().unwrap();
    assert_eq!(result.as_bytes(), br#"{"fixture":true}"#);
    assert_eq!(
        rustix::process::getrlimit(rustix::process::Resource::Nofile),
        before
    );
    assert!(next(&manager).wait().is_ok());
    assert_eq!(
        manager
            .start(command("large"))
            .unwrap()
            .wait()
            .unwrap()
            .as_bytes()
            .len(),
        256 * 1024
    );
}
#[test]
fn independent_deadline_stops_blocked_worker_and_reaps_descendants() {
    let scratch = Scratch::new();
    let manager = Supervisor::default();
    let mut cmd = command("descendant");
    cmd.env("LIMEOS_TEST_PROCESS_IDS", scratch.ids());
    let started = Instant::now();
    let result = manager.start(cmd).unwrap().wait();
    assert!(matches!(result, Err(Refusal::Deadline)));
    assert!(started.elapsed() < Duration::from_secs(6));
    assert!(next(&manager).wait().is_ok());
    scratch.gone();
}
#[test]
fn cancellation_cleans_a_child_that_changed_session() {
    let scratch = Scratch::new();
    let manager = Supervisor::default();
    let mut cmd = command("descendant");
    cmd.env("LIMEOS_TEST_PROCESS_IDS", scratch.ids())
        .env("LIMEOS_TEST_ESCAPE_GROUP", "1");
    let ticket = manager.start(cmd).unwrap();
    scratch.wait_for("leaf");
    assert!(matches!(manager.start(command("ok")), Err(Refusal::Busy)));
    ticket.cancel();
    assert!(matches!(ticket.wait(), Err(Refusal::Cancelled)));
    assert!(next(&manager).wait().is_ok());
    scratch.gone();
}
#[test]
fn dropped_ticket_cancels_without_releasing_the_owned_flight() {
    let scratch = Scratch::new();
    let manager = Supervisor::default();
    let mut cmd = command("hang");
    cmd.env("LIMEOS_TEST_PROCESS_IDS", scratch.ids());
    let ticket = manager.start(cmd).unwrap();
    scratch.wait_for("worker");
    drop(ticket);
    assert!(next(&manager).wait().is_ok());
    scratch.gone();
}
#[test]
fn leftover_output_pipe_descendant_is_reaped_before_report_acceptance() {
    let scratch = Scratch::new();
    let manager = Supervisor::default();
    let mut cmd = command("descendant-report");
    cmd.env("LIMEOS_TEST_PROCESS_IDS", scratch.ids())
        .env("LIMEOS_TEST_ESCAPE_GROUP", "1");
    let result = manager.start(cmd).unwrap().wait();
    assert!(
        result.is_ok(),
        "report result: {}",
        match result {
            Ok(_) => "ok".into(),
            Err(error) => format!("{error:?}"),
        }
    );
    scratch.gone();
}
#[test]
fn malformed_partial_trailing_and_oversized_output_refuse_without_poisoning_cleanup() {
    for mode in ["malformed", "partial", "trailing", "oversized"] {
        let manager = Supervisor::default();
        assert!(
            matches!(
                manager.start(command(mode)).unwrap().wait(),
                Err(Refusal::InvalidOutput)
            ),
            "{mode}"
        );
        assert!(next(&manager).wait().is_ok());
    }
}
#[test]
fn complete_frame_with_nonzero_worker_exit_is_refused() {
    let manager = Supervisor::default();
    assert!(matches!(
        manager.start(command("nonzero")).unwrap().wait(),
        Err(Refusal::WorkerExit)
    ));
    assert!(next(&manager).wait().is_ok());
}
#[test]
fn guardian_failure_keeps_admission_poisoned() {
    let manager = Supervisor::default();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_limeos-read-worker-probe"));
    cmd.arg("bad-guardian");
    assert!(matches!(
        manager.start(cmd).unwrap().wait(),
        Err(Refusal::CleanupUnconfirmed)
    ));
    assert!(matches!(
        manager.start(command("ok")),
        Err(Refusal::CleanupUnconfirmed)
    ));
}
#[test]
fn bootstrapped_genuine_nonroot_collection_refuses() {
    assert!(!rustix::process::geteuid().is_root());
    let manager = Supervisor::default();
    assert!(matches!(
        manager.start(command("nonroot-collector")).unwrap().wait(),
        Err(Refusal::WorkerRefused)
    ));
    assert!(next(&manager).wait().is_ok());
}
#[test]
fn manager_death_leaves_no_owned_children_or_zombies() {
    let scratch = Scratch::new();
    let mut observer = Command::new(env!("CARGO_BIN_EXE_limeos-read-worker-probe"));
    observer
        .arg("parent-death-observer")
        .env("LIMEOS_TEST_WORKER_MODE", "descendant")
        .env("LIMEOS_TEST_PROCESS_IDS", scratch.ids())
        .env("LIMEOS_TEST_ESCAPE_GROUP", "1");
    assert!(observer.status().unwrap().success());
    scratch.gone();
}
