use super::*;
use axum::extract::{Path, Query};

fn credentials<B>(app: &App<B>, headers: &HeaderMap) -> Result<(String, String)> {
    origin(app, headers)?;
    let token = cookie(headers)?;
    let csrf = headers
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
        .filter(|s| limeos_domain::opaque_id(s))
        .ok_or(Error(ErrorCode::Forbidden))?;
    Ok((token, csrf.into()))
}
async fn input<T: serde::de::DeserializeOwned>(request: Request) -> Result<T> {
    if request
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or_default().trim())
        != Some("application/json")
    {
        return Err(Error(ErrorCode::InvalidInput));
    }
    let bytes = to_bytes(request.into_body(), 16 * 1024)
        .await
        .map_err(|_| Error(ErrorCode::InvalidInput))?;
    serde_json::from_slice(&bytes).map_err(|_| Error(ErrorCode::InvalidInput))
}
pub(super) async fn plan<B: Backend>(State(app): State<App<B>>, request: Request) -> Response {
    let result = async {
        let (token, csrf) = credentials(&app, request.headers())?;
        app.backend
            .session(token.clone(), Some(csrf.clone()))
            .await?;
        app.backend
            .plan_restart(token, csrf, input(request).await?)
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
        app.backend
            .session(token.clone(), Some(csrf.clone()))
            .await?;
        let value: limeos_contracts::ApprovalInput = input(request).await?;
        app.backend
            .approve_restart(token, csrf, id, value.digest)
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
        app.backend
            .session(token.clone(), Some(csrf.clone()))
            .await?;
        app.backend
            .queue_restart(token, csrf, input(request).await?)
            .await
    }
    .await;
    match result {
        Ok(value) => (StatusCode::ACCEPTED, axum::Json(value)).into_response(),
        Err(e) => failure(e),
    }
}
pub(super) async fn jobs<B: Backend>(State(app): State<App<B>>, headers: HeaderMap) -> Response {
    let result = async { app.backend.restart_jobs(cookie(&headers)?).await }.await;
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
            .restart_progress(cookie(&headers)?, id, query.after)
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
        app.backend
            .session(token.clone(), Some(csrf.clone()))
            .await?;
        app.backend.cancel_restart(token, csrf, id, plan).await
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
