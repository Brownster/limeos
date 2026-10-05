pub mod config;
use fs2::FileExt;
use limeos_domain::{
    Error, ErrorCode, EventKind, Intent, JobState, Operation, Principal, Result, Role, Scope,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::{
    fs::{File, OpenOptions},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

pub const SCHEMA_VERSION: u32 = 5;
pub const QUEUE_CAPACITY: usize = 64;
const MIN_FREE_BYTES: u64 = 8 * 1024 * 1024;
const AUDIT_LIMIT: i64 = 64 * 1024 * 1024;
type Work = Box<dyn FnOnce(&mut Store) + Send>;

#[derive(Clone)]
pub struct Database {
    sender: mpsc::Sender<Work>,
    pub generation: i64,
}
impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let mut store = Store::open(path)?;
        let generation = store.generation;
        let (sender, mut receiver) = mpsc::channel::<Work>(QUEUE_CAPACITY);
        std::thread::Builder::new()
            .name("limeos-db".into())
            .spawn(move || {
                while let Some(work) = receiver.blocking_recv() {
                    work(&mut store);
                }
                let _ = store.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
            })
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        Ok(Self { sender, generation })
    }
    pub async fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Store) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (sender, receiver) = oneshot::channel();
        self.sender
            .try_send(Box::new(move |s| {
                let _ = sender.send(f(s));
            }))
            .map_err(|_| Error(ErrorCode::Overloaded))?;
        receiver.await.map_err(|_| Error(ErrorCode::Unavailable))?
    }
}

pub struct Store {
    conn: Connection,
    _lock: File,
    directory: PathBuf,
    pub generation: i64,
}
pub struct TaskGrant<'a> {
    pub service_uid: u32,
    pub task: &'a str,
    pub scopes: &'a [Scope],
    pub expires: i64,
}

pub struct LoginRecord {
    pub principal: Principal,
    pub password_hash: String,
}
fn durable(_: rusqlite::Error) -> Error {
    Error(ErrorCode::StateNotDurable)
}
fn json<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|_| Error(ErrorCode::InvalidInput))
}
fn parse<T: serde::de::DeserializeOwned>(value: &str) -> Result<T> {
    serde_json::from_str(value).map_err(|_| Error(ErrorCode::StateNotDurable))
}
fn event(
    tx: &Transaction<'_>,
    principal: Option<&str>,
    job: Option<&str>,
    kind: EventKind,
    now: i64,
) -> Result<()> {
    tx.execute(
        "INSERT INTO events(principal,job,kind,created) VALUES (?,?,?,?)",
        params![principal, job, json(&kind)?, now],
    )
    .map_err(durable)?;
    tx.execute("UPDATE meta SET audit_bytes=audit_bytes+256 WHERE id=1", [])
        .map_err(durable)?;
    Ok(())
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let directory = path
            .parent()
            .ok_or(Error(ErrorCode::InvalidInput))?
            .to_path_buf();
        if std::fs::symlink_metadata(path).is_ok_and(|m| !m.is_file()) {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(0x20000)
            .open(path.with_extension("lock"))
            .map_err(|_| Error(ErrorCode::StateNotDurable))?;
        lock.try_lock_exclusive()
            .map_err(|_| Error(ErrorCode::Conflict))?;
        let mut conn = Connection::open(path).map_err(durable)?;
        std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
            .map_err(|_| Error(ErrorCode::StateNotDurable))?;
        if rusqlite::version_number() < 3_051_003 {
            return Err(Error(ErrorCode::Unavailable));
        }
        conn.busy_timeout(Duration::from_millis(500))
            .map_err(durable)?;
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA wal_autocheckpoint=256; PRAGMA journal_size_limit=4194304; PRAGMA trusted_schema=OFF;").map_err(durable)?;
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(durable)?;
        match version {
            0..=4 => {
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(durable)?;
                if version < 1 {
                    tx.execute_batch(include_str!("schema.sql"))
                        .map_err(durable)?;
                }
                if version < 2 {
                    tx.execute_batch(include_str!("migration-v2.sql"))
                        .map_err(durable)?;
                }
                if version < 3 {
                    tx.execute_batch(include_str!("migration-v3.sql"))
                        .map_err(durable)?;
                }
                if version < 4 {
                    tx.execute_batch(include_str!("migration-v4.sql"))
                        .map_err(durable)?;
                }
                tx.execute_batch(include_str!("migration-v5.sql"))
                    .map_err(durable)?;
                tx.commit().map_err(durable)?;
            }
            SCHEMA_VERSION => {}
            _ => return Err(Error(ErrorCode::Conflict)),
        }
        let check: String = conn
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .map_err(durable)?;
        if check != "ok" {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        if fs2::available_space(&directory).map_err(|_| Error(ErrorCode::StateNotDurable))?
            < MIN_FREE_BYTES
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(durable)?;
        let (audit_bytes, interrupted): (i64, i64) = tx.query_row(
            "SELECT audit_bytes,(SELECT count(*) FROM jobs WHERE state IN ('running','verifying')) FROM meta WHERE id=1",
            [], |r| Ok((r.get(0)?, r.get(1)?)),
        ).map_err(durable)?;
        if audit_bytes >= AUDIT_LIMIT || interrupted > (AUDIT_LIMIT - audit_bytes) / 256 {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        tx.execute(
            "UPDATE meta SET generation=generation+1,audit_bytes=audit_bytes+? WHERE id=1",
            [interrupted * 256],
        )
        .map_err(durable)?;
        let generation = tx
            .query_row("SELECT generation FROM meta WHERE id=1", [], |r| r.get(0))
            .map_err(durable)?;
        // Set-based recovery stays bounded in memory. P01 executors have no
        // effects; uncertain jobs keep their locks and are never retried.
        let recovered_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .min(i64::MAX as u64) as i64;
        tx.execute(
            "INSERT INTO events(principal,job,kind,created) SELECT principal,id,?,? FROM jobs WHERE state IN ('running','verifying')",
            params![json(&EventKind::JobRecoveryRequired)?, recovered_at],
        ).map_err(durable)?;
        tx.execute(
            "UPDATE jobs SET state='needs_intervention' WHERE state IN ('running','verifying')",
            [],
        )
        .map_err(durable)?;
        tx.commit().map_err(durable)?;
        Ok(Self {
            conn,
            _lock: lock,
            directory,
            generation,
        })
    }
    fn write<T>(&mut self, f: impl FnOnce(&Transaction<'_>) -> Result<T>) -> Result<T> {
        if fs2::available_space(&self.directory).map_err(|_| Error(ErrorCode::StateNotDurable))?
            < MIN_FREE_BYTES
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(durable)?;
        let audit: i64 = tx
            .query_row("SELECT audit_bytes FROM meta WHERE id=1", [], |r| r.get(0))
            .map_err(durable)?;
        if audit >= AUDIT_LIMIT {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let value = f(&tx)?;
        tx.commit().map_err(durable)?;
        Ok(value)
    }
}

mod budgets;
mod compose;
mod identity_store;
mod jobs;
mod plans;
pub use compose::compose_scopes;

#[cfg(test)]
mod tests;
