use super::*;
use axum::body::Body;
use limeos_domain::automation::HealthPolicyDefinition;
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

#[derive(Clone)]
struct Stub {
    value: HealthPolicyView,
    calls: Arc<AtomicUsize>,
}
impl TestBackend for Stub {
    async fn login(&self, _: Login, _: PasswordAttempt) -> PasswordChecked<IssuedSession> {
        PasswordChecked::rejected()
    }
    /// Authority is in each call; this only confirms the session before
    /// the gate releases an input refusal.
    async fn session(&self, _: String, _: Option<String>) -> Result<SessionView> {
        crate::test_backend::live_session()
    }
    async fn logout(&self, _: String, _: String) -> Result<()> {
        panic!("unrelated")
    }
    async fn health(&self) -> Result<Health> {
        Err(Error(ErrorCode::Unavailable))
    }
    async fn observe_health_policy(
        &self,
        token: String,
        deadline: Instant,
    ) -> Result<HealthPolicyView> {
        assert_eq!(token, "a".repeat(64));
        assert!(deadline > Instant::now() && deadline <= Instant::now() + Duration::from_secs(10));
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.value.clone())
    }
    async fn replace_observe_health_policy(
        &self,
        token: String,
        csrf: String,
        _: ReplaceHealthPolicy,
        deadline: Instant,
    ) -> Result<HealthPolicyView> {
        assert_eq!(token, "a".repeat(64));
        assert_eq!(csrf, "c".repeat(64));
        assert!(deadline > Instant::now() && deadline <= Instant::now() + Duration::from_secs(10));
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.value.clone())
    }
}
fn stub(revision: i64) -> Stub {
    Stub {
        value: HealthPolicyView {
            revision,
            updated: if revision == 0 { None } else { Some(200) },
            definition: HealthPolicyDefinition::default(),
        },
        calls: Arc::new(AtomicUsize::new(0)),
    }
}
fn request(method: &str, path: &str, body: &str) -> Request {
    Request::builder()
        .method(method)
        .uri(path)
        .header("origin", "https://localhost")
        .header("cookie", format!("__Host-limeos={}", "a".repeat(64)))
        .header("x-csrf-token", "c".repeat(64))
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}
fn body() -> String {
    serde_json::to_string(&ReplaceHealthPolicy {
        revision: 0,
        definition: HealthPolicyDefinition::default(),
    })
    .unwrap()
}
#[tokio::test]
async fn registered_policy_routes_preserve_deadline_and_explicit_revision() {
    let backend = stub(0);
    let app = router(backend.clone(), "https://localhost".into());
    let response = app
        .oneshot(request("GET", "/api/v1/automation/health-policy", ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let value: HealthPolicyView =
        serde_json::from_slice(&to_bytes(response.into_body(), 70_000).await.unwrap()).unwrap();
    assert_eq!(value.revision, 0);
    assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
    let backend = stub(1);
    let app = router(backend.clone(), "https://localhost".into());
    assert_eq!(
        app.oneshot(request("POST", "/api/v1/automation/health-policy", &body()))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn invalid_authority_fields_bounds_and_unregistered_effects_never_reach_backend() {
    let backend = stub(1);
    let app = router(backend.clone(), "https://localhost".into());
    for (method, path, body) in [
        (
            "GET",
            "/api/v1/automation/health-policy?owner=other",
            "".into(),
        ),
        (
            "POST",
            "/api/v1/automation/health-policy?revision=0",
            body(),
        ),
        (
            "POST",
            "/api/v1/automation/health-policy",
            r#"{"revision":0,"definition":{},"owner":"other"}"#.into(),
        ),
        (
            "POST",
            "/api/v1/automation/health-policy",
            body().replace("\"revision\":0", "\"revision\":-1"),
        ),
        (
            "POST",
            "/api/v1/automation/health-policy",
            body().replace("\"fail_threshold\":2", "\"fail_threshold\":100"),
        ),
        (
            "POST",
            "/api/v1/automation/health-policy",
            " ".repeat(MAX_HEALTH_POLICY_BYTES + 4097),
        ),
    ] {
        assert_eq!(
            app.clone()
                .oneshot(request(method, path, &body))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST,
            "{path}"
        );
    }
    let mut missing = request("POST", "/api/v1/automation/health-policy", &body());
    missing.headers_mut().remove("x-csrf-token");
    assert_eq!(
        app.clone().oneshot(missing).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    let mut missing = request("GET", "/api/v1/automation/health-policy", "");
    missing.headers_mut().remove("cookie");
    assert_eq!(
        app.clone().oneshot(missing).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.clone()
            .oneshot(request("PUT", "/api/v1/automation/health-policy", &body()))
            .await
            .unwrap()
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    for path in [
        "/api/v1/automation/health-policy/publish",
        "/api/v1/automation/health-policy/collect",
    ] {
        assert_eq!(
            app.clone()
                .oneshot(request("POST", path, "{}"))
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn invalid_or_unrelated_backend_configuration_is_not_returned_as_success() {
    let mut backend = stub(0);
    backend.value.definition.fail_threshold = 3;
    let app = router(backend, "https://localhost".into());
    assert_eq!(
        app.oneshot(request("GET", "/api/v1/automation/health-policy", ""))
            .await
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let backend = stub(5);
    let app = router(backend, "https://localhost".into());
    assert_eq!(
        app.oneshot(request("POST", "/api/v1/automation/health-policy", &body()))
            .await
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}
