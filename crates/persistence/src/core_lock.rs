//! The single-core `flock` on `core.lock`, released explicitly when the Store drops.
use super::*;

/// Owns the locked `core.lock` descriptor. `Store` declares it after its
/// connection, so the order on drop is: SQLite connection closed, lock released,
/// descriptor closed.
pub(crate) struct CoreLock(pub(crate) File);
impl Drop for CoreLock {
    fn drop(&mut self) {
        // An flock belongs to the open file description, which a child made by
        // fork/posix_spawn shares until it execs. Closing only our descriptor would
        // leave such a child holding the lock, and a reopen in this process would be
        // refused with Conflict. LOCK_UN releases it for the whole description now.
        // If it fails, the close that follows still releases it once the last
        // descriptor is gone; never panic here (release builds abort).
        let _ = FileExt::unlock(&self.0);
    }
}

#[cfg(test)]
mod tests;
