use super::*;
use axum::body::Body;
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;
#[derive(Clone)]
struct Mutations(Arc<AtomicUsize>);
impl Backend for Mutations {
    async fn plan_compose(
        &self,
        _: String,
        _: String,
        _: limeos_domain::ComposeSelection,
    ) -> Result<limeos_domain::PlannedCompose> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(Error(ErrorCode::Unavailable))
    }
    async fn approve_compose(
        &self,
        _: String,
        _: String,
        _: String,
        _: String,
    ) -> Result<limeos_domain::PlanApproval> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(Error(ErrorCode::Unavailable))
    }
    async fn cancel_compose(&self, _: String, _: String, _: String) -> Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(Error(ErrorCode::Unavailable))
    }
    async fn login(&self, _: Login) -> Result<IssuedSession> {
        Err(Error(ErrorCode::Unauthenticated))
    }
    async fn session(&self, _: String, csrf: Option<String>) -> Result<SessionView> {
        if csrf.as_deref() != Some("b".repeat(64).as_str()) {
            return Err(Error(ErrorCode::Forbidden));
        }
        Ok(SessionView {
            principal: limeos_domain::Principal {
                id: "admin".into(),
                role: limeos_domain::Role::Administrator,
                grant_revision: 1,
            },
            csrf_token: "b".repeat(64),
            expires_at: 999,
        })
    }
    async fn logout(&self, _: String, _: String) -> Result<()> {
        panic!("unexpected mutation")
    }
    async fn health(&self) -> Result<Health> {
        Err(Error(ErrorCode::Unavailable))
    }
    async fn plan_restart(
        &self,
        _: String,
        _: String,
        _: limeos_contracts::RestartInput,
    ) -> Result<limeos_domain::PlannedRestart> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(Error(ErrorCode::Unavailable))
    }
    async fn plan_container(
        &self,
        _: String,
        _: String,
        _: limeos_contracts::ContainerInput,
    ) -> Result<limeos_domain::PlannedContainerAction> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(Error(ErrorCode::Unavailable))
    }
    async fn approve_container(
        &self,
        _: String,
        _: String,
        _: String,
        _: String,
    ) -> Result<limeos_domain::PlanApproval> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(Error(ErrorCode::Unavailable))
    }
    async fn cancel_restart(&self, _: String, _: String, _: String, _: bool) -> Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(Error(ErrorCode::Unavailable))
    }
}
#[tokio::test]
async fn every_mutation_rejects_get_origin_csrf_and_untyped_extra_fields() {
    let mutations = Arc::new(AtomicUsize::new(0));
    let router = router(Mutations(mutations.clone()), "https://localhost".into());
    for path in [
        "/api/v1/compose/plans".into(),
        format!("/api/v1/compose/plans/{}/approval", "a".repeat(64)),
        format!("/api/v1/compose/plans/{}/cancel", "a".repeat(64)),
        "/api/v1/container/plans".into(),
        format!("/api/v1/container/plans/{}/approval", "a".repeat(64)),
        format!("/api/v1/container/plans/{}/cancel", "a".repeat(64)),
        format!("/api/v1/container/jobs/{}/cancel", "a".repeat(64)),
        "/api/v1/container/restart/plans".into(),
        format!(
            "/api/v1/container/restart/plans/{}/approval",
            "a".repeat(64)
        ),
        format!("/api/v1/container/restart/plans/{}/cancel", "a".repeat(64)),
        format!("/api/v1/container/restart/jobs/{}/cancel", "a".repeat(64)),
    ] {
        for (method, origin, csrf, status) in [
            ("GET", "https://localhost", "b".repeat(64), 405),
            ("POST", "https://foreign.invalid", "b".repeat(64), 403),
            ("POST", "https://localhost", "a".repeat(64), 403),
        ] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(&path)
                        .header("origin", origin)
                        .header("cookie", format!("__Host-limeos={}", "a".repeat(64)))
                        .header("x-csrf-token", csrf)
                        .header("content-type", "application/json")
                        .body(Body::from("{}"))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), status, "{path}");
        }
    }
    assert_eq!(mutations.load(Ordering::SeqCst), 0);
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/container/restart/plans")
                .header("origin", "https://localhost")
                .header("cookie", format!("__Host-limeos={}", "a".repeat(64)))
                .header("x-csrf-token", "b".repeat(64))
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"resource":"container:fixture","command":"sh -c forbidden"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(mutations.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn compose_selections_cannot_carry_manifests_or_claimed_authority() {
    let mutations = Arc::new(AtomicUsize::new(0));
    let router = router(Mutations(mutations.clone()), "https://localhost".into());
    for extra in ["project", "yaml", "privileged", "principal", "command"] {
        let mut body = serde_json::json!({"stack":"media","template":"standard"});
        body[extra] = serde_json::json!("forbidden");
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/compose/plans")
                    .header("origin", "https://localhost")
                    .header("cookie", format!("__Host-limeos={}", "a".repeat(64)))
                    .header("x-csrf-token", "b".repeat(64))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    assert_eq!(mutations.load(Ordering::SeqCst), 0);
}
