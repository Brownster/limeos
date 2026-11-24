//! Plan lifetimes.
use super::invalid;
use crate::{Error, ErrorCode, PLAN_TTL_SECONDS, Result};

/// A plan's validity window, `[created_at, expires_at)` in seconds.
///
/// Two shapes are accepted, by two named constructors, because both are held
/// in stores today: [`PlanLifetime::bounded`] (any positive window up to
/// [`PLAN_TTL_SECONDS`]) for container, stack and storage plans, and
/// [`PlanLifetime::exact`] (exactly [`PLAN_TTL_SECONDS`]) for Compose and
/// backup-preparation plans. Every core issuer writes the exact shape
/// ([`PlanLifetime::issue`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanLifetime {
    created_at: i64,
    expires_at: i64,
}
impl PlanLifetime {
    /// `0 <= created_at < expires_at <= created_at + PLAN_TTL_SECONDS`.
    pub fn bounded(created_at: i64, expires_at: i64) -> Result<Self> {
        if created_at < 0
            || expires_at <= created_at
            || expires_at > created_at.saturating_add(PLAN_TTL_SECONDS)
        {
            return Err(invalid());
        }
        Ok(Self {
            created_at,
            expires_at,
        })
    }
    /// `0 <= created_at` and `expires_at == created_at + PLAN_TTL_SECONDS`.
    pub fn exact(created_at: i64, expires_at: i64) -> Result<Self> {
        if created_at < 0 || created_at.checked_add(PLAN_TTL_SECONDS) != Some(expires_at) {
            return Err(invalid());
        }
        Ok(Self {
            created_at,
            expires_at,
        })
    }
    /// A new plan's window, opening at `now`.
    pub fn issue(now: i64) -> Result<Self> {
        Self::exact(now, now.checked_add(PLAN_TTL_SECONDS).ok_or_else(invalid)?)
    }
    pub fn created_at(self) -> i64 {
        self.created_at
    }
    pub fn expires_at(self) -> i64 {
        self.expires_at
    }
    /// `Expired` unless `created_at <= now < expires_at`. A clock behind the
    /// plan's creation is expiry too: the plan cannot be shown current.
    pub fn current(self, now: i64) -> Result<()> {
        if now < self.created_at || now >= self.expires_at {
            return Err(Error(ErrorCode::Expired));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
