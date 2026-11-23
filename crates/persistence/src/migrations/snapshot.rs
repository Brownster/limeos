//! The snapshot `Store::open` keeps of a store before migrating it.
//!
//! Files beside `core.sqlite`, for an upgrade from schema 16:
//!
//! - `core.sqlite.v16.snapshot`: the store as it was, written by `VACUUM
//!   INTO`, synced, checked and renamed into place.
//! - `core.sqlite.v16.snapshot.state`: its record: what it was taken from
//!   (the SHA-256 and length of the store's database file, its WAL
//!   checkpointed into it) and what it is (its SHA-256, length, inode and
//!   modification time).
//! - `.snapshot.partial`, `.snapshot.partial-journal` and
//!   `.snapshot.state.partial`: leftovers of an interrupted write, removed
//!   before the store is opened ([`remove_leftovers`]).
//!
//! The upgrade runs on a connection that holds the store exclusively
//! (`locking_mode=EXCLUSIVE`, taken by an empty write transaction as its
//! first access, so no other connection can read or write the store) from
//! before the store is hashed until the migration commits.
//!
//! An upgrade that fails after its snapshot leaves the store at its old
//! version, and systemd starts core again two seconds later. With the
//! record, a repeated attempt keeps the snapshot and writes no copy, as
//! long as the store and the snapshot are exactly as recorded. The cheap
//! parts of the record (lengths, inode, modification time) rule a keep out
//! without reading; only a match is confirmed by hashing the store, then
//! the snapshot. A record that is missing, unreadable, from another version
//! or for other bytes means the snapshot is taken again, never kept.
use super::*;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;

