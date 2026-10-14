//! Supported source bytes bound to genuine retained management/file/Engine owners.
use super::*;
use crate::compose_source::{ComposeJsonSourceSummary, ComposeJsonSources};
const REPORT_BYTES: usize = 256 * 1024;
pub mod lifecycle;

/// Original metadata and parsed source facts cannot reconstruct the retained owner.
/// ```compile_fail
/// use limeos_executor_container::compose_management::sources::BoundComposeJsonSnapshot;
/// let imported: BoundComposeJsonSnapshot<'_> = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Serialize)]
pub struct BoundComposeJsonSnapshot<'a> {
    pub version: u16,
    pub binding: ComposeBindingSnapshot<'a>,
    pub sources: ComposeJsonSourceSummary<'a>,
}
/// Parsed source data inherits actual retained owners, never execution authority.
/// ```compile_fail
/// use limeos_executor_container::compose_management::sources::BoundComposeJsonEvidence;
/// let imported: BoundComposeJsonEvidence = serde_json::from_str("{}").unwrap();
/// ```
pub struct BoundComposeJsonEvidence {
    binding: ComposeBindingEvidence,
    sources: ComposeJsonSources,
    state: AtomicU8,
}
impl BoundComposeJsonEvidence {
    pub fn snapshot(&self) -> BoundComposeJsonSnapshot<'_> {
        BoundComposeJsonSnapshot {
            version: 1,
            binding: self.binding.snapshot(),
            sources: self.sources.summary(),
        }
    }
    pub fn retained_descriptor_count(&self) -> usize {
        self.binding.retained_descriptor_count()
    }
    pub async fn revalidate(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        self.validate_with(
            cancelled,
            || self.binding.catalog.revalidate(cancelled),
            || self.binding.files.revalidate(cancelled),
            || self.binding.files.admit_json_sources(cancelled),
        )
        .await
    }
    async fn validate_with(
        &self,
        cancelled: &dyn Fn() -> bool,
        catalog: impl Fn() -> Result<()>,
        files: impl Fn() -> Result<()>,
        admit: impl Fn() -> Result<ComposeJsonSources>,
    ) -> Result<()> {
        if self
            .state
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            self.state.store(2, Ordering::SeqCst);
            return Err(conflict());
        }
        let guard = Validation {
            state: &self.state,
            completed: false,
        };
        let result = tokio::time::timeout_at(
            tokio::time::Instant::from_std(self.binding.window.expires),
            async {
                self.binding.window.poll(cancelled)?;
                self.binding
                    .validate_with(cancelled, &catalog, &files)
                    .await?;
                self.binding.window.poll(cancelled)?;
                let fresh = admit()?;
                self.binding.window.poll(cancelled)?;
                same_sources(&self.sources.summary(), &fresh.summary())?;
                matched_sources(&self.binding.snapshot(), &fresh.summary())?;
                self.binding
                    .validate_with(cancelled, &catalog, &files)
                    .await?;
                self.binding.window.poll(cancelled)?;
                matched_sources(&self.binding.snapshot(), &self.sources.summary())?;
                check_source_report(&self.snapshot(), REPORT_BYTES)
            },
        )
        .await
        .map_err(|_| Error(ErrorCode::Expired))
        .and_then(|result| result);
        guard.finish(result)
    }
}
fn same_sources(
    before: &ComposeJsonSourceSummary<'_>,
    after: &ComposeJsonSourceSummary<'_>,
) -> Result<()> {
    let fingerprint = |a: &crate::compose_source::ComposeSourceFingerprint,
                       b: &crate::compose_source::ComposeSourceFingerprint| {
        a.bytes == b.bytes && a.sha256 == b.sha256
    };
    if before.version != after.version
        || before.format != after.format
        || before.operator != after.operator
        || before.current != after.current
        || before.declared_volumes != after.declared_volumes
        || !fingerprint(before.operator_fingerprint, after.operator_fingerprint)
        || match (before.managed_fingerprint, after.managed_fingerprint) {
            (None, None) => false,
            (Some(a), Some(b)) => !fingerprint(a, b),
            _ => true,
        }
    {
        return Err(conflict());
    }
    Ok(())
}
fn matched_sources(
    binding: &ComposeBindingSnapshot<'_>,
    sources: &ComposeJsonSourceSummary<'_>,
) -> Result<()> {
    let files = &binding.files.files;
    if sources.version != 1
        || sources.format != "strict_json"
        || files.len() != 1 + usize::from(sources.managed_fingerprint.is_some())
        || files.first().is_none_or(|file| {
            file.identity.size != sources.operator_fingerprint.bytes as u64
                || file.sha256 != sources.operator_fingerprint.sha256
        })
        || sources.managed_fingerprint.is_some_and(|fp| {
            files.get(1).is_none_or(|file| {
                file.identity.size != fp.bytes as u64 || file.sha256 != fp.sha256
            })
        })
        || sources
            .current
            .services
            .iter()
            .any(|service| !binding.registration.services.contains(&service.name))
    {
        return Err(conflict());
    }
    for index in binding.member_indices {
        let Some(member) = binding.engine.containers.get(*index) else {
            return Err(conflict());
        };
        let ComposeOwnership::Compose { service, .. } = &member.ownership else {
            return Err(conflict());
        };
        if !sources
            .current
            .services
            .iter()
            .any(|declared| declared.name == *service)
        {
            return Err(conflict());
        }
    }
    Ok(())
}
fn check_source_report(snapshot: &BoundComposeJsonSnapshot<'_>, max: usize) -> Result<()> {
    serde_json::to_writer(&mut Count { bytes: 0, max }, snapshot).map_err(|_| unavailable())
}
fn assemble_sources(
    binding: ComposeBindingEvidence,
    sources: ComposeJsonSources,
) -> Result<BoundComposeJsonEvidence> {
    binding.window.poll(&|| false)?;
    matched_sources(&binding.snapshot(), &sources.summary())?;
    let owner = BoundComposeJsonEvidence {
        binding,
        sources,
        state: AtomicU8::new(0),
    };
    if owner.retained_descriptor_count() > RETAINED_FDS {
        return Err(unavailable());
    }
    check_source_report(&owner.snapshot(), REPORT_BYTES)?;
    Ok(owner)
}
/// Fixed-policy read-only composition; callers select no body, digest or child policy.
pub async fn inspect_bound_compose_json(
    catalog_path: &Path,
    stack: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<BoundComposeJsonEvidence> {
    if !rustix::process::geteuid().is_root() {
        return Err(Error(ErrorCode::Forbidden));
    }
    let binding = inspect_compose_binding(catalog_path, stack, cancelled).await?;
    binding.window.poll(cancelled)?;
    let sources = binding.files.admit_json_sources(cancelled)?;
    binding.window.poll(cancelled)?;
    let owner = assemble_sources(binding, sources)?;
    owner.revalidate(cancelled).await?;
    Ok(owner)
}

#[cfg(test)]
pub(crate) mod tests;
