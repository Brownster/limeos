//! Read models contain selected public metadata, never Docker inspect secrets.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Host,
    Storage,
    Pools,
    Docker,
    History,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Loading,
    Fresh,
    Stale,
    Unavailable,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Freshness {
    pub source: Source,
    pub state: Availability,
    #[ts(type = "number | null")]
    pub sampled_at: Option<i64>,
    pub max_age_seconds: u32,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct HostMetrics {
    pub cpu_percent: Option<f64>,
    pub memory_percent: Option<f64>,
    pub temperature_celsius: Option<f64>,
    pub disk_percent: Option<f64>,
    pub load_one: Option<f64>,
    pub uptime_seconds: Option<f64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Disk,
    Partition,
    Pool,
    Container,
    Stack,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub id: String,
    pub kind: ResourceKind,
    pub name: String,
    pub status: String,
    pub source: Source,
    pub parent: Option<String>,
    pub identity: Option<String>,
    pub filesystem: Option<String>,
    pub mountpoint: Option<String>,
    #[ts(type = "number | null")]
    pub size_bytes: Option<u64>,
    pub image: Option<String>,
    pub health: Option<String>,
    pub cpu_percent: Option<f64>,
    pub memory_percent: Option<f64>,
}
impl Resource {
    pub fn new(
        id: String,
        kind: ResourceKind,
        name: String,
        status: String,
        source: Source,
    ) -> Self {
        Self {
            id,
            kind,
            name,
            status,
            source,
            parent: None,
            identity: None,
            filesystem: None,
            mountpoint: None,
            size_bytes: None,
            image: None,
            health: None,
            cpu_percent: None,
            memory_percent: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ObservationBatch {
    pub host: Option<HostMetrics>,
    pub resources: Vec<Resource>,
    pub sources: Vec<Freshness>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Overview {
    #[ts(type = "number")]
    pub revision: u64,
    #[ts(type = "number")]
    pub generated_at: i64,
    pub host: Option<HostMetrics>,
    pub sources: Vec<Freshness>,
    pub resources: Vec<Resource>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ResourcePage {
    #[ts(type = "number")]
    pub revision: u64,
    pub items: Vec<Resource>,
    pub total: u32,
    pub next_offset: Option<u32>,
    pub sources: Vec<Freshness>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, TS)]
pub enum HistoryRange {
    #[serde(rename = "24h")]
    Day,
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "30d")]
    Month,
}
impl HistoryRange {
    pub fn bounds(self) -> (i64, i64) {
        match self {
            Self::Day => (86400, 300),
            Self::Week => (604800, 1800),
            Self::Month => (2592000, 7200),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct MetricPoint {
    #[ts(type = "number")]
    pub at: i64,
    pub cpu_percent: Option<f64>,
    pub memory_percent: Option<f64>,
    pub temperature_celsius: Option<f64>,
    pub disk_percent: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct MetricHistory {
    pub range: HistoryRange,
    #[ts(type = "number")]
    pub from: i64,
    #[ts(type = "number")]
    pub to: i64,
    pub bucket_seconds: u32,
    pub points: Vec<MetricPoint>,
    pub legacy_history: String,
}
