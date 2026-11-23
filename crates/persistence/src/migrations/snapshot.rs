//! The snapshot `Store::open` keeps of a store before migrating it.
//!
//! Files beside `core.sqlite`, for an upgrade from schema 16:
//!
//! - `core.sqlite.v16.snapshot`: the store as it was, written by `VACUUM
//!   INTO`, synced, checked and renamed into place.
//! - `core.sqlite.v16.snapshot.state`: what the snapshot was taken from,
//!   so a later attempt at the same upgrade can keep it instead of writing
//!   another copy: the SHA-256 and length of the store's database file
//!   (its WAL checkpointed into it) and of the snapshot.
//! - `.snapshot.partial`, `.snapshot.partial-journal` and
//!   `.snapshot.state.partial`: leftovers of an interrupted write.
//!
//! An upgrade that fails after its snapshot leaves the store at its old
//! version, and systemd starts core again two seconds later. Without the
//! record every start would write a full, synced copy of the store, all
//! day, to an SD card. With it a repeated attempt reads the store and the
//! snapshot and writes nothing, as long as both are exactly as recorded. A
//! record that is missing, unreadable, from another version or for other
//! bytes means the snapshot is taken again, never kept.
use super::*;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

/// What a file beside the store is to its snapshots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Snapshot,
    State,
    Leftover,
}
/// The schema version and kind of `file` if it is one of the snapshot files
/// of the store named `store`.
pub(crate) fn classify(store: &str, file: &str) -> Option<(u32, Kind)> {
    let rest = file.strip_prefix(store)?.strip_prefix(".v")?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    let (version, suffix) = rest.split_at(digits);
    let kind = match suffix {
        ".snapshot" => Kind::Snapshot,
        ".snapshot.state" => Kind::State,
        ".snapshot.partial" | ".snapshot.partial-journal" | ".snapshot.state.partial" => {
            Kind::Leftover
        }
        _ => return None,
    };
    Some((version.parse().ok()?, kind))
}
fn named(path: &Path, from: u32, suffix: &str) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    Some(path.with_file_name(format!("{name}.v{from}.snapshot{suffix}")))
}
/// The snapshot of the store at `path` taken at schema `from`:
/// `core.sqlite.v16.snapshot` beside `core.sqlite`.
pub(crate) fn snapshot_path(path: &Path, from: u32) -> Option<PathBuf> {
    named(path, from, "")
}
/// Its record.
pub(crate) fn state_path(path: &Path, from: u32) -> Option<PathBuf> {
    named(path, from, ".state")
}

struct Entry {
    path: PathBuf,
    version: u32,
    kind: Kind,
    bytes: u64,
}
/// The store's snapshot files, finished, recorded or left over.
fn entries(path: &Path) -> Result<Vec<Entry>> {
    let failed = |_| Error(ErrorCode::StateNotDurable);
    let directory = path.parent().ok_or(Error(ErrorCode::InvalidInput))?;
    let store = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(Error(ErrorCode::InvalidInput))?;
    let mut found = Vec::new();
    for entry in std::fs::read_dir(directory).map_err(failed)? {
        let entry = entry.map_err(failed)?;
        let name = entry.file_name();
        let Some((version, kind)) = name.to_str().and_then(|name| classify(store, name)) else {
            continue;
        };
        let bytes = entry.metadata().map_or(0, |m| m.len());
        found.push(Entry {
            path: entry.path(),
            version,
            kind,
            bytes,
        });
    }
    Ok(found)
}
fn sync_directory(path: &Path) -> Result<()> {
    let directory = path.parent().ok_or(Error(ErrorCode::InvalidInput))?;
    File::open(directory)
        .and_then(|dir| dir.sync_all())
        .map_err(|_| Error(ErrorCode::StateNotDurable))
}

/// Removes what an interrupted snapshot left, of any version, before free
/// space is measured: it is never kept, and it must not make a start that
/// would fit look short of space.
pub(crate) fn remove_leftovers(path: &Path) -> Result<()> {
    let mut removed = false;
    for entry in entries(path)? {
        if entry.kind == Kind::Leftover {
            std::fs::remove_file(&entry.path).map_err(|_| Error(ErrorCode::StateNotDurable))?;
            removed = true;
        }
    }
    if removed {
        sync_directory(path)?;
    }
    Ok(())
}

