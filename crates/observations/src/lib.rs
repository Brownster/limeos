//! Shared observation caches. HTTP readers never execute a probe.
mod collectors;
mod docker;
pub use collectors::{start_core, start_docker, start_host};
mod probes;
pub mod telemetry;
use limeos_contracts::{Availability, Freshness, ObservationBatch, Overview, Source, VERSION};
use limeos_domain::{Error, ErrorCode, Result};
use std::{
    sync::{
        Arc, RwLock,
        atomic::{AtomicI64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Notify, watch};

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}
fn loading(source: Source) -> Freshness {
    Freshness {
        source,
        state: Availability::Loading,
        sampled_at: None,
        max_age_seconds: 90,
        warnings: Vec::new(),
    }
}
fn fresh(source: Source, warnings: Vec<String>) -> Freshness {
    Freshness {
        source,
        state: Availability::Fresh,
        sampled_at: Some(now()),
        max_age_seconds: 90,
        warnings,
    }
}
#[derive(Clone)]
pub struct Cache {
    value: Arc<RwLock<Overview>>,
    active_until: Arc<AtomicI64>,
    notify: Arc<Notify>,
    changes: watch::Sender<u64>,
}
impl Default for Cache {
    fn default() -> Self {
        Self::new(vec![
            Source::Host,
            Source::Storage,
            Source::Pools,
            Source::Docker,
        ])
    }
}
impl Cache {
    pub fn new(sources: Vec<Source>) -> Self {
        let (changes, _) = watch::channel(0);
        Self {
            value: Arc::new(RwLock::new(Overview {
                revision: 0,
                generated_at: now(),
                host: None,
                sources: sources.into_iter().map(loading).collect(),
                resources: Vec::new(),
            })),
            active_until: Arc::new(AtomicI64::new(0)),
            notify: Arc::new(Notify::new()),
            changes,
        }
    }
    pub fn demand(&self) {
        let at = now();
        let previous = self.active_until.swap(at + 45, Ordering::Relaxed);
        if previous < at {
            self.notify.notify_one();
        }
    }
    pub fn active(&self) -> bool {
        self.active_until.load(Ordering::Relaxed) >= now()
    }
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }
    pub fn snapshot(&self) -> Overview {
        let mut value = self.value.read().unwrap_or_else(|e| e.into_inner()).clone();
        value.generated_at = now();
        for source in &mut value.sources {
            if source
                .sampled_at
                .is_some_and(|at| now() - at > source.max_age_seconds as i64)
                && source.state == Availability::Fresh
            {
                source.state = Availability::Stale;
            }
        }
        value
    }
    pub fn update(&self, batch: ObservationBatch) {
        let mut value = self.value.write().unwrap_or_else(|e| e.into_inner());
        for source in batch.sources {
            if source.state == Availability::Fresh {
                let new: Vec<_> = batch
                    .resources
                    .iter()
                    .filter(|r| r.source == source.source)
                    .cloned()
                    .collect();
                let mut missing: Vec<_> = value
                    .resources
                    .iter()
                    .filter(|r| r.source == source.source && !new.iter().any(|n| n.id == r.id))
                    .take(64)
                    .cloned()
                    .collect();
                for r in &mut missing {
                    r.status = "missing".into();
                    r.cpu_percent = None;
                    r.memory_percent = None;
                }
                value.resources.retain(|r| r.source != source.source);
                value.resources.extend(new);
                value.resources.extend(missing);
                if source.source == Source::Host {
                    value.host = batch.host.clone();
                }
            }
            if let Some(s) = value.sources.iter_mut().find(|s| s.source == source.source) {
                let previous = s.sampled_at;
                *s = source;
                if s.state != Availability::Fresh {
                    s.sampled_at = previous;
                }
            } else {
                value.sources.push(source);
            }
        }
        value.resources.sort_by(|a, b| a.id.cmp(&b.id));
        // Keep the executor snapshot below the framed transport ceiling. Prefer
        // current evidence; evict bounded missing records with an explicit warning.
        while serde_json::to_vec(&ObservationBatch {
            host: value.host.clone(),
            resources: value.resources.clone(),
            sources: value.sources.clone(),
        })
        .is_ok_and(|bytes| bytes.len() > limeos_contracts::FRAME_LIMIT - 1024)
        {
            let index = value
                .resources
                .iter()
                .position(|r| r.status == "missing")
                .or_else(|| value.resources.len().checked_sub(1));
            let Some(index) = index else {
                break;
            };
            let removed = value.resources.remove(index);
            if let Some(source) = value
                .sources
                .iter_mut()
                .find(|s| s.source == removed.source)
            {
                if !source.warnings.iter().any(|w| {
                    w == "Inventory exceeds the snapshot byte limit; some records are omitted"
                }) {
                    source.warnings.push(
                        "Inventory exceeds the snapshot byte limit; some records are omitted"
                            .into(),
                    );
                }
            }
        }
        value.revision += 1;
        let revision = value.revision;
        drop(value);
        self.changes.send_replace(revision);
    }
    fn failed(&self, sources: &[Source]) {
        self.update(ObservationBatch {
            host: None,
            resources: Vec::new(),
            sources: sources
                .iter()
                .map(|source| Freshness {
                    source: *source,
                    state: Availability::Unavailable,
                    sampled_at: None,
                    max_age_seconds: 90,
                    warnings: vec![
                        "Source unavailable; retained observations may be out of date".into(),
                    ],
                })
                .collect(),
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failures_keep_last_evidence_missing_resources_are_explicit_and_readers_share_cache() {
        let cache = Cache::default();
        let r = limeos_contracts::Resource::new(
            "uuid:data".into(),
            limeos_contracts::ResourceKind::Partition,
            "data".into(),
            "mounted".into(),
            Source::Storage,
        );
        cache.update(ObservationBatch {
            host: None,
            resources: vec![r],
            sources: vec![fresh(Source::Storage, Vec::new())],
        });
        cache.failed(&[Source::Storage]);
        assert_eq!(cache.snapshot().resources[0].status, "mounted");
        assert_eq!(cache.snapshot().sources[1].state, Availability::Unavailable);
        cache.update(ObservationBatch {
            host: None,
            resources: Vec::new(),
            sources: vec![fresh(Source::Storage, Vec::new())],
        });
        assert_eq!(cache.snapshot().resources[0].status, "missing");
        let revision = cache.snapshot().revision;
        for _ in 0..100 {
            cache.clone().demand();
            assert_eq!(cache.snapshot().revision, revision);
        }
        let mut v = cache.value.write().unwrap();
        v.sources[1].sampled_at = Some(now() - 100);
        drop(v);
        assert_eq!(cache.snapshot().sources[1].state, Availability::Stale);
    }
}
