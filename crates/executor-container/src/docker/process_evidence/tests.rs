use super::*;
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, atomic::AtomicI64},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixListener,
    sync::Notify,
};

#[derive(Clone, Copy)]
enum Change {
    Pid,
    Mount,
    Snapshot,
    Membership,
    Engine,
}
#[derive(Clone)]
struct Config {
    count: usize,
    first_running: bool,
    all_running: bool,
    duplicate_pids: bool,
    pid: Option<Value>,
    changed: Option<Change>,
    mounts: usize,
    long_paths: bool,
    padding: usize,
    engine: String,
    image: String,
    source: String,
    hold: bool,
    partial: bool,
    response_delay: Duration,
    clock_on_final_info: Option<(Arc<AtomicI64>, i64)>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            count: 2,
            first_running: true,
            all_running: false,
            duplicate_pids: false,
            pid: Some(json!(1000)),
            changed: None,
            mounts: 1,
            long_paths: false,
            padding: 0,
            engine: "fixture-engine".into(),
            image: format!("sha256:{}", "f".repeat(64)),
            source: "/mnt/storage/Media".into(),
            hold: false,
            partial: false,
            response_delay: Duration::ZERO,
            clock_on_final_info: None,
        }
    }
}
#[derive(Default)]
struct ServerState {
    config: Config,
    counts: HashMap<String, usize>,
    requests: Vec<String>,
}
struct Fixture {
    _directory: tempfile::TempDir,
    docker: Docker,
    policy: EnginePeerPolicy,
    state: Arc<Mutex<ServerState>>,
    accepted: Arc<AtomicUsize>,
    held: Arc<Notify>,
    eof: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    fn set(&self, change: impl FnOnce(&mut Config)) {
        change(&mut self.state.lock().unwrap().config);
    }
    fn requests(&self) -> Vec<String> {
        self.state.lock().unwrap().requests.clone()
    }
    async fn inspect(&self) -> Result<EngineStorageProcessEvidence> {
        self.docker.inspect_storage_processes(self.policy).await
    }
}
async fn fixture(config: Config) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("engine.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let uid = rustix::process::geteuid().as_raw();
    let policy = EnginePeerPolicy {
        socket_uid: uid,
        peer_uid: uid,
    };
    let state = Arc::new(Mutex::new(ServerState {
        config,
        ..ServerState::default()
    }));
    let accepted = Arc::new(AtomicUsize::new(0));
    let eof = Arc::new(Notify::new());
    let held = Arc::new(Notify::new());
    let (seen, connections, ended, hold) =
        (state.clone(), accepted.clone(), eof.clone(), held.clone());
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            connections.fetch_add(1, Ordering::Relaxed);
            let mut header = vec![];
            while !header.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                if stream.read_exact(&mut byte).await.is_err() {
                    ended.notify_one();
                    break;
                }
                header.push(byte[0]);
                assert!(header.len() < 4096);
            }
            if header.is_empty() {
                continue;
            }
            let request = String::from_utf8(header)
                .unwrap()
                .lines()
                .next()
                .unwrap()
                .to_owned();
            let (config, nth) = {
                let mut state = seen.lock().unwrap();
                state.requests.push(request.clone());
                let nth = state.counts.entry(request.clone()).or_default();
                *nth += 1;
                let nth = *nth;
                (state.config.clone(), nth)
            };
            if request.contains("/info ") && config.hold {
                let _ = stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n{")
                    .await;
                hold.notify_one();
                let mut bytes = [0; 64];
                while stream.read(&mut bytes).await.is_ok_and(|n| n > 0) {}
                ended.notify_one();
                continue;
            }
            let value = if request.contains("/version ") {
                json!({"ApiVersion":"1.41","MinAPIVersion":"1.24"})
            } else if request.contains("/info ") {
                if nth.is_multiple_of(2) {
                    if let Some((clock, value)) = &config.clock_on_final_info {
                        clock.store(*value, Ordering::Relaxed);
                    }
                }
                json!({"ID": if matches!(config.changed, Some(Change::Engine)) && nth > 1 {
                    "replacement-engine".to_owned()
                } else { config.engine.clone() }})
            } else if request.contains("/containers/json?all=1 ") {
                let count = if matches!(config.changed, Some(Change::Membership)) && nth > 1 {
                    0
                } else {
                    config.count
                };
                json!(
                    (0..count)
                        .map(|i| json!({"Id": format!("{i:064x}")}))
                        .collect::<Vec<_>>()
                )
            } else {
                let id = request
                    .split("/containers/")
                    .nth(1)
                    .unwrap()
                    .split('/')
                    .next()
                    .unwrap();
                let index = usize::from_str_radix(id, 16).unwrap();
                let running = config.all_running || (index == 0 && config.first_running);
                let source = if matches!(config.changed, Some(Change::Mount)) && nth > 1 {
                    "/mnt/replaced".into()
                } else {
                    config.source.clone()
                };
                let mounts: Vec<_> = (0..config.mounts)
                    .map(|i| {
                        let source = if config.long_paths {
                            format!("/mnt/{}", "x".repeat(480))
                        } else {
                            source.clone()
                        };
                        let destination = if config.long_paths {
                            format!("/data/{i}/{}", "x".repeat(480))
                        } else {
                            format!("/data/{i}")
                        };
                        json!({"Type":"bind","Source":source,"Destination":destination,"RW":true})
                    })
                    .collect();
                let mut value = json!({
                    "Id": id,
                    "Image": config.image,
                    "State": {"Running": running,"StartedAt":"2026-10-07T00:00:00Z"},
                    "Mounts": mounts,
                    "Config": {"Env": ["SECRET=private"]},
                    "Labels": {"credential": "private"},
                    "padding": "x".repeat(config.padding),
                });
                if index == 0 {
                    if let Some(pid) = config.pid {
                        value["State"]["Pid"] = pid;
                    }
                } else {
                    value["State"]["Pid"] = json!(if running {
                        if config.duplicate_pids {
                            1000
                        } else {
                            1000 + index
                        }
                    } else {
                        0
                    });
                }
                if matches!(config.changed, Some(Change::Pid)) && nth > 1 && running {
                    value["State"]["Pid"] = json!(5000 + index);
                }
                if matches!(config.changed, Some(Change::Snapshot)) && nth > 1 {
                    value["State"]["StartedAt"] = json!("2026-10-07T00:00:01Z");
                }
                value
            };
            if config.partial && request.contains("/info ") {
                let _ = stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n{")
                    .await;
                continue;
            }
            let body = value.to_string();
            tokio::time::sleep(config.response_delay).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body,
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
    Fixture {
        _directory: directory,
        docker: Docker::new(socket),
        policy,
        state,
        accepted,
        held,
        eof,
        task,
    }
}
fn error(result: Result<EngineStorageProcessEvidence>) -> ErrorCode {
    result.err().expect("unexpected usable evidence").0
}
fn clock(value: i64) -> (Arc<AtomicI64>, TimeSource) {
    let wall = Arc::new(AtomicI64::new(value));
    let read = wall.clone();
    (
        wall,
        TimeSource {
            wall: Some(Arc::new(move || Ok(read.load(Ordering::Relaxed)))),
        },
    )
}

#[tokio::test]
async fn complete_gets_include_stopped_and_mountless_consumers_without_private_fields() {
    let fixture = fixture(Config {
        mounts: 0,
        ..Config::default()
    })
    .await;
    let evidence = fixture.inspect().await.unwrap();
    assert_eq!(evidence.declarations().containers.len(), 2);
    assert_eq!(evidence.running().len(), 1);
    assert_eq!(evidence.running()[0].pid(), 1000);
    assert_eq!(
        evidence.running()[0].container(),
        &evidence.declarations().containers[0].container
    );
    evidence.revalidate().await.unwrap();
    let report = serde_json::to_string(evidence.snapshot()).unwrap();
    for private in ["SECRET", "private", "Labels", "Config"] {
        assert!(!report.contains(private));
    }
    let requests = fixture.requests();
    assert!(requests.iter().all(|request| request.starts_with("GET ")));
    assert_eq!(
        requests
            .iter()
            .filter(|line| line.contains("/json "))
            .count(),
        8
    );
    assert_eq!(
        requests
            .iter()
            .filter(|line| line.contains("/containers/json?all=1 "))
            .count(),
        4
    );
}

#[tokio::test]
async fn genuinely_empty_double_enumeration_is_the_only_empty_success() {
    let fixture = fixture(Config {
        count: 0,
        ..Config::default()
    })
    .await;
    let evidence = fixture.inspect().await.unwrap();
    assert!(evidence.declarations().containers.is_empty());
    assert!(evidence.running().is_empty());
    assert_eq!(
        fixture
            .requests()
            .iter()
            .filter(|line| line.contains("/containers/json?all=1 "))
            .count(),
        2
    );
}

#[tokio::test]
async fn running_pid_requires_a_non_system_linux_integer_and_stopped_requires_zero() {
    for pid in [
        None,
        Some(Value::Null),
        Some(json!(0)),
        Some(json!(1)),
        Some(json!(-1)),
        Some(json!(1.5)),
        Some(json!("1000")),
        Some(json!(i32::MAX as u64 + 1)),
        Some(json!(u64::MAX)),
    ] {
        let fixture = fixture(Config {
            pid,
            ..Config::default()
        })
        .await;
        assert_eq!(error(fixture.inspect().await), ErrorCode::Unavailable);
    }
    for pid in [2, i32::MAX] {
        let fixture = fixture(Config {
            pid: Some(json!(pid)),
            ..Config::default()
        })
        .await;
        assert_eq!(
            fixture.inspect().await.unwrap().running()[0].pid(),
            pid as u32
        );
    }
    let stopped = fixture(Config {
        first_running: false,
        pid: Some(json!(0)),
        ..Config::default()
    })
    .await;
    assert!(stopped.inspect().await.unwrap().running().is_empty());
    stopped.set(|config| config.pid = Some(json!(1000)));
    assert_eq!(error(stopped.inspect().await), ErrorCode::Unavailable);
}

#[tokio::test]
async fn duplicate_running_pids_refuse_the_complete_result() {
    let fixture = fixture(Config {
        all_running: true,
        duplicate_pids: true,
        ..Config::default()
    })
    .await;
    assert_eq!(error(fixture.inspect().await), ErrorCode::Unavailable);
}

#[tokio::test]
async fn second_inspection_pid_snapshot_mount_membership_or_engine_change_refuses() {
    for change in [
        Change::Pid,
        Change::Snapshot,
        Change::Mount,
        Change::Membership,
        Change::Engine,
    ] {
        let fixture = fixture(Config {
            changed: Some(change),
            ..Config::default()
        })
        .await;
        assert_eq!(error(fixture.inspect().await), ErrorCode::Conflict);
    }
}

#[tokio::test]
async fn reacquisition_binds_pid_full_snapshots_mounts_membership_and_engine() {
    for change in [
        Change::Pid,
        Change::Snapshot,
        Change::Mount,
        Change::Membership,
        Change::Engine,
    ] {
        let fixture = fixture(Config::default()).await;
        let evidence = fixture.inspect().await.unwrap();
        fixture.set(|config| match change {
            Change::Pid => config.pid = Some(json!(5000)),
            Change::Snapshot => config.image = format!("sha256:{}", "e".repeat(64)),
            Change::Mount => config.source = "/mnt/replaced".into(),
            Change::Membership => config.count = 3,
            Change::Engine => config.engine = "replacement-engine".into(),
        });
        assert_eq!(
            evidence.revalidate().await.unwrap_err().0,
            ErrorCode::Conflict
        );
    }
    let fixture = fixture(Config::default()).await;
    let evidence = fixture.inspect().await.unwrap();
    fixture.set(|config| {
        config.first_running = false;
        config.pid = Some(json!(0));
    });
    assert_eq!(
        evidence.revalidate().await.unwrap_err().0,
        ErrorCode::Conflict
    );
}

#[tokio::test]
async fn mismatched_kernel_peer_uid_sends_no_http_bytes() {
    let fixture = fixture(Config::default()).await;
    let wrong = EnginePeerPolicy {
        socket_uid: fixture.policy.socket_uid,
        peer_uid: fixture.policy.peer_uid + 1,
    };
    assert_eq!(
        error(fixture.docker.inspect_storage_processes(wrong).await),
        ErrorCode::Forbidden
    );
    tokio::time::timeout(Duration::from_secs(1), fixture.eof.notified())
        .await
        .unwrap();
    assert_eq!(fixture.accepted.load(Ordering::Relaxed), 1);
    assert!(fixture.requests().is_empty());
}

#[tokio::test]
async fn root_policy_never_relaxes_to_an_unprivileged_fixture_owner() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let fixture = fixture(Config::default()).await;
    assert_eq!(
        error(
            fixture
                .docker
                .inspect_storage_processes(EnginePeerPolicy::root())
                .await
        ),
        ErrorCode::Forbidden
    );
    assert!(fixture.requests().is_empty());
}

