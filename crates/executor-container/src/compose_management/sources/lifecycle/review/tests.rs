use super::*;
use crate::compose_management::sources::lifecycle::tests::{LocalProposal, source_check};
use tempfile::TempDir;

pub(crate) struct LocalReview {
    pub(crate) owner: BoundComposeReviewSubject,
    pub(crate) tree: TempDir,
}
pub(crate) fn bind(proposal: LocalProposal, preview: &PlannedCompose) -> Result<LocalReview> {
    let LocalProposal { owner, tree } = proposal;
    Ok(LocalReview {
        owner: assemble_review(owner, preview, &|| false)?,
        tree,
    })
}
pub(crate) fn count(summary: &ComposeReviewSummary<'_>, max: usize) -> Result<()> {
    counted_report(summary, max)
}
pub(crate) fn hash_material(owner: &BoundComposeReviewSubject, max: usize) -> Result<String> {
    bounded_digest(
        &material(&owner.proposal, &owner.preview, &owner.scopes),
        max,
    )
}
pub(crate) fn material_value(owner: &BoundComposeReviewSubject) -> serde_json::Value {
    serde_json::to_value(material(&owner.proposal, &owner.preview, &owner.scopes)).unwrap()
}
impl LocalReview {
    pub(crate) async fn expire_preview_during_child(&self) -> Result<()> {
        self.owner
            .validate_with(&self.owner.preview, &self.owner.digest, &|| false, async {
                while compose_files::now()? < self.owner.preview.plan.expires_at {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                self.check_proposal_only(&|| false).await
            })
            .await
    }
    pub(crate) async fn check_proposal_only(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        let p = &self.owner.proposal;
        p.validate_with(cancelled, source_check(p, self.tree.path(), cancelled))
            .await
    }
    pub(crate) async fn check_current(
        &self,
        preview: &PlannedCompose,
        digest: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<()> {
        self.owner
            .validate_with(
                preview,
                digest,
                cancelled,
                self.check_proposal_only(cancelled),
            )
            .await
    }
    pub(crate) async fn revalidate(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        self.check_current(&self.owner.preview, &self.owner.digest, cancelled)
            .await
    }
    pub(crate) fn expire(&mut self, remaining: Duration) {
        self.owner.proposal.evidence.binding.window.expires = Instant::now() + remaining;
    }
    pub(crate) fn exhaust_reads(&self) {
        self.owner
            .proposal
            .evidence
            .binding
            .files
            .exhaust_local_reads();
    }
}
