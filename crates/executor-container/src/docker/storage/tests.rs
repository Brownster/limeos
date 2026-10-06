use super::*;
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixListener,
};

#[derive(Clone, Debug)]
enum Case {
    Valid,
    InspectChanges,
    ListChanges,
    EngineChanges,
    MissingMounts,
    MissingSource,
    UnknownType,
    DuplicateIds,
    DuplicateDestinations,
    InvalidSource,
    MissingVolumeDriver,
    CountLimit,
    MountLimit,
    AggregateLimit,
    Oversized,
    Timeout,
}
struct Fixture {
    _dir: tempfile::TempDir,
    docker: Docker,
    calls: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn fixture(case: Case) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let calls = Arc::new(Mutex::new(vec![]));
    let seen = calls.clone();
    let task = tokio::spawn(async move {
        let mut counts = HashMap::<String, usize>::new();
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = vec![];
            while !header.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                if stream.read_exact(&mut byte).await.is_err() {
                    break;
                }
                header.push(byte[0]);
            }
            let line = String::from_utf8(header)
                .unwrap()
                .lines()
                .next()
                .unwrap()
                .to_owned();
            seen.lock().unwrap().push(line.clone());
            let count = counts.entry(line.clone()).or_default();
            *count += 1;
            let mut body = if line.contains("/version ") {
                json!({"ApiVersion":"1.41","MinAPIVersion":"1.24"})
            } else if line.contains("/info ") {
                json!({"ID": if matches!(case, Case::EngineChanges) && *count > 1 {"replacement-engine"} else {"fixture-engine"}})
            } else if line.contains("/containers/json?all=1 ") {
                let count = if matches!(case, Case::CountLimit) {
                    65
                } else if matches!(case, Case::AggregateLimit) {
                    5
                } else {
                    2
                };
                if matches!(case, Case::ListChanges) && *counts.get(&line).unwrap() > 1 {
                    json!([])
                } else {
                    json!((0..count).map(|i| json!({"Id": if matches!(case, Case::DuplicateIds) {format!("{:064x}", 0)} else {format!("{i:064x}")} })).collect::<Vec<_>>())
                }
            } else {
                let id = line
                    .split("/containers/")
                    .nth(1)
                    .unwrap()
                    .split('/')
                    .next()
                    .unwrap();
                let mut mounts = if id.ends_with('0') || matches!(case, Case::AggregateLimit) {
                    vec![
                        json!({"Type":"bind","Source":"/mnt/storage/Media","Destination":"/data","RW":true}),
                        json!({"Type":"tmpfs","Destination":"/tmp","RW":true}),
                    ]
                } else {
                    vec![
                        json!({"Type":"volume","Name":"media","Driver":"local","Source":"/var/lib/docker/volumes/media/_data","Destination":"/data","RW":false}),
                    ]
                };
                if matches!(case, Case::MountLimit | Case::AggregateLimit) {
                    mounts = (0..if matches!(case, Case::MountLimit) {65} else {64}).map(|i| json!({"Type":"bind","Source":"/mnt/storage/Media","Destination":format!("/data/{i}"),"RW":true})).collect();
                }
                if matches!(case, Case::DuplicateDestinations) {
                    mounts.push(mounts[0].clone());
                }
                let mut value = json!({"Id":id,"Image":format!("sha256:{}", "f".repeat(64)),"State":{"Running":id.ends_with('0'),"StartedAt":"2026-10-06T00:00:00Z"},"Mounts":mounts,"Config":{"Env":["SECRET=private"]},"Labels":{"credential":"private"}});
                match case {
                    Case::MissingMounts => {
                        value.as_object_mut().unwrap().remove("Mounts");
                    }
                    Case::MissingSource => {
                        value["Mounts"][0].as_object_mut().unwrap().remove("Source");
                    }
                    Case::UnknownType => value["Mounts"][0]["Type"] = json!("unknown"),
                    Case::InvalidSource => value["Mounts"][0]["Source"] = json!("/mnt/../root"),
                    Case::MissingVolumeDriver if !id.ends_with('0') => {
                        value["Mounts"][0].as_object_mut().unwrap().remove("Driver");
                    }
                    Case::InspectChanges if *counts.get(&line).unwrap() > 1 => {
                        value["Mounts"][0]["Source"] = json!("/mnt/replaced")
                    }
                    _ => {}
                }
                value
            };
            if matches!(case, Case::Timeout) && line.contains("/info ") {
                tokio::time::sleep(Duration::from_secs(6)).await;
            }
            if matches!(case, Case::Oversized) && line.contains("/info ") {
                body["padding"] = json!("x".repeat(128 * 1024));
            }
            let body = body.to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
    Fixture {
        _dir: dir,
        docker: Docker::new(socket),
        calls,
        task,
    }
}
#[tokio::test]
async fn fresh_collection_includes_stopped_unmanaged_and_volume_consumers_without_secrets() {
    let server = fixture(Case::Valid).await;
    let value = server.docker.storage_inventory().await.unwrap();
    assert_eq!(value.containers.len(), 2);
    assert!(value.containers[0].container.running);
    assert!(!value.containers[1].container.running);
    assert_eq!(value.containers[0].mounts.len(), 1);
    assert!(
        matches!(&value.containers[1].mounts[0].source, ContainerStorageSource::Volume { driver, name, .. } if driver == "local" && name == "media")
    );
    let encoded = serde_json::to_string(&value).unwrap();
    assert!(
        !encoded.contains("SECRET") && !encoded.contains("Labels") && !encoded.contains("private")
    );
    let calls = server.calls.lock().unwrap();
    assert!(calls.iter().all(|call| call.starts_with("GET ")));
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.contains("/containers/json?all=1"))
            .count(),
        2
    );
    assert_eq!(
        calls.iter().filter(|call| call.contains("/json ")).count(),
        4
    );
}
#[tokio::test]
async fn changed_enumeration_mount_or_engine_identity_blocks_the_complete_result() {
    for case in [Case::InspectChanges, Case::ListChanges, Case::EngineChanges] {
        let server = fixture(case.clone()).await;
        assert_eq!(
            server.docker.storage_inventory().await.unwrap_err().0,
            ErrorCode::Conflict,
            "{case:?}"
        );
    }
}
#[tokio::test]
async fn malformed_missing_duplicate_and_unsupported_mount_evidence_is_unavailable() {
    for case in [
        Case::MissingMounts,
        Case::MissingSource,
        Case::UnknownType,
        Case::DuplicateIds,
        Case::DuplicateDestinations,
        Case::InvalidSource,
        Case::MissingVolumeDriver,
    ] {
        let server = fixture(case.clone()).await;
        assert_eq!(
            server.docker.storage_inventory().await.unwrap_err().0,
            ErrorCode::Unavailable,
            "{case:?}"
        );
    }
}
#[tokio::test]
async fn byte_container_and_mount_limits_refuse_instead_of_clipping() {
    for case in [
        Case::CountLimit,
        Case::MountLimit,
        Case::AggregateLimit,
        Case::Oversized,
    ] {
        let server = fixture(case.clone()).await;
        assert_eq!(
            server.docker.storage_inventory().await.unwrap_err().0,
            ErrorCode::Unavailable,
            "{case:?}"
        );
    }
}
#[tokio::test(start_paused = true)]
async fn a_stuck_engine_returns_no_inventory_at_the_read_deadline() {
    let server = fixture(Case::Timeout).await;
    assert_eq!(
        server.docker.storage_inventory().await.unwrap_err().0,
        ErrorCode::Unavailable
    );
}