#[tokio::test]
async fn production_root_peer_rule_rejects_actual_unprivileged_credentials_before_bytes() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let fixture = fixture(Config::default()).await;
    // Private socket-owner seam permits reaching the unchanged production UID-0
    // peer check. No nonzero peer UID is trusted for this rejection test.
    let policy = EnginePeerPolicy {
        socket_uid: fixture.policy.socket_uid,
        ..EnginePeerPolicy::root()
    };
    assert_eq!(
        error(fixture.docker.inspect_storage_processes(policy).await),
        ErrorCode::Forbidden
    );
    tokio::time::timeout(Duration::from_secs(1), fixture.eof.notified())
        .await
        .unwrap();
    assert_eq!(fixture.accepted.load(Ordering::Relaxed), 1);
    assert!(fixture.requests().is_empty());
}

#[tokio::test]
async fn replaced_socket_object_invalidates_the_original_owner_before_more_http() {
    let fixture = fixture(Config::default()).await;
    let evidence = fixture.inspect().await.unwrap();
    let requests = fixture.requests().len();
    std::fs::remove_file(&fixture.docker.socket).unwrap();
    let _replacement = UnixListener::bind(&fixture.docker.socket).unwrap();
    assert_eq!(
        evidence.revalidate().await.unwrap_err().0,
        ErrorCode::Conflict
    );
    assert_eq!(fixture.requests().len(), requests);
}

