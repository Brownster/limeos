use super::*;

pub(super) async fn inventory<B: Backend>(
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
    #[derive(Clone)]
    struct Stub {
        calls: Arc<AtomicUsize>,
        expired: bool,
        oversize: bool,
    }
    impl Backend for Stub {
        async fn login(&self, _: Login, _: PasswordAttempt) -> PasswordChecked<IssuedSession> {
            PasswordChecked::rejected()
        }
        async fn logout(&self, _: String, _: String) -> Result<()> {
            panic!("stack GET cannot mutate")
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
                    id: "reader".into(),
                    role: limeos_domain::Role::Viewer,
                    grant_revision: 1,
                },
                csrf_token: "b".repeat(64),
                expires_at: 9999,
            })
        }
        async fn stack_inventory(
            &self,
            _: String,
        ) -> Result<limeos_domain::deployment::discovery::StackInventory> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut value = limeos_domain::deployment::discovery::discover(100, None, None)?;
            if self.oversize {
                value.other_projects=vec![limeos_domain::deployment::discovery::OtherProjectView { project:limeos_domain::deployment::discovery::ProjectIdentity { name:"x".repeat(70_000),working_directory:"/x".into(),config_files:vec!["/x/compose.yaml".into()] },relation:limeos_domain::deployment::discovery::RegistrationRelation::CatalogUnknown,services:vec![] }];
            }
            Ok(value)
        }
    }
    fn request(method: &str, cookie: bool) -> Request {
        let mut r = Request::builder()
            .method(method)
            .uri("/api/v1/deployment/stacks");
        if cookie {
            r = r.header("cookie", format!("__Host-limeos={}", "a".repeat(64)));
        }
        r.body(Body::empty()).unwrap()
    }
    #[tokio::test]
    async fn registered_router_authenticates_before_cache_access_and_forbids_posts() {
        let b = Stub {
            calls: Arc::new(AtomicUsize::new(0)),
            expired: false,
            oversize: false,
        };
        for (backend, method, cookie, status) in [
            (b.clone(), "GET", false, StatusCode::UNAUTHORIZED),
            (
                Stub {
                    expired: true,
                    ..b.clone()
                },
                "GET",
                true,
                StatusCode::UNAUTHORIZED,
            ),
            (b.clone(), "POST", true, StatusCode::METHOD_NOT_ALLOWED),
        ] {
            let response = router(backend, "https://localhost".into())
                .oneshot(request(method, cookie))
                .await
                .unwrap();
            assert_eq!(response.status(), status);
        }
        assert_eq!(b.calls.load(Ordering::SeqCst), 0);
        let response = router(b.clone(), "https://localhost".into())
            .oneshot(request("GET", true))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response.headers()["cache-control"]
                .to_str()
                .unwrap()
                .contains("no-store")
        );
        assert_eq!(b.calls.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn registered_router_refuses_oversized_reports_before_serialization() {
        let b = Stub {
            calls: Arc::new(AtomicUsize::new(0)),
            expired: false,
            oversize: true,
        };
        let response = router(b, "https://localhost".into())
            .oneshot(request("GET", true))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
