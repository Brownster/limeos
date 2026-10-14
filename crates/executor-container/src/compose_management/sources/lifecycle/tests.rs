//! Private fixture selectors delegate production proposal construction and checks.
use super::*;
use crate::compose_management::{catalog, sources::tests::LocalSources};
use tempfile::TempDir;

pub(crate) struct LocalProposal {
    pub(crate) owner: BoundComposeLifecycleProposal,
    pub(crate) tree: TempDir,
}
pub(crate) fn propose(source: LocalSources, desired: &ComposeProject) -> Result<LocalProposal> {
    let LocalSources { owner, tree } = source;
    Ok(LocalProposal {
        owner: assemble_proposal(owner, desired, &|| false)?,
        tree,
    })
}
pub(crate) fn counted(summary: &ComposeLifecycleSummary<'_>, max: usize) -> Result<()> {
    check_lifecycle_report(summary, max)
}
impl LocalProposal {
    pub(crate) async fn check_source_only(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        let source = &self.owner.evidence;
        source
            .validate_with(
                cancelled,
                || {
                    source
                        .binding
                        .catalog
                        .revalidate_local(self.tree.path(), cancelled)
                },
                || {
                    source.binding.files.checked(|| {
                        source.binding.files.validate_from(
                            &catalog::local_root(self.tree.path()),
                            compose_files::now()?,
                            Instant::now() + Duration::from_secs(2),
                            cancelled,
                        )
                    })
                },
                || {
                    source
                        .binding
                        .files
                        .admit_json_local(self.tree.path(), cancelled)
                },
            )
            .await
    }
    pub(crate) async fn revalidate(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        self.owner
            .validate_with(cancelled, self.check_source_only(cancelled))
            .await
    }
    pub(crate) fn expire(&mut self, remaining: Duration) {
        self.owner.evidence.binding.window.expires = Instant::now() + remaining;
    }
    pub(crate) fn set_wall_time(&mut self, at: i64) {
        self.owner.evidence.binding.window.observed_at = at;
    }
    pub(crate) fn exhaust_reads(&self) {
        self.owner.evidence.binding.files.exhaust_local_reads();
    }
}
