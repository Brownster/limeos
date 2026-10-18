//! Fixed ObserveHealth schedule metadata. No runner, alerts or notifications.
use super::*;
use axum::extract::{Path, Query};
use limeos_domain::automation::{MAX_REVISION, MAX_TICK_SCHEDULES, PauseSchedule, ScheduleView};

pub(super) fn routes<B: Backend>() -> Router<App<B>> {
    Router::new()
        .route(
            "/api/v1/automation/schedules",
            get(page::<B>).post(create::<B>),
        )
        .route("/api/v1/automation/schedules/{id}/pause", post(pause::<B>))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    after: Option<String>,
}

fn fixed_view(value: &ScheduleView) -> Result<()> {
    value
        .validate()
        .map_err(|_| Error(ErrorCode::StateNotDurable))?;
    if !value.schedule.definition.operation.observation_only()
        || !limeos_domain::opaque_id(&value.schedule.id)
    {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    Ok(())
}

fn bounded<T: serde::Serialize>(value: T, status: StatusCode) -> Result<Response> {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > 64 * 1024 - self.0 {
                return Err(std::io::Error::other("schedule response limit"));
            }
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(&mut Count(0), &value).map_err(|_| Error(ErrorCode::StateNotDurable))?;
    Ok((status, axum::Json(value)).into_response())
}

async fn page<B: Backend>(State(app): State<App<B>>, request: Request) -> Response {
    let deadline = Instant::now() + Duration::from_secs(10);
    // Own the Send request body; borrowing Request across await would require Sync.
    let result = async move {
        let token = cookie(request.headers())?;
        let Query(cursor) = Query::<Cursor>::try_from_uri(request.uri())
            .map_err(|_| Error(ErrorCode::InvalidInput))?;
        if cursor
            .after
            .as_deref()
            .is_some_and(|id| !limeos_domain::opaque_id(id))
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let value = app
            .backend
            .observe_health_schedules(token, cursor.after.clone(), deadline)
            .await?;
        if value.items.len() > MAX_TICK_SCHEDULES {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let mut previous = cursor.after.as_deref();
        for row in &value.items {
            fixed_view(row)?;
            if previous.is_some_and(|id| row.schedule.id.as_str() <= id) {
                return Err(Error(ErrorCode::StateNotDurable));
            }
            previous = Some(&row.schedule.id);
        }
        if value
            .next
            .as_deref()
            .is_some_and(|id| Some(id) != previous || value.items.is_empty())
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        bounded(value, StatusCode::OK)
    }
    .await;
    result.unwrap_or_else(failure)
}

async fn create<B: Backend>(State(app): State<App<B>>, request: Request) -> Response {
    let deadline = Instant::now() + Duration::from_secs(10);
    let result = async {
        let (token, csrf) = operations::credentials(&app, request.headers())?;
        let input: limeos_contracts::ObserveHealthScheduleInput =
            operations::input(request).await?;
        input.definition().validate()?;
        let value = app
            .backend
            .create_observe_health_schedule(token, csrf, input, deadline)
            .await?;
        fixed_view(&value)?;
        bounded(value, StatusCode::CREATED)
    }
    .await;
    result.unwrap_or_else(failure)
}

async fn pause<B: Backend>(
    State(app): State<App<B>>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let deadline = Instant::now() + Duration::from_secs(10);
    let result = async {
        let (token, csrf) = operations::credentials(&app, request.headers())?;
        let input: PauseSchedule = operations::input(request).await?;
        if !limeos_domain::opaque_id(&id) || !(1..=MAX_REVISION).contains(&input.revision) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let value = app
            .backend
            .pause_observe_health_schedule(token, csrf, id, input, deadline)
            .await?;
        fixed_view(&value)?;
        bounded(value, StatusCode::OK)
    }
    .await;
    result.unwrap_or_else(failure)
}

#[cfg(test)]
mod tests;
