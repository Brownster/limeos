//! `limeosctl action-log [--limit N] [DIRECTORY]`: the container executor's
//! action log (ADR 0013), read where it lives: the `effect_log` and
//! `effect_days` tables of its receipt store, which core's account cannot
//! open. Run as the store's owner, the executor's own account; as anyone
//! else (root included) it refuses, because SQLite may create the store's
//! `-wal` and `-shm` files on open, and files root created there would stop
//! the executor from opening its store. The connection is read-only.
use rusqlite::{Connection, OpenFlags};
use std::{os::unix::fs::MetadataExt, path::Path, time::Duration};

pub const USAGE: &str = "action-log [--limit N] [DIRECTORY]: the container executor's action log, one JSON line per row (newest N, default 100, at most 8192, printed oldest first), then one line per day of totals. Run as its account: sudo -u limeos-containerd /usr/lib/limeos/limeosctl action-log. DIRECTORY defaults to /var/lib/limeos/executors/container.";
const DIRECTORY: &str = "/var/lib/limeos/executors/container";
const DEFAULT_LIMIT: u32 = 100;
const MAX_LIMIT: u32 = 8192;
/// The receipt schema that added the log.
const LOG_SCHEMA: u32 = 9;

pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if let [first] = args
        && crate::args::is_help(first)
    {
        println!("Usage: limeosctl {USAGE}");
        return Ok(());
    }
    let (limit, directory) = parse(args)?;
    let directory = Path::new(directory);
    let owner = std::fs::symlink_metadata(directory)
        .map_err(|e| format!("{}: {e}", directory.display()))?
        .uid();
    owned_by_caller(owner, rustix::process::geteuid().as_raw())?;
    let (rows, days) = read(directory, limit)?;
    for line in rows.iter().chain(&days) {
        println!("{line}");
    }
    Ok(())
}

fn parse(args: &[String]) -> Result<(u32, &str), crate::args::Usage> {
    let invalid = || crate::args::Usage("invalid arguments; run limeosctl --help".into());
    let (limit, rest) = match args {
        [flag, rest @ ..] if flag == "--limit" => {
            let value = crate::args::value(flag, rest.first())?;
            let limit = value
                .parse::<u32>()
                .ok()
                .filter(|n| (1..=MAX_LIMIT).contains(n))
                .ok_or_else(|| {
                    crate::args::Usage(format!("--limit takes 1 to {MAX_LIMIT}, not {value}"))
                })?;
            (limit, &rest[1..])
        }
        _ => (DEFAULT_LIMIT, args),
    };
    match rest {
        [] => Ok((limit, DIRECTORY)),
        [directory] if !directory.starts_with('-') => Ok((limit, directory)),
        _ => Err(invalid()),
    }
}

fn owned_by_caller(owner: u32, caller: u32) -> Result<(), String> {
    if owner != caller {
        return Err(format!(
            "run this as the receipt store's owner (uid {owner}): sudo -u limeos-containerd /usr/lib/limeos/limeosctl action-log. Another account, root included, could leave SQLite files there that the executor cannot open."
        ));
    }
    Ok(())
}

