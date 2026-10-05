//! Disposable, bounded aggregates. Authority never depends on this database.
use limeos_contracts::{HistoryRange, HostMetrics, MetricHistory, MetricPoint};
use limeos_domain::{Error, ErrorCode, Result};
use rusqlite::{Connection, params};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

pub const RETENTION: i64 = 31 * 86400;
pub const LEGACY_HISTORY: &str = "Earlier history remains in the current build's System history view. This shadow instance starts a separate 31-day history and never opens the legacy database.";
enum Message {
    Sample(i64, HostMetrics),
    Query(HistoryRange, i64, oneshot::Sender<Result<MetricHistory>>),
}
#[derive(Clone)]
pub struct Telemetry {
    sender: mpsc::Sender<Message>,
    healthy: Arc<AtomicBool>,
}
impl Telemetry {
    pub fn open(path: &Path) -> Result<Self> {
        if std::fs::symlink_metadata(path).is_ok_and(|m| !m.is_file() || m.len() > 8 * 1024 * 1024)
        {
            return Err(Error(ErrorCode::Unavailable));
        }
        let mut connection = Connection::open(path).map_err(|_| Error(ErrorCode::Unavailable))?;
        std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        connection
            .busy_timeout(Duration::from_millis(200))
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA trusted_schema=OFF; PRAGMA max_page_count=2048; PRAGMA wal_autocheckpoint=128;
            CREATE TABLE IF NOT EXISTS samples(at INTEGER PRIMARY KEY,cpu REAL,memory REAL,temperature REAL,disk REAL);").map_err(|_|Error(ErrorCode::Unavailable))?;
        if connection
            .query_row("PRAGMA quick_check(1)", [], |r| r.get::<_, String>(0))
            .map_err(|_| Error(ErrorCode::Unavailable))?
            != "ok"
        {
            return Err(Error(ErrorCode::Unavailable));
        }
        {
            let _ = connection
                .prepare("SELECT at,cpu,memory,temperature,disk FROM samples LIMIT 0")
                .map_err(|_| Error(ErrorCode::Unavailable))?;
        }
        let (sender, mut receiver) = mpsc::channel(16);
        let healthy = Arc::new(AtomicBool::new(true));
        let health = healthy.clone();
        std::thread::Builder::new()
            .name("limeos-telemetry".into())
            .spawn(move || {
                let mut pending = Vec::new();
                let mut last_minute = None;
                while let Some(message) = receiver.blocking_recv() {
                    match message {
                        Message::Sample(at, value) => {
                            if pending.len() >= 5 {
                                let result = flush(&mut connection, &mut pending, at);
                                health.store(result.is_ok(), Ordering::Relaxed);
                                if result.is_err() {
                                    continue;
                                }
                            }
                            let minute = at / 60 * 60;
                            if last_minute == Some(minute) {
                                continue;
                            }
                            last_minute = Some(minute);
                            pending.push((minute, value));
                            if pending.len() >= 5 {
                                health.store(
                                    flush(&mut connection, &mut pending, at).is_ok(),
                                    Ordering::Relaxed,
                                );
                            }
                        }
                        Message::Query(range, at, reply) => {
                            // Reads include in-memory samples without forcing a write.
                            let result = if health.load(Ordering::Relaxed) {
                                query(&connection, range, at, &pending)
                            } else {
                                Err(Error(ErrorCode::Unavailable))
                            };
                            let _ = reply.send(result);
                        }
                    }
                }
                if let Some((at, _)) = pending.last() {
                    let at = *at;
                    let _ = flush(&mut connection, &mut pending, at);
                }
            })
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        Ok(Self { sender, healthy })
    }
    pub fn healthy(&self) -> bool {
        self.healthy.load(Ordering::Relaxed)
    }
    pub fn record(&self, at: i64, value: HostMetrics) -> Result<()> {
        self.sender
            .try_send(Message::Sample(at, value))
            .map_err(|_| Error(ErrorCode::Overloaded))
    }
    pub async fn query(&self, range: HistoryRange, at: i64) -> Result<MetricHistory> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .try_send(Message::Query(range, at, tx))
            .map_err(|_| Error(ErrorCode::Overloaded))?;
        tokio::time::timeout(Duration::from_secs(1), rx)
            .await
            .map_err(|_| Error(ErrorCode::Unavailable))?
            .map_err(|_| Error(ErrorCode::Unavailable))?
    }
}
fn finite(value: Option<f64>) -> Option<f64> {
    value.filter(|v| v.is_finite())
}
fn flush(conn: &mut Connection, pending: &mut Vec<(i64, HostMetrics)>, at: i64) -> Result<()> {
    let tx = conn
        .transaction()
        .map_err(|_| Error(ErrorCode::Unavailable))?;
    for (time, m) in pending.iter() {
        tx.execute(
            "INSERT OR REPLACE INTO samples VALUES(?,?,?,?,?)",
            params![
                time,
                finite(m.cpu_percent),
                finite(m.memory_percent),
                finite(m.temperature_celsius),
                finite(m.disk_percent)
            ],
        )
        .map_err(|_| Error(ErrorCode::Unavailable))?;
    }
    tx.execute("DELETE FROM samples WHERE at<?", [at - RETENTION])
        .map_err(|_| Error(ErrorCode::Unavailable))?;
    tx.commit().map_err(|_| Error(ErrorCode::Unavailable))?;
    pending.clear();
    Ok(())
}
fn query(
    conn: &Connection,
    range: HistoryRange,
    at: i64,
    pending: &[(i64, HostMetrics)],
) -> Result<MetricHistory> {
    let (duration, bucket) = range.bounds();
    let from = at - duration;
    let mut stmt=conn.prepare("SELECT at,cpu,memory,temperature,disk FROM samples WHERE at>=? AND at<=? ORDER BY at LIMIT 44641").map_err(|_|Error(ErrorCode::Unavailable))?;
    let rows = stmt
        .query_map(params![from, at], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                [r.get::<_, Option<f64>>(1)?, r.get(2)?, r.get(3)?, r.get(4)?],
            ))
        })
        .map_err(|_| Error(ErrorCode::Unavailable))?;
    let mut buckets = std::collections::BTreeMap::<i64, ([f64; 4], [u32; 4])>::new();
    let mut add = |time: i64, values: [Option<f64>; 4]| {
        if time < from || time > at {
            return;
        }
        let key = from + ((time - from) / bucket).min(duration / bucket - 1) * bucket;
        let (sums, counts) = buckets.entry(key).or_default();
        for i in 0..4 {
            if let Some(v) = finite(values[i]) {
                sums[i] += v;
                counts[i] += 1;
            }
        }
    };
    for row in rows {
        let (time, values) = row.map_err(|_| Error(ErrorCode::Unavailable))?;
        add(time, values);
    }
    for (time, m) in pending {
        add(
            *time,
            [
                m.cpu_percent,
                m.memory_percent,
                m.temperature_celsius,
                m.disk_percent,
            ],
        );
    }
    let mut points = Vec::new();
    if let (Some(first), Some(last)) = (
        buckets.keys().next().copied(),
        buckets.keys().last().copied(),
    ) {
        for time in (first..=last).step_by(bucket as usize) {
            let values = buckets
                .get(&time)
                .map(|(sum, count)| {
                    std::array::from_fn::<_, 4, _>(|i| {
                        (count[i] > 0).then(|| sum[i] / count[i] as f64)
                    })
                })
                .unwrap_or([None; 4]);
            points.push(MetricPoint {
                at: time,
                cpu_percent: values[0],
                memory_percent: values[1],
                temperature_celsius: values[2],
                disk_percent: values[3],
            });
        }
    }
    Ok(MetricHistory {
        range,
        from,
        to: at,
        bucket_seconds: bucket as u32,
        points,
        legacy_history: LEGACY_HISTORY.into(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retention_gaps_and_independent_metric_averages_match_reference_ranges() {
        let mut c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE samples(at INTEGER PRIMARY KEY,cpu REAL,memory REAL,temperature REAL,disk REAL)").unwrap();
        let at = RETENTION + 100000;
        let mut samples = vec![
            (0, HostMetrics::default()),
            (
                at - 600,
                HostMetrics {
                    cpu_percent: Some(20.0),
                    ..Default::default()
                },
            ),
            (
                at - 540,
                HostMetrics {
                    cpu_percent: Some(40.0),
                    memory_percent: Some(80.0),
                    ..Default::default()
                },
            ),
            (at, HostMetrics::default()),
        ];
        flush(&mut c, &mut samples, at).unwrap();
        assert_eq!(
            c.query_row("SELECT count(*) FROM samples", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            3
        );
        let h = query(&c, HistoryRange::Day, at, &[]).unwrap();
        assert_eq!(h.points[0].cpu_percent, Some(30.0));
        assert_eq!(h.points[0].memory_percent, Some(80.0));
        assert!(h.points.iter().any(|p| p.cpu_percent.is_none()));
        assert!(h.points.len() <= 288);
    }
    #[test]
    fn aggregate_values_match_frozen_python_history_fixture() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/read-semantics.json"))
                .unwrap();
        let at = v["history_at"].as_i64().unwrap();
        let mut c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE samples(at INTEGER PRIMARY KEY,cpu REAL,memory REAL,temperature REAL,disk REAL)").unwrap();
        let mut pending = vec![
            (
                at - 600,
                HostMetrics {
                    cpu_percent: Some(20.0),
                    ..Default::default()
                },
            ),
            (
                at - 540,
                HostMetrics {
                    cpu_percent: Some(40.0),
                    memory_percent: Some(80.0),
                    ..Default::default()
                },
            ),
            (at, HostMetrics::default()),
        ];
        flush(&mut c, &mut pending, at).unwrap();
        let h = query(&c, HistoryRange::Day, at, &[]).unwrap();
        let expected = &v["expected"]["history"];
        assert_eq!(
            h.bucket_seconds,
            expected["bucket_seconds"].as_u64().unwrap() as u32
        );
        assert_eq!(h.points.len(), expected["points"].as_array().unwrap().len());
        for (a, b) in h.points.iter().zip(expected["points"].as_array().unwrap()) {
            assert_eq!(a.cpu_percent, b["cpu_percent"].as_f64());
            assert_eq!(a.memory_percent, b["memory_percent"].as_f64());
        }
    }
    #[tokio::test]
    async fn history_reads_do_not_flush_and_oversized_or_linked_files_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("metrics.sqlite");
        let telemetry = Telemetry::open(&path).unwrap();
        telemetry
            .record(
                1200,
                HostMetrics {
                    cpu_percent: Some(33.0),
                    ..Default::default()
                },
            )
            .unwrap();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        for _ in 0..3 {
            assert_eq!(
                telemetry
                    .query(HistoryRange::Day, 1200)
                    .await
                    .unwrap()
                    .points[0]
                    .cpu_percent,
                Some(33.0)
            );
        }
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            before
        );
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(Telemetry::open(&link).is_err());
    }
    #[tokio::test]
    async fn failed_sample_writes_are_visible_and_recover_without_unbounded_buffering() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("metrics.sqlite");
        let t = Telemetry::open(&path).unwrap();
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TRIGGER reject_samples BEFORE INSERT ON samples BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
        for i in 1..=5 {
            t.record(i * 60, HostMetrics::default()).unwrap();
        }
        for _ in 0..10 {
            if !t.healthy() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!t.healthy());
        assert_eq!(
            t.query(HistoryRange::Day, 300).await.unwrap_err().0,
            ErrorCode::Unavailable
        );
        // Every queued message is bounded; a persistent write failure cannot grow
        // the five-sample batch while input continues.
        for i in 6..100 {
            let _ = t.record(i * 60, HostMetrics::default());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        db.execute_batch("DROP TRIGGER reject_samples").unwrap();
        t.record(6000, HostMetrics::default()).unwrap();
        for _ in 0..20 {
            if t.healthy() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(t.healthy());
        assert!(t.query(HistoryRange::Day, 6000).await.is_ok());
        assert!(std::fs::metadata(path).unwrap().len() < 8 * 1024 * 1024);
    }
}
