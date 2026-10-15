//! Cached read only. Mount under the shared API middleware after registration.
use super::*;
use limeos_domain::deployment::discovery::StackInventory;

pub trait DeploymentBackend: Backend {
    /// Current session/grants and cached observations only; never poll services.
    fn media_activity(
        &self,
        _token: String,
    ) -> impl Future<Output = Result<limeos_domain::deployment::activity::ActivitySnapshot>> + Send
    {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    /// Resolve current session/grants and read the shared observation cache.
    /// This method must not launch an inspection, shell, Compose command or write.
    fn stack_inventory(
        &self,
        _token: String,
    ) -> impl Future<Output = Result<StackInventory>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
}

pub(super) fn routes<B: DeploymentBackend>() -> Router<App<B>> {
    Router::new()
        .route("/api/v1/deployment/stacks", get(inventory::<B>))
        .route("/api/v1/deployment/activity", get(activity::<B>))
}

async fn activity<B: DeploymentBackend>(State(app): State<App<B>>, headers: HeaderMap) -> Response {
    let result = async {
        let token = cookie(&headers)?;
        app.backend.session(token.clone(), None).await?;
        let report = app.backend.media_activity(token).await?;
        report.check_size()?;
        Ok(report)
    }
    .await;
    match result {
        Ok(report) => axum::Json(report).into_response(),
        Err(error) => failure(error),
    }
}

async fn inventory<B: DeploymentBackend>(
    State(app): State<App<B>>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        let token = cookie(&headers)?;
        app.backend.session(token.clone(), None).await?;
        let report = app.backend.stack_inventory(token).await?;
        report.check_size()?;
        Ok(report)
    }
    .await;
    match result {
        Ok(report) => axum::Json(report).into_response(),
        Err(error) => failure(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    #[derive(Clone, Default)]
    struct Stub {
        calls: Arc<AtomicUsize>,
        expired: bool,
        oversize: bool,
    }
    impl Backend for Stub {
        async fn login(&self, _: Login) -> Result<IssuedSession> {
            Err(Error(ErrorCode::Unauthenticated))
        }
        async fn logout(&self, _: String, _: String) -> Result<()> {
            panic!("GET cannot mutate")
        }
        async fn health(&self) -> Result<Health> {
            Err(Error(ErrorCode::Unavailable))
        }
        async fn session(&self, _: String, _: Option<String>) -> Result<SessionView> {
            if self.expired {
                return Err(Error(ErrorCode::Unauthenticated));
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
        async fn media_activity(
            &self,
            _: String,
        ) -> Result<limeos_domain::deployment::activity::ActivitySnapshot> {
            use limeos_domain::deployment::activity as model;
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut report = model::snapshot(100, "engine-a", None)?;
            if self.oversize {
                report.services.push(model::ServiceView {
                    target: model::Target {
                        container: limeos_domain::ContainerSnapshot {
                            resource: format!("container:{}", "a".repeat(64)),
                            image: format!("sha256:{}", "b".repeat(64)),
                            started_at: "2026-10-15T01:02:03.000000000Z".into(),
                            running: true,
                        },
                        name: "x".repeat(70_000),
                        provider: model::Provider::Jellyfin,
                    },
                    sampled_at: None,
                    state: model::ServiceState::Unavailable,
                });
            }
            Ok(report)
        }
        async fn stack_inventory(&self, _: String) -> Result<StackInventory> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut report = limeos_domain::deployment::discovery::discover(100, None, None)?;
            if self.oversize {
                report.other_projects = vec![limeos_domain::deployment::discovery::OtherProjectView {
                    project: limeos_domain::deployment::discovery::ProjectIdentity { name: "x".repeat(70_000), working_directory: "/opt/stacks/x".into(), config_files: vec![] },
                    relation: limeos_domain::deployment::discovery::RegistrationRelation::CatalogUnknown, services: vec![],
                }];
            }
            Ok(report)
        }
    }
    fn app(stub: Stub) -> Router {
        routes().with_state(App {
            backend: stub,
            origin: "https://localhost".into(),
            limiter: Arc::new(Mutex::new(Limiter::default())),
            readers: Arc::new(Semaphore::new(2)),
        })
    }
    fn request(method: &str, cookie: bool) -> Request {
        let mut builder = Request::builder()
            .method(method)
            .uri("/api/v1/deployment/stacks");
        if cookie {
            builder = builder.header("cookie", format!("__Host-limeos={}", "a".repeat(64)));
        }
        builder.body(Body::empty()).unwrap()
    }
    #[tokio::test]
    async fn authenticates_before_cache_access_and_get_never_mutates() {
        let stub = Stub::default();
        let calls = stub.calls.clone();
        assert_eq!(
            app(stub.clone())
                .oneshot(request("GET", false))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            app(Stub {
                expired: true,
                ..stub.clone()
            })
            .oneshot(request("GET", true))
            .await
            .unwrap()
            .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            app(stub.clone())
                .oneshot(request("POST", true))
                .await
                .unwrap()
                .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let response = app(stub).oneshot(request("GET", true)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 65_536).await.unwrap()).unwrap();
        assert_eq!(value["engine"]["state"], "unavailable");
        assert!(value["standalone_containers"].is_null());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn rejects_oversized_backend_report() {
        let response = app(Stub {
            oversize: true,
            ..Stub::default()
        })
        .oneshot(request("GET", true))
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
    #[tokio::test]
    async fn activity_authenticates_before_cache_reads_and_refuses_mutation_methods() {
        let stub = Stub::default();
        let calls = stub.calls.clone();
        for (method, has_cookie, expired, expected) in [
            ("GET", false, false, StatusCode::UNAUTHORIZED),
            ("GET", true, true, StatusCode::UNAUTHORIZED),
            ("POST", true, false, StatusCode::METHOD_NOT_ALLOWED),
            ("GET", true, false, StatusCode::OK),
        ] {
            let mut req = request(method, has_cookie);
            *req.uri_mut() = "/api/v1/deployment/activity".parse().unwrap();
            let response = app(Stub {
                expired,
                ..stub.clone()
            })
            .oneshot(req)
            .await
            .unwrap();
            assert_eq!(response.status(), expected);
            if expected == StatusCode::OK {
                let value: serde_json::Value =
                    serde_json::from_slice(&to_bytes(response.into_body(), 65_536).await.unwrap())
                        .unwrap();
                assert_eq!(value["source"], "unavailable");
                assert!(value["totals"]["streams"].is_null());
            } else {
                assert_eq!(calls.load(Ordering::SeqCst), 0);
            }
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn oversized_activity_cache_report_is_unavailable() {
        let mut req = request("GET", true);
        *req.uri_mut() = "/api/v1/deployment/activity".parse().unwrap();
        assert_eq!(
            app(Stub {
                oversize: true,
                ..Stub::default()
            })
            .oneshot(req)
            .await
            .unwrap()
            .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
