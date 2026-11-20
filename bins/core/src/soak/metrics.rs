//! `metrics.sqlite` (`limeos_observations::telemetry`), the other store core
//! writes on its own: the host collector records a sample every 60 s when
//! nobody is watching (10 s while a page is open), the store keeps one row a
//! minute and flushes every five minutes, each flush one transaction with an
//! unbatched `DELETE` of rows past its 31-day history. It never stops
//! writing, so it has no idle hour; what must hold is that its file stays
//! bounded (its open refuses a file over 8 MiB), its writes stay at the same
//! cost however long it runs, and it opens again after 90 days.
//!
//! The real `Telemetry` (its writer thread and all) is driven at that
//! cadence for 90 simulated days, one sample per simulated minute stamped
//! with the simulated clock, as the collector stamps `sampled_at`. A query
//! after every fifteen samples keeps its 16-message queue from overflowing
//! and orders the measurements. Its writes are counted on its own thread
//! (`/proc/self/task/<tid>/io` of `limeos-telemetry`).
use super::{
    sim::soak_dir,
    support::{DAY, HOUR, MINUTE, START, mib, slope},
};
use limeos_contracts::{HistoryRange, HostMetrics};
use limeos_observations::telemetry::{RETENTION, Telemetry};
use std::path::Path;

/// Half the size at which `Telemetry::open` refuses the file.
const FILE_BOUND: u64 = 4 << 20;
/// Rows the store keeps: one a minute over its history, plus one flush.
const ROWS: i64 = RETENTION / MINUTE + 5;

/// Bytes and calls written so far by the telemetry writer thread.
fn written() -> (u64, u64) {
    let tasks = std::fs::read_dir("/proc/self/task").expect("per-thread accounting");
    let mut found = None;
    for task in tasks.flatten() {
        let comm = std::fs::read_to_string(task.path().join("comm")).unwrap_or_default();
        // `comm` keeps the first 15 bytes of the thread's name.
        if comm.trim() == &"limeos-telemetry"[..15] {
            assert!(
                found.is_none(),
                "one telemetry writer at a time in this binary"
            );
            let io = std::fs::read_to_string(task.path().join("io")).unwrap();
            let field = |name: &str| -> u64 {
                io.lines()
                    .find_map(|line| line.strip_prefix(name)?.strip_prefix(": "))
                    .and_then(|value| value.trim().parse().ok())
                    .unwrap()
            };
            found = Some((field("wchar"), field("syscw")));
        }
    }
    found.expect("the telemetry writer thread is running")
}

fn size(path: &Path) -> u64 {
    let len = |p: &Path| std::fs::metadata(p).map_or(0, |m| m.len());
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    len(path) + len(Path::new(&wal))
}

fn sample(minute: i64) -> HostMetrics {
    HostMetrics {
        cpu_percent: Some((minute % 97) as f64),
        memory_percent: Some(40.0 + (minute % 13) as f64),
        temperature_celsius: Some(50.0),
        disk_percent: Some(61.0),
        load_one: Some(0.4),
        uptime_seconds: Some((minute * 60) as f64),
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "90 simulated days of host metrics: in the soak CI step"]
async fn soak_metrics_store_ninety_days() {
    let dir = soak_dir();
    let path = dir.path().join("metrics.sqlite");
    let telemetry = Telemetry::open(&path).expect("metrics.sqlite opens");
    let mut problems = Vec::new();
    // Per day: the file and WAL at 03:00, and an hour's writes from 02:00.
    let (mut sizes, mut hours) = (Vec::new(), Vec::new());
    let mut hour_start = None;
    for minute in 0..90 * DAY / MINUTE {
        let at = START + minute * MINUTE;
        let of_day = (at - START) % DAY;
        if of_day == 2 * HOUR || of_day == 3 * HOUR {
            telemetry.query(HistoryRange::Day, at).await.unwrap();
            let now = written();
            if of_day == 3 * HOUR {
                let (bytes, calls): (u64, u64) = hour_start
                    .take()
                    .map_or((0, 0), |(b, c)| (now.0 - b, now.1 - c));
                let day = ((at - START) / DAY) as f64;
                hours.push((day, bytes as f64, calls));
                sizes.push((day, size(&path) as f64));
                if size(&path) > FILE_BOUND {
                    problems.push(format!(
                        "day {day}: metrics.sqlite and its WAL take {:.2} MiB (bound {:.0})",
                        mib(size(&path)),
                        mib(FILE_BOUND)
                    ));
                }
            } else {
                hour_start = Some(now);
            }
        }
        if let Err(error) = telemetry.record(at, sample(minute)) {
            problems.push(format!(
                "day {}: a sample was refused: {error:?}",
                minute / 1440
            ));
            break;
        }
        if minute % 15 == 14 {
            telemetry.query(HistoryRange::Day, at).await.unwrap();
        }
    }
    assert!(
        telemetry.healthy(),
        "the metrics store reported a failed flush"
    );
    let end = START + 90 * DAY;
    let month = telemetry.query(HistoryRange::Month, end).await.unwrap();
    drop(telemetry);
    // Every flush writes: an hour is twelve of them, at the same cost on
    // day 89 as on day 35, after the history filled.
    let full: Vec<&(f64, f64, u64)> = hours.iter().filter(|(day, _, _)| *day > 32.0).collect();
    let middle = &full[..full.len() / 2];
    let last = &full[full.len() / 2..];
    let mean =
        |part: &[&(f64, f64, u64)]| part.iter().map(|h| h.1).sum::<f64>() / part.len() as f64;
    if mean(last) > mean(middle) * 1.25 + 4096.0 {
        problems.push(format!(
            "an hour of flushes wrote {:.0} KiB over days 61-89 against {:.0} KiB over days 33-60",
            mean(last) / 1024.0,
            mean(middle) / 1024.0
        ));
    }
    let last_third: Vec<(f64, f64)> = sizes.iter().filter(|(d, _)| *d >= 60.0).copied().collect();
    let grown = slope(&last_third) * 30.0;
    if grown > 64.0 * 1024.0 {
        problems.push(format!(
            "metrics.sqlite grew {:.0} KiB over the last third: its history is not bounded",
            grown / 1024.0
        ));
    }
    let rows: i64 = super::checks::reader(&path)
        .unwrap()
        .query_row("SELECT count(*) FROM samples", [], |r| r.get(0))
        .unwrap();
    if rows > ROWS {
        problems.push(format!("{rows} samples kept, more than {ROWS}"));
    }
    // It opens again (as core does at every start), and still answers.
    match Telemetry::open(&path) {
        Ok(reopened) => {
            if let Err(error) = reopened.query(HistoryRange::Day, end).await {
                problems.push(format!("the reopened store refused a query: {error:?}"));
            }
        }
        Err(error) => problems.push(format!("metrics.sqlite no longer opens: {error:?}")),
    }
    let calls: Vec<u64> = full.iter().map(|h| h.2).collect();
    eprintln!(
        "metrics.sqlite: {rows} rows, {:.2} MiB with its WAL at the end (bound {:.0}); an hour of flushes: {:.0} KiB and {}..{} write calls; the 30-day view has {} points",
        mib(size(&path)),
        mib(FILE_BOUND),
        mean(&full) / 1024.0,
        calls.iter().min().unwrap(),
        calls.iter().max().unwrap(),
        month.points.len()
    );
    assert!(
        problems.is_empty(),
        "metrics.sqlite:\n{}",
        problems.join("\n")
    );
}
