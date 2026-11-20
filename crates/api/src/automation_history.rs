//! Owned persisted alert/notification history. GET never collects or publishes.
use super::*;
use gate::{Entry, Path, Query, read, write};
use limeos_domain::automation::{
    AcknowledgeNotification, AlertState, AutomationPage, DeliveryChannel, DeliveryState,
    MAX_REVISION, MAX_TICK_SCHEDULES, Notification,
};

pub(super) fn routes<B: Backend>() -> Vec<Entry<B>> {
    vec![
        read("/api/v1/automation/alerts", alerts::<B>),
        read("/api/v1/automation/notifications", notifications::<B>),
        write(
            "/api/v1/automation/notifications/{id}/acknowledgment",
            acknowledge::<B>,
        ),
    ]
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    after: Option<String>,
}

fn bounded<T: serde::Serialize>(value: T) -> Result<Response> {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > 64 * 1024 - self.0 {
                return Err(std::io::Error::other("history response limit"));
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

fn page_order<T>(
    page: &AutomationPage<T>,
    after: Option<&str>,
    key: impl Fn(&T) -> &str,
) -> Result<()> {
    if page.items.len() > MAX_TICK_SCHEDULES {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    let mut previous = after;
    for row in &page.items {
        let current = key(row);
        if previous.is_some_and(|value| current <= value) {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        previous = Some(current);
    }
    if page
        .next
        .as_deref()
        .is_some_and(|next| page.items.is_empty() || Some(next) != previous)
    {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    Ok(())
}

fn notification(value: &Notification) -> Result<()> {
    value
        .validate()
        .map_err(|_| Error(ErrorCode::StateNotDurable))?;
    if !limeos_domain::opaque_id(&value.id) || value.channel != DeliveryChannel::InApp {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    match value.delivery {
        DeliveryState::Pending if value.acknowledged_at.is_none() => {}
        DeliveryState::Delivered { at } if value.acknowledged_at.is_none_or(|ack| ack >= at) => {}
        _ => return Err(Error(ErrorCode::StateNotDurable)),
    }
    Ok(())
}

async fn alerts<B: Backend>(State(app): State<App<B>>, request: Request) -> Response {
    let deadline = Instant::now() + Duration::from_secs(10);
    let result = async move {
        let token = cookie(request.headers())?;
        let Query(cursor) = Query::<Cursor>::try_from_uri(request.uri())
            .map_err(|_| Error(ErrorCode::InvalidInput))?;
        if cursor.after.as_deref().is_some_and(|value| {
            value.is_empty() || value.len() > 256 || value.chars().any(char::is_control)
        }) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let value = app
            .backend
            .observe_health_alerts(token, cursor.after.clone(), deadline)
            .await?;
        page_order(&value, cursor.after.as_deref(), |row: &AlertState| &row.key)?;
        for row in &value.items {
            row.validate()
                .map_err(|_| Error(ErrorCode::StateNotDurable))?;
        }
        bounded(value)
    }
    .await;
    result.unwrap_or_else(failure)
}

async fn notifications<B: Backend>(State(app): State<App<B>>, request: Request) -> Response {
    let deadline = Instant::now() + Duration::from_secs(10);
    let result = async move {
        let token = cookie(request.headers())?;
        let Query(cursor) = Query::<Cursor>::try_from_uri(request.uri())
            .map_err(|_| Error(ErrorCode::InvalidInput))?;
        if cursor
            .after
            .as_deref()
            .is_some_and(|value| !limeos_domain::opaque_id(value))
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let value = app
            .backend
            .observe_health_notifications(token, cursor.after.clone(), deadline)
            .await?;
        page_order(&value, cursor.after.as_deref(), |row: &Notification| {
            &row.id
        })?;
        for row in &value.items {
            notification(row)?;
        }
        bounded(value)
    }
    .await;
    result.unwrap_or_else(failure)
}

async fn acknowledge<B: Backend>(
    State(app): State<App<B>>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let deadline = Instant::now() + Duration::from_secs(10);
    let result = async {
        let (token, csrf) = operations::credentials(&app, request.headers())?;
        let input: AcknowledgeNotification = operations::input(request).await?;
        if !limeos_domain::opaque_id(&id) || !(1..=MAX_REVISION).contains(&input.revision) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let revision = input.revision;
        let value = app
            .backend
            .acknowledge_observe_health_notification(token, csrf, id.clone(), input, deadline)
            .await?;
        notification(&value)?;
        if value.id != id
            || (value.revision != revision && Some(value.revision) != revision.checked_add(1))
            || value.acknowledged_at.is_none()
            || !matches!(value.delivery, DeliveryState::Delivered { .. })
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        bounded(value)
    }
    .await;
    result.unwrap_or_else(failure)
}

#[cfg(test)]
mod tests;
