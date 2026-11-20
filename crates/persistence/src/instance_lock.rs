//! One Store per database, across and within processes.
//!
//! The lock is a POSIX record lock (`fcntl(F_SETLK)`), not `flock`. A `flock`
//! belongs to the open file description, which every child created by
//! `fork`/`posix_spawn` shares until it execs (or for its whole life, if handed
//! the descriptor). A child spawned while a Store was open (a password worker,
//! `lsblk`, a test helper) could therefore still hold the lock after the Store
//! was dropped, and reopening it failed with `Conflict`. Record locks belong to
//! the process and are never inherited.
//!
//! Record locks do not exclude a second holder in the same process, and closing
//! any descriptor for the file releases them. The registry below refuses a second
//! in-process holder before it opens the file, so the holder's lock is never
//! released by someone else's close. Nothing else may open the lock file.
use super::*;
use std::{collections::BTreeSet, os::unix::fs::MetadataExt, sync::Mutex};

type Key = (u64, u64);
static HELD: Mutex<BTreeSet<Key>> = Mutex::new(BTreeSet::new());

fn held() -> std::sync::MutexGuard<'static, BTreeSet<Key>> {
    // Insert and remove cannot leave the set half-updated, so a poisoned guard is usable.
    HELD.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) struct InstanceLock {
    file: Option<File>,
    key: Key,
}
impl InstanceLock {
    pub(crate) fn acquire(path: &Path) -> Result<Self> {
        let mut held = held();
        if let Ok(metadata) = std::fs::symlink_metadata(path)
            && held.contains(&(metadata.dev(), metadata.ino()))
        {
            return Err(Error(ErrorCode::Conflict));
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(path)
            .map_err(|_| Error(ErrorCode::StateNotDurable))?;
        let metadata = file
            .metadata()
            .map_err(|_| Error(ErrorCode::StateNotDurable))?;
        let key = (metadata.dev(), metadata.ino());
        if held.contains(&key) {
            // The path was swapped for a held file after the check above. Closing this
            // descriptor would release that holder's lock, so it is kept open instead.
            std::mem::forget(file);
            return Err(Error(ErrorCode::Conflict));
        }
        rustix::fs::fcntl_lock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive)
            .map_err(|_| Error(ErrorCode::Conflict))?;
        held.insert(key);
        Ok(Self {
            file: Some(file),
            key,
        })
    }
}
impl Drop for InstanceLock {
    fn drop(&mut self) {
        // Close before unregistering, under the registry lock: a concurrent acquire must
        // not open the file while this descriptor can still release the lock.
        let mut held = held();
        drop(self.file.take());
        held.remove(&self.key);
    }
}

#[cfg(test)]
mod tests;
