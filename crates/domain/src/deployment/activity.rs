//! Read-only media activity projections. Plain samples confer no source authority.
use crate::{ContainerSnapshot, Error, ErrorCode, Result};
use serde::Serialize;
use std::io::Write;

pub const MAX_TARGETS: usize = 8;
pub const MAX_ITEMS: usize = 24;
pub const MAX_DISPLAY: usize = 12;
pub const MAX_TEXT_BYTES: usize = 32 * 1024;
pub const MAX_REPORT_BYTES: usize = 64 * 1024;
pub const MAX_AGE_SECONDS: i64 = 8;
pub const MAX_NUMBER: f64 = 9_007_199_254_740_991.0;
const MAX_TIME: i64 = 253_402_300_799;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Jellyfin,
    Emby,
    Sabnzbd,
    Transmission,
    Sonarr,
    Radarr,
    Lidarr,
    Readarr,
}
impl Provider {
    pub fn family(self) -> u8 {
        match self {
            Self::Jellyfin | Self::Emby => 0,
            Self::Sabnzbd | Self::Transmission => 1,
            _ => 2,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Target {
    pub container: ContainerSnapshot,
    pub name: String,
    pub provider: Provider,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Stream {
    pub title: String,
    pub subtitle: String,
    pub user: String,
    pub client: String,
    pub device: String,
    pub play_method: Option<String>,
    pub paused: bool,
    pub transcoding: bool,
    pub transcode_reason: Option<String>,
    pub bitrate: Option<f64>,
    pub progress_percent: Option<f64>,
    pub position_seconds: Option<f64>,
    pub duration_seconds: Option<f64>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Download {
    pub name: String,
    pub state: String,
    pub progress_percent: Option<f64>,
    pub size_bytes: Option<f64>,
    pub remaining_bytes: Option<f64>,
    pub speed_bytes: Option<f64>,
    pub eta: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct QueueItem {
    pub title: String,
    pub state: String,
    pub status: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Queue {
    pub total: u32,
    pub observed: u16,
    pub problems: u16,
    pub complete: bool,
    /// At most three selected records; observed/problems cover the admitted page.
    pub items: Vec<QueueItem>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ActivityData {
    Streams(Vec<Stream>),
    Downloads {
        items: Vec<Download>,
        complete: bool,
        aggregate_speed: Option<f64>,
    },
    Queue(Queue),
}
impl ActivityData {
    pub fn complete(&self) -> bool {
        match self {
            Self::Streams(_) => true,
            Self::Downloads { complete, .. } => *complete,
            Self::Queue(q) => q.complete,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadFailure {
    Unavailable,
    CredentialUnavailable,
    Timeout,
    InvalidResponse,
    Unsupported,
}
pub struct Sample<'a> {
    pub target: &'a Target,
    pub sampled_at: i64,
    pub result: std::result::Result<&'a ActivityData, ReadFailure>,
}
pub struct ActivityInput<'a> {
    pub engine_id: &'a str,
    pub sampled_at: i64,
    pub complete: bool,
    pub targets: &'a [Target],
    pub samples: &'a [Sample<'a>],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    Fresh,
    Stale,
    Future,
    Incomplete,
    Unavailable,
    IdentityMismatch,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    Ready,
    Partial,
    Stopped,
    Stale,
    Future,
    IdentityMismatch,
    Unavailable,
    CredentialUnavailable,
    Timeout,
    InvalidResponse,
    Unsupported,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ServiceView {
    pub target: Target,
    pub sampled_at: Option<i64>,
    pub state: ServiceState,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Sourced<T> {
    pub source: String,
    pub data: T,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Totals {
    pub streams: Option<u16>,
    pub transcodes: Option<u16>,
    pub downloads: Option<u16>,
    pub download_speed: Option<f64>,
    pub queued: Option<u32>,
    pub queue_problems: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ActivitySnapshot {
    pub version: u16,
    pub generated_at: i64,
    pub sampled_at: Option<i64>,
    pub max_age_seconds: i64,
    pub source: SourceState,
    pub complete: bool,
    pub services: Vec<ServiceView>,
    pub streams: Vec<Sourced<Stream>>,
    pub downloads: Vec<Sourced<Download>>,
    pub queues: Vec<Sourced<Queue>>,
    pub omitted_streams: u16,
    pub omitted_downloads: u16,
    pub totals: Totals,
}
fn check(ok: bool) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error(ErrorCode::InvalidInput))
    }
}
pub fn selected_text(s: &str, max: usize) -> bool {
    s.len() <= max && !s.chars().any(char::is_control)
}
fn number(v: Option<f64>, max: f64) -> bool {
    v.is_none_or(|v| v.is_finite() && (0.0..=max).contains(&v))
}
impl Target {
    fn validate(&self) -> Result<()> {
        self.container.validate()?;
        check(!self.name.is_empty() && selected_text(&self.name, 128))
    }
}
impl ActivityData {
    /// Validate selected display data even when supplied without a byte decoder.
    pub fn validate(&self, provider: Provider) -> Result<()> {
        match self {
            Self::Streams(rows) => {
                check(provider.family() == 0 && rows.len() <= MAX_ITEMS)?;
                for r in rows {
                    check(
                        [&r.title, &r.subtitle, &r.user, &r.client, &r.device]
                            .iter()
                            .all(|s| selected_text(s, 200))
                            && r.play_method.as_ref().is_none_or(|s| selected_text(s, 64))
                            && r.transcode_reason
                                .as_ref()
                                .is_none_or(|s| selected_text(s, 512))
                            && number(r.bitrate, MAX_NUMBER)
                            && number(r.progress_percent, 100.0)
                            && number(r.position_seconds, MAX_NUMBER)
                            && number(r.duration_seconds, MAX_NUMBER),
                    )?;
                }
            }
            Self::Downloads {
                items,
                aggregate_speed,
                ..
            } => {
                check(
                    provider.family() == 1
                        && items.len() <= MAX_ITEMS
                        && number(*aggregate_speed, MAX_NUMBER),
                )?;
                for r in items {
                    check(
                        selected_text(&r.name, 200)
                            && selected_text(&r.state, 64)
                            && r.eta.as_ref().is_none_or(|s| selected_text(s, 64))
                            && number(r.progress_percent, 100.0)
                            && number(r.size_bytes, MAX_NUMBER)
                            && number(r.remaining_bytes, MAX_NUMBER)
                            && number(r.speed_bytes, MAX_NUMBER),
                    )?;
                }
            }
            Self::Queue(q) => {
                check(
                    provider.family() == 2
                        && usize::from(q.observed) <= MAX_ITEMS
                        && q.problems <= q.observed
                        && u32::from(q.observed) <= q.total
                        && (!q.complete || u32::from(q.observed) == q.total)
                        && q.items.len() <= 3
                        && q.items.len() <= usize::from(q.observed),
                )?;
                for r in &q.items {
                    check(
                        selected_text(&r.title, 200)
                            && selected_text(&r.state, 64)
                            && selected_text(&r.status, 64),
                    )?;
                }
            }
        }
        Ok(())
    }
    fn text_bytes(&self) -> usize {
        match self {
            Self::Streams(rows) => rows
                .iter()
                .map(|r| {
                    r.title.len()
                        + r.subtitle.len()
                        + r.user.len()
                        + r.client.len()
                        + r.device.len()
                        + r.play_method.as_ref().map_or(0, String::len)
                        + r.transcode_reason.as_ref().map_or(0, String::len)
                })
                .sum(),
            Self::Downloads { items, .. } => items
                .iter()
                .map(|r| r.name.len() + r.state.len() + r.eta.as_ref().map_or(0, String::len))
                .sum(),
            Self::Queue(q) => q
                .items
                .iter()
                .map(|r| r.title.len() + r.state.len() + r.status.len())
                .sum(),
        }
    }
}
fn admit(expected: &str, input: Option<&ActivityInput<'_>>) -> Result<()> {
    let engine = |s: &str| !s.is_empty() && selected_text(s, 128);
    check(engine(expected))?;
    let Some(i) = input else { return Ok(()) };
    check(
        engine(i.engine_id)
            && (0..=MAX_TIME).contains(&i.sampled_at)
            && i.targets.len() <= MAX_TARGETS
            && i.samples.len() <= MAX_TARGETS,
    )?;
    let mut text = expected.len() + i.engine_id.len();
    for (index, t) in i.targets.iter().enumerate() {
        t.validate()?;
        check(
            !i.targets[..index]
                .iter()
                .any(|p| p.container.resource == t.container.resource),
        )?;
        text += t.name.len()
            + t.container.resource.len()
            + t.container.image.len()
            + t.container.started_at.len();
    }
    for (index, s) in i.samples.iter().enumerate() {
        s.target.validate()?;
        check(
            (0..=MAX_TIME).contains(&s.sampled_at)
                && i.targets
                    .iter()
                    .any(|t| t.container.resource == s.target.container.resource)
                && !i.samples[..index]
                    .iter()
                    .any(|p| p.target.container.resource == s.target.container.resource),
        )?;
        text += s.target.name.len()
            + s.target.container.resource.len()
            + s.target.container.image.len()
            + s.target.container.started_at.len();
        if let Ok(data) = s.result {
            data.validate(s.target.provider)?;
            text += data.text_bytes();
        }
        check(text <= MAX_TEXT_BYTES)?;
    }
    Ok(())
}
fn failure(f: ReadFailure) -> ServiceState {
    match f {
        ReadFailure::Unavailable => ServiceState::Unavailable,
        ReadFailure::CredentialUnavailable => ServiceState::CredentialUnavailable,
        ReadFailure::Timeout => ServiceState::Timeout,
        ReadFailure::InvalidResponse => ServiceState::InvalidResponse,
        ReadFailure::Unsupported => ServiceState::Unsupported,
    }
}
/// Uses original times and caller-supplied plain facts; never acquires a clock or I/O.
pub fn snapshot(
    now: i64,
    expected_engine: &str,
    input: Option<ActivityInput<'_>>,
) -> Result<ActivitySnapshot> {
    check((0..=MAX_TIME).contains(&now))?;
    admit(expected_engine, input.as_ref())?;
    let source = match input.as_ref() {
        None => SourceState::Unavailable,
        Some(i) if i.engine_id != expected_engine => SourceState::IdentityMismatch,
        Some(i) if i.sampled_at > now => SourceState::Future,
        Some(i) if now - i.sampled_at > MAX_AGE_SECONDS => SourceState::Stale,
        Some(i) if !i.complete => SourceState::Incomplete,
        Some(_) => SourceState::Fresh,
    };
    let mut report = ActivitySnapshot {
        version: 1,
        generated_at: now,
        sampled_at: input.as_ref().map(|i| i.sampled_at),
        max_age_seconds: MAX_AGE_SECONDS,
        source,
        complete: false,
        services: vec![],
        streams: vec![],
        downloads: vec![],
        queues: vec![],
        omitted_streams: 0,
        omitted_downloads: 0,
        totals: Totals::default(),
    };
    if !matches!(source, SourceState::Fresh | SourceState::Incomplete) {
        return Ok(report);
    }
    let input = input.unwrap();
    let mut targets: Vec<_> = input.targets.iter().collect();
    targets.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then(a.container.resource.cmp(&b.container.resource))
    });
    let mut totals = Totals {
        streams: Some(0),
        transcodes: Some(0),
        downloads: Some(0),
        download_speed: Some(0.0),
        queued: Some(0),
        queue_problems: Some(0),
    };
    report.complete = source == SourceState::Fresh;
    for target in targets {
        let sample = input
            .samples
            .iter()
            .find(|s| s.target.container.resource == target.container.resource);
        let state = match sample {
            _ if !target.container.running => ServiceState::Stopped,
            None => ServiceState::Unavailable,
            Some(s) if s.target != target => ServiceState::IdentityMismatch,
            Some(s) if s.sampled_at > now => ServiceState::Future,
            Some(s) if s.sampled_at < input.sampled_at || now - s.sampled_at > MAX_AGE_SECONDS => {
                ServiceState::Stale
            }
            Some(s) => match s.result {
                Err(f) => failure(f),
                Ok(d) if !d.complete() => ServiceState::Partial,
                Ok(_) => ServiceState::Ready,
            },
        };
        report.services.push(ServiceView {
            target: target.clone(),
            sampled_at: sample.map(|s| s.sampled_at),
            state,
        });
        report.complete &= state == ServiceState::Ready;
        if matches!(state, ServiceState::Ready | ServiceState::Partial) {
            match sample.unwrap().result.unwrap() {
                ActivityData::Streams(items) => {
                    *totals.streams.as_mut().unwrap() += items.len() as u16;
                    *totals.transcodes.as_mut().unwrap() +=
                        items.iter().filter(|s| s.transcoding).count() as u16;
                    report
                        .streams
                        .extend(items.iter().cloned().map(|data| Sourced {
                            source: target.container.resource.clone(),
                            data,
                        }));
                }
                ActivityData::Downloads {
                    items,
                    aggregate_speed,
                    ..
                } => {
                    *totals.downloads.as_mut().unwrap() += items.len() as u16;
                    totals.download_speed =
                        totals
                            .download_speed
                            .zip(*aggregate_speed)
                            .and_then(|(a, b)| {
                                let n = a + b;
                                (n <= MAX_NUMBER).then_some(n)
                            });
                    report
                        .downloads
                        .extend(items.iter().cloned().map(|data| Sourced {
                            source: target.container.resource.clone(),
                            data,
                        }));
                }
                ActivityData::Queue(q) => {
                    totals.queued = totals.queued.and_then(|n| n.checked_add(q.total));
                    totals.queue_problems = totals
                        .queue_problems
                        .zip(q.complete.then_some(u32::from(q.problems)))
                        .and_then(|(a, b)| a.checked_add(b));
                    report.queues.push(Sourced {
                        source: target.container.resource.clone(),
                        data: q.clone(),
                    });
                }
            }
        }
    }
    if report.complete {
        report.totals = totals;
    }
    report.streams.sort_by(|a, b| {
        a.data
            .paused
            .cmp(&b.data.paused)
            .then(a.data.title.cmp(&b.data.title))
            .then(a.source.cmp(&b.source))
    });
    report.downloads.sort_by(|a, b| {
        b.data
            .speed_bytes
            .unwrap_or(-1.0)
            .total_cmp(&a.data.speed_bytes.unwrap_or(-1.0))
            .then(a.data.name.cmp(&b.data.name))
            .then(a.source.cmp(&b.source))
    });
    report.omitted_streams = report.streams.len().saturating_sub(MAX_DISPLAY) as u16;
    report.streams.truncate(MAX_DISPLAY);
    report.omitted_downloads = report.downloads.len().saturating_sub(MAX_DISPLAY) as u16;
    report.downloads.truncate(MAX_DISPLAY);
    report.check_size()?;
    Ok(report)
}
impl ActivitySnapshot {
    pub fn check_size(&self) -> Result<()> {
        struct Count(usize);
        impl Write for Count {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if bytes.len() > MAX_REPORT_BYTES - self.0 {
                    return Err(std::io::Error::other("activity report bound"));
                }
                self.0 += bytes.len();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        serde_json::to_writer(&mut Count(0), self).map_err(|_| Error(ErrorCode::Unavailable))
    }
}
#[cfg(test)]
mod tests;
