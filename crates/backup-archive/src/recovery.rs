//! Reconstruct a fresh private preparation from exact saved binding evidence.
//! Current policy, selection and workspace identity are trusted caller inputs;
//! no production defaults, destination authority or durable owner is supplied.

use crate::{
    custody::{self, CustodyError, CustodyLimits, CustodyRoot},
    preparation::{
        self, PreparationError, PreparationLimits, PreparationRoot, PreparedRestore, Selection,
    },
    staging::{StagingError, StagingRoot, VerifiedCatalog},
};
use limeos_domain::{BackupPreparationBinding, backups::AdmissionPolicy};
use std::{error::Error, fmt};

#[cfg(test)]
mod tests;

/// Independently trusted current facts, separate from saved expected evidence.
/// Selection moves into the genuine plan. Current policy validation does not
/// establish aggregate allocation admission or protected filesystem ancestry.
#[derive(Debug)]
pub struct CurrentInputs<'a> {
    pub custody_root: &'a CustodyRoot,
    pub staging_root: &'a StagingRoot,
    pub preparation_root: &'a PreparationRoot,
    pub admission_policy: &'a AdmissionPolicy,
    pub custody_limits: &'a CustodyLimits,
    pub selection: Selection,
    pub preparation_limits: PreparationLimits,
    /// Supplied from current trusted configuration; never copied from expected.
    pub workspace_policy_identity: &'a str,
}

#[derive(Debug)]
pub enum Failure {
    InvalidExpectedBinding,
    InvalidCurrentWorkspaceIdentity,
    BindingMismatch,
    Cancelled,
    Custody(Box<CustodyError>),
    Plan(Box<PreparationError>),
    Preparation(Box<PreparationError>),
    CatalogDiscard(Box<StagingError>),
}

/// The primary refusal and independently owned cleanup failures all survive.
/// Existing nested library errors retain their own cleanup reports unchanged.
#[derive(Debug)]
pub struct RecoveryError {
    pub failure: Failure,
    pub catalog_cleanup: Option<Box<StagingError>>,
    pub prepared_cleanup: Option<Box<PreparationError>>,
}
impl RecoveryError {
    fn new(failure: Failure) -> Self {
        Self {
            failure,
            catalog_cleanup: None,
            prepared_cleanup: None,
        }
    }
}
impl fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "bound private reconstruction refused: {:?}",
            self.failure
        )?;
        if let Some(error) = &self.catalog_cleanup {
            write!(f, "; catalog cleanup: {error}")?;
        }
        if let Some(error) = &self.prepared_cleanup {
            write!(f, "; prepared cleanup: {error}")?;
        }
        Ok(())
    }
}
impl Error for RecoveryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.failure {
            Failure::Custody(error) => Some(error.as_ref()),
            Failure::Plan(error) | Failure::Preparation(error) => Some(error.as_ref()),
            Failure::CatalogDiscard(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}

fn refuse_catalog(failure: Failure, catalog: VerifiedCatalog) -> RecoveryError {
    let mut error = RecoveryError::new(failure);
    error.catalog_cleanup = catalog.discard().err().map(Box::new);
    error
}

/// Consume genuine owners; catalog cleanup ambiguity never returns success.
fn finish(
    prepared: PreparedRestore,
    catalog: VerifiedCatalog,
    cancelled: &dyn Fn() -> bool,
) -> Result<PreparedRestore, RecoveryError> {
    if cancelled() {
        let mut error = refuse_catalog(Failure::Cancelled, catalog);
        error.prepared_cleanup = prepared.discard().err().map(Box::new);
        return Err(error);
    }
    if let Err(source) = catalog.discard() {
        let mut error = RecoveryError::new(Failure::CatalogDiscard(Box::new(source)));
        error.prepared_cleanup = prepared.discard().err().map(Box::new);
        return Err(error);
    }
    if cancelled() {
        let mut error = RecoveryError::new(Failure::Cancelled);
        error.prepared_cleanup = prepared.discard().err().map(Box::new);
        return Err(error);
    }
    Ok(prepared)
}

/// Re-admit actual custody, derive and compare the complete current binding,
/// then create fresh private replacements. Saved metadata authorizes no path.
/// Custody stays retained; returned owners are ephemeral and explicitly discarded.
pub fn reconstruct(
    expected: &BackupPreparationBinding,
    current: CurrentInputs<'_>,
    cancelled: &dyn Fn() -> bool,
) -> Result<PreparedRestore, RecoveryError> {
    expected
        .validate()
        .map_err(|_| RecoveryError::new(Failure::InvalidExpectedBinding))?;
    if current.workspace_policy_identity.len() != 64
        || !current
            .workspace_policy_identity
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(RecoveryError::new(Failure::InvalidCurrentWorkspaceIdentity));
    }
    if cancelled() {
        return Err(RecoveryError::new(Failure::Cancelled));
    }
    current.admission_policy.validate().map_err(|error| {
        RecoveryError::new(Failure::Custody(Box::new(CustodyError::Admission(error))))
    })?;
    // Existing recover consumes policy, while stage needs the same current
    // policy borrowed. It remains trusted caller configuration; its validation
    // does not promise bounded aggregate cloning or global OOM survival.
    let mut retained = custody::recover(
        current.custody_root,
        &expected.custody,
        current.admission_policy.clone(),
        current.custody_limits,
        cancelled,
    )
    .map_err(|error| RecoveryError::new(Failure::Custody(Box::new(error))))?;
    let catalog = retained
        .stage(current.admission_policy, current.staging_root, cancelled)
        .map_err(|error| RecoveryError::new(Failure::Custody(Box::new(error))))?;
    if cancelled() {
        return Err(refuse_catalog(Failure::Cancelled, catalog));
    }
    let plan = match preparation::plan(
        &catalog,
        catalog.manifest().clone(),
        current.selection,
        current.preparation_limits,
    ) {
        Ok(plan) => plan,
        Err(error) => return Err(refuse_catalog(Failure::Plan(Box::new(error)), catalog)),
    };
    if cancelled() {
        return Err(refuse_catalog(Failure::Cancelled, catalog));
    }
    if retained.binding() != &expected.custody
        || plan.identity() != expected.preparation_identity
        || plan
            .selection()
            .resources
            .iter()
            .map(|r| r.id.as_str())
            .ne(expected.selected_resources.iter().map(String::as_str))
        || current.workspace_policy_identity != expected.workspace_policy_identity
    {
        return Err(refuse_catalog(Failure::BindingMismatch, catalog));
    }
    if cancelled() {
        return Err(refuse_catalog(Failure::Cancelled, catalog));
    }
    let prepared = match preparation::prepare(
        &catalog,
        &plan,
        plan.selection(),
        current.preparation_root,
        plan.limits(),
        cancelled,
    ) {
        Ok(prepared) => prepared,
        Err(error) => {
            return Err(refuse_catalog(
                Failure::Preparation(Box::new(error)),
                catalog,
            ));
        }
    };
    finish(prepared, catalog, cancelled)
}