#[tokio::test]
async fn socket_symlink_and_world_writable_socket_refuse() {
    let fixture = fixture(Config::default()).await;
    let link = fixture._directory.path().join("alias.sock");
    std::os::unix::fs::symlink(&fixture.docker.socket, &link).unwrap();
    assert_eq!(
        error(
            Docker::new(link)
                .inspect_storage_processes(fixture.policy)
                .await
        ),
        ErrorCode::Forbidden
    );
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        &fixture.docker.socket,
        std::fs::Permissions::from_mode(0o666),
    )
    .unwrap();
    assert_eq!(error(fixture.inspect().await), ErrorCode::Forbidden);
    assert!(fixture.requests().is_empty());
}

#[tokio::test]
async fn count_mount_and_report_limits_refuse_without_truncation() {
    for config in [
        Config {
            count: 65,
            ..Config::default()
        },
        Config {
            mounts: 65,
            ..Config::default()
        },
        Config {
            count: 5,
            mounts: 64,
            ..Config::default()
        },
        Config {
            count: 1,
            mounts: 64,
            long_paths: true,
            ..Config::default()
        },
        Config {
            padding: 512 * 1024,
            ..Config::default()
        },
    ] {
        let fixture = fixture(config).await;
        assert_eq!(error(fixture.inspect().await), ErrorCode::Unavailable);
    }
}

