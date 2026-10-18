//! Untrusted stack event text, independent of transport, identity and delivery.
use crate::{Error, ErrorCode, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const MAX_FIELD_BYTES: usize = 4096;
pub const MAX_SOURCE_CHARS: usize = 128;
pub const MAX_DETAIL_CHARS: usize = 500;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Quiet,
    Verbose,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    Imported,
    Upgraded,
    Health,
    HealthRestored,
    Failed,
    Manual,
    Grabbed,
    Renamed,
    Added,
    AppUpdate,
}
impl Event {
    fn title(self) -> &'static str {
        match self {
            Self::Imported => "Imported",
            Self::Upgraded => "Upgraded",
            Self::Health => "Health issue",
            Self::HealthRestored => "Health restored",
            Self::Failed => "Download failed",
            Self::Manual => "Manual interaction required",
            Self::Grabbed => "Grabbed",
            Self::Renamed => "Renamed",
            Self::Added => "Added to library",
            Self::AppUpdate => "Application updated",
        }
    }
    fn quiet(self) -> bool {
        matches!(
            self,
            Self::Imported
                | Self::Upgraded
                | Self::Health
                | Self::HealthRestored
                | Self::Failed
                | Self::Manual
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IgnoreReason {
    UnknownEvent,
    ConnectionTest,
    Policy,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Normalization {
    Ignored { reason: IgnoreReason },
    Notification { notification: StackNotification },
}

/// Fields are text data. A consumer must use text rendering, never HTML/markup evaluation.
/// Private fields and no Deserialize prevent unchecked construction of this bounded output.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct StackNotification {
    source: String,
    event: Event,
    severity: Severity,
    title: &'static str,
    detail: String,
}
impl fmt::Debug for StackNotification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StackNotification")
            .field("event", &self.event)
            .field("severity", &self.severity)
            .field("source_bytes", &self.source.len())
            .field("detail_bytes", &self.detail.len())
            .finish()
    }
}
impl StackNotification {
    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn event(&self) -> Event {
        self.event
    }
    pub fn severity(&self) -> Severity {
        self.severity
    }
    pub fn title(&self) -> &str {
        self.title
    }
    pub fn detail(&self) -> &str {
        &self.detail
    }
}
#[derive(Clone, Copy, Default)]
pub enum Media<'a> {
    #[default]
    None,
    Movie {
        title: Option<&'a str>,
        year: Option<u16>,
    },
    Series {
        title: Option<&'a str>,
        /// Missing season means no episode code. Missing episode number is zero.
        episode: Option<(u16, u16)>,
    },
}
#[derive(Default)]
pub struct Input<'a> {
    pub event_type: &'a str,
    pub instance_name: Option<&'a str>,
    pub is_upgrade: bool,
    pub media: Media<'a>,
    pub quality: Option<&'a str>,
    pub message: Option<&'a str>,
    pub health_level: Option<&'a str>,
}

pub fn valid_field(value: &str) -> bool {
    value.len() <= MAX_FIELD_BYTES
        && !value
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
}
fn bounded(value: &str, chars: usize) -> String {
    value.chars().take(chars).collect()
}

/// This does not authenticate the sender, assign a durable key, queue or send anything.
pub fn normalize(mode: Mode, source_default: &str, input: Input<'_>) -> Result<Normalization> {
    let title = match input.media {
        Media::None => None,
        Media::Movie { title, year } => {
            if year.is_some_and(|year| year > 9999) {
                return Err(Error(ErrorCode::InvalidInput));
            }
            title
        }
        Media::Series { title, .. } => title,
    };
    for field in [
        Some(input.event_type),
        Some(source_default),
        input.instance_name,
        title,
        input.quality,
        input.message,
        input.health_level,
    ]
    .into_iter()
    .flatten()
    {
        if !valid_field(field) {
            return Err(Error(ErrorCode::InvalidInput));
        }
    }
    if source_default.trim().is_empty() {
        return Err(Error(ErrorCode::InvalidInput));
    }
    let ignored = |reason| Ok(Normalization::Ignored { reason });
    let mut event = match input.event_type.trim().to_ascii_lowercase().as_str() {
        "download" | "import" => Event::Imported,
        "upgrade" => Event::Upgraded,
        "healthissue" | "health" => Event::Health,
        "healthrestored" => Event::HealthRestored,
        "downloadfailure" | "grabfailure" | "importfailure" => Event::Failed,
        "manualinteractionrequired" => Event::Manual,
        "grab" => Event::Grabbed,
        "rename" => Event::Renamed,
        "movieadded" | "seriesadd" => Event::Added,
        "applicationupdate" => Event::AppUpdate,
        "test" => return ignored(IgnoreReason::ConnectionTest),
        _ => return ignored(IgnoreReason::UnknownEvent),
    };
    if mode == Mode::Quiet && !event.quiet() {
        return ignored(IgnoreReason::Policy);
    }
    if event == Event::Imported && input.is_upgrade {
        event = Event::Upgraded;
    }
    let severity = match event {
        Event::Health
            if input
                .health_level
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("error")) =>
        {
            Severity::Critical
        }
        Event::Health | Event::Failed | Event::Manual => Severity::Warning,
        _ => Severity::Info,
    };
    let source = input.instance_name.map(str::trim).filter(|s| !s.is_empty());
    let source: String = match source {
        // Apply the bound after lowercasing, which can expand a Unicode scalar.
        Some(source) => source
            .chars()
            .flat_map(char::to_lowercase)
            .take(MAX_SOURCE_CHARS)
            .collect(),
        // The frozen fallback preserves the configured display label's case/spacing.
        None => bounded(source_default, MAX_SOURCE_CHARS),
    };
    let detail = if matches!(event, Event::Health | Event::HealthRestored) {
        input.message.unwrap_or("").trim().to_owned()
    } else {
        let media = match input.media {
            Media::None => String::new(),
            Media::Movie { title, year } => {
                let title = title.unwrap_or("?");
                match year.filter(|year| *year != 0) {
                    Some(year) => format!("{title} ({year})"),
                    None => title.into(),
                }
            }
            Media::Series { title, episode } => {
                let title = title.unwrap_or("?");
                match episode {
                    Some((season, episode)) => format!("{title} S{season:02}E{episode:02}"),
                    None => title.into(),
                }
                .trim()
                .to_owned()
            }
        };
        let quality = input.quality.unwrap_or("");
        match (media.is_empty(), quality.is_empty()) {
            (false, false) => format!("{media} — {quality}"),
            (false, true) => media,
            (true, _) => quality.into(),
        }
    };
    Ok(Normalization::Notification {
        notification: StackNotification {
            source,
            event,
            severity,
            title: event.title(),
            detail: bounded(
                if detail.is_empty() {
                    "(no details)"
                } else {
                    &detail
                },
                MAX_DETAIL_CHARS,
            ),
        },
    })
}

#[cfg(test)]
mod tests;
