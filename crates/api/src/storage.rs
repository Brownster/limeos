use super::*;
use axum::extract::Path;
use operations::{credentials, input};

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
