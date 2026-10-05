use axum::{
    Router,
    body::to_bytes,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use limeos_contracts::{ErrorEnvelope, Health, Login, SessionView};
use limeos_domain::{Error, ErrorCode, Result};
use std::{
    collections::HashMap,
    future::Future,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{net::TcpListener, sync::Semaphore};
mod compose;
#[cfg(test)]
mod operation_tests;
mod operations;
#[cfg(test)]
mod read_tests;
mod reads;
mod storage;

pub struct IssuedSession {
    pub token: String,
    pub view: SessionView,
}
pub trait Backend: Clone + Send + Sync + 'static {
    fn storage_inventory(
        &self,
        _token: String,
    ) -> impl Future<Output = Result<limeos_domain::StorageInventoryView>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn plan_storage(
        &self,
        _token: String,
        _csrf: String,
        _input: limeos_domain::StorageSetupInput,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageSetup>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn storage_plan(
        &self,
        _token: String,
        _id: String,
    ) -> impl Future<Output = Result<limeos_domain::PlannedStorageSetup>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn approve_storage(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
        _digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn cancel_storage(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
    ) -> impl Future<Output = Result<()>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn plan_compose(
        &self,
        _token: String,
        _csrf: String,
        _selection: limeos_domain::ComposeSelection,
    ) -> impl Future<Output = Result<limeos_domain::PlannedCompose>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn compose_plan(
        &self,
        _token: String,
        _id: String,
    ) -> impl Future<Output = Result<limeos_domain::PlannedCompose>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn approve_compose(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
        _digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn cancel_compose(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
    ) -> impl Future<Output = Result<()>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn login(&self, request: Login) -> impl Future<Output = Result<IssuedSession>> + Send;
    fn session(
        &self,
        token: String,
        csrf: Option<String>,
    ) -> impl Future<Output = Result<SessionView>> + Send;
    fn logout(&self, token: String, csrf: String) -> impl Future<Output = Result<()>> + Send;
    fn health(&self) -> impl Future<Output = Result<Health>> + Send;
    fn observations(
        &self,
        _token: String,
    ) -> impl Future<Output = Result<limeos_contracts::Overview>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn history(
        &self,
        _token: String,
        _range: limeos_contracts::HistoryRange,
    ) -> impl Future<Output = Result<limeos_contracts::MetricHistory>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        tokio::sync::watch::channel(0).1
    }
    fn plan_restart(
        &self,
        _token: String,
        _csrf: String,
        _input: limeos_contracts::RestartInput,
    ) -> impl Future<Output = Result<limeos_domain::PlannedRestart>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn plan_container(
        &self,
        _token: String,
        _csrf: String,
        _input: limeos_contracts::ContainerInput,
    ) -> impl Future<Output = Result<limeos_domain::PlannedContainerAction>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn container_logs(
        &self,
        _token: String,
        _resource: String,
        _options: limeos_domain::LogOptions,
    ) -> impl Future<Output = Result<limeos_domain::ContainerLogs>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn approve_container(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
        _digest: String,
    ) -> impl Future<Output = Result<limeos_domain::PlanApproval>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn queue_container(
        &self,
        _token: String,
        _csrf: String,
        _input: limeos_contracts::QueueRestartInput,
    ) -> impl Future<Output = Result<limeos_domain::RestartJob>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn restart_progress(
        &self,
        _token: String,
        _id: String,
        _after: i64,
    ) -> impl Future<Output = Result<limeos_contracts::JobProgress>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn container_jobs(
        &self,
        _token: String,
    ) -> impl Future<Output = Result<Vec<limeos_domain::RestartJob>>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn cancel_restart(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
        _plan: bool,
    ) -> impl Future<Output = Result<()>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
}
#[derive(Clone)]
struct App<B> {
    backend: B,
    origin: Arc<str>,
    limiter: Arc<Mutex<Limiter>>,
    readers: Arc<Semaphore>,
}
#[derive(Clone, Copy)]
struct Peer(SocketAddr);
#[derive(Default)]
struct Limiter {
    peers: HashMap<IpAddr, (Instant, u32)>,
    global: Option<(Instant, u32)>,
}
impl Limiter {
    fn allow(&mut self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let window = Duration::from_secs(60);
        self.peers
            .retain(|_, (at, _)| now.duration_since(*at) < window);
        let global = self.global.get_or_insert((now, 0));
        if now.duration_since(global.0) >= window {
            *global = (now, 0);
        }
        if global.1 >= 120 || (!self.peers.contains_key(&ip) && self.peers.len() >= 1024) {
            return false;
        }
        let peer = self.peers.entry(ip).or_insert((now, 0));
        if peer.1 >= 10 {
            return false;
        }
        peer.1 += 1;
        global.1 += 1;
        true
    }
}
pub fn router<B: Backend>(backend: B, origin: String) -> Router {
    Router::new()
        .route("/api/v1/health", get(health::<B>))
        .route("/api/v1/storage/inventory", get(storage::inventory::<B>))
        .route("/api/v1/storage/plans", post(storage::plan::<B>))
        .route("/api/v1/storage/plans/{id}", get(storage::read::<B>))
        .route(
            "/api/v1/storage/plans/{id}/approval",
            post(storage::approve::<B>),
        )
        .route(
            "/api/v1/storage/plans/{id}/cancel",
            post(storage::cancel::<B>),
        )
        .route("/api/v1/auth/login", post(login::<B>))
        .route("/api/v1/auth/session", get(session::<B>))
        .route("/api/v1/auth/logout", post(logout::<B>))
        .route("/api/v1/overview", get(reads::overview::<B>))
        .route("/api/v1/resources", get(reads::resources::<B>))
        .route("/api/v1/system/history", get(reads::history::<B>))
        .route("/api/v1/observations/stream", get(reads::stream::<B>))
        .route("/api/v1/compose/plans", post(compose::plan::<B>))
        .route("/api/v1/compose/plans/{id}", get(compose::read::<B>))
        .route(
            "/api/v1/compose/plans/{id}/approval",
            post(compose::approve::<B>),
        )
        .route(
            "/api/v1/compose/plans/{id}/cancel",
            post(compose::cancel::<B>),
        )
        .route(
            "/api/v1/container/restart/plans",
            post(operations::plan::<B>),
        )
        .route(
            "/api/v1/container/plans",
            post(operations::container_plan::<B>),
        )
        .route(
            "/api/v1/container/plans/{id}/approval",
            post(operations::approve::<B>),
        )
        .route(
            "/api/v1/container/plans/{id}/cancel",
            post(operations::cancel_plan::<B>),
        )
        .route(
            "/api/v1/container/jobs",
            post(operations::container_queue::<B>).get(operations::jobs::<B>),
        )
        .route(
            "/api/v1/container/jobs/{id}",
            get(operations::progress::<B>),
        )
        .route(
            "/api/v1/container/jobs/{id}/cancel",
            post(operations::cancel_job::<B>),
        )
        .route("/api/v1/containers/{id}/logs", get(operations::logs::<B>))
        .route(
            "/api/v1/container/restart/plans/{id}/approval",
            post(operations::approve::<B>),
        )
        .route(
            "/api/v1/container/restart/plans/{id}/cancel",
            post(operations::cancel_plan::<B>),
        )
        .route(
            "/api/v1/container/restart/jobs",
            post(operations::queue::<B>).get(operations::jobs::<B>),
        )
        .route(
            "/api/v1/container/restart/jobs/{id}",
            get(operations::progress::<B>),
        )
        .route(
            "/api/v1/container/restart/jobs/{id}/cancel",
            post(operations::cancel_job::<B>),
        )
        .fallback(|| async { failure(Error(ErrorCode::NotFound)) })
        .layer(middleware::from_fn(limits))
        .with_state(App {
            backend,
            origin: origin.into(),
            limiter: Arc::new(Mutex::new(Limiter::default())),
            readers: Arc::new(Semaphore::new(8)),
        })
}
fn failure(error: Error) -> Response {
    let status = match error.0 {
        ErrorCode::InvalidInput => StatusCode::BAD_REQUEST,
        ErrorCode::Unauthenticated => StatusCode::UNAUTHORIZED,
        ErrorCode::Forbidden | ErrorCode::Expired => StatusCode::FORBIDDEN,
        ErrorCode::Conflict => StatusCode::CONFLICT,
        ErrorCode::NotFound => StatusCode::NOT_FOUND,
        ErrorCode::Overloaded => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    // Identifier correlates responses without including private request material.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    (
        status,
        axum::Json(ErrorEnvelope {
            code: error.0,
            message: error.message().into(),
            retry: matches!(error.0, ErrorCode::Overloaded | ErrorCode::Unavailable),
            audit_id: format!("http-{id}"),
        }),
    )
        .into_response()
}
async fn limits(request: Request, next: Next) -> Response {
    let header_bytes: usize = request
        .headers()
        .iter()
        .map(|(k, v)| k.as_str().len() + v.len())
        .sum();
    if header_bytes > 8192 {
        return failure(Error(ErrorCode::InvalidInput));
    }
    let mut response = match tokio::time::timeout(Duration::from_secs(10), next.run(request)).await
    {
        Ok(r) => r,
        Err(_) => failure(Error(ErrorCode::Unavailable)),
    };
    let headers = response.headers_mut();
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response
}
fn origin<B>(app: &App<B>, headers: &HeaderMap) -> Result<()> {
    if headers.get("origin").and_then(|v| v.to_str().ok()) != Some(&app.origin) {
        return Err(Error(ErrorCode::Forbidden));
    }
    if headers
        .get("sec-fetch-site")
        .is_some_and(|v| v != "same-origin" && v != "none")
    {
        return Err(Error(ErrorCode::Forbidden));
    }
    Ok(())
}
fn cookie(headers: &HeaderMap) -> Result<String> {
    let mut found = None;
    for value in headers.get_all("cookie") {
        for item in value
            .to_str()
            .map_err(|_| Error(ErrorCode::Unauthenticated))?
            .split(';')
        {
            if let Some(token) = item.trim().strip_prefix("__Host-limeos=") {
                if found.is_some()
                    || token.len() != 64
                    || !token.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err(Error(ErrorCode::Unauthenticated));
                }
                found = Some(token.to_owned());
            }
        }
    }
    found.ok_or(Error(ErrorCode::Unauthenticated))
}
async fn health<B: Backend>(State(app): State<App<B>>) -> Response {
    match app.backend.health().await {
        Ok(h) => axum::Json(h).into_response(),
        Err(e) => failure(e),
    }
}
async fn login<B: Backend>(State(app): State<App<B>>, request: Request) -> Response {
    if let Err(e) = origin(&app, request.headers()) {
        return failure(e);
    }
    let ip = request
        .extensions()
        .get::<Peer>()
        .map(|p| p.0.ip())
        .unwrap_or(IpAddr::from([127, 0, 0, 1]));
    if !app.limiter.lock().is_ok_and(|mut l| l.allow(ip)) {
        return failure(Error(ErrorCode::Overloaded));
    }
    if request
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .map(|h| h.split(';').next().unwrap_or("").trim())
        != Some("application/json")
    {
        return failure(Error(ErrorCode::InvalidInput));
    }
    let body = match to_bytes(request.into_body(), 16 * 1024).await {
        Ok(b) => b,
        Err(_) => return failure(Error(ErrorCode::InvalidInput)),
    };
    let input: Login = match serde_json::from_slice(&body) {
        Ok(i) => i,
        Err(_) => return failure(Error(ErrorCode::InvalidInput)),
    };
    match app.backend.login(input).await {
        Ok(issued) => {
            let value = format!(
                "__Host-limeos={}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=28800",
                issued.token
            );
            match HeaderValue::from_str(&value) {
                Ok(header) => ([("set-cookie", header)], axum::Json(issued.view)).into_response(),
                Err(_) => failure(Error(ErrorCode::Unavailable)),
            }
        }
        Err(e) => failure(e),
    }
}
async fn session<B: Backend>(State(app): State<App<B>>, headers: HeaderMap) -> Response {
    let token = match cookie(&headers) {
        Ok(t) => t,
        Err(e) => return failure(e),
    };
    match app.backend.session(token, None).await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => failure(e),
    }
}
async fn logout<B: Backend>(State(app): State<App<B>>, headers: HeaderMap) -> Response {
    if let Err(e) = origin(&app, &headers) {
        return failure(e);
    }
    let token = match cookie(&headers) {
        Ok(t) => t,
        Err(e) => return failure(e),
    };
    let Some(csrf) = headers
        .get("x-csrf-token")
        .and_then(|h| h.to_str().ok())
        .filter(|s| s.len() == 64)
    else {
        return failure(Error(ErrorCode::Forbidden));
    };
    match app.backend.logout(token, csrf.into()).await {
        Ok(()) => (
            StatusCode::NO_CONTENT,
            [(
                "set-cookie",
                "__Host-limeos=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0",
            )],
        )
            .into_response(),
        Err(e) => failure(e),
    }
}

/// All connections have bounded parsing, occupancy, and lifetime. Hyper's timer
/// covers incomplete headers; the application deadline covers bodies and work.
pub async fn serve(listener: TcpListener, router: Router) -> std::io::Result<()> {
    let permits = Arc::new(Semaphore::new(64));
    loop {
        let (stream, peer) = listener.accept().await?;
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            continue;
        };
        let app = router.clone().layer(axum::Extension(Peer(peer)));
        tokio::spawn(async move {
            let _permit = permit;
            let io = hyper_util::rt::TokioIo::new(stream);
            let service = hyper_util::service::TowerToHyperService::new(app);
            let mut builder = hyper::server::conn::http1::Builder::new();
            builder
                .timer(hyper_util::rt::TokioTimer::new())
                .header_read_timeout(Duration::from_secs(5))
                .max_headers(32)
                .max_buf_size(16 * 1024)
                .keep_alive(true);
            let _ = tokio::time::timeout(
                Duration::from_secs(30),
                builder.serve_connection(io, service),
            )
            .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use limeos_contracts::VERSION;
    use tower::ServiceExt;
    #[derive(Clone)]
    struct Fake;
    impl Backend for Fake {
        async fn login(&self, _: Login) -> Result<IssuedSession> {
            Err(Error(ErrorCode::Unauthenticated))
        }
        async fn session(&self, _: String, _: Option<String>) -> Result<SessionView> {
            Err(Error(ErrorCode::Unauthenticated))
        }
        async fn logout(&self, _: String, _: String) -> Result<()> {
            panic!("unvalidated mutation")
        }
        async fn health(&self) -> Result<Health> {
            Ok(Health {
                version: VERSION,
                generation: 1,
                schema_version: 1,
                ready: true,
            })
        }
    }
    #[tokio::test]
    async fn no_default_login_cross_site_or_get_mutation() {
        let app = router(Fake, "https://localhost".into());
        for (method, path, status) in [
            ("GET", "/api/v1/auth/logout", 405),
            ("POST", "/api/v1/auth/logout", 403),
            ("POST", "/api/v1/auth/login", 403),
            ("GET", "/api/v1/auth/session", 401),
        ] {
            let r = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(r.status().as_u16(), status);
        }
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/auth/login")
            .header("origin", "https://localhost")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"username":"admin","password":"pihealth"}"#))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("pihealth"));
    }
    #[tokio::test]
    async fn malformed_and_oversized_body_and_rate_limit() {
        let app = router(Fake, "https://localhost".into());
        for body in ["{bad".to_owned(), "x".repeat(20 * 1024)] {
            let r = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/auth/login")
                        .header("origin", "https://localhost")
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::BAD_REQUEST);
        }
        for _ in 0..8 {
            let _ = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/auth/login")
                        .header("origin", "https://localhost")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await;
        }
        let r = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header("origin", "https://localhost")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    }
}
