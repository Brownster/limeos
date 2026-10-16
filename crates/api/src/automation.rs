//! Proposed routes; the integrator mounts these under the existing API middleware.
use super::*;
use axum::extract::{Path, Query};
use limeos_domain::automation::*;

/// Implement with the existing Core database worker and current session/grant
/// checks. Inputs carry no actor, permission overrides or reusable approval token.
pub trait AutomationBackend: Backend {
    fn automation_schedules(
        &self,
        _token: String,
        _after: Option<String>,
    ) -> impl Future<Output = Result<AutomationPage<ScheduleView>>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn automation_alerts(
        &self,
        _token: String,
        _after: Option<String>,
    ) -> impl Future<Output = Result<AutomationPage<AlertState>>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn automation_notifications(
        &self,
        _token: String,
        _after: Option<String>,
    ) -> impl Future<Output = Result<AutomationPage<Notification>>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn automation_create(
        &self,
        _token: String,
        _csrf: String,
        _input: CreateSchedule,
    ) -> impl Future<Output = Result<ScheduleView>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn automation_pause(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
        _input: PauseSchedule,
    ) -> impl Future<Output = Result<ScheduleView>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
    fn automation_acknowledge(
        &self,
        _token: String,
        _csrf: String,
        _id: String,
        _input: AcknowledgeNotification,
    ) -> impl Future<Output = Result<Notification>> + Send {
        async { Err(Error(ErrorCode::Unavailable)) }
    }
}
pub(super) fn routes<B: AutomationBackend>() -> Router<App<B>> {
    Router::new()
        .route(
            "/api/v1/automation/schedules",
            get(schedules::<B>).post(create::<B>),
        )
        .route("/api/v1/automation/schedules/{id}/pause", post(pause::<B>))
        .route("/api/v1/automation/alerts", get(alerts::<B>))
        .route("/api/v1/automation/notifications", get(notifications::<B>))
        .route(
            "/api/v1/automation/notifications/{id}/acknowledgment",
            post(acknowledge::<B>),
        )
}
#[derive(serde::Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Cursor {
    after: Option<String>,
}
impl Cursor {
    fn validate(&self) -> Result<()> {
        if self
            .after
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 256 || id.chars().any(char::is_control))
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        Ok(())
    }
}
fn check_page<T>(value: &AutomationPage<T>) -> Result<()> {
    if value.items.len() > MAX_TICK_SCHEDULES
        || value
            .next
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 256 || id.chars().any(char::is_control))
    {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    Ok(())
}
fn bounded_json<T: serde::Serialize>(value: T, status: StatusCode) -> Response {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > 64 * 1024 - self.0 {
                return Err(std::io::Error::other("response bound"));
            }
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    if serde_json::to_writer(&mut Count(0), &value).is_err() {
        return failure(Error(ErrorCode::StateNotDurable));
    }
    (status, axum::Json(value)).into_response()
}
fn page<T: serde::Serialize>(value: AutomationPage<T>) -> Response {
    if let Err(error) = check_page(&value) {
        return failure(error);
    }
    bounded_json(value, StatusCode::OK)
}
async fn schedules<B: AutomationBackend>(
    State(app): State<App<B>>,
    Query(cursor): Query<Cursor>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        let token = cookie(&headers)?;
        app.backend.session(token.clone(), None).await?;
        cursor.validate()?;
        let rows = app
            .backend
            .automation_schedules(token, cursor.after)
            .await?;
        check_page(&rows)?;
        for row in &rows.items {
            row.validate()?;
        }
        Ok(rows)
    }
    .await;
    match result {
        Ok(value) => page(value),
        Err(error) => failure(error),
    }
}
async fn alerts<B: AutomationBackend>(
    State(app): State<App<B>>,
    Query(cursor): Query<Cursor>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        let token = cookie(&headers)?;
        app.backend.session(token.clone(), None).await?;
        cursor.validate()?;
        let rows = app.backend.automation_alerts(token, cursor.after).await?;
        check_page(&rows)?;
        for row in &rows.items {
            row.validate()?;
        }
        Ok(rows)
    }
    .await;
    match result {
        Ok(value) => page(value),
        Err(error) => failure(error),
    }
}
async fn notifications<B: AutomationBackend>(
    State(app): State<App<B>>,
    Query(cursor): Query<Cursor>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        let token = cookie(&headers)?;
        app.backend.session(token.clone(), None).await?;
        cursor.validate()?;
        let rows = app
            .backend
            .automation_notifications(token, cursor.after)
            .await?;
        check_page(&rows)?;
        for row in &rows.items {
            row.validate()?;
        }
        Ok(rows)
    }
    .await;
    match result {
        Ok(value) => page(value),
        Err(error) => failure(error),
    }
}
async fn create<B: AutomationBackend>(State(app): State<App<B>>, request: Request) -> Response {
    let result = async {
        let (token, csrf) = operations::credentials(&app, request.headers())?;
        app.backend
            .session(token.clone(), Some(csrf.clone()))
            .await?;
        let input: CreateSchedule = operations::input(request).await?;
        input.definition.validate()?;
        let value = app.backend.automation_create(token, csrf, input).await?;
        value.validate()?;
        Ok(value)
    }
    .await;
    match result {
        Ok(value) => bounded_json(value, StatusCode::CREATED),
        Err(error) => failure(error),
    }
}
async fn pause<B: AutomationBackend>(
    State(app): State<App<B>>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let result = async {
        let (token, csrf) = operations::credentials(&app, request.headers())?;
        app.backend
            .session(token.clone(), Some(csrf.clone()))
            .await?;
        let input: PauseSchedule = operations::input(request).await?;
        if !limeos_domain::identifier(&id) || !(1..=MAX_REVISION).contains(&input.revision) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let value = app.backend.automation_pause(token, csrf, id, input).await?;
        value.validate()?;
        Ok(value)
    }
    .await;
    match result {
        Ok(value) => bounded_json(value, StatusCode::OK),
        Err(error) => failure(error),
    }
}
async fn acknowledge<B: AutomationBackend>(
    State(app): State<App<B>>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let result = async {
        let (token, csrf) = operations::credentials(&app, request.headers())?;
        app.backend
            .session(token.clone(), Some(csrf.clone()))
            .await?;
        let input: AcknowledgeNotification = operations::input(request).await?;
        if !limeos_domain::identifier(&id) || !(1..=MAX_REVISION).contains(&input.revision) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let value = app
            .backend
            .automation_acknowledge(token, csrf, id, input)
            .await?;
        value.validate()?;
        Ok(value)
    }
    .await;
    match result {
        Ok(value) => bounded_json(value, StatusCode::OK),
        Err(error) => failure(error),
    }
}

#[cfg(test)]
#[path = "automation/tests.rs"]
mod tests;
