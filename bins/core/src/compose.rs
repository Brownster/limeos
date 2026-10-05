use super::*;
use limeos_domain::{ComposeCatalog, ComposeSelection, PlannedCompose};

impl Core {
    pub(super) fn catalog(&self) -> Result<Arc<ComposeCatalog>> {
        self.compose_catalog
            .clone()
            .ok_or(Error(ErrorCode::Unavailable))
    }
    pub(super) async fn propose_compose_task(
        &self,
        token: String,
        uid: u32,
        task: String,
        selection: ComposeSelection,
    ) -> Result<PlannedCompose> {
        let catalog = self.catalog()?;
        selection.validate()?;
        let digest = limeos_identity::digest(&token);
        self.db
            .call(move |s| {
                let principal = s.check_task(
                    &digest,
                    uid,
                    &task,
                    &Scope {
                        operation: limeos_domain::Operation::DeploymentManage,
                        resource: format!("stack:{}", selection.stack),
                    },
                    now(),
                )?;
                for scope in limeos_persistence::compose_scopes(&catalog, &selection)?
                    .into_iter()
                    .skip(1)
                {
                    let current = s.check_task(&digest, uid, &task, &scope, now())?;
                    if principal != current {
                        return Err(Error(ErrorCode::Forbidden));
                    }
                }
                s.plan_compose(&principal, &catalog, &selection, now())
            })
            .await
    }
}
