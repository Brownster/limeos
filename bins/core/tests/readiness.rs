//! Core's systemd protocol under `Type=notify`: READY=1 only once it can
//! serve, and watchdog keep-alives only after that.
use limeos_persistence::config::Config;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::net::UnixDatagram,
    process::{Child, Command, Stdio},
    time::Duration,
};

struct CoreChild(Child);
impl Drop for CoreChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn core_says_ready_once_it_serves_and_only_then_sends_keep_alives() {
    let dir = tempfile::tempdir().unwrap();
    let notify = UnixDatagram::bind(dir.path().join("notify")).unwrap();
    notify
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let unused = dir
        .path()
        .join("missing.sock")
        .to_str()
        .unwrap()
        .to_string();
    let config = Config {
        listen: address.to_string(),
        storage_socket: unused.clone(),
        host_socket: unused.clone(),
        container_socket: unused.clone(),
        storage_target_socket: unused.clone(),
        storage_mount_socket: unused,
        ..Config::default()
    };
    config.validate().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    let _core = CoreChild(
        Command::new(env!("CARGO_BIN_EXE_limeos-core"))
            .arg("--state-dir")
            .arg(dir.path().join("state"))
            .arg("--config")
            .arg(&path)
            .arg("--socket")
            .arg(dir.path().join("core.sock"))
            .env("NOTIFY_SOCKET", dir.path().join("notify"))
            // A 300 ms watchdog: keep-alives every 100 ms.
            .env("WATCHDOG_USEC", "300000")
            .env_remove("WATCHDOG_PID")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut buffer = [0; 256];
    let length = notify.recv(&mut buffer).expect("core never said ready");
    let first = String::from_utf8_lossy(&buffer[..length]).into_owned();
    assert!(
        first.split('\n').any(|line| line == "READY=1"),
        "the first notification is not readiness: {first:?}"
    );
    // Ready means serving: the HTTP listener answers at once.
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .write_all(b"GET /api/v1/health HTTP/1.1\r\nhost: localhost\r\nconnection: close\r\n\r\n")
        .unwrap();
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).unwrap();
    assert!(answer.starts_with(b"HTTP/1.1 200"));
    // Then keep-alives, from the RPC accept loop.
    for _ in 0..2 {
        let length = notify.recv(&mut buffer).expect("no keep-alive after ready");
        assert_eq!(&buffer[..length], b"WATCHDOG=1");
    }
}
