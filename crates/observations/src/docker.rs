//! GET-only Engine adapter; selected metadata prevents environment/label leakage.
use crate::probes::text;
use http_body_util::{BodyExt, Empty};
use hyper::{Request, body::Bytes};
use limeos_contracts::{Resource, ResourceKind, Source};
use limeos_domain::{Error, ErrorCode, Result};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf, time::Duration};
use tokio::{net::UnixStream, sync::Notify};

#[derive(Clone)]
pub(crate) struct Docker {
    pub socket: PathBuf,
}
impl Docker {
    async fn response(&self, path: String) -> Result<hyper::Response<hyper::body::Incoming>> {
        let stream = UnixStream::connect(&self.socket)
            .await
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        let (mut sender, connection) =
            hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream))
                .await
                .map_err(|_| Error(ErrorCode::Unavailable))?;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let request = Request::builder()
            .method("GET")
            .uri(path)
            .header("Host", "localhost")
            .header("Accept", "application/json")
            .body(Empty::<Bytes>::new())
            .map_err(|_| Error(ErrorCode::InvalidInput))?;
        let response = sender
            .send_request(request)
            .await
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        if !response.status().is_success() {
            return Err(Error(ErrorCode::Unavailable));
        }
        Ok(response)
    }
    async fn json(&self, path: String, limit: usize) -> Result<Value> {
        tokio::time::timeout(Duration::from_secs(2), async {
            let mut body = self.response(path).await?.into_body();
            let mut bytes = Vec::new();
            while let Some(frame) = body.frame().await {
                if let Ok(data) = frame
                    .map_err(|_| Error(ErrorCode::Unavailable))?
                    .into_data()
                {
                    if bytes.len() + data.len() > limit {
                        return Err(Error(ErrorCode::Unavailable));
                    }
                    bytes.extend_from_slice(&data);
                }
            }
            serde_json::from_slice(&bytes).map_err(|_| Error(ErrorCode::Unavailable))
        })
        .await
        .map_err(|_| Error(ErrorCode::Unavailable))?
    }
    pub async fn version(&self) -> Result<String> {
        let value = self.json("/version".into(), 8192).await?;
        let parse = |s: &str| s.strip_prefix("1.")?.parse::<u32>().ok();
        let max = parse(value["ApiVersion"].as_str().unwrap_or_default())
            .ok_or(Error(ErrorCode::Unavailable))?
            .min(56);
        let min = parse(value["MinAPIVersion"].as_str().unwrap_or("1.24"))
            .ok_or(Error(ErrorCode::Unavailable))?;
        if max < 41 || min > max {
            return Err(Error(ErrorCode::Unavailable));
        }
        Ok(format!("v1.{max}"))
    }
    pub async fn inventory(&self, version: &str) -> Result<Vec<Resource>> {
        containers(
            &self
                .json(format!("/{version}/containers/json?all=true"), 1024 * 1024)
                .await?,
        )
    }
    pub async fn stats(&self, version: &str, id: &str) -> Result<Value> {
        if id.len() != 64 || !id.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        self.json(
            format!("/{version}/containers/{id}/stats?stream=false&one-shot=true"),
            65536,
        )
        .await
    }
    pub async fn events(&self, notify: &Notify) -> Result<()> {
        let version = self.version().await?;
        let mut body = tokio::time::timeout(
            Duration::from_secs(3),
            self.response(format!(
                "/{version}/events?filters=%7B%22type%22%3A%5B%22container%22%5D%7D"
            )),
        )
        .await
        .map_err(|_| Error(ErrorCode::Unavailable))??
        .into_body();
        // Finite stream lifetime and byte/line bounds; reconnect always reconciles.
        tokio::time::timeout(Duration::from_secs(60), async {
            let mut pending = Vec::new();
            let mut total = 0usize;
            while let Some(frame) = body.frame().await {
                if let Ok(data) = frame
                    .map_err(|_| Error(ErrorCode::Unavailable))?
                    .into_data()
                {
                    total += data.len();
                    if total > 1024 * 1024 {
                        return Err(Error(ErrorCode::Unavailable));
                    }
                    for byte in data {
                        if byte == b'\n' {
                            serde_json::from_slice::<Value>(&pending)
                                .map_err(|_| Error(ErrorCode::Unavailable))?;
                            pending.clear();
                            notify.notify_one();
                        } else {
                            pending.push(byte);
                            if pending.len() > 16384 {
                                return Err(Error(ErrorCode::Unavailable));
                            }
                        }
                    }
                }
            }
            Ok(())
        })
        .await
        .map_err(|_| Error(ErrorCode::Unavailable))?
    }
}
pub(crate) fn containers(value: &Value) -> Result<Vec<Resource>> {
    let list = value.as_array().ok_or(Error(ErrorCode::Unavailable))?;
    if list.len() > 64 {
        return Err(Error(ErrorCode::Unavailable));
    }
    let mut result = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for c in list {
        let id = text(&c["Id"], 65);
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) || !ids.insert(id.clone()) {
            return Err(Error(ErrorCode::Unavailable));
        }
        let name = c["Names"]
            .as_array()
            .and_then(|a| a.first())
            .map(|v| text(v, 80))
            .unwrap_or_else(|| id[..12].into())
            .trim_start_matches('/')
            .to_owned();
        let state = text(&c["State"], 32);
        if !matches!(
            state.as_str(),
            "created" | "running" | "paused" | "restarting" | "removing" | "exited" | "dead"
        ) {
            return Err(Error(ErrorCode::Unavailable));
        }
        let mut resource = Resource::new(
            format!("container:{id}"),
            ResourceKind::Container,
            name,
            state,
            Source::Docker,
        );
        resource.image = Some(text(&c["Image"], 128));
        let stack = text(&c["Labels"]["com.docker.compose.project"], 64);
        resource.parent = (!stack.is_empty()).then(|| format!("stack:{stack}"));
        let status = text(&c["Status"], 256);
        resource.health = if status.contains("(unhealthy)") {
            Some("unhealthy".into())
        } else if status.contains("(healthy)") {
            Some("healthy".into())
        } else if status.contains("health: starting") {
            Some("starting".into())
        } else {
            None
        };
        result.push(resource);
    }
    result.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(result)
}
pub(crate) fn stacks(containers: &[Resource]) -> Vec<Resource> {
    let mut groups = BTreeMap::<String, Vec<&Resource>>::new();
    for c in containers {
        if let Some(parent) = &c.parent {
            groups.entry(parent.clone()).or_default().push(c);
        }
    }
    groups
        .into_iter()
        .map(|(id, children)| {
            let state = if children
                .iter()
                .all(|c| c.status == "running" && c.health.as_deref() != Some("unhealthy"))
            {
                "running"
            } else if children
                .iter()
                .all(|c| matches!(c.status.as_str(), "exited" | "created"))
            {
                "stopped"
            } else {
                "degraded"
            };
            Resource::new(
                id.clone(),
                ResourceKind::Stack,
                id.trim_start_matches("stack:").into(),
                state.into(),
                Source::Docker,
            )
        })
        .collect()
}
pub(crate) fn counters(v: &Value) -> Option<(u64, u64, u64)> {
    Some((
        v["cpu_stats"]["cpu_usage"]["total_usage"].as_u64()?,
        v["cpu_stats"]["system_cpu_usage"].as_u64()?,
        v["cpu_stats"]["online_cpus"].as_u64()?,
    ))
}
pub(crate) fn stats(resource: &mut Resource, v: &Value, previous: Option<(u64, u64, u64)>) {
    if let (Some(p), Some(c)) = (previous, counters(v)) {
        resource.cpu_percent =
            c.0.checked_sub(p.0)
                .zip(c.1.checked_sub(p.1))
                .filter(|(_, system)| *system > 0)
                .map(|(cpu, system)| 100.0 * cpu as f64 / system as f64 * c.2 as f64)
                .filter(|v| v.is_finite());
    }
    let mem = &v["memory_stats"];
    resource.memory_percent =
        mem["usage"]
            .as_u64()
            .zip(mem["limit"].as_u64())
            .and_then(|(usage, limit)| {
                if limit == 0 {
                    return None;
                }
                let cache = mem["stats"]["inactive_file"]
                    .as_u64()
                    .or_else(|| mem["stats"]["total_inactive_file"].as_u64())
                    .unwrap_or(0);
                let used = if cache < usage { usage - cache } else { usage };
                Some(100.0 * used as f64 / limit as f64)
            });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inventory_only_exposes_selected_metadata_and_stack_health() {
        let v = serde_json::json!([{"Id":"a".repeat(64),"Names":["/jellyfin"],"State":"running","Status":"Up (unhealthy)","Image":"jellyfin:stable","Labels":{"com.docker.compose.project":"media","secret":"do-not-export"},"Env":["TOKEN=secret"]}]);
        let resources = containers(&v).unwrap();
        assert_eq!(resources[0].id, format!("container:{}", "a".repeat(64)));
        assert_eq!(stacks(&resources)[0].status, "degraded");
        assert!(
            !serde_json::to_string(&resources)
                .unwrap()
                .contains("secret")
        );
        assert!(containers(&serde_json::json!([{"Id":"bad","State":"running"}])).is_err());
    }
    #[test]
    fn missing_cgroups_and_counter_resets_are_unknown() {
        let mut r = Resource::new(
            "a".into(),
            ResourceKind::Container,
            "a".into(),
            "running".into(),
            Source::Docker,
        );
        stats(&mut r, &serde_json::json!({}), None);
        assert!(r.cpu_percent.is_none() && r.memory_percent.is_none());
        let v = serde_json::json!({"cpu_stats":{"cpu_usage":{"total_usage":300},"system_cpu_usage":1000,"online_cpus":4},"memory_stats":{"usage":60,"limit":100,"stats":{"inactive_file":10}}});
        stats(&mut r, &v, Some((200, 600, 4)));
        assert_eq!(r.cpu_percent, Some(100.0));
        assert_eq!(r.memory_percent, Some(50.0));
        stats(&mut r, &v, Some((400, 1200, 4)));
        assert_eq!(r.cpu_percent, None);
    }
    #[test]
    fn frozen_reference_container_fixture_agrees_on_full_identity_and_status() {
        let v: Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/read-semantics.json"))
                .unwrap();
        let r = containers(&v["docker"]).unwrap();
        for (a, b) in r
            .iter()
            .zip(v["expected"]["containers"].as_array().unwrap())
        {
            assert!(
                a.id.trim_start_matches("container:")
                    .starts_with(b["id_prefix"].as_str().unwrap())
            );
            assert_eq!(a.name, b["name"].as_str().unwrap());
            assert_eq!(a.status, b["status"].as_str().unwrap());
            assert_eq!(a.image.as_deref(), b["image"].as_str());
            assert_eq!(a.health.as_deref(), b["health"].as_str());
            assert_eq!(
                a.parent.as_deref(),
                Some(format!("stack:{}", b["stack"].as_str().unwrap()).as_str())
            );
        }
    }
    #[tokio::test]
    async fn docker_output_interruption_overflow_and_deadlines_are_bounded() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for mode in ["overflow", "interrupted", "slow"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("docker.sock");
            let server = tokio::net::UnixListener::bind(&path).unwrap();
            let task = tokio::spawn(async move {
                let (mut socket, _) = server.accept().await.unwrap();
                let mut request = [0; 3];
                socket.read_exact(&mut request).await.unwrap();
                assert_eq!(&request, b"GET");
                match mode {
                    "overflow" => {
                        let data = "x".repeat(2000);
                        socket
                            .write_all(
                                format!("HTTP/1.1 200 OK\r\nContent-Length: 2000\r\n\r\n{data}")
                                    .as_bytes(),
                            )
                            .await
                            .unwrap();
                    }
                    "interrupted" => {
                        socket
                            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2000\r\n\r\n{")
                            .await
                            .unwrap();
                    }
                    _ => {
                        tokio::time::sleep(Duration::from_secs(4)).await;
                    }
                }
            });
            let a = Docker { socket: path };
            let started = std::time::Instant::now();
            assert!(a.json("/version".into(), 1024).await.is_err());
            assert!(started.elapsed() < Duration::from_secs(3));
            task.abort();
        }
    }
}