#[cfg(test)]
thread_local! {
    /// Tests: the named step of writing a snapshot fails.
    pub(crate) static FAIL_AT: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
    /// Tests: what was removed and written, in order.
    pub(crate) static EVENTS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Tests: runs between the store's hash and the copy.
    pub(crate) static INTERLOPER: std::cell::Cell<Option<fn(&Path)>> = const { std::cell::Cell::new(None) };
}
fn fail_point(_at: &'static str) -> Result<()> {
    #[cfg(test)]
    if FAIL_AT.get() == Some(_at) {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    Ok(())
}
fn event(_what: &str, _file: &Path) {
    #[cfg(test)]
    EVENTS.with(|events| {
        let name = _file.file_name().and_then(|n| n.to_str()).unwrap_or("");
        events.borrow_mut().push(format!("{_what} {name}"));
    });
}

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

/// Bytes removing a file gives back: its allocated blocks, and only when no
/// other name keeps them (a hard link) — not its length, which a sparse
/// file exceeds.
pub(crate) fn freed_by_removing(meta: &std::fs::Metadata) -> u64 {
    if meta.is_file() && meta.nlink() == 1 {
        meta.blocks().saturating_mul(512)
    } else {
        0
    }
}

struct Entry {
    path: PathBuf,
    version: u32,
    kind: Kind,
    freed: u64,
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
        // Only regular files: a directory or other object under one of these
        // names is never removed or counted (a write that needs the name
        // then fails on it).
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let freed = freed_by_removing(&meta);
        found.push(Entry {
            path: entry.path(),
            version,
            kind,
            freed,
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
/// Removes the entries `remove` selects, then syncs the directory.
fn remove(path: &Path, remove: impl Fn(&Entry) -> bool) -> Result<()> {
    let mut removed = false;
    for entry in entries(path)? {
        if remove(&entry) {
            event("remove", &entry.path);
            std::fs::remove_file(&entry.path).map_err(|_| Error(ErrorCode::StateNotDurable))?;
            removed = true;
        }
    }
    if removed {
        sync_directory(path)?;
    }
    Ok(())
}

/// Removes what an interrupted snapshot left, of any version, by plain file
/// operations before the store is even opened: opening it may need space
/// of its own (SQLite allocates the WAL index first), which a leftover the
/// size of the store could otherwise hold forever. Only the partial names
/// go, nothing an operator could want. `Store::open` ignores a failure: a
/// store that needs no upgrade does not need the names, and an upgrade
/// that does fails when it writes its partial file.
pub(crate) fn remove_leftovers(path: &Path) -> Result<()> {
    remove(path, |e| e.kind == Kind::Leftover)
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

/// How a plan decides whether a snapshot can be kept.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Look<'a> {
    /// `Store::open`, holding the store exclusively: checkpoint its WAL into
    /// the database file and confirm a match by hashing it through `store`,
    /// a descriptor kept open until the exclusive connection has closed.
    Writer { store: &'a File },
    /// A diagnostic estimate on a read-only connection: decide from the
    /// record's cheap parts alone, with the WAL already empty. Never used
    /// to keep a snapshot, only to report figures.
    Estimate,
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
    /// Bytes that removing the store's other snapshot files gives back
    /// (every one but a kept snapshot and its record), counted honestly
    /// ([`freed_by_removing`]).
    pub(crate) freed: u64,
}
/// What [`take`] will do for an upgrade of the store at `path` from `from`.
pub(crate) fn plan(conn: &Connection, path: &Path, from: u32, look: Look<'_>) -> Result<Plan> {
    let reuse = reusable(conn, path, from, look)?;
    let bytes = if reuse { 0 } else { snapshot_bytes(conn)? };
    let freed = entries(path)?
        .iter()
        .filter(|e| !(reuse && e.version == from && e.kind != Kind::Leftover))
        .map(|e| e.freed)
        .fold(0u64, u64::saturating_add);
    Ok(Plan {
        from,
        reuse,
        bytes,
        freed,
    })
}

/// Has the snapshot ready, on `conn`, which holds the store exclusively
/// (`locking_mode=EXCLUSIVE`, taken before its first read): no other
/// connection can read or write the store from before the store is hashed
/// until the migration transaction commits, so the hash, the copy and the
/// migration see one state.
///
/// A kept snapshot: the store's other snapshot files go. A new one: when
/// the free space alone fits it with the write reserve to spare
/// (`write_first`), it is written first and the other files go after, so
/// the previous snapshot survives a failed write; only when it does not are
/// they removed first. The store itself is unchanged until the new snapshot
/// is durable either way, and an older version's snapshot could only roll
/// back across an upgrade the store has since completed. Any failure
/// refuses the upgrade.
pub(crate) fn take(
    conn: &Connection,
    store: &File,
    path: &Path,
    plan: Plan,
    write_first: bool,
) -> Result<()> {
    let from = plan.from;
    if plan.reuse {
        return remove(path, |e| e.version != from || e.kind == Kind::Leftover);
    }
    // The old record of this version describes another snapshot: it goes
    // before anything else.
    remove(path, |e| e.version == from && e.kind == Kind::State)?;
    if !write_first {
        remove(path, |_| true)?;
    }
    write(conn, store, path, from)?;
    remove(path, |e| {
        !(e.version == from && matches!(e.kind, Kind::Snapshot | Kind::State))
    })
}

/// The SHA-256 and length of a file's contents.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Digested {
    bytes: u64,
    sha256: String,
}
fn digest_open(mut file: &File) -> Option<Digested> {
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(0)).ok()?;
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
/// The digest of a file no SQLite connection has open (the snapshot, or its
/// partial file). Never the store: closing any descriptor of a file drops
/// every POSIX lock this process holds on it, SQLite's included, so the
/// store is read only through the descriptor `Look::Writer` carries.
fn digest(path: &Path) -> Option<Digested> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)
        .ok()?;
    digest_open(&file)
}
/// The parts of a snapshot file's identity that cost no read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cheap {
    bytes: u64,
    inode: u64,
    modified: (i64, i64),
}
fn cheap(path: &Path) -> Option<Cheap> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    meta.is_file().then(|| Cheap {
        bytes: meta.len(),
        inode: meta.ino(),
        modified: (meta.mtime(), meta.mtime_nsec()),
    })
}
/// Whether the store's WAL holds nothing: absent or empty. Any other answer
/// (an error but "not found") counts as not empty.
fn wal_empty(path: &Path) -> bool {
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    match std::fs::symlink_metadata(PathBuf::from(wal)) {
        Ok(meta) => meta.len() == 0,
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}
/// Everything committed copied into the store's database file: a complete
/// `TRUNCATE` checkpoint, which nothing can hold up on an exclusive
/// connection.
fn checkpoint(conn: &Connection, path: &Path) -> Result<()> {
    let busy: i64 = conn
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))
        .map_err(durable)?;
    if busy != 0 || !wal_empty(path) {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    Ok(())
}

