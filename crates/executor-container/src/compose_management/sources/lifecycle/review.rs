//! P03 preview facts associated with actual retained proposals, never executable approval.
use super::*;
use crate::docker::process_evidence::{EngineReviewFacts, EngineStorageProcessSnapshot};
use limeos_domain::{Operation, PlannedCompose, Scope, opaque_id};
use std::io::Write;
const PREVIEW_BYTES: usize = 48 * 1024;

/// Required scopes and a review digest do not authenticate a principal or approve effects.
/// ```compile_fail
/// use limeos_executor_container::compose_management::sources::lifecycle::review::ComposeReviewSummary;
/// let imported: ComposeReviewSummary<'_> = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Serialize)]
pub struct ComposeReviewSummary<'a> {
    pub version: u16,
    pub preview: &'a PlannedCompose,
    pub lifecycle: ComposeLifecycleSummary<'a>,
    pub engine_context: &'a EngineStorageProcessSnapshot,
    pub required_scopes: &'a [Scope],
    pub review_digest: &'a str,
}
/// The private original proposal cannot be reconstructed from a preview approval.
/// ```compile_fail
/// use limeos_executor_container::compose_management::sources::lifecycle::review::BoundComposeReviewSubject;
/// let imported: BoundComposeReviewSubject = serde_json::from_str("{}").unwrap();
/// ```
pub struct BoundComposeReviewSubject {
    proposal: BoundComposeLifecycleProposal,
    preview: PlannedCompose,
    scopes: Vec<Scope>,
    digest: String,
    state: AtomicU8,
}
impl BoundComposeReviewSubject {
    pub fn summary(&self) -> ComposeReviewSummary<'_> {
        ComposeReviewSummary {
            version: 1,
            preview: &self.preview,
            lifecycle: self.proposal.summary(),
            engine_context: self.proposal.evidence.binding.engine.storage().snapshot(),
            required_scopes: &self.scopes,
            review_digest: &self.digest,
        }
    }
    pub fn artifact(&self) -> &[u8] {
        self.proposal.artifact()
    }
    pub fn retained_descriptor_count(&self) -> usize {
        self.proposal.retained_descriptor_count()
    }
    pub async fn revalidate(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        self.check_current(&self.preview, &self.digest, cancelled)
            .await
    }
    /// Compare ordinary current-preview input; the composition root must authenticate it.
    pub async fn check_current(
        &self,
        current: &PlannedCompose,
        review_digest: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<()> {
        self.validate_with(
            current,
            review_digest,
            cancelled,
            self.proposal.revalidate(cancelled),
        )
        .await
    }
    async fn validate_with(
        &self,
        current: &PlannedCompose,
        review_digest: &str,
        cancelled: &dyn Fn() -> bool,
        child_check: impl Future<Output = Result<()>>,
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
        let window = &self.proposal.evidence.binding.window;
        let result =
            tokio::time::timeout_at(tokio::time::Instant::from_std(window.expires), async {
                window.poll(cancelled)?;
                self.preview.plan.validate(compose_files::now()?)?;
                if current != &self.preview
                    || !opaque_id(review_digest)
                    || !limeos_identity::constant_eq(review_digest, &self.digest)
                {
                    return Err(conflict());
                }
                child_check.await?;
                window.poll(cancelled)?;
                if self.digest != material_digest(&self.proposal, &self.preview, &self.scopes)? {
                    return Err(conflict());
                }
                counted_report(&self.summary(), REPORT_BYTES)?;
                self.preview.plan.validate(compose_files::now()?)?;
                window.poll(cancelled)
            })
            .await
            .map_err(|_| Error(ErrorCode::Expired))
            .and_then(|result| result);
        guard.finish(result)
    }
}
impl BoundComposeLifecycleProposal {
    /// Ordinary preview input gains a distinct bound review identity, without permission.
    pub async fn bind_review(
        self,
        preview: &PlannedCompose,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<BoundComposeReviewSubject> {
        let subject = assemble_review(self, preview, cancelled)?;
        subject.revalidate(cancelled).await?;
        Ok(subject)
    }
}
fn assemble_review(
    proposal: BoundComposeLifecycleProposal,
    preview: &PlannedCompose,
    cancelled: &dyn Fn() -> bool,
) -> Result<BoundComposeReviewSubject> {
    if proposal.state.load(Ordering::SeqCst) != 0 {
        return Err(conflict());
    }
    proposal.evidence.binding.window.poll(cancelled)?;
    matched_preview(&proposal, preview)?;
    let mut scopes = vec![Scope {
        operation: Operation::DeploymentManage,
        resource: proposal.evidence.binding.stack_resource.clone(),
    }];
    if !proposal.plan.summary().desired.elevated().is_empty() {
        scopes.push(Scope {
            operation: Operation::DeploymentElevated,
            resource: format!(
                "stack:{}:template:{}:{}",
                preview.plan.selection.stack,
                preview.plan.selection.template,
                preview.plan.template_digest
            ),
        });
    }
    let digest = material_digest(&proposal, preview, &scopes)?;
    proposal.evidence.binding.window.poll(cancelled)?;
    let subject = BoundComposeReviewSubject {
        proposal,
        preview: preview.clone(),
        scopes,
        digest,
        state: AtomicU8::new(0),
    };
    counted_report(&subject.summary(), REPORT_BYTES)?;
    subject.proposal.evidence.binding.window.poll(cancelled)?;
    Ok(subject)
}
fn matched_preview(
    proposal: &BoundComposeLifecycleProposal,
    preview: &PlannedCompose,
) -> Result<()> {
    let plan = &preview.plan;
    plan.validate(compose_files::now()?)?;
    let binding = proposal.evidence.binding.snapshot();
    let compiled = proposal.plan.summary();
    // Compare actual bounded models before serializing or cloning ordinary caller input.
    if !opaque_id(&preview.digest)
        || plan.selection.stack != binding.registration.stack
        || plan.operator_files.len() != 1
        || plan.operator_files[0].name != binding.registration.operator.name
        || plan.operator_files[0].sha256 != binding.files.files[0].sha256
        || &plan.before != compiled.before
        || &plan.desired != compiled.desired
        || &plan.impact != compiled.impact
    {
        return Err(conflict());
    }
    let canonical =
        bounded_digest(plan, PREVIEW_BYTES).map_err(|_| Error(ErrorCode::InvalidInput))?;
    if !limeos_identity::constant_eq(&canonical, &preview.digest)
        || plan.template_digest != bounded_digest(compiled.desired, PREVIEW_BYTES)?
    {
        return Err(conflict());
    }
    Ok(())
}

#[derive(Serialize)]
struct CatalogFacts<'a> {
    version: u16,
    path: &'a str,
    mount_view: &'a Option<compose_files::ComposeMountView>,
    root: &'a compose_files::ComposeObjectIdentity,
    ancestors: &'a [compose_files::ComposeObjectIdentity],
    identity: &'a compose_files::ComposeFileIdentity,
    sha256: &'a str,
}
#[derive(Serialize)]
struct FileFacts<'a> {
    version: u16,
    selection: &'a compose_files::ComposeFileSelection,
    mount_view: &'a Option<compose_files::ComposeMountView>,
    root: &'a compose_files::ComposeObjectIdentity,
    ancestors: &'a [compose_files::ComposeObjectIdentity],
    files: &'a [compose_files::ComposeFileObservation],
    absent_names: &'a [String],
}
#[derive(Serialize)]
struct ReviewMaterial<'a> {
    domain: &'static str,
    preview: &'a PlannedCompose,
    scopes: &'a [Scope],
    lifecycle_version: u16,
    source_version: u16,
    binding_version: u16,
    stack_resource: &'a str,
    registration: &'a catalog::StackRegistration,
    catalog: CatalogFacts<'a>,
    files: FileFacts<'a>,
    engine_version: u16,
    engine_id: &'a str,
    engine_members: &'a [crate::docker::compose_evidence::ComposeContainerMembership],
    engine_context: EngineReviewFacts<'a>,
    member_indices: &'a [usize],
    sources: ComposeJsonSourceSummary<'a>,
    proposal: ManagedOverrideSummary<'a>,
    artifact_sha256: &'a str,
    changes: &'a [ComposeLifecycleChange],
    unresolved: &'a [UnresolvedLifecycleGate],
}
fn material<'a>(
    proposal: &'a BoundComposeLifecycleProposal,
    preview: &'a PlannedCompose,
    scopes: &'a [Scope],
) -> ReviewMaterial<'a> {
    let s = proposal.summary();
    let b = s.evidence.binding;
    let c = b.catalog;
    let f = b.files;
    ReviewMaterial {
        domain: "limeos.compose.review.v1",
        preview,
        scopes,
        lifecycle_version: s.version,
        source_version: s.evidence.version,
        binding_version: b.version,
        stack_resource: b.stack_resource,
        registration: b.registration,
        catalog: CatalogFacts {
            version: c.version,
            path: &c.path,
            mount_view: &c.mount_view,
            root: &c.root,
            ancestors: &c.ancestors,
            identity: &c.identity,
            sha256: &c.sha256,
        },
        files: FileFacts {
            version: f.version,
            selection: &f.selection,
            mount_view: &f.mount_view,
            root: &f.root,
            ancestors: &f.ancestors,
            files: &f.files,
            absent_names: &f.absent_names,
        },
        engine_version: b.engine.version,
        engine_id: &b.engine.engine_id,
        engine_members: &b.engine.containers,
        engine_context: proposal.evidence.binding.engine.storage().review_facts(),
        member_indices: b.member_indices,
        sources: s.evidence.sources,
        proposal: s.proposal,
        artifact_sha256: s.artifact_sha256,
        changes: s.changes,
        unresolved: s.unresolved,
    }
}
fn material_digest(
    proposal: &BoundComposeLifecycleProposal,
    preview: &PlannedCompose,
    scopes: &[Scope],
) -> Result<String> {
    bounded_digest(&material(proposal, preview, scopes), REPORT_BYTES)
}
struct DigestWriter {
    hash: Sha256,
    bytes: usize,
    max: usize,
}
impl Write for DigestWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.max.saturating_sub(self.bytes) {
            return Err(std::io::Error::other(
                "Compose review material exceeds its bound",
            ));
        }
        self.hash.update(bytes);
        self.bytes += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn bounded_digest(value: &impl Serialize, max: usize) -> Result<String> {
    let mut writer = DigestWriter {
        hash: Sha256::new(),
        bytes: 0,
        max,
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| unavailable())?;
    Ok(hex::encode(writer.hash.finalize()))
}
fn counted_report(summary: &ComposeReviewSummary<'_>, max: usize) -> Result<()> {
    serde_json::to_writer(&mut Count { bytes: 0, max }, summary).map_err(|_| unavailable())
}

#[cfg(test)]
pub(crate) mod tests;
