//! Bounded deterministic automation facts; no authority, persistence or effects.
mod alerts;
mod delivery;
mod runner;
mod schedule;
mod signals;
mod views;
pub use alerts::*;
pub use delivery::*;
pub use runner::*;
pub use schedule::*;
pub use signals::*;
pub use views::*;

pub const MAX_SCHEDULES: usize = 128;
pub const MAX_TICK_SCHEDULES: usize = 32;
pub const MAX_ALERTS: usize = 256;
pub const MAX_NOTIFICATIONS: usize = 1024;
pub const MAX_TIME: i64 = 253_402_300_799;
/// Exact integer bound for the proposed JSON/TypeScript wire representation.
pub const MAX_REVISION: i64 = 9_007_199_254_740_991;

pub(crate) fn valid_time(value: i64) -> bool {
    (0..=MAX_TIME).contains(&value)
}
pub(crate) fn text(value: &str, cap: usize) -> bool {
    !value.is_empty() && value.len() <= cap && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests;
