//! Selective supplied-response decoding. Never HTTP/status/endpoint/credential proof.
use limeos_domain::deployment::activity::{
    ActivityData, Download, MAX_ITEMS, MAX_NUMBER, Provider, Queue, QueueItem, ReadFailure, Stream,
};
use serde::{
    Deserialize, Deserializer,
    de::{
        DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor, value::MapAccessDeserializer,
    },
};
use std::{fmt, marker::PhantomData};
pub const MAX_RESPONSE_BYTES: usize = 32 * 1024;
type Result<T> = std::result::Result<T, ReadFailure>;

struct Bounded<T, const N: usize>(Vec<T>);
impl<'de, T: Deserialize<'de>, const N: usize> Deserialize<'de> for Bounded<T, N> {
    fn deserialize<D: Deserializer<'de>>(de: D) -> std::result::Result<Self, D::Error> {
        struct Items<T, const N: usize>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for Items<T, N> {
            type Value = Bounded<T, N>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a bounded activity array")
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut rows = Vec::new();
                for _ in 0..N {
                    match seq.next_element()? {
                        Some(row) => rows.push(row),
                        None => return Ok(Bounded(rows)),
                    }
                }
                if seq.next_element::<IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::custom("activity row limit"));
                }
                Ok(Bounded(rows))
            }
        }
        de.deserialize_seq(Items(PhantomData))
    }
}
struct Object<T> {
    value: T,
    nonempty: bool,
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: Deserializer<'de>>(de: D) -> std::result::Result<Self, D::Error> {
        struct Map<T>(PhantomData<T>);
        struct Count<'a, A> {
            inner: A,
            seen: &'a mut bool,
        }
        impl<'de, A: MapAccess<'de>> MapAccess<'de> for Count<'_, A> {
            type Error = A::Error;
            fn next_key_seed<K: DeserializeSeed<'de>>(
                &mut self,
                seed: K,
            ) -> std::result::Result<Option<K::Value>, Self::Error> {
                let value = self.inner.next_key_seed(seed)?;
                *self.seen |= value.is_some();
                Ok(value)
            }
            fn next_value_seed<V: DeserializeSeed<'de>>(
                &mut self,
                seed: V,
            ) -> std::result::Result<V::Value, Self::Error> {
                self.inner.next_value_seed(seed)
            }
        }
        impl<'de, T: Deserialize<'de>> Visitor<'de> for Map<T> {
            type Value = Object<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an activity object")
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut seen = false;
                let value = T::deserialize(MapAccessDeserializer::new(Count {
                    inner: map,
                    seen: &mut seen,
                }))?;
                Ok(Object {
                    value,
                    nonempty: seen,
                })
            }
        }
        de.deserialize_map(Map(PhantomData))
    }
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Numeric {
    Number(f64),
    Text(String),
}
fn finite(v: Option<f64>) -> Option<f64> {
    v.filter(|v| v.is_finite() && (0.0..=MAX_NUMBER).contains(v))
}
fn numeric(v: Option<Numeric>) -> Option<f64> {
    finite(v.and_then(|v| match v {
        Numeric::Number(n) => Some(n),
        Numeric::Text(s) if s.len() <= 64 => s.trim().replace(',', "").parse().ok(),
        _ => None,
    }))
}
fn scaled(n: Option<f64>, scale: f64) -> Option<f64> {
    finite(n.map(|n| n * scale))
}
fn text(s: Option<String>, default: &str, max: usize) -> Result<String> {
    let s = s.unwrap_or_else(|| default.into());
    let s = s.trim();
    if s.chars().any(char::is_control) {
        return Err(ReadFailure::InvalidResponse);
    }
    let mut value = String::new();
    for c in s.chars() {
        if value.len() + c.len_utf8() > max {
            break;
        }
        value.push(c);
    }
    Ok(value)
}
fn optional_text(s: Option<String>, max: usize) -> Result<Option<String>> {
    let s = text(s, "", max)?;
    Ok((!s.is_empty()).then_some(s))
}
fn percent(part: Option<f64>, whole: Option<f64>) -> Option<f64> {
    part.zip(whole)
        .filter(|(_, w)| *w > 0.0)
        .map(|(p, w)| ((p / w * 100.0).clamp(0.0, 100.0) * 10.0).round() / 10.0)
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Session {
    user_name: Option<String>,
    client: Option<String>,
    device_name: Option<String>,
    now_playing_item: Option<Object<Item>>,
    play_state: Option<Object<Play>>,
    transcoding_info: Option<Object<Transcode>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Item {
    name: Option<String>,
    #[serde(rename = "Type")]
    kind: Option<String>,
    series_name: Option<String>,
    parent_index_number: Option<u32>,
    index_number: Option<u32>,
    production_year: Option<u32>,
    run_time_ticks: Option<f64>,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Play {
    position_ticks: Option<f64>,
    play_method: Option<String>,
    is_paused: Option<bool>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Transcode {
    bitrate: Option<f64>,
    transcode_reasons: Option<Bounded<String, 8>>,
}
fn sessions(bytes: &[u8]) -> Result<ActivityData> {
    let rows: Bounded<Object<Session>, MAX_ITEMS> = parse(bytes)?;
    let mut streams = Vec::new();
    for row in rows.0 {
        let s = row.value;
        let Some(item) = s.now_playing_item.filter(|i| i.nonempty) else {
            continue;
        };
        let item = item.value;
        let play = s.play_state.map(|p| p.value).unwrap_or_default();
        let method = optional_text(play.play_method, 64)?;
        let transcoding = s.transcoding_info.as_ref().is_some_and(|t| t.nonempty)
            || method
                .as_ref()
                .is_some_and(|m| m.eq_ignore_ascii_case("transcode"));
        let bitrate = finite(s.transcoding_info.as_ref().and_then(|t| t.value.bitrate));
        let mut reasons = Vec::new();
        if let Some(t) = s.transcoding_info
            && let Some(rs) = t.value.transcode_reasons
        {
            for r in rs.0 {
                reasons.push(text(Some(r), "", 64)?);
            }
        }
        let reason = optional_text(Some(reasons.join(", ")), 512)?;
        let series = text(item.series_name, "", 160)?;
        let subtitle = if item.kind.as_deref() == Some("Episode") {
            match item.parent_index_number.zip(item.index_number) {
                Some((season, episode)) => format!("{series} · S{season:02}E{episode:02}")
                    .trim_matches([' ', '·'])
                    .to_owned(),
                None => series,
            }
        } else {
            item.production_year
                .map_or_else(String::new, |y| y.to_string())
        };
        streams.push(Stream {
            title: text(item.name, "Unknown title", 200)?,
            subtitle: text(Some(subtitle), "", 200)?,
            user: text(s.user_name, "", 200)?,
            client: text(s.client, "", 200)?,
            device: text(s.device_name, "", 200)?,
            play_method: method,
            paused: play.is_paused.unwrap_or(false),
            transcoding,
            transcode_reason: reason,
            bitrate,
            progress_percent: percent(finite(play.position_ticks), finite(item.run_time_ticks)),
            position_seconds: finite(play.position_ticks).map(|n| (n / 10_000_000.0).round()),
            duration_seconds: finite(item.run_time_ticks).map(|n| (n / 10_000_000.0).round()),
        });
    }
    Ok(ActivityData::Streams(streams))
}
#[derive(Deserialize)]
struct Sab {
    queue: Object<SabQueue>,
}
#[derive(Deserialize)]
struct SabQueue {
    kbpersec: Option<Numeric>,
    noofslots_total: Option<u32>,
    start: Option<u32>,
    slots: Bounded<Object<SabSlot>, MAX_ITEMS>,
}
#[derive(Deserialize)]
struct SabSlot {
    filename: Option<String>,
    status: Option<String>,
    percentage: Option<Numeric>,
    mb: Option<Numeric>,
    mbleft: Option<Numeric>,
    timeleft: Option<String>,
}
fn sab(bytes: &[u8]) -> Result<ActivityData> {
    let parsed: Object<Sab> = parse(bytes)?;
    let queue = parsed.value.queue.value;
    let speed = scaled(numeric(queue.kbpersec), 1024.0);
    let complete =
        queue.start == Some(0) && queue.noofslots_total == Some(queue.slots.0.len() as u32);
    if queue
        .noofslots_total
        .is_some_and(|n| n < (queue.slots.0.len() as u32))
    {
        return Err(ReadFailure::InvalidResponse);
    }
    let mut items = Vec::new();
    let mut rate_assigned = false;
    for slot in queue.slots.0 {
        let s = slot.value;
        let state = text(s.status, "queued", 64)?.to_ascii_lowercase();
        let rate = if state == "downloading" {
            if rate_assigned {
                None
            } else {
                rate_assigned = true;
                speed
            }
        } else {
            Some(0.0)
        };
        let size = scaled(numeric(s.mb), 1024.0 * 1024.0);
        let remaining =
            scaled(numeric(s.mbleft), 1024.0 * 1024.0).filter(|n| size.is_none_or(|s| *n <= s));
        items.push(Download {
            name: text(s.filename, "Unknown item", 200)?,
            state,
            progress_percent: numeric(s.percentage).map(|n| n.min(100.0)),
            size_bytes: size,
            remaining_bytes: remaining,
            speed_bytes: rate,
            eta: optional_text(s.timeleft, 64)?,
        });
    }
    Ok(ActivityData::Downloads {
        items,
        complete,
        aggregate_speed: speed,
    })
}
#[derive(Deserialize)]
struct Transmission {
    result: String,
    arguments: Object<Torrents>,
}
#[derive(Deserialize)]
struct Torrents {
    torrents: Bounded<Object<Torrent>, MAX_ITEMS>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Torrent {
    name: Option<String>,
    status: u8,
    rate_download: Option<f64>,
    rate_upload: Option<f64>,
    percent_done: Option<f64>,
    eta: Option<f64>,
    total_size: Option<f64>,
}
fn transmission(bytes: &[u8]) -> Result<ActivityData> {
    let parsed: Object<Transmission> = parse(bytes)?;
    if parsed.value.result != "success" {
        return Err(ReadFailure::InvalidResponse);
    }
    let mut items = Vec::new();
    let mut total = Some(0.0);
    for t in parsed.value.arguments.value.torrents.0 {
        let t = t.value;
        if !matches!(t.status, 4 | 6) {
            continue;
        }
        let done = finite(t.percent_done).map(|n| n.min(1.0));
        let size = finite(t.total_size);
        let rate = finite(if t.status == 4 {
            t.rate_download
        } else {
            t.rate_upload
        });
        total = total.zip(rate).and_then(|(a, b)| finite(Some(a + b)));
        let eta = finite(t.eta).map(|n| {
            let minutes = (n / 60.0).floor() as u64;
            if minutes >= 60 {
                format!("{}h {}m", minutes / 60, minutes % 60)
            } else {
                format!("{minutes}m")
            }
        });
        items.push(Download {
            name: text(t.name, "Unknown torrent", 200)?,
            state: if t.status == 4 {
                "downloading"
            } else {
                "seeding"
            }
            .into(),
            progress_percent: done.map(|n| (n * 1000.0).round() / 10.0),
            size_bytes: size,
            remaining_bytes: size.zip(done).map(|(s, d)| (s * (1.0 - d)).round()),
            speed_bytes: rate,
            eta,
        });
    }
    Ok(ActivityData::Downloads {
        items,
        complete: true,
        aggregate_speed: total,
    })
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Arr {
    total_records: u32,
    records: Bounded<Object<ArrRecord>, MAX_ITEMS>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArrRecord {
    title: Option<String>,
    status: Option<String>,
    tracked_download_state: Option<String>,
    tracked_download_status: Option<String>,
}
fn arr(bytes: &[u8]) -> Result<ActivityData> {
    let parsed: Object<Arr> = parse(bytes)?;
    let a = parsed.value;
    let observed = a.records.0.len() as u16;
    if u32::from(observed) > a.total_records {
        return Err(ReadFailure::InvalidResponse);
    }
    let mut problems = 0;
    let mut items = Vec::new();
    let mut known_statuses = true;
    for r in a.records.0 {
        let r = r.value;
        let status = text(r.status, "unknown", 64)?;
        let tracked = text(r.tracked_download_status, "", 64)?;
        known_statuses &= !(matches!(status.as_str(), "unknown" | "") && tracked.is_empty());
        if matches!(tracked.to_ascii_lowercase().as_str(), "warning" | "error")
            || matches!(
                status.to_ascii_lowercase().as_str(),
                "warning" | "failed" | "error"
            )
        {
            problems += 1;
        }
        if items.len() < 3 {
            items.push(QueueItem {
                title: text(r.title, "Unknown item", 200)?,
                state: text(r.tracked_download_state, "queued", 64)?,
                status,
            });
        }
    }
    Ok(ActivityData::Queue(Queue {
        total: a.total_records,
        observed,
        problems,
        complete: known_statuses && u32::from(observed) == a.total_records,
        items,
    }))
}
fn parse<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|_| ReadFailure::InvalidResponse)
}
pub fn decode(provider: Provider, bytes: &[u8]) -> Result<ActivityData> {
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(ReadFailure::InvalidResponse);
    }
    let data = match provider {
        Provider::Jellyfin | Provider::Emby => sessions(bytes)?,
        Provider::Sabnzbd => sab(bytes)?,
        Provider::Transmission => transmission(bytes)?,
        _ => arr(bytes)?,
    };
    data.validate(provider)
        .map_err(|_| ReadFailure::InvalidResponse)?;
    Ok(data)
}
#[cfg(test)]
mod tests;
