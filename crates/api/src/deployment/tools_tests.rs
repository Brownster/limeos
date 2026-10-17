use super::*;
use axum::body::Body;
use limeos_domain::deployment::tools as model;
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

fn report() -> model::ToolsReport {
    let config = model::Observation {
        sampled_at: 100,
        value: model::ConfigState::Stored {
            config: model::Config::new("/data/Media Library", 8080).unwrap(),
        },
    };
    let payload = model::Observation {
        sampled_at: 100,
        value: model::PayloadState::Present,
    };
    let service = model::Observation {
        sampled_at: 100,
        value: model::ServiceState::Active,
    };
    model::observe(
        105,
        model::Facts {
            config: Some(&config),
            payload: Some(&payload),
            service: Some(&service),
        },
    )
    .unwrap()
}
#[derive(Clone, Default)]
struct Stub {
    calls: Arc<AtomicUsize>,
    session_error: Option<ErrorCode>,
    cache_error: Option<ErrorCode>,
    missing: bool,
}
impl Backend for Stub {
    async fn login(&self, _: Login) -> Result<IssuedSession> {
        panic!("tools GET cannot login")
    }
    async fn logout(&self, _: String, _: String) -> Result<()> {
        panic!("tools GET cannot mutate")
    }
    async fn health(&self) -> Result<Health> {
        panic!("tools GET cannot collect")
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
    async fn tools_status(&self, _: String) -> Result<model::ToolsReport> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(code) = self.cache_error {
            return Err(Error(code));
        }
        if self.missing {
            return model::observe(105, model::Facts::default());
        }
        Ok(report())
    }
}
#[derive(Clone)]
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
fn request(method: &str, authenticated: bool) -> Request {
    let mut request = Request::builder()
        .method(method)
        .uri("/api/v1/deployment/tools/copyparty");
    if authenticated {
        request = request.header("cookie", format!("__Host-limeos={}", "a".repeat(64)));
    }
    request.body(Body::empty()).unwrap()
}
async fn json(response: Response) -> serde_json::Value {
    serde_json::from_slice(
        &to_bytes(response.into_body(), model::MAX_REPORT_BYTES)
            .await
            .unwrap(),
    )
    .unwrap()
}
#[test]
fn tools_wire_fixture_matches_projection() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/deployment/tools/wire.json"
    ))
    .unwrap();
    assert_eq!(serde_json::to_value(report()).unwrap(), fixture);
}
#[tokio::test]
async fn session_and_permissions_precede_cache_access() {
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
async fn authenticated_read_returns_only_cached_projection() {
    let stub = Stub::default();
    let calls = stub.calls.clone();
    let response = app(stub).oneshot(request("GET", true)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json(response).await,
        serde_json::to_value(report()).unwrap()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn backend_rechecks_current_grants_and_unavailability() {
    for (code, expected) in [
        (ErrorCode::Unauthenticated, StatusCode::UNAUTHORIZED),
        (ErrorCode::Forbidden, StatusCode::FORBIDDEN),
        (ErrorCode::Unavailable, StatusCode::SERVICE_UNAVAILABLE),
    ] {
        let stub = Stub {
            cache_error: Some(code),
            ..Stub::default()
        };
        let calls = stub.calls.clone();
        let response = app(stub).oneshot(request("GET", true)).await.unwrap();
        assert_eq!(response.status(), expected);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(!json(response).await.to_string().contains("Media Library"));
    }
}
#[tokio::test]
async fn no_collection_has_null_values_instead_of_absence_booleans() {
    let response = app(Stub {
        missing: true,
        ..Stub::default()
    })
    .oneshot(request("GET", true))
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let report = json(response).await;
    for component in ["config", "payload", "service"] {
        assert_eq!(report[component]["freshness"], "unavailable");
        assert!(report[component]["value"].is_null());
    }
    assert!(report.get("installed").is_none());
    assert!(report.get("url").is_none());
}
#[tokio::test]
async fn unregistered_backend_defaults_to_unavailable() {
    let stub = Stub::default();
    let calls = stub.calls.clone();
    let response = app(Unregistered(stub))
        .oneshot(request("GET", true))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn mutation_methods_are_not_registered() {
    for method in ["POST", "PUT", "PATCH", "DELETE"] {
        let stub = Stub::default();
        let calls = stub.calls.clone();
        assert_eq!(
            app(stub)
                .oneshot(request(method, true))
                .await
                .unwrap()
                .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn request_host_cannot_invent_a_service_url() {
    let mut req = request("GET", true);
    req.headers_mut()
        .insert("host", "attacker.invalid:8080".parse().unwrap());
    req.headers_mut()
        .insert("x-forwarded-host", "attacker.invalid".parse().unwrap());
    let value = json(app(Stub::default()).oneshot(req).await.unwrap()).await;
    assert!(!value.to_string().contains("attacker.invalid"));
    assert!(value.get("url").is_none());
}
