use super::*;
use axum::body::Body;
use limeos_domain::deployment::{discovery::ProjectIdentity, networks as model};
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

fn wire_report() -> model::NetworkInventory {
    let project = ProjectIdentity {
        name: "media".into(),
        working_directory: "/opt/stacks/Media".into(),
        config_files: vec!["/opt/stacks/Media/compose.yaml".into()],
    };
    let node = |id: char,
                name: &str,
                service: &str,
                running,
                health,
                mode: String,
                one_off,
                hint: Option<String>| model::Node {
        id: id.to_string().repeat(64),
        name: name.into(),
        running,
        health,
        network_mode: Some(mode),
        dependency_hint: hint,
        compose: Some(model::ComposeMember {
            project: project.clone(),
            service: service.into(),
            one_off,
        }),
    };
    let nodes = vec![
        node(
            'a',
            "vpn",
            "vpn",
            true,
            model::Health::NotConfigured,
            "bridge".into(),
            false,
            None,
        ),
        node(
            'b',
            "sonarr",
            "sonarr",
            true,
            model::Health::Healthy,
            format!("container:{}", "a".repeat(64)),
            false,
            None,
        ),
        node(
            'c',
            "transmission",
            "transmission",
            false,
            model::Health::Unknown,
            format!("container:{}", "e".repeat(64)),
            false,
            Some("vpn".into()),
        ),
        node(
            'd',
            "sonarr-oneoff",
            "sonarr",
            false,
            model::Health::NotConfigured,
            format!("container:{}", "a".repeat(64)),
            true,
            None,
        ),
    ];
    model::observe(
        105,
        "engine-a",
        Some(model::NetworkInput {
            engine_id: "engine-a",
            sampled_at: 100,
            completeness: model::Completeness::Complete,
            nodes: &nodes,
        }),
    )
    .unwrap()
}

#[derive(Clone, Default)]
struct Stub {
    calls: Arc<AtomicUsize>,
    session_error: Option<ErrorCode>,
    cache_error: Option<ErrorCode>,
    oversize: bool,
    source_missing: bool,
}
impl Backend for Stub {
    async fn login(&self, _: Login) -> Result<IssuedSession> {
        Err(Error(ErrorCode::Unauthenticated))
    }
    async fn logout(&self, _: String, _: String) -> Result<()> {
        panic!("network GET cannot mutate")
    }
    async fn health(&self) -> Result<Health> {
        panic!("network GET cannot collect")
    }
    async fn session(&self, _: String, _: Option<String>) -> Result<SessionView> {
        if let Some(code) = self.session_error {
            return Err(Error(code));
        }
        Ok(SessionView {
            principal: limeos_domain::Principal {
                id: "viewer".into(),
                role: limeos_domain::Role::Viewer,
                grant_revision: 1,
            },
            csrf_token: "b".repeat(64),
            expires_at: 9999,
        })
    }
}
impl DeploymentBackend for Stub {
    async fn network_inventory(&self, _: String) -> Result<model::NetworkInventory> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(code) = self.cache_error {
            return Err(Error(code));
        }
        if self.source_missing {
            return model::observe(105, "engine-a", None);
        }
        let mut report = wire_report();
        if self.oversize {
            report.groups[0].provider.name = "x".repeat(70_000);
        }
        Ok(report)
    }
}
#[derive(Clone, Default)]
struct Unregistered(Stub);
impl Backend for Unregistered {
    async fn login(&self, input: Login) -> Result<IssuedSession> {
        self.0.login(input).await
    }
    async fn logout(&self, token: String, csrf: String) -> Result<()> {
        self.0.logout(token, csrf).await
    }
    async fn health(&self) -> Result<Health> {
        self.0.health().await
    }
    async fn session(&self, token: String, csrf: Option<String>) -> Result<SessionView> {
        self.0.session(token, csrf).await
    }
}
impl DeploymentBackend for Unregistered {}
fn app<B: DeploymentBackend>(backend: B) -> Router {
    routes().with_state(App {
        backend,
        origin: "https://localhost".into(),
        limiter: Arc::new(Mutex::new(Limiter::default())),
        readers: Arc::new(Semaphore::new(2)),
    })
}
fn request(method: &str, cookie: bool) -> Request {
    let mut request = Request::builder()
        .method(method)
        .uri("/api/v1/deployment/networks");
    if cookie {
        request = request.header("cookie", format!("__Host-limeos={}", "a".repeat(64)));
    }
    request.body(Body::empty()).unwrap()
}
async fn json(response: Response) -> serde_json::Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 65_536).await.unwrap()).unwrap()
}
#[test]
fn network_wire_fixture_matches_the_domain_projection() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/deployment/networks/wire.json"
    ))
    .unwrap();
    assert_eq!(serde_json::to_value(wire_report()).unwrap(), fixture);
}
#[tokio::test]
async fn authenticates_and_checks_session_scope_before_cache_access() {
    for (cookie, error, expected) in [
        (false, None, StatusCode::UNAUTHORIZED),
        (
            true,
            Some(ErrorCode::Unauthenticated),
            StatusCode::UNAUTHORIZED,
        ),
        (true, Some(ErrorCode::Forbidden), StatusCode::FORBIDDEN),
    ] {
        let stub = Stub {
            session_error: error,
            ..Stub::default()
        };
        let calls = stub.calls.clone();
        assert_eq!(
            app(stub)
                .oneshot(request("GET", cookie))
                .await
                .unwrap()
                .status(),
            expected
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn get_returns_cached_wire_without_collection_or_mutation() {
    let stub = Stub::default();
    let calls = stub.calls.clone();
    let response = app(stub).oneshot(request("GET", true)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json(response).await,
        serde_json::to_value(wire_report()).unwrap()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn mutation_methods_never_enter_the_cache_backend() {
    let stub = Stub::default();
    let calls = stub.calls.clone();
    for method in ["POST", "PUT", "PATCH", "DELETE"] {
        assert_eq!(
            app(stub.clone())
                .oneshot(request(method, true))
                .await
                .unwrap()
                .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn oversized_network_reports_fail_with_a_bounded_error() {
    let response = app(Stub {
        oversize: true,
        ..Stub::default()
    })
    .oneshot(request("GET", true))
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json(response).await["code"], "unavailable");
}
#[tokio::test]
async fn an_unregistered_backend_is_unavailable_not_an_empty_healthy_inventory() {
    let backend = Unregistered::default();
    let calls = backend.0.calls.clone();
    let response = app(backend).oneshot(request("GET", true)).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json(response).await["code"], "unavailable");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn a_cache_permission_recheck_can_refuse_after_session_resolution() {
    let response = app(Stub {
        cache_error: Some(ErrorCode::Forbidden),
        ..Stub::default()
    })
    .oneshot(request("GET", true))
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let value = json(response).await;
    assert!(value.get("groups").is_none());
}
#[tokio::test]
async fn missing_observations_remain_unknown_with_null_counts() {
    let response = app(Stub {
        source_missing: true,
        ..Stub::default()
    })
    .oneshot(request("GET", true))
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value = json(response).await;
    assert_eq!(value["source"], "unavailable");
    assert!(value["observed_containers"].is_null());
    assert_eq!(value["groups"], serde_json::json!([]));
}
