//! Actual Core startup/HTTP behavior, without unit credential or clock overrides.
use limeos_domain::{Operation, Role, Scope, automation::*};
use limeos_persistence::{Store, config::Config};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    os::unix::net::UnixListener,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct CoreChild(Child);
impl Drop for CoreChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn get(address: SocketAddr, token: &str) -> Option<serde_json::Value> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(200)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    write!(stream,"GET /api/v1/automation/schedules HTTP/1.1\r\nHost: localhost\r\nCookie: __Host-limeos={token}\r\nConnection: close\r\n\r\n").ok()?;
    let mut bytes = Vec::new();
    stream.take(65537).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 65536 || !bytes.starts_with(b"HTTP/1.1 200") {
        return None;
    }
    let start = bytes.windows(4).position(|v| v == b"\r\n\r\n")? + 4;
    serde_json::from_slice(&bytes[start..]).ok()
}
fn start(directory: &Path, config: &Path) -> CoreChild {
    CoreChild(
        Command::new(env!("CARGO_BIN_EXE_limeos-core"))
            .arg("--state-dir")
            .arg(directory)
            .arg("--config")
            .arg(config)
            .arg("--socket")
            .arg(directory.join("core.sock"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    )
}
fn wait_unavailable(child: &mut CoreChild, address: SocketAddr, token: &str, schedule: &str) {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "Core exited before automatic observation status"
        );
        if let Some(page) = get(address, token) {
            assert_eq!(page["items"].as_array().unwrap().len(), 1);
            assert_eq!(page["items"][0]["schedule"]["id"], schedule);
            if page["items"][0]["runtime"]["phase"] == "unavailable" {
                assert_eq!(page["items"][0]["status"], "scheduled");
                assert_eq!(page["partial"], true);
                assert!(
                    page["items"][0]["runtime"]["last_attempt"]
                        .as_i64()
                        .is_some()
                );
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "automatic partial/unavailable observation status did not appear"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn actual_core_startup_and_restart_report_unavailable_without_trusting_a_nonroot_reader() {
    assert!(!rustix::process::geteuid().is_root());
    let dir = tempfile::tempdir().unwrap();
    let at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    store.issue_bootstrap("boot", at - 100).unwrap();
    store
        .enroll("boot", "fixture", "$argon2id$fixture", at - 99)
        .unwrap();
    let initial = store.login_record("fixture").unwrap().unwrap().principal;
    store
        .revise_grants(
            &initial.id,
            Role::Operator,
            &[
                Scope {
                    operation: Operation::HealthRead,
                    resource: "host:system".into(),
                },
                Scope {
                    operation: Operation::HealthRead,
                    resource: format!("container:{}", "0".repeat(64)),
                },
                Scope {
                    operation: Operation::AutomationManage,
                    resource: "automation:observe-health".into(),
                },
            ],
            at - 98,
        )
        .unwrap();
    let record = store.login_record("fixture").unwrap().unwrap();
    let token = "a".repeat(64);
    store
        .create_session(
            &record,
            None,
            &limeos_identity::digest(&token),
            &limeos_identity::digest(&"b".repeat(64)),
            at - 90,
        )
        .unwrap();
    let schedule = store
        .automation_create(
            &record.principal,
            &CreateSchedule {
                definition: ScheduleDefinition {
                    operation: ScheduledOperation::ObserveHealth,
                    anchor: at,
                    interval_seconds: 30,
                    grace_seconds: 29,
                    missed: MissedRunPolicy::CoalesceLatest,
                    window: None,
                },
            },
            at,
        )
        .unwrap()
        .schedule
        .id;
    drop(store);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let reader = UnixListener::bind(dir.path().join("reader.sock")).unwrap();
    reader.set_nonblocking(true).unwrap();
    let unused = dir
        .path()
        .join("missing.sock")
        .to_str()
        .unwrap()
        .to_string();
    let config = Config {
        listen: address.to_string(),
        storage_socket: dir.path().join("reader.sock").to_str().unwrap().into(),
        host_socket: unused.clone(),
        container_socket: unused.clone(),
        storage_target_socket: unused.clone(),
        storage_mount_socket: unused,
        ..Config::default()
    };
    config.validate().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    for round in 0..2 {
        let mut child = start(dir.path(), &path);
        let pid = child.0.id();
        wait_unavailable(&mut child, address, &token, &schedule);
        drop(child);
        println!(
            "LIMEOS_CORE_OBSERVATION pid={pid} restart_round={round} partial_unavailable=true unit_overrides=false"
        );
    }
    // Neither health nor the existing stack proxy may send to an untrusted peer.
    while let Ok((mut stream, _)) = reader.accept() {
        stream
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let mut byte = [0u8; 1];
        assert!(matches!(stream.read(&mut byte), Ok(0)));
    }
    let connection = rusqlite::Connection::open_with_flags(
        dir.path().join("core.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let results: i64 = connection
        .query_row(
            "SELECT count(*) FROM automation_observation_results",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(results, 0);
}
