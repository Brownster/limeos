use super::*;
use limeos_domain::{Operation, deployment::discovery::StackInventory};
use std::collections::BTreeSet;

impl Core {
    pub(super) async fn human_stack_inventory(&self, token: String) -> Result<StackInventory> {
        let digest = limeos_identity::digest(&token);
        let cache = self.stacks.clone();
        // The Store actor serializes current session/grant resolution with the
        // bounded cache read. No stale browser principal or role grants access.
        self.db
            .call(move |store| {
                let principal = store.authenticate(&digest, None, now())?;
                let grants = store.grants(&principal)?;
                let allowed = |resource: String| {
                    limeos_policy::authorize(
                        &principal,
                        &grants,
                        &Scope {
                            operation: Operation::HealthRead,
                            resource,
                        },
                    )
                };
                // Summary/source availability and complete cardinality need their
                // own read scope, including an actually empty or unavailable view.
                allowed("deployment:stacks".into())?;
                let cohort = cache.cohort()?;
                let mut resources = BTreeSet::new();
                if let Some(catalog) = cohort.catalog() {
                    for stack in &catalog.stacks {
                        resources.insert(format!("stack:{}", stack.stack));
                        resources.insert(format!("stack:{}", stack.project));
                    }
                }
                if let Some(engine) = cohort.engine() {
                    for member in &engine.members {
                        resources.insert(member.container().resource.clone());
                        if let limeos_domain::deployment::discovery::Membership::Compose {
                            project,
                            ..
                        } = member
                        {
                            resources.insert(format!("stack:{}", project.name));
                        }
                    }
                }
                // Fail the complete response. Filtering a private member would
                // reveal counts or falsely turn a partial project into a healthy one.
                for resource in resources {
                    allowed(resource)?;
                }
                cohort.project(now())
            })
            .await
    }
}

#[cfg(test)]
mod tests;