#[tokio::test]
async fn all_sixty_four_running_consumers_are_admitted_without_mounts() {
    let fixture = fixture(Config {
        count: 64,
        all_running: true,
        mounts: 0,
        ..Config::default()
    })
    .await;
    let evidence = fixture.inspect().await.unwrap();
    assert_eq!(evidence.declarations().containers.len(), 64);
    assert_eq!(evidence.running().len(), 64);
    assert_eq!(
        fixture
            .requests()
            .iter()
            .filter(|line| line.contains("/json "))
            .count(),
        128
    );
}

#[tokio::test]
async fn aggregate_body_budget_spans_both_inspection_passes() {
    let fixture = fixture(Config {
        count: 9,
        all_running: true,
        padding: 500_000,
        mounts: 0,
        ..Config::default()
    })
    .await;
    assert_eq!(error(fixture.inspect().await), ErrorCode::Unavailable);
    let requests = fixture.requests();
    let inspect_count = requests
        .iter()
        .filter(|line| line.contains("/json "))
        .count();
    assert!(inspect_count > 9 && inspect_count < 18);
    assert_eq!(
        requests
            .iter()
            .filter(|line| line.contains("/containers/json?all=1 "))
            .count(),
        1
    );
}

#[tokio::test]
async fn partial_body_produces_no_owner_or_empty_success() {
    let fixture = fixture(Config {
        partial: true,
        ..Config::default()
    })
    .await;
    assert_eq!(error(fixture.inspect().await), ErrorCode::Unavailable);
}

#[tokio::test]
async fn cancellation_aborts_the_connection_task_and_peer_observes_eof() {
    let fixture = fixture(Config {
        hold: true,
        ..Config::default()
    })
    .await;
    let docker = fixture.docker.clone();
    let policy = fixture.policy;
    let request = tokio::spawn(async move { docker.inspect_storage_processes(policy).await });
    tokio::time::timeout(Duration::from_secs(1), fixture.held.notified())
        .await
        .unwrap();
    request.abort();
    assert!(matches!(request.await, Err(error) if error.is_cancelled()));
    tokio::time::timeout(Duration::from_secs(1), fixture.eof.notified())
        .await
        .unwrap();
}

