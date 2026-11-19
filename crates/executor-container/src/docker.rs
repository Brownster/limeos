//! Selected Docker Engine API over a fixed Unix socket. No automatic POST retry.
use crate::Engine;
use http_body_util::{BodyExt, Empty};
use hyper::{Request, StatusCode, body::Bytes};
use limeos_domain::{ContainerAction, ContainerSnapshot, Error, ErrorCode, Result, opaque_id};
use serde_json::Value;
use std::{path::PathBuf, time::Duration};
use tokio::net::UnixStream;
pub mod compose_evidence;
pub mod health_evidence;
pub mod process_evidence;
mod read_admission;
mod storage;
#[cfg(test)]
mod tests;

#[derive(Clone)]
pub struct Docker {
    socket: PathBuf,
}
impl Docker {
    pub fn new(socket: PathBuf) -> Self {
        Self { socket }
    }
    pub async fn logs(
        &self,
        resource: &str,
        options: limeos_domain::LogOptions,
    ) -> Result<limeos_domain::ContainerLogs> {
        options.validate()?;
        let id = resource.strip_prefix("container:").unwrap_or_default();
        if !opaque_id(id) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        tokio::time::timeout(Duration::from_secs(4), async {
            let version = self.version().await?;
            let inspect = self.json(format!("/{version}/containers/{id}/json"), 512 * 1024).await?;
            if inspect["Id"].as_str() != Some(id) {return Err(Error(ErrorCode::Conflict));}
            let tty = inspect["Config"]["Tty"].as_bool().ok_or(Error(ErrorCode::Unavailable))?;
            // Read the whole bounded-time response; keep only its newest output.
            let mut exchange = self.send("GET", format!("/{version}/containers/{id}/logs?stdout=1&stderr=1&follow=0&tail={}&timestamps=1", options.tail), None).await?;
            if exchange.status != StatusCode::OK {return Err(Error(ErrorCode::Unavailable));}
            let mut tail = crate::logs::Tail::new(tty);
            while let Some(data) = exchange.data().await? {tail.push(&data)?;}
            tail.finish(resource.into())
        }).await.map_err(|_| Error(ErrorCode::Unavailable))?
    }
    async fn request(
        &self,
        method: &str,
        path: String,
        limit: usize,
    ) -> Result<(StatusCode, Vec<u8>)> {
        self.request_inner(method, path, limit, None).await
    }
    async fn request_inner(
        &self,
        method: &str,
        path: String,
        limit: usize,
        authenticated: Option<(
            &process_evidence::AuthenticatedEngine,
            &process_evidence::ResponseBudget,
        )>,
    ) -> Result<(StatusCode, Vec<u8>)> {
        let mut exchange = self
            .send(method, path, authenticated.map(|(peer, _)| peer))
            .await?;
        let mut bytes = Vec::new();
        while let Some(data) = exchange.data().await? {
            if let Some((_, budget)) = authenticated {
                budget.consume(data.len())?;
            }
            if bytes.len() + data.len() > limit {
                return Err(Error(ErrorCode::Unavailable));
            }
            if let Some((_, budget)) = authenticated {
                if budget.constrained() {
                    bytes
                        .try_reserve_exact(data.len())
                        .map_err(|_| Error(ErrorCode::Unavailable))?;
                }
            }
            bytes.extend_from_slice(&data);
        }
        if let Some((peer, _)) = authenticated {
            peer.revalidate_socket()?;
        }
        Ok((exchange.status, bytes))
    }
    async fn send(
        &self,
        method: &str,
        path: String,
        authenticated: Option<&process_evidence::AuthenticatedEngine>,
    ) -> Result<Exchange> {
        if let Some(peer) = authenticated {
            peer.revalidate_socket()?;
        }
        let stream = UnixStream::connect(&self.socket)
            .await
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        if let Some(peer) = authenticated {
            peer.authenticate(&stream)?;
            peer.revalidate_socket()?;
        }
        let mut builder = hyper::client::conn::http1::Builder::new();
        if authenticated.is_some() {
            builder.max_buf_size(32 * 1024).max_headers(64);
        }
        let (mut sender, connection) = builder
            .handshake(hyper_util::rt::TokioIo::new(stream))
            .await
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        // A dropped request must also end its connection task. In particular a
        // timeout cannot leave an unbounded set of stuck Engine connections.
        let connection = ConnectionTask(tokio::spawn(async move {
            let _ = connection.await;
        }));
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("Host", "localhost")
            .header("Accept", "application/json")
            .body(Empty::<Bytes>::new())
            .map_err(|_| Error(ErrorCode::InvalidInput))?;
        let response = sender
            .send_request(request)
            .await
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        Ok(Exchange {
            status: response.status(),
            body: response.into_body(),
            _connection: connection,
        })
    }
    async fn json(&self, path: String, limit: usize) -> Result<Value> {
        self.json_with(path, limit, |_| Error(ErrorCode::Unavailable))
            .await
    }
    /// A GET naming one container, where the Engine's 404 and 409 mean that
    /// container is gone or in conflict. Every other path keeps `json`.
    async fn container_json(&self, path: String, limit: usize) -> Result<Value> {
        self.json_with(path, limit, container_status).await
    }
    async fn json_with(
        &self,
        path: String,
        limit: usize,
        refused: fn(StatusCode) -> Error,
    ) -> Result<Value> {
        tokio::time::timeout(Duration::from_secs(2), async {
            let (status, bytes) = self.request("GET", path, limit).await?;
            if status != StatusCode::OK {
                return Err(refused(status));
            }
            serde_json::from_slice(&bytes).map_err(|_| Error(ErrorCode::Unavailable))
        })
        .await
        .map_err(|_| Error(ErrorCode::Unavailable))?
    }
    async fn version(&self) -> Result<String> {
        let value = self.json("/version".into(), 8192).await?;
        parse_api_version(&value)
    }
    async fn effect(&self, id: &str, operation: ContainerAction) -> Result<()> {
        if !opaque_id(id) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let version = self.version().await?;
        let query = if operation == ContainerAction::Start {
            ""
        } else {
            "?t=10"
        };
        let (status, _) = self
            .request(
                "POST",
                format!("/{version}/containers/{id}/{}{query}", operation.as_str()),
                8192,
            )
            .await?;
        match status {
            StatusCode::NO_CONTENT => Ok(()),
            // Start of a running or stop of a stopped container. The effect's
            // intended state already holds; fresh verification still decides.
            StatusCode::NOT_MODIFIED if operation != ContainerAction::Restart => Ok(()),
            other => Err(container_status(other)),
        }
    }
}
/// The Engine's documented refusals for a request naming one container.
fn container_status(status: StatusCode) -> Error {
    match status {
        StatusCode::NOT_FOUND => Error(ErrorCode::NotFound),
        StatusCode::CONFLICT => Error(ErrorCode::Conflict),
        _ => Error(ErrorCode::Unavailable),
    }
}
struct ConnectionTask(tokio::task::JoinHandle<()>);
impl Drop for ConnectionTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}
/// One response whose body is read as it arrives. Dropping it ends the
/// connection task.
struct Exchange {
    status: StatusCode,
    body: hyper::body::Incoming,
    _connection: ConnectionTask,
}
impl Exchange {
    async fn data(&mut self) -> Result<Option<Bytes>> {
        while let Some(frame) = self.body.frame().await {
            if let Ok(data) = frame
                .map_err(|_| Error(ErrorCode::Unavailable))?
                .into_data()
            {
                return Ok(Some(data));
            }
        }
        Ok(None)
    }
}
fn parse_api_version(value: &Value) -> Result<String> {
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
impl Docker {
    async fn inspect_value(&self, version: &str, id: &str) -> Result<Value> {
        if !opaque_id(id) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let value = self
            .container_json(format!("/{version}/containers/{id}/json"), 512 * 1024)
            .await?;
        // Reject an inconsistent identity; never leak Config.Env, labels or paths.
        if value["Id"].as_str() != Some(id) {
            return Err(Error(ErrorCode::Unavailable));
        }
        Ok(value)
    }
}
fn selected_snapshot(id: &str, value: &Value) -> Result<ContainerSnapshot> {
    let snapshot = ContainerSnapshot {
        resource: format!("container:{id}"),
        image: value["Image"]
            .as_str()
            .ok_or(Error(ErrorCode::Unavailable))?
            .into(),
        started_at: value["State"]["StartedAt"]
            .as_str()
            .ok_or(Error(ErrorCode::Unavailable))?
            .into(),
        running: value["State"]["Running"]
            .as_bool()
            .ok_or(Error(ErrorCode::Unavailable))?,
    };
    snapshot
        .validate()
        .map_err(|_| Error(ErrorCode::Unavailable))?;
    Ok(snapshot)
}
impl Engine for Docker {
    async fn inspect(&self, id: &str) -> Result<ContainerSnapshot> {
        if !opaque_id(id) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let version = self.version().await?;
        selected_snapshot(id, &self.inspect_value(&version, id).await?)
    }
    /// Engine-declared project label only; never a Compose file. One-off
    /// containers are excluded. More than the stack bound is refused whole.
    async fn project_members(&self, project: &str) -> Result<Vec<ContainerSnapshot>> {
        if !limeos_domain::stack_project(project) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let version = self.version().await?;
        let filter =
            format!("%7B%22label%22%3A%5B%22com.docker.compose.project%3D{project}%22%5D%7D");
        let listed = self
            .json(
                format!("/{version}/containers/json?all=1&filters={filter}"),
                256 * 1024,
            )
            .await?;
        let rows = listed.as_array().ok_or(Error(ErrorCode::Unavailable))?;
        if rows.len() > limeos_domain::deployment::discovery::MAX_ENGINE_CONTAINERS {
            return Err(Error(ErrorCode::Unavailable));
        }
        let mut members = Vec::new();
        for row in rows {
            let id = row["Id"].as_str().ok_or(Error(ErrorCode::Unavailable))?;
            // A member removed between listing and inspection is a changing
            // project, not a missing one: keep the listing's refusal.
            let value = self
                .inspect_value(&version, id)
                .await
                .map_err(|e| match e.0 {
                    ErrorCode::NotFound | ErrorCode::Conflict => Error(ErrorCode::Unavailable),
                    _ => e,
                })?;
            let labels = &value["Config"]["Labels"];
            if labels["com.docker.compose.project"].as_str() != Some(project) {
                return Err(Error(ErrorCode::Unavailable));
            }
            match labels["com.docker.compose.oneoff"].as_str() {
                Some("True") => continue,
                Some("False") => {}
                _ => return Err(Error(ErrorCode::Unavailable)),
            }
            members.push(selected_snapshot(id, &value)?);
        }
        if members.len() > limeos_domain::MAX_STACK_MEMBERS {
            return Err(Error(ErrorCode::Unavailable));
        }
        members.sort_by(|a, b| a.resource.cmp(&b.resource));
        Ok(members)
    }
    async fn restart(&self, id: &str) -> Result<()> {
        // Negotiate before POST. A lost/failed response is uncertainty, never a
        // reason to issue a second restart. The 10-second grace is server owned.
        self.effect(id, ContainerAction::Restart).await
    }
    async fn start(&self, id: &str) -> Result<()> {
        self.effect(id, ContainerAction::Start).await
    }
    async fn stop(&self, id: &str) -> Result<()> {
        self.effect(id, ContainerAction::Stop).await
    }
}
