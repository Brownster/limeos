//! Current owned configuration only; no collector, timer or send endpoint.
use super::*;
use limeos_domain::automation::{HealthPolicyView, MAX_HEALTH_POLICY_BYTES, ReplaceHealthPolicy};

pub(super) fn routes<B: Backend>() -> Router<App<B>> {
    Router::new().route(
        "/api/v1/automation/health-policy",
        get(read::<B>).post(replace::<B>),
    )
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NoQuery {}
fn query(request: &Request) -> Result<()> {
    axum::extract::Query::<NoQuery>::try_from_uri(request.uri())
        .map_err(|_| Error(ErrorCode::InvalidInput))?;
    Ok(())
}
fn output(value: HealthPolicyView) -> Result<Response> {
    value
        .validate()
        .map_err(|_| Error(ErrorCode::StateNotDurable))?;
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_HEALTH_POLICY_BYTES + 4096 - self.0 {
                return Err(std::io::Error::other("policy response limit"));
            }
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(&mut Count(0), &value).map_err(|_| Error(ErrorCode::StateNotDurable))?;
    Ok(axum::Json(value).into_response())
}
async fn read<B: Backend>(State(app): State<App<B>>, request: Request) -> Response {
    let deadline = Instant::now() + Duration::from_secs(10);
    let result = async move {
        let token = cookie(request.headers())?;
        query(&request)?;
        output(app.backend.observe_health_policy(token, deadline).await?)
    }
    .await;
    result.unwrap_or_else(failure)
}
async fn replace<B: Backend>(State(app): State<App<B>>, request: Request) -> Response {
    let deadline = Instant::now() + Duration::from_secs(10);
    let result = async move {
        let (token, csrf) = operations::credentials(&app, request.headers())?;
        query(&request)?;
        if request
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(|value| value.split(';').next().unwrap_or_default().trim())
            != Some("application/json")
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let bytes = to_bytes(request.into_body(), MAX_HEALTH_POLICY_BYTES + 4096)
            .await
            .map_err(|_| Error(ErrorCode::InvalidInput))?;
        let input: ReplaceHealthPolicy =
            serde_json::from_slice(&bytes).map_err(|_| Error(ErrorCode::InvalidInput))?;
        input.validate()?;
        let expected = input.clone();
        let value = app
            .backend
            .replace_observe_health_policy(token, csrf, input, deadline)
            .await?;
        if value.revision == 0
            || (value.revision != expected.revision
                && Some(value.revision) != expected.revision.checked_add(1))
            || value.definition != expected.definition
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        output(value)
    }
    .await;
    result.unwrap_or_else(failure)
}

#[cfg(test)]
mod tests;
