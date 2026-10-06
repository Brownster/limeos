use super::*;
use axum::extract::Path;
use operations::{credentials, input};

pub(super) async fn container_dependencies<B: Backend>(
    State(app): State<App<B>>,
    uri: axum::http::Uri,
    headers: HeaderMap,
) -> Response {
    let result = async {
        let token = cookie(&headers)?;
        if uri.query().is_some() {
            return Err(Error(ErrorCode::InvalidInput));
        }
        app.backend.container_storage_inventory(token).await
    }
    .await;
    match result {
        Ok(value) => axum::Json(value).into_response(),
        Err(e) => failure(e),
    }
}

pub(super) async fn target_plan<B: Backend>(
    State(app): State<App<B>>,
    request: Request,
) -> Response {
    let result = async {
        let (token, csrf) = credentials(&app, request.headers())?;
        app.backend
            .session(token.clone(), Some(csrf.clone()))
            .await?;
        app.backend
            .plan_storage_targets(token, csrf, input(request).await?)
            .await
    }
    .await;
    match result {
        Ok(value) => (StatusCode::CREATED, axum::Json(value)).into_response(),
        Err(e) => failure(e),
    }
}
pub(super) async fn target_read<B: Backend>(
    State(app): State<App<B>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    match async { app.backend.storage_target_plan(cookie(&headers)?, id).await }.await {
        Ok(value) => axum::Json(value).into_response(),
        Err(e) => failure(e),
    }
}
pub(super) async fn target_approve<B: Backend>(
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
            .approve_storage_targets(token, csrf, id, value.digest)
            .await
    }
    .await;
    match result {
        Ok(value) => axum::Json(value).into_response(),
        Err(e) => failure(e),
    }
}
pub(super) async fn target_queue<B: Backend>(
    State(app): State<App<B>>,
    request: Request,
) -> Response {
    let result = async {
        let (token, csrf) = credentials(&app, request.headers())?;
        app.backend
            .session(token.clone(), Some(csrf.clone()))
            .await?;
        app.backend
            .queue_storage_targets(token, csrf, input(request).await?)
            .await
    }
    .await;
    match result {
        Ok(value) => (StatusCode::ACCEPTED, axum::Json(value)).into_response(),
        Err(e) => failure(e),
    }
}
pub(super) async fn target_job<B: Backend>(
    State(app): State<App<B>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    match async { app.backend.storage_target_job(cookie(&headers)?, id).await }.await {
        Ok(value) => axum::Json(value).into_response(),
        Err(e) => failure(e),
    }
}
pub(super) async fn target_cancel<B: Backend>(
    State(app): State<App<B>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        let (token, csrf) = credentials(&app, &headers)?;
        app.backend
            .session(token.clone(), Some(csrf.clone()))
            .await?;
        app.backend.cancel_storage_targets(token, csrf, id).await
    }
    .await;
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failure(e),
    }
}
pub(super) async fn target_cancel_job<B: Backend>(
    State(app): State<App<B>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        let (token, csrf) = credentials(&app, &headers)?;
        app.backend
            .session(token.clone(), Some(csrf.clone()))
            .await?;
        app.backend.cancel_storage_target_job(token, csrf, id).await
    }
    .await;
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failure(e),
    }
}

pub(super) async fn inventory<B: Backend>(
    State(app): State<App<B>>,
    headers: HeaderMap,
) -> Response {
    match async { app.backend.storage_inventory(cookie(&headers)?).await }.await {
        Ok(value) => axum::Json(value).into_response(),
        Err(e) => failure(e),
    }
}
pub(super) async fn plan<B: Backend>(State(app): State<App<B>>, request: Request) -> Response {
    let result = async {
        let (token, csrf) = credentials(&app, request.headers())?;
        app.backend
            .session(token.clone(), Some(csrf.clone()))
            .await?;
        app.backend
            .plan_storage(token, csrf, input(request).await?)
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
    match async { app.backend.storage_plan(cookie(&headers)?, id).await }.await {
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
            .approve_storage(token, csrf, id, value.digest)
            .await
    }
    .await;
    match result {
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
        app.backend.cancel_storage(token, csrf, id).await
    }
    .await;
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failure(e),
    }
}
