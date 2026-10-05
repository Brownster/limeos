//! Selected Docker Engine API over a fixed Unix socket. No automatic POST retry.
use crate::Engine;
use http_body_util::{BodyExt, Empty};
use hyper::{Request, StatusCode, body::Bytes};
use limeos_domain::{ContainerSnapshot, Error, ErrorCode, Result, opaque_id};
use serde_json::Value;
use std::{path::PathBuf, time::Duration};
use tokio::net::UnixStream;
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
    async fn request(
        &self,
        method: &str,
        path: String,
        limit: usize,
    ) -> Result<(StatusCode, Vec<u8>)> {
        let stream = UnixStream::connect(&self.socket)
            .await
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        let (mut sender, connection) =
            hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream))
                .await
                .map_err(|_| Error(ErrorCode::Unavailable))?;
        // A dropped request must also end its connection task. In particular a
        // timeout cannot leave an unbounded set of stuck Engine connections.
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });
        struct ConnectionTask(tokio::task::JoinHandle<()>);
        impl Drop for ConnectionTask {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let _connection = ConnectionTask(connection);
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
        let status = response.status();
        let mut body = response.into_body();
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
        Ok((status, bytes))
    }
    async fn json(&self, path: String, limit: usize) -> Result<Value> {
        tokio::time::timeout(Duration::from_secs(2), async {
            let (status, bytes) = self.request("GET", path, limit).await?;
            if status != StatusCode::OK {
                return Err(Error(ErrorCode::Unavailable));
            }
            serde_json::from_slice(&bytes).map_err(|_| Error(ErrorCode::Unavailable))
        })
        .await
        .map_err(|_| Error(ErrorCode::Unavailable))?
    }
    async fn version(&self) -> Result<String> {
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
}
impl Engine for Docker {
    async fn inspect(&self, id: &str) -> Result<ContainerSnapshot> {
        if !opaque_id(id) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let version = self.version().await?;
        let value = self
            .json(format!("/{version}/containers/{id}/json"), 512 * 1024)
            .await?;
        // Reject an inconsistent identity; never leak Config.Env, labels or paths.
        if value["Id"].as_str() != Some(id) {
            return Err(Error(ErrorCode::Unavailable));
        }
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
    async fn restart(&self, id: &str) -> Result<()> {
        if !opaque_id(id) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        // Negotiate before POST. A lost/failed response is uncertainty, never a
        // reason to issue a second restart. The 10-second grace is server owned.
        let version = self.version().await?;
        let (status, _) = self
            .request(
                "POST",
                format!("/{version}/containers/{id}/restart?t=10"),
                8192,
            )
            .await?;
        if status != StatusCode::NO_CONTENT {
            return Err(Error(ErrorCode::Unavailable));
        }
        Ok(())
    }
}
