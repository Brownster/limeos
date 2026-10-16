use super::*;
use axum::extract::Path;
use operations::{credentials, input};
// A valid reviewed mount proposal may occupy 24 KiB. Keep its queue wrapper
// bounded without changing the 16 KiB parser used by other operation routes.
async fn queue_input(request: Request) -> Result<limeos_contracts::QueueStorageMountInput> {
    if request
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or_default().trim())
        != Some("application/json")
    {
        return Err(Error(ErrorCode::InvalidInput));
    }
    let body = to_bytes(request.into_body(), 32 * 1024)
        .await
        .map_err(|_| Error(ErrorCode::InvalidInput))?;
    serde_json::from_slice(&body).map_err(|_| Error(ErrorCode::InvalidInput))
}
pub(super) async fn catalog<B: Backend>(State(app): State<App<B>>, headers: HeaderMap) -> Response {
    match async { app.backend.storage_mount_catalog(cookie(&headers)?).await }.await {
        Ok(value) => axum::Json(value).into_response(),
        Err(error) => failure(error),
    }
}

pub(super) async fn plan<B: Backend>(State(app): State<App<B>>, request: Request) -> Response {
    let result = async {
        let (token, csrf) = credentials(&app, request.headers())?;
        app.backend
            .session(token.clone(), Some(csrf.clone()))
            .await?;
        app.backend
            .plan_storage_mount(token, csrf, input(request).await?)
            .await
    }
    .await;
    match result {
        Ok(value) => (StatusCode::CREATED, axum::Json(value)).into_response(),
        Err(e) => failure(e),
    }
}
pub(super) async fn read<B: Backend>(
    State(app): State<App<B>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    match async { app.backend.storage_mount_plan(cookie(&headers)?, id).await }.await {
        Ok(value) => axum::Json(value).into_response(),
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
            .approve_storage_mount(token, csrf, id, value.digest)
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
            .queue_storage_mount(token, csrf, queue_input(request).await?)
            .await
    }
    .await;
    match result {
        Ok(value) => (StatusCode::ACCEPTED, axum::Json(value)).into_response(),
        Err(e) => failure(e),
    }
}
pub(super) async fn job<B: Backend>(
    State(app): State<App<B>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    match async { app.backend.storage_mount_job(cookie(&headers)?, id).await }.await {
        Ok(value) => axum::Json(value).into_response(),
        Err(e) => failure(e),
    }
}
pub(super) async fn cancel<B: Backend>(
    State(app): State<App<B>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        let (token, csrf) = credentials(&app, &headers)?;
        app.backend
            .session(token.clone(), Some(csrf.clone()))
            .await?;
        app.backend.cancel_storage_mount(token, csrf, id).await
    }
    .await;
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failure(e),
    }
}
pub(super) async fn cancel_job<B: Backend>(
    State(app): State<App<B>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        let (token, csrf) = credentials(&app, &headers)?;
        app.backend
            .session(token.clone(), Some(csrf.clone()))
            .await?;
        app.backend.cancel_storage_mount_job(token, csrf, id).await
    }
    .await;
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failure(e),
    }
}

pub(super) async fn jobs<B: Backend>(State(app): State<App<B>>, headers: HeaderMap) -> Response {
    match async { app.backend.storage_mount_jobs(cookie(&headers)?).await }.await {
        Ok(value) => axum::Json(value).into_response(),
        Err(error) => failure(error),
    }
}
