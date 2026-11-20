//! The route table and the one gate every route passes.
//!
//! The router is built only from `table`, and each entry's access decides the
//! gate wrapped around its handler before the handler runs: a read needs the
//! session cookie; a mutation also needs the exact Origin (and no cross-site
//! fetch) and a CSRF header. The gate never reads the body and never calls
//! core. The session itself, its CSRF binding and the principal's scope are
//! checked by core in the same store call as the work, so every handler hands
//! the backend the token (and CSRF value) and does not pre-check the session.
//!
//! Extractor rejections (path, query) use the API's error envelope through
//! the `Path` and `Query` wrappers below, never axum's plain-text answer.
use super::*;
use axum::{
    extract::{FromRequestParts, Request, State},
    handler::Handler,
    http::{Method, request::Parts},
    routing::MethodRouter,
};

/// What a route asks for before its handler runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Access {
    /// No credentials: health and login only.
    Public,
    /// The session cookie.
    Read,
    /// The session cookie, the exact Origin and a CSRF header.
    Mutate,
}

/// One route of the table: its path, method, access and handler.
pub(crate) struct Entry<B> {
    pub(crate) path: &'static str,
    /// Read by the route-table test, which sends each request with it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) method: Method,
    pub(crate) access: Access,
    handler: MethodRouter<App<B>>,
}