struct Record {
    from: u32,
    store: Digested,
    snapshot: Digested,
    cheap: Cheap,
}
fn read_record(path: &Path) -> Option<Record> {
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
    let snapshot = value.get("snapshot")?;
    Some(Record {
        from: u32::try_from(value.get("from")?.as_u64()?).ok()?,
        store: digested("store")?,
        snapshot: digested("snapshot")?,
        cheap: Cheap {
            bytes: snapshot.get("bytes")?.as_u64()?,
            inode: snapshot.get("inode")?.as_u64()?,
            modified: (
                snapshot.get("mtime")?.as_i64()?,
                snapshot.get("mtime_nsec")?.as_i64()?,
            ),
        },
    })
}
/// Whether the finished snapshot of `from` can be kept.
///
/// Ruled out without reading when the record is missing, unreadable or for
/// another version, when the snapshot's length, inode or modification time
/// differ from the record, or when the store's length does. Otherwise, for
/// the writer, confirmed by hashing the store (its WAL checkpointed into it)
/// and then the snapshot; the estimate stops at the cheap checks, with the
/// WAL already empty.
fn reusable(conn: &Connection, path: &Path, from: u32, look: Look<'_>) -> Result<bool> {
    let (Some(snapshot), Some(state)) = (snapshot_path(path, from), state_path(path, from)) else {
        return Ok(false);
    };
    let Some(record) = read_record(&state) else {
        return Ok(false);
    };
    if record.from != from || cheap(&snapshot) != Some(record.cheap) {
        return Ok(false);
    }
    match look {
        Look::Writer { .. } => checkpoint(conn, path)?,
        Look::Estimate if !wal_empty(path) => return Ok(false),
        Look::Estimate => {}
    }
    let store_bytes = std::fs::symlink_metadata(path).map_or(u64::MAX, |m| m.len());
    if store_bytes != record.store.bytes {
        return Ok(false);
    }
    let Look::Writer { store } = look else {
        return Ok(true);
    };
    Ok(digest_open(store).as_ref() == Some(&record.store)
        && digest(&snapshot).as_ref() == Some(&record.snapshot))
}

/// Writes the snapshot of the store at `path` (schema `from`, held
/// exclusively by `conn`, outside any transaction), then its record.
///
/// `VACUUM INTO` reads one consistent state of the database through SQLite,
/// the WAL's committed frames included, which a copy of the database file
/// alone would miss. It goes to a partial file first, created here with the
/// store's mode (0600, owned by this process like the store); SQLite does
/// not sync it, so it is synced here, checked (its version and
/// `quick_check`), renamed into place and the directory synced. Any failure
/// removes the partial file and fails, so the upgrade does not start.
///
/// The record holds the store's hash, taken (through `store`, after a full
/// checkpoint) just before the copy; the exclusive connection keeps every
/// other connection out between the two, so it describes exactly what the
/// snapshot holds. A record that cannot be written leaves the next attempt
/// to take the snapshot again; it is noted for the daemon to log
/// ([`crate::unrecorded_snapshot`]) and does not stop this upgrade.
fn write(conn: &Connection, store: &File, path: &Path, from: u32) -> Result<()> {
    let failed = |_| Error(ErrorCode::StateNotDurable);
    let target = snapshot_path(path, from).ok_or(Error(ErrorCode::InvalidInput))?;
    let partial = named(path, from, ".partial").ok_or(Error(ErrorCode::InvalidInput))?;
    checkpoint(conn, path)?;
    let hashed = digest_open(store);
    #[cfg(test)]
    if let Some(interloper) = INTERLOPER.get() {
        interloper(path);
    }
    let written = (|| {
        event("write", &target);
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(&partial)
            .map_err(failed)?;
        let into = partial.to_str().ok_or(Error(ErrorCode::InvalidInput))?;
        fail_point("vacuum")?;
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
        fail_point("sync")?;
        File::open(&partial)
            .and_then(|file| file.sync_all())
            .map_err(failed)?;
        let copied = digest(&partial);
        fail_point("rename")?;
        std::fs::rename(&partial, &target).map_err(failed)?;
        sync_directory(path)?;
        Ok(copied)
    })();
    let copied = match written {
        Ok(copied) => copied,
        Err(error) => {
            let _ = std::fs::remove_file(&partial);
            if let Some(journal) = named(path, from, ".partial-journal") {
                let _ = std::fs::remove_file(journal);
            }
            return Err(error);
        }
    };
    let recorded = match (hashed, copied, cheap(&target)) {
        (Some(hashed), Some(copied), Some(identity)) => {
            record(path, from, &hashed, &copied, identity)
        }
        _ => false,
    };
    if !recorded {
        crate::note_unrecorded(path);
    }
    Ok(())
}
/// Writes the record of a snapshot, as a partial file synced and renamed
/// into place; on failure removes what it wrote and returns false.
fn record(path: &Path, from: u32, store: &Digested, copy: &Digested, identity: Cheap) -> bool {
    let (Some(state), Some(partial)) =
        (state_path(path, from), named(path, from, ".state.partial"))
    else {
        return false;
    };
    let text = serde_json::json!({
        "from": from,
        "store": {"bytes": store.bytes, "sha256": store.sha256},
        "snapshot": {
            "bytes": copy.bytes,
            "sha256": copy.sha256,
            "inode": identity.inode,
            "mtime": identity.modified.0,
            "mtime_nsec": identity.modified.1,
        },
    })
    .to_string();
    let written = (|| {
        let _ = std::fs::remove_file(&partial);
        fail_point("record").ok()?;
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
        return false;
    }
    true
}
