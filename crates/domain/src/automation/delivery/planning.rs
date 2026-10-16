use super::*;
use crate::automation::AlertEventKind;

/// Pure decisions for B1's in-app outbox. These are neither Store receipts nor
/// permission to contact an external receiver. Store supplies owner-scoped rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryDecision {
    Complete,
    ClockRollback,
    WaitingRetry,
    InFlight,
    Blocked,
    NeedsReconciliation,
    AdapterUnavailable,
    OpeningMissing,
    OpeningPending,
    OpeningBlocked,
    OpeningAmbiguous,
    PublishInApp,
}

/// Store must load the exact opening and apply this decision under its existing
/// transaction with current authority and both row revisions. Missing rows are
/// incomplete evidence; acknowledgment alone never proves receiver delivery.
pub fn plan_in_app_delivery(
    notification: &Notification,
    opening: Option<&Notification>,
    now: i64,
) -> Result<DeliveryDecision> {
    notification.validate()?;
    if !valid_time(now) {
        return Err(Error(ErrorCode::InvalidInput));
    }
    if let Some(incident) = opening {
        incident.validate()?;
        if notification.event.event != AlertEventKind::Recovery
            || incident.event.event != AlertEventKind::Incident
            || incident.id == notification.id
            || incident.event.key != notification.event.key
            || incident.event.kind != notification.event.kind
            || incident.event.generation != notification.event.generation
            || incident.channel != notification.channel
            || incident.event.at > notification.event.at
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
    }
    if now < notification.event.at {
        return Ok(DeliveryDecision::ClockRollback);
    }
    match notification.delivery {
        DeliveryState::Delivered { at } => {
            return Ok(if now < at {
                DeliveryDecision::ClockRollback
            } else {
                DeliveryDecision::Complete
            });
        }
        DeliveryState::InFlight { started_at } => {
            return Ok(if now < started_at {
                DeliveryDecision::ClockRollback
            } else {
                DeliveryDecision::InFlight
            });
        }
        DeliveryState::Ambiguous { .. } => return Ok(DeliveryDecision::NeedsReconciliation),
        DeliveryState::Blocked | DeliveryState::Failed { retry_at: None } => {
            return Ok(DeliveryDecision::Blocked);
        }
        _ => {}
    }
    if notification.channel != DeliveryChannel::InApp {
        return Ok(DeliveryDecision::AdapterUnavailable);
    }
    if !notification.retry_due(now)? {
        return Ok(DeliveryDecision::WaitingRetry);
    }
    if notification.event.event == AlertEventKind::Recovery {
        let Some(incident) = opening else {
            return Ok(DeliveryDecision::OpeningMissing);
        };
        match incident.delivery {
            DeliveryState::Delivered { at } if now >= at => {}
            DeliveryState::Delivered { .. } => return Ok(DeliveryDecision::ClockRollback),
            DeliveryState::Ambiguous { .. } => return Ok(DeliveryDecision::OpeningAmbiguous),
            DeliveryState::Blocked | DeliveryState::Failed { retry_at: None } => {
                return Ok(DeliveryDecision::OpeningBlocked);
            }
            _ => return Ok(DeliveryDecision::OpeningPending),
        }
    }
    Ok(DeliveryDecision::PublishInApp)
}

#[cfg(test)]
mod tests;