/// The newest `limit` rows, oldest first, and every day's totals, each as
/// one JSON object.
pub fn read(
    directory: &Path,
    limit: u32,
) -> Result<(Vec<serde_json::Value>, Vec<serde_json::Value>), Box<dyn std::error::Error>> {
    let conn = Connection::open_with_flags(
        directory.join("receipts.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(Duration::from_millis(500))?;
    // A small page cache: this reads at most a few MiB once.
    conn.pragma_update(None, "cache_size", -256)?;
    let version: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version < LOG_SCHEMA {
        return Err(format!(
            "the receipt store is at schema {version}, from before the action log (schema {LOG_SCHEMA}); the executor adds it when it next opens the store"
        )
        .into());
    }
    let mut statement = conn.prepare(
        "SELECT seq,at,kind,resource,operation,stack,action,peer,principal,outcome,refusals FROM (SELECT * FROM effect_log ORDER BY seq DESC LIMIT ?1) ORDER BY seq",
    )?;
    let rows = statement
        .query_map([limit], |r| {
            let at: i64 = r.get(1)?;
            Ok(serde_json::json!({
                "seq": r.get::<_, i64>(0)?,
                "at": at,
                "time": utc(at),
                "kind": r.get::<_, String>(2)?,
                "resource": r.get::<_, String>(3)?,
                "operation": r.get::<_, String>(4)?,
                "stack": r.get::<_, Option<String>>(5)?,
                "action": r.get::<_, String>(6)?,
                "peer": r.get::<_, u32>(7)?,
                "principal": r.get::<_, String>(8)?,
                "outcome": r.get::<_, Option<String>>(9)?,
                "refusals": r.get::<_, u32>(10)?,
            }))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut statement =
        conn.prepare("SELECT day,charges,refusals FROM effect_days ORDER BY day")?;
    let days = statement
        .query_map([], |r| {
            let day: i64 = r.get(0)?;
            Ok(serde_json::json!({
                "day": &utc(day.saturating_mul(86_400))[..10],
                "charges": r.get::<_, i64>(1)?,
                "refusals": r.get::<_, i64>(2)?,
            }))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok((rows, days))
}

/// `at` (Unix seconds) as an RFC 3339 UTC time, from the proleptic
/// Gregorian calendar (days to civil date, H. Hinnant's algorithm).
fn utc(at: i64) -> String {
    let (days, secs) = (at.div_euclid(86_400), at.rem_euclid(86_400));
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
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use limeos_executor_container::ReceiptStore;
    use std::os::unix::fs::PermissionsExt;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn arguments_and_owner() {
        assert_eq!(parse(&[]).unwrap(), (100, DIRECTORY));
        let args = strings(&["--limit", "5", "/x"]);
        assert_eq!(parse(&args).unwrap(), (5, "/x"));
        for bad in [
            &["--limit"][..],
            &["--limit", "0"],
            &["--limit", "8193"],
            &["--limit", "-1"],
            &["/x", "/y"],
            &["--other"],
        ] {
            assert!(parse(&strings(bad)).is_err(), "{bad:?}");
        }
        assert!(owned_by_caller(1000, 1000).is_ok());
        let refused = owned_by_caller(998, 0).unwrap_err();
        assert!(refused.contains("sudo -u limeos-containerd"), "{refused}");
    }

    #[test]
    fn utc_times() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(1_800_000_000), "2027-01-15T08:00:00Z");
        assert_eq!(utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(utc(-1), "1969-12-31T23:59:59Z");
    }

    /// The executor's own store, as its open creates it, read back newest
    /// rows first and printed oldest first.
    #[test]
    fn reads_the_newest_rows_of_the_executors_own_table() {
        let dir = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        drop(ReceiptStore::open(dir.path()).unwrap());
        let conn = Connection::open(dir.path().join("receipts.sqlite")).unwrap();
        for i in 0..5i64 {
            conn.execute(
                "INSERT INTO effect_log(at,kind,resource,operation,stack,action,peer,principal,outcome) VALUES(?1,'effect',?2,'restart',NULL,?3,1001,'alice','effect_accepted')",
                rusqlite::params![1_800_000_000 + i, format!("container:{}", "a".repeat(64)), format!("{i:064x}")],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO effect_log(at,kind,resource,operation,stack,action,peer,principal,outcome,refusals) VALUES(1800000100,'refused',?1,'stop','stack:media',?2,1001,'alice','rate_limited',7)",
            rusqlite::params![format!("container:{}", "a".repeat(64)), "f".repeat(64)],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO effect_days(day,charges,refusals) VALUES(20833,5,7)",
            [],
        )
        .unwrap();
        drop(conn);
        let (rows, days) = read(dir.path(), 3).unwrap();
        let seqs: Vec<i64> = rows.iter().map(|r| r["seq"].as_i64().unwrap()).collect();
        assert_eq!(seqs, [4, 5, 6]);
        assert_eq!(rows[2]["kind"], "refused");
        assert_eq!(rows[2]["refusals"], 7);
        assert_eq!(rows[2]["stack"], "stack:media");
        assert_eq!(rows[0]["time"], "2027-01-15T08:00:03Z");
        assert_eq!(rows[0]["stack"], serde_json::Value::Null);
        assert_eq!(
            days,
            [serde_json::json!({"day": "2027-01-15", "charges": 5, "refusals": 7})]
        );
        // Read-only: the reader wrote nothing.
        let (all, _) = read(dir.path(), 8192).unwrap();
        assert_eq!(all.len(), 6);
    }

    #[test]
    fn a_store_from_before_the_log_is_named_as_such() {
        let dir = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        drop(ReceiptStore::open(dir.path()).unwrap());
        let conn = Connection::open(dir.path().join("receipts.sqlite")).unwrap();
        conn.execute_batch("PRAGMA user_version=8;").unwrap();
        drop(conn);
        let error = read(dir.path(), 10).unwrap_err().to_string();
        assert!(error.contains("schema 8"), "{error}");
    }
}
