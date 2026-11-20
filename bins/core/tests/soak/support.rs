//! The simulated clock, a seeded PRNG, Engine timestamps and the per-thread
//! I/O counters the harness measures work with.
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

/// 2026-01-01T00:00:00Z: day 0 of every run.
pub const START: i64 = 1_767_225_600;
pub const MINUTE: i64 = 60;
pub const HOUR: i64 = 3_600;
pub const DAY: i64 = 86_400;

/// The one simulated clock. Every component reads it: the harness passes it
/// as `now` to the core store, as the `clock` of the receipt store and to the
/// fake Engine for container start times. It only moves forward, and only
/// when the driver advances it; nothing reads the wall clock for a decision.
#[derive(Clone)]
pub struct Clock(Arc<AtomicI64>);
impl Clock {
    pub fn new(at: i64) -> Self {
        Self(Arc::new(AtomicI64::new(at)))
    }
    pub fn now(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
    pub fn set(&self, at: i64) {
        assert!(at >= self.now(), "the simulated clock never steps back");
        self.0.store(at, Ordering::SeqCst);
    }
}

/// SplitMix64: small, seeded and identical on every platform. Only integer
/// arithmetic, so a run draws the same numbers on x86-64 and arm64.
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    /// Uniform in `0..n` (the modulo bias is far below anything measured).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    /// True with probability `p` per thousand.
    pub fn per_mille(&mut self, p: u64) -> bool {
        self.below(1000) < p
    }
    /// Uniform in `[mean/2, 3*mean/2)`, at least 1: a jittered interval.
    pub fn around(&mut self, mean: i64) -> i64 {
        let mean = mean.max(1);
        (mean / 2 + self.below(mean as u64) as i64).max(1)
    }
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
    /// 64 lowercase hex characters: a token or key the harness controls.
    pub fn hex(&mut self) -> String {
        (0..4).map(|_| format!("{:016x}", self.next())).collect()
    }
}

/// An Engine start time as Docker reports it (UTC, nanoseconds).
pub fn rfc3339(seconds: i64, nanos: u32) -> String {
    let days = seconds.div_euclid(DAY);
    let rest = seconds.rem_euclid(DAY);
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{nanos:09}Z",
        rest / HOUR,
        rest % HOUR / MINUTE,
        rest % MINUTE
    )
}

/// The calling thread's I/O accounting (`/proc/thread-self/io`). Every store
/// call of a run happens on the test thread (a current-thread runtime, no
/// writer thread), so the delta across a call is exactly that call's reads
/// and writes: `syscr` counts SQLite's page reads, `syscw` any write at all.
/// Deterministic for a given database file, unlike wall time on a CI runner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Io {
    pub rchar: u64,
    pub wchar: u64,
    pub syscr: u64,
    pub syscw: u64,
}
impl Io {
    pub fn now() -> Self {
        let text = std::fs::read_to_string("/proc/thread-self/io")
            .expect("per-thread I/O accounting (/proc/thread-self/io) is required");
        let field = |name: &str| -> u64 {
            text.lines()
                .find_map(|line| line.strip_prefix(name)?.strip_prefix(": "))
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or_else(|| panic!("{name} missing from /proc/thread-self/io"))
        };
        Self {
            rchar: field("rchar"),
            wchar: field("wchar"),
            syscr: field("syscr"),
            syscw: field("syscw"),
        }
    }
    pub fn since(self, earlier: Self) -> Self {
        Self {
            rchar: self.rchar - earlier.rchar,
            wchar: self.wchar - earlier.wchar,
            syscr: self.syscr - earlier.syscr,
            syscw: self.syscw - earlier.syscw,
        }
    }
}

/// Least-squares slope of `(x, y)` points, in y per x.
pub fn slope(points: &[(f64, f64)]) -> f64 {
    let n = points.len() as f64;
    if points.len() < 2 {
        return 0.0;
    }
    let (sx, sy) = points
        .iter()
        .fold((0.0, 0.0), |(sx, sy), (x, y)| (sx + x, sy + y));
    let (mx, my) = (sx / n, sy / n);
    let (num, den) = points.iter().fold((0.0, 0.0), |(num, den), (x, y)| {
        (num + (x - mx) * (y - my), den + (x - mx) * (x - mx))
    });
    if den == 0.0 { 0.0 } else { num / den }
}

pub fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

#[test]
fn engine_timestamps_are_rfc3339_utc() {
    assert_eq!(rfc3339(START, 0), "2026-01-01T00:00:00.000000000Z");
    assert_eq!(
        rfc3339(START + 59 * DAY + 3661, 7),
        "2026-03-01T01:01:01.000000007Z"
    );
    assert_eq!(rfc3339(951_782_400, 0), "2000-02-29T00:00:00.000000000Z");
}

#[test]
fn the_generator_is_fixed_by_its_seed() {
    let (mut a, mut b) = (Rng::new(7), Rng::new(7));
    assert!((0..100).all(|_| a.next() == b.next()));
    assert_eq!(Rng::new(1).hex().len(), 64);
}
