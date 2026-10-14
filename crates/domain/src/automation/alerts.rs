use super::{MAX_REVISION, text, valid_time};
use crate::{Error, ErrorCode, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertKind {
    Container,
    Smart,
    Mount,
    Snapraid,
    Host,
}
impl AlertKind {
    fn prefix(self) -> &'static str {
        match self {
            Self::Container => "container:",
            Self::Smart => "smart:",
            Self::Mount => "mount:",
            Self::Snapraid => "snapraid:",
            Self::Host => "host:",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Warning,
    Critical,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalStatus {
    Healthy,
    Failing,
    Unavailable,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlertSignal {
    pub key: String,
    pub kind: AlertKind,
    pub status: SignalStatus,
    pub severity: Severity,
    pub summary: String,
}
impl AlertSignal {
    pub fn validate(&self) -> Result<()> {
        if !text(&self.key, 256)
            || !self.key.starts_with(self.kind.prefix())
            || self.key.len() == self.kind.prefix().len()
            || !text(&self.summary, 512)
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Silence {
    pub kind: AlertKind,
    pub key: String,
    pub expires_at: Option<i64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlertPolicy {
    pub fail_threshold: u8,
    pub disabled: Vec<AlertKind>,
    pub silences: Vec<Silence>,
}
impl Default for AlertPolicy {
    fn default() -> Self {
        Self {
            fail_threshold: 2,
            disabled: vec![],
            silences: vec![],
        }
    }
}
impl AlertPolicy {
    pub fn validate(&self) -> Result<()> {
        if !(1..=16).contains(&self.fail_threshold)
            || self.disabled.len() > 5
            || self.silences.len() > 64
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        for (i, kind) in self.disabled.iter().enumerate() {
            if self.disabled[..i].contains(kind) {
                return Err(Error(ErrorCode::InvalidInput));
            }
        }
        for silence in &self.silences {
            if !text(&silence.key, 256)
                || !silence.key.starts_with(silence.kind.prefix())
                || silence.key.len() == silence.kind.prefix().len()
                || silence.expires_at.is_some_and(|at| !valid_time(at))
            {
                return Err(Error(ErrorCode::InvalidInput));
            }
        }
        Ok(())
    }
    pub fn allows(&self, signal: &AlertSignal, now: i64) -> bool {
        !self.disabled.contains(&signal.kind)
            && !self.silences.iter().any(|silence| {
                silence.kind == signal.kind
                    && silence.key == signal.key
                    && silence.expires_at.is_none_or(|at| now < at)
            })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub enum IncidentState {
    Clear {
        failures: u8,
    },
    Open {
        generation: i64,
        severity: Severity,
        summary: String,
        announced: bool,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlertState {
    pub key: String,
    pub kind: AlertKind,
    pub last_sequence: i64,
    pub incident: IncidentState,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertEventKind {
    Incident,
    Recovery,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlertEvent {
    pub key: String,
    pub generation: i64,
    pub event: AlertEventKind,
    pub kind: AlertKind,
    pub severity: Severity,
    pub summary: String,
    pub at: i64,
}
impl AlertEvent {
    /// Store's unique event tuple is (key, generation, event); never use summary text.
    pub fn validate(&self) -> Result<()> {
        if !text(&self.key, 256)
            || !self.key.starts_with(self.kind.prefix())
            || self.key.len() == self.kind.prefix().len()
            || !text(&self.summary, 512)
            || !(1..=MAX_REVISION).contains(&self.generation)
            || !valid_time(self.at)
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        Ok(())
    }
}
pub struct AlertFold {
    pub next: AlertState,
    /// Persist the material change with the event/outbox atomically. Store separately
    /// consumes each observation sequence even when this is false.
    pub changed: bool,
    pub event: Option<AlertEvent>,
}
impl AlertState {
    pub fn validate(&self) -> Result<()> {
        if !text(&self.key, 256)
            || !self.key.starts_with(self.kind.prefix())
            || self.key.len() == self.kind.prefix().len()
            || !(0..=MAX_REVISION).contains(&self.last_sequence)
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        match &self.incident {
            IncidentState::Clear { failures } if *failures > 16 => {
                return Err(Error(ErrorCode::StateNotDurable));
            }
            IncidentState::Open {
                generation,
                summary,
                ..
            } if *generation < 1 || *generation > self.last_sequence || !text(summary, 512) => {
                return Err(Error(ErrorCode::StateNotDurable));
            }
            _ => {}
        }
        Ok(())
    }
}

/// An absent key is not submitted here. Unavailable input leaves its incident/streak intact.
pub fn fold_alert(
    state: Option<&AlertState>,
    signal: &AlertSignal,
    policy: &AlertPolicy,
    sequence: i64,
    now: i64,
) -> Result<AlertFold> {
    signal.validate()?;
    policy.validate()?;
    if !(1..=MAX_REVISION).contains(&sequence) || !valid_time(now) {
        return Err(Error(ErrorCode::InvalidInput));
    }
    if let Some(state) = state {
        state.validate()?;
    }
    let mut next = state.cloned().unwrap_or_else(|| AlertState {
        key: signal.key.clone(),
        kind: signal.kind,
        last_sequence: 0,
        incident: IncidentState::Clear { failures: 0 },
    });
    if next.key != signal.key || next.kind != signal.kind || next.last_sequence < 0 {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    if sequence <= next.last_sequence {
        return Ok(AlertFold {
            next,
            changed: false,
            event: None,
        });
    }
    let previous = next.incident.clone();
    next.last_sequence = sequence;
    let allows = policy.allows(signal, now);
    let mut event = None;
    let make_event = |generation, event, severity| AlertEvent {
        key: signal.key.clone(),
        generation,
        event,
        kind: signal.kind,
        severity,
        summary: signal.summary.clone(),
        at: now,
    };
    match (&previous, signal.status) {
        (_, SignalStatus::Unavailable) => {}
        (IncidentState::Clear { .. }, SignalStatus::Healthy) => {
            next.incident = IncidentState::Clear { failures: 0 }
        }
        (IncidentState::Clear { failures }, SignalStatus::Failing) => {
            let count = failures.saturating_add(1);
            if count >= policy.fail_threshold {
                next.incident = IncidentState::Open {
                    generation: sequence,
                    severity: signal.severity,
                    summary: signal.summary.clone(),
                    announced: allows,
                };
                if allows {
                    event = Some(make_event(
                        sequence,
                        AlertEventKind::Incident,
                        signal.severity,
                    ));
                }
            } else {
                next.incident = IncidentState::Clear { failures: count };
            }
        }
        (
            IncidentState::Open {
                generation,
                severity,
                announced,
                ..
            },
            SignalStatus::Healthy,
        ) => {
            next.incident = IncidentState::Clear { failures: 0 };
            if *announced && allows {
                event = Some(make_event(*generation, AlertEventKind::Recovery, *severity));
            }
        }
        (
            IncidentState::Open {
                generation,
                announced,
                ..
            },
            SignalStatus::Failing,
        ) => {
            let announce = allows && !announced;
            next.incident = IncidentState::Open {
                generation: *generation,
                severity: signal.severity,
                summary: signal.summary.clone(),
                announced: *announced || announce,
            };
            if announce {
                event = Some(make_event(
                    *generation,
                    AlertEventKind::Incident,
                    signal.severity,
                ));
            }
        }
    }
    let changed = next.incident != previous;
    Ok(AlertFold {
        next,
        changed,
        event,
    })
}