/// A signed-in read.
pub(crate) fn read<B, H, T>(path: &'static str, handler: H) -> Entry<B>
where
    B: Backend,
    H: Handler<T, App<B>>,
    T: 'static,
{
    entry(path, Method::GET, Access::Read, get(handler))
}
/// A mutation: session, Origin and CSRF.
pub(crate) fn write<B, H, T>(path: &'static str, handler: H) -> Entry<B>
where
    B: Backend,
    H: Handler<T, App<B>>,
    T: 'static,
{
    entry(path, Method::POST, Access::Mutate, post(handler))
}
/// An unauthenticated route. Only health and login; the route-table test
/// fails on any other.
pub(crate) fn public<B: Backend>(
    path: &'static str,
    method: Method,
    handler: MethodRouter<App<B>>,
) -> Entry<B> {
    entry(path, method, Access::Public, handler)
}
fn entry<B: Backend>(
    path: &'static str,
    method: Method,
    access: Access,
    handler: MethodRouter<App<B>>,
) -> Entry<B> {
    Entry {
        path,
        method,
        access,
        handler,
    }
}

/// Every route, mounted from this list and nothing else.
pub(crate) fn table<B: Backend>() -> Vec<Entry<B>> {
    let mut routes = vec![
        read("/api/v1/deployment/stacks", stacks::inventory::<B>),
        public("/api/v1/health", Method::GET, get(health::<B>)),
        read(
            "/api/v1/storage/mounts/targets",
            storage_mounts::catalog::<B>,
        ),
        write("/api/v1/storage/mounts/plans", storage_mounts::plan::<B>),
        read(
            "/api/v1/storage/mounts/plans/{id}",
            storage_mounts::read::<B>,
        ),
        write(
            "/api/v1/storage/mounts/plans/{id}/approval",
            storage_mounts::approve::<B>,
        ),
        write(
            "/api/v1/storage/mounts/plans/{id}/cancel",
            storage_mounts::cancel::<B>,
        ),
        read("/api/v1/storage/mounts/jobs", storage_mounts::jobs::<B>),
        write("/api/v1/storage/mounts/jobs", storage_mounts::queue::<B>),
        read("/api/v1/storage/mounts/jobs/{id}", storage_mounts::job::<B>),
        write(
            "/api/v1/storage/mounts/jobs/{id}/cancel",
            storage_mounts::cancel_job::<B>,
        ),
        write("/api/v1/storage/targets/plans", storage::target_plan::<B>),
        read(
            "/api/v1/storage/targets/plans/{id}",
            storage::target_read::<B>,
        ),
        write(
            "/api/v1/storage/targets/plans/{id}/approval",
            storage::target_approve::<B>,
        ),
        write(
            "/api/v1/storage/targets/plans/{id}/cancel",
            storage::target_cancel::<B>,
        ),
        write("/api/v1/storage/targets/jobs", storage::target_queue::<B>),
        read(
            "/api/v1/storage/targets/jobs/{id}",
            storage::target_job::<B>,
        ),
        write(
            "/api/v1/storage/targets/jobs/{id}/cancel",
            storage::target_cancel_job::<B>,
        ),
        read("/api/v1/storage/inventory", storage::inventory::<B>),
        read(
            "/api/v1/storage/container-dependencies",
            storage::container_dependencies::<B>,
        ),
        write("/api/v1/storage/plans", storage::plan::<B>),
        read("/api/v1/storage/plans/{id}", storage::read::<B>),
        write("/api/v1/storage/plans/{id}/approval", storage::approve::<B>),
        write("/api/v1/storage/plans/{id}/cancel", storage::cancel::<B>),
        public("/api/v1/auth/login", Method::POST, post(login::<B>)),
        write(
            "/api/v1/auth/capabilities/observe-health/enable",
            identity_enablement::enable::<B>,
        ),
        read("/api/v1/auth/session", session::<B>),
        write("/api/v1/auth/logout", logout::<B>),
        read("/api/v1/overview", reads::overview::<B>),
        read("/api/v1/resources", reads::resources::<B>),
        read("/api/v1/system/history", reads::history::<B>),
        read("/api/v1/observations/stream", reads::stream::<B>),
        write("/api/v1/compose/plans", compose::plan::<B>),
        read("/api/v1/compose/plans/{id}", compose::read::<B>),
        write("/api/v1/compose/plans/{id}/approval", compose::approve::<B>),
        write("/api/v1/compose/plans/{id}/cancel", compose::cancel::<B>),
        write("/api/v1/container/restart/plans", operations::plan::<B>),
        write("/api/v1/container/plans", operations::container_plan::<B>),
        write(
            "/api/v1/container/plans/{id}/approval",
            operations::approve::<B>,
        ),
        write(
            "/api/v1/container/plans/{id}/cancel",
            operations::cancel_plan::<B>,
        ),
        write("/api/v1/container/jobs", operations::container_queue::<B>),
        read("/api/v1/container/jobs", operations::jobs::<B>),
        read("/api/v1/container/jobs/{id}", operations::progress::<B>),
        write(
            "/api/v1/container/jobs/{id}/cancel",
            operations::cancel_job::<B>,
        ),
        read("/api/v1/containers/{id}/logs", operations::logs::<B>),
        write(
            "/api/v1/container/restart/plans/{id}/approval",
            operations::approve::<B>,
        ),
        write(
            "/api/v1/container/restart/plans/{id}/cancel",
            operations::cancel_plan::<B>,
        ),
        write("/api/v1/container/restart/jobs", operations::queue::<B>),
        read("/api/v1/container/restart/jobs", operations::jobs::<B>),
        read(
            "/api/v1/container/restart/jobs/{id}",
            operations::progress::<B>,
        ),
        write(
            "/api/v1/container/restart/jobs/{id}/cancel",
            operations::cancel_job::<B>,
        ),
        write("/api/v1/stack/plans", stack_operations::plan::<B>),
        write(
            "/api/v1/stack/plans/{id}/approval",
            stack_operations::approve::<B>,
        ),
        write(
            "/api/v1/stack/plans/{id}/cancel",
            stack_operations::cancel_plan::<B>,
        ),
        write("/api/v1/stack/jobs", stack_operations::queue::<B>),
        read("/api/v1/stack/jobs", stack_operations::jobs::<B>),
        read("/api/v1/stack/jobs/{id}", stack_operations::progress::<B>),
        write(
            "/api/v1/stack/jobs/{id}/cancel",
            stack_operations::cancel_job::<B>,
        ),
        read(
            "/api/v1/jobs/needs-attention",
            job_resolution::needs_attention::<B>,
        ),
        read("/api/v1/jobs/{id}/resolution", job_resolution::read::<B>),
        write("/api/v1/jobs/{id}/resolution", job_resolution::resolve::<B>),
    ];
    routes.extend(automation_schedules::routes::<B>());
    routes.extend(automation_history::routes::<B>());
    routes.extend(automation_policy::routes::<B>());
    routes
}

/// The table mounted, each handler behind the gate its access names.
pub(crate) fn mount<B: Backend>(app: &App<B>) -> Router<App<B>> {
    table::<B>()
        .into_iter()
        .fold(Router::new(), |router, entry| {
            let handler = match entry.access {
                Access::Public => entry.handler,
                Access::Read => entry
                    .handler
                    .route_layer(middleware::from_fn_with_state(app.clone(), read_gate::<B>)),
                Access::Mutate => entry.handler.route_layer(middleware::from_fn_with_state(
                    app.clone(),
                    mutate_gate::<B>,
                )),
            };
            router.route(entry.path, handler)
        })
}

/// The session cookie is the only credential: a request that offers a bearer
/// or key header is refused, with or without a cookie.
fn cookie_only(headers: &HeaderMap) -> Result<()> {
    if headers.contains_key("authorization") || headers.contains_key("x-api-key") {
        return Err(Error(ErrorCode::Forbidden));
    }
    Ok(())
}
async fn read_gate<B: Backend>(State(_): State<App<B>>, request: Request, next: Next) -> Response {
    match cookie_only(request.headers()).and_then(|()| cookie(request.headers())) {
        Ok(_) => next.run(request).await,
        Err(error) => failure(error),
    }
}
async fn mutate_gate<B: Backend>(
    State(app): State<App<B>>,
    request: Request,
    next: Next,
) -> Response {
    match cookie_only(request.headers())
        .and_then(|()| operations::credentials(&app, request.headers()))
    {
        Ok(_) => next.run(request).await,
        Err(error) => failure(error),
    }
}

/// `axum::extract::Path` whose rejection is the API's `InvalidInput`
/// envelope, with no detail of what failed to parse.
pub(crate) struct Path<T>(pub(crate) T);
impl<S: Send + Sync, T: serde::de::DeserializeOwned + Send> FromRequestParts<S> for Path<T> {
    type Rejection = Response;
    async fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> std::result::Result<Self, Response> {
        axum::extract::Path::<T>::from_request_parts(parts, state)
            .await
            .map(|axum::extract::Path(value)| Path(value))
            .map_err(|_| failure(Error(ErrorCode::InvalidInput)))
    }
}
/// `axum::extract::Query` whose rejection is the API's `InvalidInput`
/// envelope, with no serde detail.
pub(crate) struct Query<T>(pub(crate) T);
impl<T: serde::de::DeserializeOwned> Query<T> {
    /// For handlers that take the whole request: the same rule, as a Result.
    pub(crate) fn try_from_uri(uri: &axum::http::Uri) -> Result<Self> {
        axum::extract::Query::<T>::try_from_uri(uri)
            .map(|axum::extract::Query(value)| Query(value))
            .map_err(|_| Error(ErrorCode::InvalidInput))
    }
}
impl<S: Send + Sync, T: serde::de::DeserializeOwned> FromRequestParts<S> for Query<T> {
    type Rejection = Response;
    async fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> std::result::Result<Self, Response> {
        axum::extract::Query::<T>::from_request_parts(parts, state)
            .await
            .map(|axum::extract::Query(value)| Query(value))
            .map_err(|_| failure(Error(ErrorCode::InvalidInput)))
    }
}