/// Bytes a new snapshot of the store will take: its pages in use (`VACUUM
/// INTO` leaves out the free ones).
pub(crate) fn snapshot_bytes(conn: &Connection) -> Result<u64> {
    let (pages, free, size): (i64, i64, i64) = conn
        .query_row(
            "SELECT (SELECT page_count FROM pragma_page_count),(SELECT freelist_count FROM pragma_freelist_count),(SELECT page_size FROM pragma_page_size)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(durable)?;
    Ok(u64::try_from(pages.saturating_sub(free).saturating_mul(size)).unwrap_or(0))
}

/// How the snapshot before an upgrade will be had, and what it costs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    from: u32,
    /// The finished snapshot of this version is exactly as recorded and the
    /// store has not changed since: it is kept and nothing is written.
    pub(crate) reuse: bool,
    /// Bytes the new snapshot takes: none when one is kept.
    pub(crate) bytes: u64,
    /// Bytes of the store's snapshot files that go before anything is
    /// written: every one but a kept snapshot and its record.
    pub(crate) reclaimable: u64,
}
/// What [`take`] will do for an upgrade of the store at `path` from `from`.
/// `checkpoint`: the store is open for writing, so its WAL can be
/// checkpointed into the database file before comparing; without it (a
/// read-only look, for `upgrade_space`) a snapshot is kept only when the
/// WAL is already empty.
pub(crate) fn plan(conn: &Connection, path: &Path, from: u32, checkpoint: bool) -> Result<Plan> {
    let reuse = reusable(conn, path, from, checkpoint)?;
    let bytes = if reuse { 0 } else { snapshot_bytes(conn)? };
    let reclaimable = entries(path)?
        .iter()
        .filter(|e| !(reuse && e.version == from && e.kind != Kind::Leftover))
        .map(|e| e.bytes)
        .fold(0u64, u64::saturating_add);
    Ok(Plan {
        from,
        reuse,
        bytes,
        reclaimable,
    })
}

/// Has the snapshot ready: removes every other snapshot file of the store
/// (what the space check counted as reclaimable), then keeps the planned
/// snapshot or writes a new one. Older snapshots go first, so the space is
/// really there; the store itself is unchanged until the new snapshot is
/// durable, and an older version's snapshot could only roll back across an
/// upgrade the store has since completed. Any failure refuses the upgrade.
pub(crate) fn take(conn: &Connection, path: &Path, plan: Plan) -> Result<()> {
    let from = plan.from;
    let mut removed = false;
    for entry in entries(path)? {
        if plan.reuse && entry.version == from && entry.kind != Kind::Leftover {
            continue;
        }
        std::fs::remove_file(&entry.path).map_err(|_| Error(ErrorCode::StateNotDurable))?;
        removed = true;
    }
    if removed {
        sync_directory(path)?;
    }
    if plan.reuse {
        return Ok(());
    }
    write(conn, path, from)
}

/// The SHA-256 and length of a file.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Digested {
    bytes: u64,
    sha256: String,
}
fn digest(path: &Path) -> Option<Digested> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)
        .ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut bytes = 0u64;
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        bytes += read as u64;
    }
    Some(Digested {
        bytes,
        sha256: hex::encode(hasher.finalize()),
    })
}
fn data_version(conn: &Connection) -> Result<i64> {
    conn.pragma_query_value(None, "data_version", |r| r.get(0))
        .map_err(durable)
}
fn wal_empty(path: &Path) -> bool {
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    std::fs::symlink_metadata(PathBuf::from(wal)).map_or(true, |m| m.len() == 0)
}
/// Whether everything committed is in the database file: after a complete
/// `TRUNCATE` checkpoint when `checkpoint`, else when the WAL is empty.
fn settled(conn: &Connection, path: &Path, checkpoint: bool) -> Result<bool> {
    if checkpoint {
        let busy: i64 = conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))
            .map_err(durable)?;
        if busy != 0 {
            return Ok(false);
        }
    }
    Ok(wal_empty(path))
}
/// The store's database file, read with nothing committed outside it and no
/// commit by any connection while it was read; `None` when that cannot be
/// had, which never permits keeping a snapshot.
fn store_identity(conn: &Connection, path: &Path, checkpoint: bool) -> Result<Option<Digested>> {
    if !settled(conn, path, checkpoint)? {
        return Ok(None);
    }
    let before = data_version(conn)?;
    let store = digest(path);
    let after = data_version(conn)?;
    Ok(store.filter(|_| before == after && wal_empty(path)))
}

