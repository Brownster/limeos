//! Whole-stack start, stop and restart: exact member IDs, never Compose files.
//! One-off containers are never members. Targets are chosen by state so a plan
//! records exactly which containers a later effect may touch.
use crate::{
    ContainerAction, ContainerSnapshot, Error, ErrorCode, ExecutionState, Operation,
    OperationDefinition, PLAN_TTL_SECONDS, Recovery, Result, Risk, Scope, identifier, opaque_id,
    verify_container,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const STACK_VERSION: u16 = 1;
pub const MAX_STACK_MEMBERS: usize = 16;
/// Per-member Engine call budget. A stack's budget is the sum over its targets.
pub const STACK_MEMBER_TIMEOUT_SECONDS: u32 = 30;

pub const STACK_RESTART: OperationDefinition = OperationDefinition {
    name: "stack.restart",
    version: STACK_VERSION,
    permission: Operation::ContainerManage,
    risk: Risk::Disruptive,
    timeout_seconds: STACK_MEMBER_TIMEOUT_SECONDS,
    output_limit: 64 * 1024,
    recovery: Recovery::ReconcileBeforeRetry,
};
pub const STACK_START: OperationDefinition = OperationDefinition {
    name: "stack.start",
    ..STACK_RESTART
};
pub const STACK_STOP: OperationDefinition = OperationDefinition {
    name: "stack.stop",
    ..STACK_RESTART
};

pub fn stack_definition(action: ContainerAction) -> &'static OperationDefinition {
    match action {
        ContainerAction::Restart => &STACK_RESTART,
        ContainerAction::Start => &STACK_START,
        ContainerAction::Stop => &STACK_STOP,
    }
}

/// Compose project names as the Engine labels them.
pub fn stack_project(value: &str) -> bool {
    let first = value.as_bytes().first().copied();
    value.len() <= 64
        && first.is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
}
pub fn stack_resource(project: &str) -> String {
    format!("stack:{project}")
}

