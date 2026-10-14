//! Opt-in owned-process fixture. Never built or packaged by default.
use limeos_executor::read_supervision::{self as supervision, Supervisor, WorkerBootstrap};
use rustix::process::{self, Pid, WaitOptions};
use std::{
    fs::OpenOptions,
    io::Write,
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};

fn marker(role: &str) {
    if let Some(path) = std::env::var_os("LIMEOS_TEST_PROCESS_IDS") {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(file, "{role} {}", std::process::id()).unwrap();
    }
}
fn command(role: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.arg(role);
    command
}
fn pause() {
    thread::sleep(Duration::from_secs(60));
}
fn main() {
    let role = std::env::args()
        .nth(1)
        .expect("closed fixture role required");
    match role.as_str() {
        "guardian" => {
            marker("guardian");
            let mode = std::env::var("LIMEOS_TEST_WORKER_MODE").unwrap();
            if supervision::guardian(command(&mode)).is_err() {
                std::process::exit(80);
            }
        }
        "bad-guardian" => std::process::exit(81),
        "parent" => {
            marker("parent");
            let manager = Supervisor::default();
            let _ticket = manager.start(command("guardian")).unwrap();
            pause();
        }
        "parent-death-observer" => {
            // Isolate adoption from the integration-test process and its other children.
            process::set_child_subreaper(Some(Pid::INIT)).unwrap();
            let mut parent = command("parent").spawn().unwrap();
            let started = Instant::now();
            let path = std::env::var_os("LIMEOS_TEST_PROCESS_IDS").unwrap();
            while !std::fs::read_to_string(&path)
                .unwrap_or_default()
                .lines()
                .any(|line| line.starts_with("leaf "))
            {
                assert!(started.elapsed() < Duration::from_secs(3));
                thread::sleep(Duration::from_millis(10));
            }
            parent.kill().unwrap();
            parent.wait().unwrap();
            loop {
                match process::wait(WaitOptions::NOHANG) {
                    Err(rustix::io::Errno::CHILD) => break,
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        assert!(started.elapsed() < Duration::from_secs(6));
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("observer wait: {error}"),
                }
            }
        }
        "leaf" => {
            if std::env::var_os("LIMEOS_TEST_ESCAPE_GROUP").is_some() {
                process::setsid().unwrap();
            }
            marker("leaf");
            pause();
        }
        _ => {
            let guardian = std::env::var("LIMEOS_READ_GUARDIAN_PID")
                .unwrap()
                .parse()
                .ok()
                .and_then(Pid::from_raw)
                .unwrap();
            let boot = WorkerBootstrap::establish(guardian).unwrap();
            marker("worker");
            match role.as_str() {
                "ok" => supervision::write_worker_report(br#"{"fixture":true}"#).unwrap(),
                "large" => {
                    let mut body = vec![b'x'; 256 * 1024];
                    body[0] = b'"';
                    *body.last_mut().unwrap() = b'"';
                    supervision::write_worker_report(&body).unwrap();
                }
                "hang" => pause(),
                "descendant" | "descendant-report" => {
                    // Child ownership is deliberately delegated to the guardian fixture.
                    #[allow(clippy::zombie_processes)]
                    // Guardian adopts/reaps this fixture descendant.
                    let child = command("leaf").spawn().unwrap();
                    if let Some(path) = std::env::var_os("LIMEOS_TEST_PROCESS_IDS") {
                        let mut file = OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(path)
                            .unwrap();
                        writeln!(file, "spawned-leaf {}", child.id()).unwrap();
                    }
                    if role == "descendant-report" {
                        supervision::write_worker_report(br#"{"fixture":true}"#).unwrap();
                    } else {
                        pause();
                    }
                }
                "nonzero" => {
                    supervision::write_worker_report(br#"{"fixture":true}"#).unwrap();
                    std::process::exit(82);
                }
                "partial" => {
                    std::io::stdout()
                        .write_all(&[0, 0, 0, 0, 10, b'{'])
                        .unwrap();
                }
                "oversized" => {
                    std::io::stdout().write_all(&[0, 0, 4, 0, 1]).unwrap();
                }
                "trailing" => {
                    supervision::write_worker_report(br#"{"fixture":true}"#).unwrap();
                    std::io::stdout().write_all(b"extra").unwrap();
                }
                "malformed" => supervision::write_worker_report(b"{broken").unwrap(),
                "nonroot-collector" => {
                    assert!(!process::geteuid().is_root());
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap();
                    let paths = limeos_executor_storage::configuration::ConfigurationPaths {
                        contract: Path::new("/not-opened/contract.json"),
                        pools: Path::new("/not-opened/pools.json"),
                        protection: Path::new("/not-opened/protection.json"),
                        shares: Path::new("/not-opened/shares.json"),
                    };
                    assert!(matches!(
                        runtime.block_on(boot.collect(paths)),
                        Err(limeos_executor::read_collection::Refusal {
                            stage: limeos_executor::read_collection::Stage::Collect(
                                limeos_executor::read_budget::Phase::Configuration
                            ),
                            failure: limeos_executor::read_collection::Failure::Storage(
                                limeos_executor_storage::Failure::Unavailable
                            ),
                        })
                    ));
                    supervision::write_worker_refusal().unwrap();
                }
                _ => std::process::exit(83),
            }
        }
    }
}
