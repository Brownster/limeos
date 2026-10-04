use limeos_domain::{Error, ErrorCode, Operation, Principal, Result, Role, Scope};

pub fn authorize(principal: &Principal, grants: &[Scope], request: &Scope) -> Result<()> {
    use Operation::*;
    let role_allows = match principal.role {
        Role::Administrator => true,
        Role::Operator => matches!(request.operation, HealthRead | ContainerManage),
        Role::MediaRequester => matches!(request.operation, HealthRead | MediaRequest),
        Role::Viewer => request.operation == HealthRead,
    };
    if role_allows
        && grants.iter().any(|g| {
            g.operation == request.operation
                && (g.resource == request.resource || g.resource == "*")
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
                    g.operation == r.operation && (g.resource == r.resource || g.resource == "*")
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
