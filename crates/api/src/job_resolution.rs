//! Operator resolution of stuck jobs. Interactive administrator sessions only:
//! no bearer or key credential is ever read, and the decision needs the CSRF
//! header, the Origin rule and the administrator's password like other
//! irreversible actions. See `docs/p06-job-resolution.md`.
use super::operations::{credentials, input};
use super::*;
use axum::extract::Path;
use limeos_contracts::{ResolveJobInput, ResolveStatus};

/// Reject anything that tries to authenticate with a header credential. The
/// only accepted credential is the session cookie.
fn cookie_only(headers: &HeaderMap) -> Result<()> {
    if headers.contains_key("authorization") || headers.contains_key("x-api-key") {
        return Err(Error(ErrorCode::Forbidden));
    }
    Ok(())
}

pub(super) async fn read<B: Backend>(
    State(app): State<App<B>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        cookie_only(&headers)?;
        if !limeos_domain::opaque_id(&id) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let token = cookie(&headers)?;
        app.backend.session(token.clone(), None).await?;
        app.backend.job_resolution(token, id).await
    }
    .await;
    match result {
        Ok(value) => axum::Json(value).into_response(),
        Err(e) => failure(e),
    }
}

/// Every principal's stuck jobs, for the administrator's banner. A GET: no
/// CSRF, but the same cookie-only, administrator-only rule as the evidence.
pub(super) async fn needs_attention<B: Backend>(
    State(app): State<App<B>>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        cookie_only(&headers)?;
        let token = cookie(&headers)?;
        app.backend.session(token.clone(), None).await?;
        app.backend.needs_attention_jobs(token).await
    }
    .await;
    match result {
        Ok(value) => axum::Json(value).into_response(),
        Err(e) => failure(e),
    }
}

pub(super) async fn resolve<B: Backend>(
    State(app): State<App<B>>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let result = async {
        cookie_only(request.headers())?;
        if !limeos_domain::opaque_id(&id) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let (token, csrf) = credentials(&app, request.headers())?;
        let principal = app
            .backend
            .session(token.clone(), Some(csrf.clone()))
            .await?
            .principal;
        let value: ResolveJobInput = input(request).await?;
        if !limeos_contracts::password_well_formed(&value.password) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        // Failed password attempts are budgeted per authenticated principal.
        password_attempt(&app, Attempt::Principal(&principal.id), |attempt| {
            app.backend.resolve_job(token, csrf, id, value, attempt)
        })
        .await
    }
    .await;
    match result {
        Ok(value) => {
            let status = match value.status {
                ResolveStatus::Resolved => StatusCode::OK,
                ResolveStatus::Pending => StatusCode::ACCEPTED,
                ResolveStatus::StaleEvidence | ResolveStatus::UnsupportedDeclaration => {
                    StatusCode::CONFLICT
                }
            };
            (status, axum::Json(value)).into_response()
        }
        Err(e) => failure(e),
    }
}

#[cfg(test)]
mod tests;
