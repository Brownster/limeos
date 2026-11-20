use super::*;
use axum::{
    extract::Query,
    response::sse::{Event, Sse},
};
use limeos_contracts::{HistoryRange, ResourceKind, ResourcePage};
use serde::Deserialize;
use std::convert::Infallible;

pub(super) async fn overview<B: Backend>(
    State(app): State<App<B>>,
    headers: HeaderMap,
) -> Response {
    let result = async { app.backend.observations(cookie(&headers)?).await }.await;
    match result {
        Ok(value) => axum::Json(value).into_response(),
        Err(e) => failure(e),
    }
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PageQuery {
    offset: Option<u32>,
    limit: Option<u32>,
    kind: Option<ResourceKind>,
    revision: Option<u64>,
}
pub(super) async fn resources<B: Backend>(
    State(app): State<App<B>>,
    headers: HeaderMap,
    query: std::result::Result<Query<PageQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let result = async {
        let Query(query) = query.map_err(|_| Error(ErrorCode::InvalidInput))?;
        let limit = query.limit.unwrap_or(20);
        let offset = query.offset.unwrap_or(0);
        if !(1..=50).contains(&limit) || offset > 1024 {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let overview = app.backend.observations(cookie(&headers)?).await?;
        if query.revision.is_some_and(|r| r != overview.revision) {
            return Err(Error(ErrorCode::Conflict));
        }
        let resources: Vec<_> = overview
            .resources
            .into_iter()
            .filter(|r| query.kind.is_none_or(|kind| r.kind == kind))
            .collect();
        let total = resources.len() as u32;
        let items = resources
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect();
        Ok(ResourcePage {
            revision: overview.revision,
            items,
            total,
            next_offset: (offset + limit < total).then_some(offset + limit),
            sources: overview.sources,
        })
    }
    .await;
    match result {
        Ok(value) => axum::Json(value).into_response(),
        Err(e) => failure(e),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HistoryQuery {
    range: HistoryRange,
}
pub(super) async fn history<B: Backend>(
    State(app): State<App<B>>,
    headers: HeaderMap,
    query: std::result::Result<Query<HistoryQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let result = async {
        let Query(q) = query.map_err(|_| Error(ErrorCode::InvalidInput))?;
        app.backend.history(cookie(&headers)?, q.range).await
    }
    .await;
    match result {
        Ok(value) => axum::Json(value).into_response(),
        Err(e) => failure(e),
    }
}
/// A stream ends after this; EventSource reconnects and gets a full snapshot.
const STREAM_LIFETIME: Duration = Duration::from_secs(25);
/// Open streams one session may hold, so one viewer's tabs cannot take all
/// eight. Hidden tabs close theirs, so two covers two visible windows.
pub(super) const STREAMS_PER_SESSION: usize = 2;
/// One of a session's stream places, given back when the stream ends.
struct StreamSlot {
    streams: Arc<Mutex<HashMap<String, usize>>>,
    token: String,
}
impl StreamSlot {
    fn take(streams: &Arc<Mutex<HashMap<String, usize>>>, token: &str) -> Option<Self> {
        let mut open = streams.lock().ok()?;
        let count = open.entry(token.to_owned()).or_default();
        if *count >= STREAMS_PER_SESSION {
            return None;
        }
        *count += 1;
        Some(Self {
            streams: streams.clone(),
            token: token.to_owned(),
        })
    }
}
impl Drop for StreamSlot {
    fn drop(&mut self) {
        let mut open = self
            .streams
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(count) = open.get_mut(&self.token) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                open.remove(&self.token);
            }
        }
    }
}
fn session_streams_full() -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        axum::Json(ErrorEnvelope {
            code: ErrorCode::Overloaded,
            message: format!(
                "This session already has {STREAMS_PER_SESSION} live observation streams open. Close another window or tab and retry."
            ),
            retry: true,
            audit_id: "http-session-streams".into(),
        }),
    )
        .into_response()
}
pub(super) async fn stream<B: Backend>(State(app): State<App<B>>, headers: HeaderMap) -> Response {
    if headers
        .get("sec-fetch-site")
        .is_some_and(|h| h != "same-origin" && h != "none")
    {
        return failure(Error(ErrorCode::Forbidden));
    }
    if headers.get("origin").is_some() {
        if let Err(e) = origin(&app, &headers) {
            return failure(e);
        }
    }
    if headers
        .get("last-event-id")
        .is_some_and(|h| h.len() > 20 || !h.as_bytes().iter().all(u8::is_ascii_digit))
    {
        return failure(Error(ErrorCode::InvalidInput));
    }
    let token = match cookie(&headers) {
        Ok(t) => t,
        Err(e) => return failure(e),
    };
    let permit = match app.readers.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => return failure(Error(ErrorCode::Overloaded)),
    };
    // Held with the global permit, so the map has at most eight entries.
    let Some(slot) = StreamSlot::take(&app.streams, &token) else {
        return session_streams_full();
    };
    let initial = match app.backend.observations(token.clone()).await {
        Ok(v) => v,
        Err(e) => return failure(e),
    };
    let receiver = app.backend.changes();
    let state = (
        app.backend,
        token,
        receiver,
        Some(initial),
        tokio::time::Instant::now(),
        0usize,
        (permit, slot),
    );
    let events = futures_util::stream::unfold(
        state,
        |(backend, token, mut changes, initial, start, mut bytes, permit)| async move {
            if start.elapsed() > STREAM_LIFETIME {
                return None;
            }
            let value = if let Some(v) = initial {
                v
            } else {
                // Revalidate identity/scopes even when sources stop producing events.
                tokio::select! {_=tokio::time::sleep_until(start+STREAM_LIFETIME)=>{return None;},_=tokio::time::sleep(Duration::from_secs(5))=>{},result=changes.changed()=>{if result.is_err(){return None;}}}
                match tokio::time::timeout(
                    Duration::from_secs(1),
                    backend.observations(token.clone()),
                )
                .await
                {
                    Ok(Ok(v)) => v,
                    Err(_) => return None,
                    Ok(Err(_)) => return None,
                }
            };
            let json = serde_json::to_string(&value).ok()?;
            bytes += json.len() + 64; // Include the SSE envelope in the byte budget.
            if bytes > 128 * 1024 {
                return None;
            }
            let event = Event::default()
                .event("snapshot")
                .id(value.revision.to_string())
                .data(json);
            Some((
                Ok::<_, Infallible>(event),
                (backend, token, changes, None, start, bytes, permit),
            ))
        },
    );
    Sse::new(events).into_response()
}