/// Every ordinary (non one-off) member, stopped or running, sorted by ID.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StackSnapshot {
    pub resource: String,
    pub members: Vec<ContainerSnapshot>,
}
impl StackSnapshot {
    pub fn new(project: &str, mut members: Vec<ContainerSnapshot>) -> Result<Self> {
        members.sort_by(|a, b| a.resource.cmp(&b.resource));
        let value = Self {
            resource: stack_resource(project),
            members,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn project(&self) -> Result<&str> {
        self.resource
            .strip_prefix("stack:")
            .filter(|p| stack_project(p))
            .ok_or(Error(ErrorCode::InvalidInput))
    }
    pub fn validate(&self) -> Result<()> {
        self.project()?;
        if self.members.is_empty() || self.members.len() > MAX_STACK_MEMBERS {
            return Err(Error(ErrorCode::InvalidInput));
        }
        for member in &self.members {
            member.validate()?;
        }
        if self
            .members
            .windows(2)
            .any(|w| w[0].resource >= w[1].resource)
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        Ok(())
    }
    /// Start acts on stopped members; stop and restart on running members.
    pub fn targets(&self, operation: ContainerAction) -> Vec<&ContainerSnapshot> {
        self.members
            .iter()
            .filter(|m| m.running != (operation == ContainerAction::Start))
            .collect()
    }
    pub fn validate_for(&self, operation: ContainerAction) -> Result<()> {
        self.validate()?;
        if self.targets(operation).is_empty() {
            return Err(Error(ErrorCode::Conflict));
        }
        Ok(())
    }
    pub fn scope(&self) -> Scope {
        Scope {
            operation: STACK_RESTART.permission,
            resource: self.resource.clone(),
        }
    }
    /// The stack itself, then every member: no member is reachable through the
    /// stack grant alone.
    pub fn scopes(&self) -> Vec<Scope> {
        std::iter::once(self.scope())
            .chain(self.members.iter().map(ContainerSnapshot::scope))
            .collect()
    }
    /// Stack first (job primary resource), then every member's lock.
    pub fn resources(&self) -> Vec<String> {
        std::iter::once(self.resource.clone())
            .chain(self.members.iter().map(|m| m.resource.clone()))
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StackPlan {
    pub id: String,
    pub version: u16,
    pub operation: ContainerAction,
    pub principal: String,
    #[ts(type = "number")]
    pub grant_revision: i64,
    pub expected: StackSnapshot,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub expires_at: i64,
}
impl StackPlan {
    pub fn validate(&self, now: i64) -> Result<()> {
        if self.version != STACK_VERSION
            || !opaque_id(&self.id)
            || !identifier(&self.principal)
            || self.grant_revision < 1
            || self.created_at < 0
            || self.expires_at <= self.created_at
            || self.expires_at > self.created_at.saturating_add(PLAN_TTL_SECONDS)
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        self.expected.validate_for(self.operation)?;
        if now < self.created_at || now >= self.expires_at {
            return Err(Error(ErrorCode::Expired));
        }
        Ok(())
    }
    /// Any difference in any member, including one the action would skip,
    /// refuses the plan: there is no partial guess about what changed.
    pub fn check_current(&self, actual: &StackSnapshot, now: i64) -> Result<()> {
        self.validate(now)?;
        actual.validate()?;
        if self.expected != *actual {
            return Err(Error(ErrorCode::Conflict));
        }
        Ok(())
    }
    pub fn targets(&self) -> Vec<&ContainerSnapshot> {
        self.expected.targets(self.operation)
    }
    pub fn resources(&self) -> Vec<String> {
        self.expected.resources()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct PlannedStackAction {
    pub plan: StackPlan,
    pub digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StackJob {
    pub id: String,
    pub state: crate::JobState,
    pub plan: StackPlan,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StackRequest {
    pub action: String,
    pub plan_digest: String,
    pub plan: StackPlan,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StackMemberReceipt {
    pub resource: String,
    pub state: ExecutionState,
    pub error: Option<ErrorCode>,
}
/// `state` is always the aggregate of `members`, which lists exactly the
/// plan's targets. A member that was never sent is `precondition_changed`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct StackReceipt {
    pub action: String,
    pub plan_digest: String,
    pub operation: ContainerAction,
    pub before: StackSnapshot,
    pub state: ExecutionState,
    pub members: Vec<StackMemberReceipt>,
    pub error: Option<ErrorCode>,
}
impl StackReceipt {
    pub fn validate(&self) -> Result<()> {
        self.before.validate_for(self.operation)?;
        let targets = self.before.targets(self.operation);
        if !opaque_id(&self.action)
            || !opaque_id(&self.plan_digest)
            || self.members.len() != targets.len()
            || self
                .members
                .iter()
                .zip(targets)
                .any(|(m, t)| m.resource != t.resource)
            || aggregate_state(&self.members) != self.state
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        Ok(())
    }
    pub fn member(&self, resource: &str) -> Option<&StackMemberReceipt> {
        self.members.iter().find(|m| m.resource == resource)
    }
}

/// Verified only when all are; unsent only when none were sent; accepted when
/// every member is accepted or already verified; any other mix is uncertain.
pub fn aggregate_state(members: &[StackMemberReceipt]) -> ExecutionState {
    use ExecutionState::*;
    let all = |f: fn(ExecutionState) -> bool| members.iter().all(|m| f(m.state));
    if members.is_empty() {
        OutcomeUnknown
    } else if all(|s| s == Verified) {
        Verified
    } else if all(|s| s == PreconditionChanged) {
        PreconditionChanged
    } else if all(|s| s == Prepared) {
        Prepared
    } else if all(|s| matches!(s, EffectAccepted | Verified)) {
        EffectAccepted
    } else {
        OutcomeUnknown
    }
}

/// Fresh inspections must prove each target's own post-state; a stack-level
/// Engine success alone never does.
pub fn verify_stack_member(
    operation: ContainerAction,
    before: &ContainerSnapshot,
    after: &ContainerSnapshot,
) -> Result<()> {
    verify_container(operation, before, after)
}
pub fn verify_stack(
    operation: ContainerAction,
    before: &StackSnapshot,
    afters: &[ContainerSnapshot],
) -> Result<()> {
    before.validate_for(operation)?;
    for target in before.targets(operation) {
        let after = afters
            .iter()
            .find(|a| a.resource == target.resource)
            .ok_or(Error(ErrorCode::Conflict))?;
        verify_stack_member(operation, target, after)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn member(n: char, running: bool) -> ContainerSnapshot {
        ContainerSnapshot {
            resource: format!("container:{}", n.to_string().repeat(64)),
            image: format!("sha256:{}", "b".repeat(64)),
            started_at: "2026-10-05T07:00:00.000000000Z".into(),
            running,
        }
    }
    fn stack(members: Vec<ContainerSnapshot>) -> StackSnapshot {
        StackSnapshot::new("media", members).unwrap()
    }
    fn plan(operation: ContainerAction, expected: StackSnapshot) -> StackPlan {
        StackPlan {
            id: "c".repeat(64),
            version: STACK_VERSION,
            operation,
            principal: "user".into(),
            grant_revision: 1,
            expected,
            created_at: 100,
            expires_at: 400,
        }
    }
    #[test]
    fn targets_follow_state_and_an_action_without_targets_is_refused() {
        let s = stack(vec![
            member('b', false),
            member('a', true),
            member('c', true),
        ]);
        let ids = |op| {
            s.targets(op)
                .iter()
                .map(|m| m.resource[10..11].to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(ContainerAction::Start), ["b"]);
        assert_eq!(ids(ContainerAction::Stop), ["a", "c"]);
        assert_eq!(ids(ContainerAction::Restart), ["a", "c"]);
        let stopped = stack(vec![member('a', false)]);
        assert_eq!(
            stopped.validate_for(ContainerAction::Stop).unwrap_err().0,
            ErrorCode::Conflict
        );
        assert_eq!(
            plan(ContainerAction::Restart, stopped)
                .validate(101)
                .unwrap_err()
                .0,
            ErrorCode::Conflict
        );
    }
    #[test]
    fn snapshot_identity_is_bounded_sorted_and_exact() {
        assert!(StackSnapshot::new("media", vec![]).is_err());
        assert!(StackSnapshot::new("Media", vec![member('a', true)]).is_err());
        assert!(StackSnapshot::new("../x", vec![member('a', true)]).is_err());
        assert!(StackSnapshot::new("media", vec![member('a', true), member('a', true)]).is_err());
        let many: Vec<_> = (0..=MAX_STACK_MEMBERS)
            .map(|i| ContainerSnapshot {
                resource: format!("container:{i:064x}"),
                ..member('a', true)
            })
            .collect();
        assert_eq!(many.len(), MAX_STACK_MEMBERS + 1);
        assert!(StackSnapshot::new("media", many).is_err());
        let short = ContainerSnapshot {
            resource: "container:abcd".into(),
            ..member('a', true)
        };
        assert!(StackSnapshot::new("media", vec![short]).is_err());
        let unsorted = StackSnapshot {
            resource: "stack:media".into(),
            members: vec![member('b', true), member('a', true)],
        };
        assert!(unsorted.validate().is_err());
        let s = stack(vec![member('b', true), member('a', true)]);
        assert_eq!(s.resources()[0], "stack:media");
        assert_eq!(s.resources().len(), 3);
        assert_eq!(s.scopes().len(), 3);
    }
    #[test]
    fn any_member_change_expiry_or_addition_refuses_the_plan() {
        let s = stack(vec![member('a', true), member('b', false)]);
        let p = plan(ContainerAction::Restart, s.clone());
        p.check_current(&s, 399).unwrap();
        assert_eq!(p.check_current(&s, 400).unwrap_err().0, ErrorCode::Expired);
        assert_eq!(p.check_current(&s, 99).unwrap_err().0, ErrorCode::Expired);
        // Also a member the action would skip.
        for changed in [
            vec![
                member('a', true),
                ContainerSnapshot {
                    running: true,
                    ..member('b', false)
                },
            ],
            vec![member('a', true)],
            vec![member('a', true), member('b', false), member('c', true)],
            vec![
                ContainerSnapshot {
                    started_at: "2026-10-05T07:01:00Z".into(),
                    ..member('a', true)
                },
                member('b', false),
            ],
        ] {
            assert_eq!(
                p.check_current(&stack(changed), 101).unwrap_err().0,
                ErrorCode::Conflict
            );
        }
        let mut late = p.clone();
        late.expires_at = 401 + PLAN_TTL_SECONDS;
        assert_eq!(late.validate(101).unwrap_err().0, ErrorCode::InvalidInput);
    }
    #[test]
    fn receipt_aggregate_and_member_order_are_enforced() {
        use ExecutionState::*;
        let s = stack(vec![
            member('a', true),
            member('b', true),
            member('c', false),
        ]);
        let row = |r: &str, state| StackMemberReceipt {
            resource: r.into(),
            state,
            error: None,
        };
        let a = s.members[0].resource.clone();
        let b = s.members[1].resource.clone();
        let mk = |states: [ExecutionState; 2]| {
            let members = vec![row(&a, states[0]), row(&b, states[1])];
            StackReceipt {
                action: "d".repeat(64),
                plan_digest: "e".repeat(64),
                operation: ContainerAction::Restart,
                before: s.clone(),
                state: aggregate_state(&members),
                members,
                error: None,
            }
        };
        for (states, expected) in [
            ([Verified, Verified], Verified),
            ([Prepared, Prepared], Prepared),
            ([EffectAccepted, Verified], EffectAccepted),
            ([Verified, OutcomeUnknown], OutcomeUnknown),
            ([EffectAccepted, PreconditionChanged], OutcomeUnknown),
            (
                [PreconditionChanged, PreconditionChanged],
                PreconditionChanged,
            ),
        ] {
            let r = mk(states);
            assert_eq!(r.state, expected);
            r.validate().unwrap();
        }
        let mut wrong = mk([Verified, Verified]);
        wrong.state = EffectAccepted;
        assert!(wrong.validate().is_err());
        let mut skipped = mk([Verified, Verified]);
        skipped.members.pop();
        skipped.state = Verified;
        assert!(skipped.validate().is_err());
        let mut swapped = mk([Verified, Verified]);
        swapped.members.reverse();
        assert!(swapped.validate().is_err());
    }
    #[test]
    fn stack_verification_requires_every_target_to_prove_its_own_post_state() {
        let s = stack(vec![
            member('a', true),
            member('b', true),
            member('c', false),
        ]);
        let later = |m: &ContainerSnapshot| ContainerSnapshot {
            started_at: "2026-10-05T07:01:00Z".into(),
            ..m.clone()
        };
        let good = [later(&s.members[0]), later(&s.members[1])];
        verify_stack(ContainerAction::Restart, &s, &good).unwrap();
        // Partial proof: one member did not restart. Not a stack success.
        let partial = [later(&s.members[0]), s.members[1].clone()];
        assert!(verify_stack(ContainerAction::Restart, &s, &partial).is_err());
        assert!(verify_stack(ContainerAction::Restart, &s, &good[..1]).is_err());
        verify_stack_member(ContainerAction::Restart, &s.members[0], &good[0]).unwrap();
        let stopped = [
            ContainerSnapshot {
                running: false,
                ..s.members[0].clone()
            },
            ContainerSnapshot {
                running: false,
                ..s.members[1].clone()
            },
        ];
        verify_stack(ContainerAction::Stop, &s, &stopped).unwrap();
    }
    #[test]
    fn plan_bytes_round_trip_and_unknown_fields_are_refused() {
        let p = plan(ContainerAction::Stop, stack(vec![member('a', true)]));
        let text = serde_json::to_string(&p).unwrap();
        assert!(text.contains("\"operation\":\"stop\""));
        assert_eq!(serde_json::from_str::<StackPlan>(&text).unwrap(), p);
        let extra = text.replacen('{', "{\"compose_file\":\"x\",", 1);
        assert!(serde_json::from_str::<StackPlan>(&extra).is_err());
        assert_eq!(stack_definition(ContainerAction::Start).name, "stack.start");
    }
}
