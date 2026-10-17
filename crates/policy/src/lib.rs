use limeos_domain::{Error, ErrorCode, Operation, Principal, Result, Role, Scope};

pub fn authorize(principal: &Principal, grants: &[Scope], request: &Scope) -> Result<()> {
    use Operation::*;
    let role_allows = match principal.role {
        Role::Administrator => true,
        Role::Operator => matches!(
            request.operation,
            HealthRead | ContainerManage | DeploymentManage | AutomationManage
        ),
        Role::MediaRequester => matches!(request.operation, HealthRead | MediaRequest),
        Role::Viewer => request.operation == HealthRead,
    };
    if role_allows
        && grants.iter().any(|g| {
            g.operation == request.operation
                && (g.resource == request.resource
                    || (g.resource == "*"
                        && !matches!(request.operation, DeploymentElevated | AutomationManage)))
                && (request.operation != DeploymentElevated || request.resource != "*")
                && (request.operation != AutomationManage
                    || request.resource == "automation:observe-health")
        })
    {
        Ok(())
    } else {
        Err(Error(ErrorCode::Forbidden))
    }
}

pub fn task_subset(grants: &[Scope], requested: &[Scope]) -> Result<()> {
    if requested.is_empty()
        || requested.len() > 32
        || !requested.iter().all(|r| {
            r.resource != "*"
                && grants.iter().any(|g| {
                    g.operation == r.operation
                        && (g.resource == r.resource
                            || (g.resource == "*"
                                && !matches!(
                                    r.operation,
                                    Operation::DeploymentElevated | Operation::AutomationManage
                                )))
                })
        })
    {
        return Err(Error(ErrorCode::Forbidden));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automation_management_requires_its_exact_grant_and_role_ceiling() {
        let requested = Scope {
            operation: Operation::AutomationManage,
            resource: "automation:observe-health".into(),
        };
        for role in [
            Role::Administrator,
            Role::Operator,
            Role::Viewer,
            Role::MediaRequester,
        ] {
            let principal = Principal {
                id: "alice".into(),
                role,
                grant_revision: 1,
            };
            assert_eq!(
                authorize(&principal, std::slice::from_ref(&requested), &requested).is_ok(),
                matches!(role, Role::Administrator | Role::Operator)
            );
            assert!(authorize(&principal, &[], &requested).is_err());
            let wildcard = Scope {
                resource: "*".into(),
                ..requested.clone()
            };
            assert!(authorize(&principal, &[wildcard], &requested).is_err());
        }
        let principal = Principal {
            id: "alice".into(),
            role: Role::Administrator,
            grant_revision: 1,
        };
        let foreign = Scope {
            resource: "automation:other".into(),
            ..requested.clone()
        };
        assert!(authorize(&principal, std::slice::from_ref(&foreign), &foreign).is_err());
        assert!(
            task_subset(
                &[Scope {
                    resource: "*".into(),
                    ..requested.clone()
                }],
                &[requested]
            )
            .is_err()
        );
    }
    #[test]
    fn elevated_template_authority_cannot_be_wildcard_or_inherited_by_a_task() {
        let elevated = Scope {
            operation: Operation::DeploymentElevated,
            resource: "stack:media:template:test:digest".into(),
        };
        let wildcard = Scope {
            resource: "*".into(),
            ..elevated.clone()
        };
        let admin = Principal {
            id: "admin".into(),
            role: Role::Administrator,
            grant_revision: 1,
        };
        assert!(authorize(&admin, std::slice::from_ref(&wildcard), &elevated).is_err());
        assert!(authorize(&admin, std::slice::from_ref(&wildcard), &wildcard).is_err());
        assert!(task_subset(&[wildcard], std::slice::from_ref(&elevated)).is_err());
        assert!(authorize(&admin, std::slice::from_ref(&elevated), &elevated).is_ok());
        let operator = Principal {
            role: Role::Operator,
            ..admin
        };
        assert!(authorize(&operator, std::slice::from_ref(&elevated), &elevated).is_err());
    }
    #[test]
    fn household_cannot_turn_media_grant_into_storage_or_deployment_authority() {
        let p = Principal {
            id: "household".into(),
            role: Role::MediaRequester,
            grant_revision: 1,
        };
        let grants = [Scope {
            operation: Operation::MediaRequest,
            resource: "library".into(),
        }];
        assert!(authorize(&p, &grants, &grants[0]).is_ok());
        for op in [
            Operation::StorageManage,
            Operation::ContainerManage,
            Operation::DeploymentManage,
            Operation::DeploymentElevated,
            Operation::IdentityManage,
        ] {
            assert!(
                authorize(
                    &p,
                    &grants,
                    &Scope {
                        operation: op,
                        resource: "library".into()
                    }
                )
                .is_err()
            );
        }
        assert!(
            authorize(
                &p,
                &grants,
                &Scope {
                    operation: Operation::MediaRequest,
                    resource: "other".into()
                }
            )
            .is_err()
        );
    }
    #[test]
    fn even_admin_needs_an_explicit_grant_and_task_cannot_expand_it() {
        let p = Principal {
            id: "admin".into(),
            role: Role::Administrator,
            grant_revision: 1,
        };
        let s = Scope {
            operation: Operation::HealthRead,
            resource: "system".into(),
        };
        assert!(authorize(&p, &[], &s).is_err());
        assert!(
            task_subset(
                std::slice::from_ref(&s),
                &[Scope {
                    resource: "*".into(),
                    ..s
                }]
            )
            .is_err()
        );
    }
}