#[tokio::test]
async fn request_timeout_also_aborts_the_connection_task() {
    let fixture = fixture(Config {
        hold: true,
        ..Config::default()
    })
    .await;
    assert_eq!(error(fixture.inspect().await), ErrorCode::Unavailable);
    tokio::time::timeout(Duration::from_secs(1), fixture.eof.notified())
        .await
        .unwrap();
}

#[tokio::test]
async fn complete_collection_deadline_refuses_responses_each_below_request_timeout() {
    let fixture = fixture(Config {
        response_delay: Duration::from_millis(750),
        ..Config::default()
    })
    .await;
    let started = Instant::now();
    assert_eq!(error(fixture.inspect().await), ErrorCode::Unavailable);
    assert!(started.elapsed() >= Duration::from_millis(3900));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(fixture.requests().len() >= 5);
}

#[tokio::test]
async fn declarations_preserve_issue_time_before_socket_acquisition() {
    let fixture = fixture(Config::default()).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let clock = TimeSource {
        wall: Some(Arc::new(move || {
            Ok(if calls.fetch_add(1, Ordering::Relaxed) == 0 {
                100
            } else {
                104
            })
        })),
    };
    let evidence = inspect(&fixture.docker, fixture.policy, clock)
        .await
        .unwrap();
    assert_eq!(evidence.declarations().observed_at, 100);
}

#[tokio::test]
async fn fresh_revalidation_timestamp_does_not_renew_original_lifetime() {
    let fixture = fixture(Config::default()).await;
    let (wall, clock) = clock(100);
    let evidence = inspect(&fixture.docker, fixture.policy, clock)
        .await
        .unwrap();
    wall.store(104, Ordering::Relaxed);
    evidence.revalidate().await.unwrap();
    assert_eq!(evidence.declarations().observed_at, 100);
    wall.store(106, Ordering::Relaxed);
    let requests = fixture.requests().len();
    assert_eq!(
        evidence.revalidate().await.unwrap_err().0,
        ErrorCode::Conflict
    );
    assert_eq!(fixture.requests().len(), requests);
}

#[tokio::test]
async fn final_reacquisition_response_cannot_cross_the_original_wall_expiry() {
    let fixture = fixture(Config::default()).await;
    let (wall, clock) = clock(100);
    let evidence = inspect(&fixture.docker, fixture.policy, clock)
        .await
        .unwrap();
    wall.store(104, Ordering::Relaxed);
    fixture.set(|config| config.clock_on_final_info = Some((wall, 106)));
    assert_eq!(
        evidence.revalidate().await.unwrap_err().0,
        ErrorCode::Conflict
    );
    assert_eq!(
        fixture
            .requests()
            .iter()
            .filter(|line| line.contains("/info "))
            .count(),
        4
    );
}

#[tokio::test]
async fn initial_collection_checks_wall_rollback_and_age_at_its_final_response() {
    for next in [99, 106] {
        let fixture = fixture(Config::default()).await;
        let (wall, clock) = clock(100);
        fixture.set(|config| config.clock_on_final_info = Some((wall, next)));
        assert_eq!(
            error(inspect(&fixture.docker, fixture.policy, clock).await),
            ErrorCode::Conflict
        );
    }
}

#[tokio::test]
async fn unchanged_wall_clock_cannot_extend_monotonic_expiry() {
    let fixture = fixture(Config::default()).await;
    let (_wall, clock) = clock(100);
    let evidence = inspect(&fixture.docker, fixture.policy, clock)
        .await
        .unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(5)).await;
    let requests = fixture.requests().len();
    assert_eq!(
        evidence.revalidate().await.unwrap_err().0,
        ErrorCode::Conflict
    );
    assert_eq!(fixture.requests().len(), requests);
}

#[tokio::test]
async fn advisory_inventory_keeps_its_missing_pid_behavior() {
    let fixture = fixture(Config {
        pid: None,
        ..Config::default()
    })
    .await;
    let inventory = fixture.docker.storage_inventory().await.unwrap();
    assert_eq!(inventory.containers.len(), 2);
    assert_eq!(error(fixture.inspect().await), ErrorCode::Unavailable);
}
