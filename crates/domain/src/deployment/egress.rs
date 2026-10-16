//! Supplied address comparisons; these facts establish no VPN or namespace proof.
use crate::{ContainerSnapshot, Error, ErrorCode, Result};
use serde::Serialize;
use std::{io::Write, net::IpAddr};

pub const MAX_AGE_SECONDS: i64 = 90;
pub const MAX_SAMPLE_SKEW_SECONDS: i64 = 5;
pub const MAX_REPORT_BYTES: usize = 4 * 1024;
const MAX_TIME: i64 = 253_402_300_799;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Target {
    pub host_boot_id: String,
    pub engine_id: String,
    pub provider: ContainerSnapshot,
}
impl Target {
    pub fn validate(&self) -> Result<()> {
        if !boot_id(&self.host_boot_id)
            || self.engine_id.is_empty()
            || self.engine_id.len() > 128
            || self.engine_id.chars().any(char::is_control)
        {
            return Err(invalid());
        }
        self.provider.validate()
    }
}

/// Ordinary supplied outcomes. An address or terminal result is not a receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "address", rename_all = "snake_case")]
pub enum ProbeOutcome {
    Address(IpAddr),
    Failed,
    MissingTerminalStatus,
    TimedOut,
    Incomplete,
    InvalidResponse,
    ResponseLimit,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub sampled_at: i64,
    pub outcome: ProbeOutcome,
}
pub struct HostObservation {
    pub host_boot_id: String,
    pub sample: Sample,
}
pub struct ProviderObservation {
    pub target: Target,
    pub sample: Sample,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    Fresh,
    Unavailable,
    ScopeMismatch,
    Future,
    Stale,
    Failed,
    MissingTerminalStatus,
    TimedOut,
    Incomplete,
    InvalidResponse,
    ResponseLimit,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct SourceView {
    pub sampled_at: Option<i64>,
    pub state: SourceState,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownReason {
    UnusableSources,
    ProviderNotRunning,
    SampleSkew,
    AddressFamilyMismatch,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Comparison {
    SameAddress,
    DifferentAddresses,
    Unknown { reason: UnknownReason },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct AddressPair {
    pub host: IpAddr,
    pub provider: IpAddr,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VpnEvidence {
    NotEstablished,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EgressComparison {
    pub version: u16,
    pub generated_at: i64,
    pub target: Target,
    pub host: SourceView,
    pub provider: SourceView,
    pub max_age_seconds: i64,
    pub max_sample_skew_seconds: i64,
    pub comparison: Comparison,
    /// Both addresses are withheld unless the complete pair is comparable.
    pub addresses: Option<AddressPair>,
    pub vpn_evidence: VpnEvidence,
}
impl EgressComparison {
    pub fn check_size(&self) -> Result<()> {
        struct Count(usize);
        impl Write for Count {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if bytes.len() > MAX_REPORT_BYTES - self.0 {
                    return Err(std::io::Error::other("report bound"));
                }
                self.0 += bytes.len();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        serde_json::to_writer(&mut Count(0), self).map_err(|_| Error(ErrorCode::Unavailable))
    }
}

fn invalid() -> Error {
    Error(ErrorCode::InvalidInput)
}
fn boot_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

/// Normalize equality and reject obvious non-egress literals. This is not a
/// public-routability classifier or endpoint/provenance check.
pub fn admit_address(address: IpAddr) -> Option<IpAddr> {
    let address = match address {
        IpAddr::V6(value) => value.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(address),
        _ => address,
    };
    if address.is_unspecified()
        || address.is_loopback()
        || address.is_multicast()
        || matches!(address, IpAddr::V4(value) if value.is_broadcast())
    {
        None
    } else {
        Some(address)
    }
}
fn admit_sample(sample: Sample) -> Result<()> {
    if !(0..=MAX_TIME).contains(&sample.sampled_at)
        || matches!(sample.outcome, ProbeOutcome::Address(ip) if admit_address(ip).is_none())
    {
        return Err(invalid());
    }
    Ok(())
}
fn source(now: i64, sample: Option<Sample>, matches: bool) -> SourceView {
    let Some(sample) = sample else {
        return SourceView {
            sampled_at: None,
            state: SourceState::Unavailable,
        };
    };
    let state = if !matches {
        SourceState::ScopeMismatch
    } else if sample.sampled_at > now {
        SourceState::Future
    } else if now - sample.sampled_at > MAX_AGE_SECONDS {
        SourceState::Stale
    } else {
        match sample.outcome {
            ProbeOutcome::Address(_) => SourceState::Fresh,
            ProbeOutcome::Failed => SourceState::Failed,
            ProbeOutcome::MissingTerminalStatus => SourceState::MissingTerminalStatus,
            ProbeOutcome::TimedOut => SourceState::TimedOut,
            ProbeOutcome::Incomplete => SourceState::Incomplete,
            ProbeOutcome::InvalidResponse => SourceState::InvalidResponse,
            ProbeOutcome::ResponseLimit => SourceState::ResponseLimit,
        }
    };
    SourceView {
        sampled_at: Some(sample.sampled_at),
        state,
    }
}

pub fn compare(
    now: i64,
    expected: &Target,
    host: Option<&HostObservation>,
    provider: Option<&ProviderObservation>,
) -> Result<EgressComparison> {
    if !(0..=MAX_TIME).contains(&now) {
        return Err(invalid());
    }
    expected.validate()?;
    if let Some(host) = host {
        if !boot_id(&host.host_boot_id) {
            return Err(invalid());
        }
        admit_sample(host.sample)?;
    }
    if let Some(provider) = provider {
        provider.target.validate()?;
        admit_sample(provider.sample)?;
    }
    let host_view = source(
        now,
        host.map(|o| o.sample),
        host.is_some_and(|o| o.host_boot_id == expected.host_boot_id),
    );
    let provider_view = source(
        now,
        provider.map(|o| o.sample),
        provider.is_some_and(|o| o.target == *expected),
    );
    let mut report = EgressComparison {
        version: 1,
        generated_at: now,
        target: expected.clone(),
        host: host_view,
        provider: provider_view,
        max_age_seconds: MAX_AGE_SECONDS,
        max_sample_skew_seconds: MAX_SAMPLE_SKEW_SECONDS,
        comparison: Comparison::Unknown {
            reason: UnknownReason::UnusableSources,
        },
        addresses: None,
        vpn_evidence: VpnEvidence::NotEstablished,
    };
    if !expected.provider.running {
        report.comparison = Comparison::Unknown {
            reason: UnknownReason::ProviderNotRunning,
        };
    } else if let (Some(host), Some(provider)) = (host, provider)
        && host_view.state == SourceState::Fresh
        && provider_view.state == SourceState::Fresh
    {
        if (host.sample.sampled_at - provider.sample.sampled_at).abs() > MAX_SAMPLE_SKEW_SECONDS {
            report.comparison = Comparison::Unknown {
                reason: UnknownReason::SampleSkew,
            };
        } else if let (ProbeOutcome::Address(host), ProbeOutcome::Address(provider)) =
            (host.sample.outcome, provider.sample.outcome)
        {
            // Both samples were admitted above, including hand-built inputs.
            let host = admit_address(host).ok_or_else(invalid)?;
            let provider = admit_address(provider).ok_or_else(invalid)?;
            if host.is_ipv4() != provider.is_ipv4() {
                report.comparison = Comparison::Unknown {
                    reason: UnknownReason::AddressFamilyMismatch,
                };
            } else {
                report.addresses = Some(AddressPair { host, provider });
                report.comparison = if host == provider {
                    Comparison::SameAddress
                } else {
                    Comparison::DifferentAddresses
                };
            }
        }
    }
    report.check_size()?;
    Ok(report)
}

#[cfg(test)]
mod tests;