fn read_state(path: &Path) -> Option<(u32, Digested, Digested)> {
    let mut text = String::new();
    OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)
        .ok()?
        .take(4096)
        .read_to_string(&mut text)
        .ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let digested = |key: &str| {
        Some(Digested {
            bytes: value.get(key)?.get("bytes")?.as_u64()?,
            sha256: value.get(key)?.get("sha256")?.as_str()?.to_owned(),
        })
    };
    let from = u32::try_from(value.get("from")?.as_u64()?).ok()?;
    Some((from, digested("store")?, digested("snapshot")?))
}
/// Whether the finished snapshot of `from` can be kept: its record is for
/// this version, the snapshot's bytes are the recorded ones, and so are the
/// store's, read as [`store_identity`] reads them.
fn reusable(conn: &Connection, path: &Path, from: u32, checkpoint: bool) -> Result<bool> {
    let (Some(snapshot), Some(state)) = (snapshot_path(path, from), state_path(path, from)) else {
        return Ok(false);
    };
    let Some((recorded, store, copy)) = read_state(&state) else {
        return Ok(false);
    };
    if recorded != from {
        return Ok(false);
    }
    let Some(now) = store_identity(conn, path, checkpoint)? else {
        return Ok(false);
    };
    Ok(now == store && digest(&snapshot).as_ref() == Some(&copy))
}

/// Writes the snapshot of the store at `path` (schema `from`, open on `conn`
/// outside any transaction), then its record.
///
/// `VACUUM INTO` reads one consistent state of the database through SQLite,
/// the WAL's committed frames included, which a copy of the database file
/// alone would miss. It goes to a partial file first, created here with the
/// store's mode (0600, owned by this process like the store); SQLite does
/// not sync it, so it is synced here, checked (its version and
/// `quick_check`), renamed into place and the directory synced. Any failure
/// removes the partial file and fails, so the upgrade does not start.
///
/// The record is written only when the store's identity was read before the
/// copy and no connection committed between that read and the end of the
/// copy, so it describes exactly what the snapshot holds. Without a record
/// the next attempt takes the snapshot again; a record that cannot be
/// written does not stop this upgrade.
fn write(conn: &Connection, path: &Path, from: u32) -> Result<()> {
    let failed = |_| Error(ErrorCode::StateNotDurable);
    let target = snapshot_path(path, from).ok_or(Error(ErrorCode::InvalidInput))?;
    let partial = named(path, from, ".partial").ok_or(Error(ErrorCode::InvalidInput))?;
    let store = store_identity(conn, path, true)?;
    let before = data_version(conn)?;
    let written = (|| {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(&partial)
            .map_err(failed)?;
        let into = partial.to_str().ok_or(Error(ErrorCode::InvalidInput))?;
        conn.execute("VACUUM INTO ?", [into]).map_err(durable)?;
        let copy = Connection::open_with_flags(
            &partial,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(durable)?;
        let (version, check): (u32, String) = copy
            .query_row(
                "SELECT (SELECT user_version FROM pragma_user_version),(SELECT quick_check FROM pragma_quick_check)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(durable)?;
        drop(copy);
        if version != from || check != "ok" {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        File::open(&partial)
            .and_then(|file| file.sync_all())
            .map_err(failed)?;
        let copied = digest(&partial);
        std::fs::rename(&partial, &target).map_err(failed)?;
        sync_directory(path)?;
        Ok(copied)
    })();
    let copied = match written {
        Ok(copied) => copied,
        Err(error) => {
            let _ = std::fs::remove_file(&partial);
            let _ = std::fs::remove_file(named(path, from, ".partial-journal").unwrap_or_default());
            return Err(error);
        }
    };
    let after = data_version(conn)?;
    if let (Some(store), Some(copied)) = (store, copied)
        && before == after
        && wal_empty(path)
    {
        record(path, from, &store, &copied);
    }
    Ok(())
}
/// Writes the record of a snapshot, as a partial file synced and renamed
/// into place; on failure removes what it wrote and leaves no record.
fn record(path: &Path, from: u32, store: &Digested, copy: &Digested) {
    let (Some(state), Some(partial)) =
        (state_path(path, from), named(path, from, ".state.partial"))
    else {
        return;
    };
    let text = serde_json::json!({
        "from": from,
        "store": {"bytes": store.bytes, "sha256": store.sha256},
        "snapshot": {"bytes": copy.bytes, "sha256": copy.sha256},
    })
    .to_string();
    let written = (|| {
        let _ = std::fs::remove_file(&partial);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(&partial)
            .ok()?;
        file.write_all(text.as_bytes()).ok()?;
        file.sync_all().ok()?;
        std::fs::rename(&partial, &state).ok()?;
        sync_directory(path).ok()
    })();
    if written.is_none() {
        let _ = std::fs::remove_file(&partial);
        let _ = std::fs::remove_file(&state);
    }
}
