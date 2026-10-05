use super::*;
use limeos_domain::{ComposeCatalog, ComposePlan, ComposeSelection, PlanApproval, PlannedCompose};

const COMPOSE_PLAN_LIMIT: i64 = 1024;
const COMPOSE_PLAN_BYTES: usize = 48 * 1024;

fn normalized(catalog: &ComposeCatalog) -> Result<ComposeCatalog> {
    if json(catalog)?.len() > config::CONFIG_LIMIT as usize {
        return Err(Error(ErrorCode::InvalidInput));
    }
    let mut catalog = catalog.clone();
    catalog.normalize()?;
    Ok(catalog)
}

/// Every caller uses these exact scopes; template content cannot inherit another
/// version's elevated grant. Wildcard elevated grants are rejected by policy.
pub fn compose_scopes(
    catalog: &ComposeCatalog,
    selection: &ComposeSelection,
) -> Result<Vec<Scope>> {
    let catalog = normalized(catalog)?;
    let (_, template) = catalog.select(selection)?;
    let mut scopes = vec![Scope {
        operation: Operation::DeploymentManage,
        resource: format!("stack:{}", selection.stack),
    }];
    if !template.project.elevated().is_empty() {
        scopes.push(Scope {
            operation: Operation::DeploymentElevated,
            resource: format!(
                "stack:{}:template:{}:{}",
                selection.stack,
                selection.template,
                limeos_identity::digest(&json(&template.project)?)
            ),
        });
    }
    Ok(scopes)
}

impl Store {
    fn authorize_compose(
        &self,
        principal: &Principal,
        catalog: &ComposeCatalog,
        selection: &ComposeSelection,
    ) -> Result<()> {
        selection.validate()?;
        let grants = self.grants(principal)?;
        limeos_policy::authorize(
            principal,
            &grants,
            &Scope {
                operation: Operation::DeploymentManage,
                resource: format!("stack:{}", selection.stack),
            },
        )?;
        for scope in compose_scopes(catalog, selection)? {
            limeos_policy::authorize(principal, &grants, &scope)?;
        }
        Ok(())
    }
    pub fn plan_compose(
        &mut self,
        principal: &Principal,
        catalog: &ComposeCatalog,
        selection: &ComposeSelection,
        now: i64,
    ) -> Result<PlannedCompose> {
        let catalog = normalized(catalog)?;
        self.authorize_compose(principal, &catalog, selection)?;
        let (stack, template) = catalog.select(selection)?;
        let plan = ComposePlan {
            id: limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?,
            version: limeos_domain::COMPOSE_VERSION,
            principal: principal.id.clone(),
            grant_revision: principal.grant_revision,
            catalog_digest: limeos_identity::digest(&json(&catalog)?),
            template_digest: limeos_identity::digest(&json(&template.project)?),
            selection: selection.clone(),
            operator_files: stack.operator_files.clone(),
            managed_file: limeos_domain::COMPOSE_MANAGED_FILE.into(),
            before: stack.current.clone(),
            desired: template.project.clone(),
            impact: limeos_domain::ComposeImpact::between(&stack.current, &template.project),
            created_at: now,
            expires_at: now
                .checked_add(limeos_domain::PLAN_TTL_SECONDS)
                .ok_or(Error(ErrorCode::InvalidInput))?,
        };
        plan.validate(now)?;
        let body = json(&plan)?;
        if body.len() > COMPOSE_PLAN_BYTES {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let digest = limeos_identity::digest(&body);
        self.write(|tx| {
            tx.execute("DELETE FROM compose_plans WHERE expires<=?", [now]).map_err(durable)?;
            let count: i64 = tx.query_row("SELECT count(*) FROM compose_plans", [], |r| r.get(0)).map_err(durable)?;
            if count >= COMPOSE_PLAN_LIMIT { return Err(Error(ErrorCode::Overloaded)); }
            tx.execute("INSERT INTO compose_plans(id,principal,body,digest,revision,expires) VALUES(?,?,?,?,?,?)", params![plan.id,principal.id,body,digest,principal.grant_revision,plan.expires_at]).map_err(durable)?;
            event(tx, Some(&principal.id), None, EventKind::PlanCreated { plan: plan.id.clone() }, now)
        })?;
        Ok(PlannedCompose { plan, digest })
    }
    pub fn compose_plan(
        &self,
        principal: &Principal,
        catalog: &ComposeCatalog,
        id: &str,
        now: i64,
    ) -> Result<PlannedCompose> {
        if !limeos_domain::opaque_id(id) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        self.grants(principal)?;
        let row: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT body,digest FROM compose_plans WHERE id=? AND principal=? AND canceled=0",
                params![id, principal.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(durable)?;
        let (body, digest) = row.ok_or(Error(ErrorCode::NotFound))?;
        if body.len() > COMPOSE_PLAN_BYTES {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let plan: ComposePlan = parse(&body)?;
        if plan.id != id
            || plan.principal != principal.id
            || limeos_identity::digest(&json(&plan)?) != digest
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        if plan.grant_revision != principal.grant_revision {
            return Err(Error(ErrorCode::Expired));
        }
        plan.validate(now)?;
        let catalog = normalized(catalog)?;
        if plan.catalog_digest != limeos_identity::digest(&json(&catalog)?) {
            return Err(Error(ErrorCode::Conflict));
        }
        let (stack, template) = catalog.select(&plan.selection)?;
        if plan.operator_files != stack.operator_files
            || plan.before != stack.current
            || plan.desired != template.project
            || plan.template_digest != limeos_identity::digest(&json(&template.project)?)
            || plan.impact
                != limeos_domain::ComposeImpact::between(&stack.current, &template.project)
        {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        self.authorize_compose(principal, &catalog, &plan.selection)?;
        Ok(PlannedCompose { plan, digest })
    }
    /// Only a composition root resolving a current human session calls this.
    /// There is deliberately no executable Compose intent or queue in P03.
    pub fn approve_compose(
        &mut self,
        principal: &Principal,
        catalog: &ComposeCatalog,
        id: &str,
        digest: &str,
        now: i64,
    ) -> Result<PlanApproval> {
        let proposal = self.compose_plan(principal, catalog, id, now)?;
        if !limeos_identity::constant_eq(digest, &proposal.digest) {
            return Err(Error(ErrorCode::Conflict));
        }
        let token = limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?;
        let hashed = limeos_identity::digest(&token);
        self.write(|tx| {
            if tx.execute("UPDATE compose_plans SET approval_digest=? WHERE id=? AND principal=? AND canceled=0", params![hashed,id,principal.id]).map_err(durable)? != 1 { return Err(Error(ErrorCode::Conflict)); }
            event(tx, Some(&principal.id), None, EventKind::PlanApproved { plan: id.into() }, now)
        })?;
        Ok(PlanApproval {
            token,
            expires_at: proposal.plan.expires_at,
        })
    }
    pub fn cancel_compose(
        &mut self,
        principal: &Principal,
        catalog: &ComposeCatalog,
        id: &str,
        now: i64,
    ) -> Result<()> {
        self.compose_plan(principal, catalog, id, now)?;
        self.write(|tx| {
            tx.execute("UPDATE compose_plans SET canceled=1,approval_digest=NULL WHERE id=? AND principal=?", params![id,principal.id]).map_err(durable)?;
            event(tx, Some(&principal.id), None, EventKind::PlanCanceled { plan: id.into() }, now)
        })
    }
}

#[cfg(test)]
mod tests;
