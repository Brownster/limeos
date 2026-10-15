//! Cached read only. Mount under the shared API middleware after registration.
use super::*;
use limeos_domain::deployment::discovery::StackInventory;

pub trait DeploymentBackend: Backend {
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
    Router::new().route("/api/v1/deployment/stacks", get(inventory::<B>))
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
}
