//! Decode supplied probe bytes only. No transport, process or namespace access.
use limeos_domain::deployment::egress::{ProbeOutcome, admit_address};
use std::net::IpAddr;

pub const MAX_RESPONSE_BYTES: usize = 64;

fn decode(completed: Option<bool>, bytes: &[u8]) -> ProbeOutcome {
    if bytes.len() > MAX_RESPONSE_BYTES {
        return ProbeOutcome::ResponseLimit;
    }
    match completed {
        None => return ProbeOutcome::MissingTerminalStatus,
        Some(false) => return ProbeOutcome::Failed,
        Some(true) => {}
    }
    let address = std::str::from_utf8(bytes)
        .ok()
        .and_then(|s| {
            s.trim_matches(|c: char| c.is_ascii_whitespace())
                .parse::<IpAddr>()
                .ok()
        })
        .and_then(admit_address);
    match address {
        Some(address) => ProbeOutcome::Address(address),
        None => ProbeOutcome::InvalidResponse,
    }
}

/// A supplied status is ordinary metadata, not verified HTTP/endpoint evidence.
pub fn parse_http_address(status: Option<u16>, bytes: &[u8]) -> ProbeOutcome {
    decode(status.map(|status| status == 200), bytes)
}

/// A nullable/missing terminal code never turns plausible output into success.
/// The collector must separately prove endpoint, process/namespace and lifetime.
pub fn parse_exec_address(exit_code: Option<i64>, bytes: &[u8]) -> ProbeOutcome {
    decode(exit_code.map(|code| code == 0), bytes)
}

#[cfg(test)]
mod tests;
