use super::*;
use axum::body::Body;
use http_body_util::BodyExt;
use limeos_contracts::*;
use std::sync::atomic::{AtomicBool, Ordering};
use tower::ServiceExt;
#[derive(Clone)]
struct ReadBackend {
    authorized: Arc<AtomicBool>,
    changes: tokio::sync::watch::Sender<u64>,
    size: usize,
}
impl Backend for ReadBackend {
    async fn container_storage_inventory(
        &self,
        _: String,
    ) -> Result<limeos_domain::ContainerStorageInventoryView> {
        if !self.authorized.load(Ordering::Relaxed) {
            return Err(Error(ErrorCode::Unauthenticated));
        }
        Ok(limeos_domain::ContainerStorageInventoryView {
            inventory: limeos_domain::ContainerStorageInventory {
                version: 1,
                engine_id: "fixture-engine".into(),
                observed_at: 100,
                containers: vec![],
            },
            digest: "a".repeat(64),
        })
    }
    async fn login(&self, _: Login) -> Result<IssuedSession> {
        Err(Error(ErrorCode::Unauthenticated))
    }
    async fn session(&self, _: String, _: Option<String>) -> Result<SessionView> {
        Err(Error(ErrorCode::Unauthenticated))
    }
    async fn logout(&self, _: String, _: String) -> Result<()> {
        Ok(())
    }
    async fn health(&self) -> Result<Health> {
        Ok(Health {
            version: 1,
            generation: 1,
            schema_version: 1,
            ready: true,
        })
    }
    async fn observations(&self, _: String) -> Result<Overview> {
        if !self.authorized.load(Ordering::Relaxed) {
            return Err(Error(ErrorCode::Unauthenticated));
        }
        Ok(Overview {
            revision: 7,
            generated_at: 100,
            host: None,
            sources: Vec::new(),
            resources: (0..self.size)
                .map(|i| {
                    Resource::new(
                        format!("container:{i:064}"),
                        ResourceKind::Container,
                        format!("container-{i}"),
                        "running".into(),
                        Source::Docker,
                    )
                })
                .collect(),
        })
    }
    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changes.subscribe()
    }
}
fn backend(size: usize) -> ReadBackend {
    ReadBackend {
        authorized: Arc::new(AtomicBool::new(true)),
        changes: tokio::sync::watch::channel(7).0,
        size,
    }
}
fn request(path: &str) -> Request {
    Request::builder()
        .uri(path)
        .header("cookie", format!("__Host-limeos={}", "a".repeat(64)))
        .body(Body::empty())
        .unwrap()
}
#[tokio::test]
async fn container_storage_read_requires_authentication_has_no_filters_and_rejects_post() {
    let b = backend(0);
    let app = router(b.clone(), "https://localhost".into());
    let path = "/api/v1/storage/container-dependencies";
    assert_eq!(
        app.clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.clone()
            .oneshot(request(&format!("{path}?paths=/mnt/data")))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    let response = app.clone().oneshot(request(path)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let view: limeos_domain::ContainerStorageInventoryView =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    assert!(view.inventory.containers.is_empty());
    b.authorized.store(false, Ordering::Relaxed);
    assert_eq!(
        app.oneshot(request(path)).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn reads_require_session_reject_bad_queries_and_paginate_with_revision() {
    let b = backend(25);
    let app = router(b.clone(), "https://localhost".into());
    for path in [
        "/api/v1/overview",
        "/api/v1/resources",
        "/api/v1/system/history?range=24h",
        "/api/v1/observations/stream",
    ] {
        assert_eq!(
            app.clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    for path in [
        "/api/v1/resources?limit=0",
        "/api/v1/resources?limit=51",
        "/api/v1/resources?offset=1025",
        "/api/v1/resources?principal=admin",
        "/api/v1/system/history?range=all",
    ] {
        assert_eq!(
            app.clone().oneshot(request(path)).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        app.clone()
            .oneshot(request("/api/v1/resources?revision=6"))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    let response = app
        .clone()
        .oneshot(request("/api/v1/resources?offset=20&revision=7"))
        .await
        .unwrap();
    let page: ResourcePage =
        serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap();
    assert_eq!(page.items.len(), 5);
    assert_eq!(page.total, 25);
    assert_eq!(page.next_offset, None);
    b.authorized.store(false, Ordering::Relaxed);
    assert_eq!(
        app.oneshot(request("/api/v1/overview"))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test(start_paused = true)]
async fn sse_reconnect_always_sends_snapshot_revocation_ends_stream_and_drop_releases_reader() {
    let b = backend(1);
    let app = router(b.clone(), "https://localhost".into());
    let mut req = request("/api/v1/observations/stream");
    req.headers_mut()
        .insert("last-event-id", HeaderValue::from_static("7"));
    let mut body = app.clone().oneshot(req).await.unwrap().into_body();
    let frame = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let text = String::from_utf8_lossy(&frame);
    assert!(
        text.contains("event: snapshot") && text.contains("id: 7") && text.contains("container-0")
    );
    b.authorized.store(false, Ordering::Relaxed);
    assert!(body.frame().await.is_none());
    drop(body);
    b.authorized.store(true, Ordering::Relaxed);
    let mut responses = Vec::new();
    for _ in 0..8 {
        let r = app
            .clone()
            .oneshot(request("/api/v1/observations/stream"))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        responses.push(r);
    }
    assert_eq!(
        app.clone()
            .oneshot(request("/api/v1/observations/stream"))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    responses.clear();
    assert_eq!(
        app.clone()
            .oneshot(request("/api/v1/observations/stream"))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let mut bad = request("/api/v1/observations/stream");
    bad.headers_mut()
        .insert("origin", HeaderValue::from_static("https://evil.invalid"));
    assert_eq!(
        app.oneshot(bad).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}
#[tokio::test(start_paused = true)]
async fn sse_has_byte_and_age_limits() {
    let app = router(backend(64), "https://localhost".into());
    let bytes = to_bytes(
        app.oneshot(request("/api/v1/observations/stream"))
            .await
            .unwrap()
            .into_body(),
        132000,
    )
    .await
    .unwrap();
    assert!(bytes.len() < 132000);
    assert!(!bytes.is_empty());
}
