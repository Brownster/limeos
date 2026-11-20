//! Whole-stack lifecycle routes. Same session, CSRF, origin and body rules as
//! the single-container routes; no file, path or container selection exists.
use super::operations::{credentials, input};
use super::*;
use gate::{Path, Query};

pub(super) async fn plan<B: Backend>(State(app): State<App<B>>, request: Request) -> Response {
    let result = async {
        let (token, csrf) = credentials(&app, request.headers())?;
        app.backend
            .plan_stack(token, csrf, input(request).await?)
            .await
    }
    .await;
    match result {
        Ok(value) => (StatusCode::CREATED, axum::Json(value)).into_response(),
        Err(e) => failure(e),
    }
}
pub(super) async fn approve<B: Backend>(
    State(app): State<App<B>>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let result = async {
        let (token, csrf) = credentials(&app, request.headers())?;
        let value: limeos_contracts::ApprovalInput = input(request).await?;
        app.backend
            .approve_stack(token, csrf, id, value.digest)
            .await
    }
    .await;
    match result {
        Ok(value) => axum::Json(value).into_response(),
        Err(e) => failure(e),
    }
}
pub(super) async fn queue<B: Backend>(State(app): State<App<B>>, request: Request) -> Response {
    let result = async {
        let (token, csrf) = credentials(&app, request.headers())?;
        let value: limeos_contracts::QueueStackInput = input(request).await?;
        app.backend.queue_stack(token, csrf, value).await
    }
    .await;
    match result {
        Ok(value) => (StatusCode::ACCEPTED, axum::Json(value)).into_response(),
        Err(e) => failure(e),
    }
}
pub(super) async fn jobs<B: Backend>(State(app): State<App<B>>, headers: HeaderMap) -> Response {
    let result = async { app.backend.stack_jobs(cookie(&headers)?).await }.await;
    match result {
        Ok(value) => axum::Json(value).into_response(),
        Err(e) => failure(e),
    }
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Cursor {
    #[serde(default)]
    after: i64,
}
pub(super) async fn progress<B: Backend>(
    State(app): State<App<B>>,
    Path(id): Path<String>,
    Query(query): Query<Cursor>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        app.backend
            .stack_job_progress(cookie(&headers)?, id, query.after)
            .await
    }
    .await;
    match result {
        Ok(value) => axum::Json(value).into_response(),
        Err(e) => failure(e),
    }
}
async fn cancel<B: Backend>(app: App<B>, id: String, headers: HeaderMap, plan: bool) -> Response {
    let result = async {
        let (token, csrf) = credentials(&app, &headers)?;
        app.backend.cancel_stack_action(token, csrf, id, plan).await
    }
    .await;
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failure(e),
    }
}
pub(super) async fn cancel_plan<B: Backend>(
    State(app): State<App<B>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    cancel(app, id, headers, true).await
}
pub(super) async fn cancel_job<B: Backend>(
    State(app): State<App<B>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    cancel(app, id, headers, false).await
}
