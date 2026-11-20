//! The receipt-store fields of the executor's response to a receipt lookup
//! (`RestartReceipt`, `StackReceipt`) or a `ResolveAction`, as pure functions
//! of what the store answered.
//!
//! These fields decide what core does with a held job: a receipt with
//! `closed` releases it as a refusal, an absence keeps it held (and `closed`
//! says whether an execute can still arrive), a confirmation completes an
//! operator resolution. A field filled from the wrong case would release a
//! job that may have run, or hold one forever, so the mapping lives here,
//! tested case by case, and the handlers only copy it out.
use super::*;
use crate::closure::Lookup;
use crate::resolution::ResolveOutcome;
use limeos_domain::{ReceiptAbsence, ResolveReceipt, ResolveRefusal};

/// The response fields a receipt-store answer sets. Each field has the name
/// and meaning of the executor protocol's `Receipt` field it fills; the ones
/// not listed are never set from a lookup or resolution.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answer {
    pub restart: Option<RestartReceipt>,
    pub stack: Option<StackReceipt>,
    pub resolution: Option<ResolveReceipt>,
    pub resolution_refusal: Option<ResolveRefusal>,
    pub absence: Option<ReceiptAbsence>,
    pub closed: bool,
}

impl Answer {
    /// A `RestartReceipt` lookup: the receipt (an execute's, or a
    /// never-recorded closure's with `closed`) or the absence.
    pub fn restart_lookup(lookup: Lookup<RestartReceipt>) -> Self {
        match lookup {
            Lookup::Receipt { receipt, closed } => Self {
                restart: Some(receipt),
                closed,
                ..Self::default()
            },
            Lookup::Absent { absence, closed } => Self {
                absence: Some(absence),
                closed,
                ..Self::default()
            },
        }
    }

    /// A `StackReceipt` lookup, as `restart_lookup`.
    pub fn stack_lookup(lookup: Lookup<StackReceipt>) -> Self {
        match lookup {
            Lookup::Receipt { receipt, closed } => Self {
                stack: Some(receipt),
                closed,
                ..Self::default()
            },
            Lookup::Absent { absence, closed } => Self {
                absence: Some(absence),
                closed,
                ..Self::default()
            },
        }
    }

    /// A `ResolveAction`. An absence is always closed: the store closes the
    /// action before it answers one.
    pub fn resolve(outcome: ResolveOutcome) -> Self {
        match outcome {
            ResolveOutcome::Resolved(confirmation) => Self {
                resolution: Some(confirmation),
                ..Self::default()
            },
            ResolveOutcome::Absent(absence) => Self {
                absence: Some(absence),
                closed: true,
                ..Self::default()
            },
            ResolveOutcome::Refused(refusal) => Self {
                resolution_refusal: Some(refusal),
                ..Self::default()
            },
        }
    }
}

#[cfg(test)]
mod tests;
