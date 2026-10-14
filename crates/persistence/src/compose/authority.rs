//! Read-only current identity and scope resolution; no approval or execution capability.
use super::*;
use serde::Serialize;
use std::{
    io::Write,
    time::{SystemTime, UNIX_EPOCH},
};
const OBSERVATION_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComposeAuthorityKind {
    HumanSession,
    Task,
}

/// Borrowed past authority observations cannot reconstruct a checked Store result.
/// ```compile_fail
/// use limeos_persistence::ComposeAuthorityObservation;
/// let imported: ComposeAuthorityObservation<'_> = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Serialize)]
pub struct ComposeAuthorityObservation<'a> {
    pub version: u16,
    pub kind: ComposeAuthorityKind,
    pub principal: &'a Principal,
    pub preview: &'a PlannedCompose,
    pub required_scopes: &'a [Scope],
    pub generation: i64,
    pub checked_at: i64,
    pub identity_expires_at: i64,
}
/// Only a fresh authenticated Store read constructs this value; it is no approval.
/// ```compile_fail
/// use limeos_persistence::AuthenticatedComposePreview;
/// let imported: AuthenticatedComposePreview = serde_json::from_str("{}").unwrap();
/// ```
pub struct AuthenticatedComposePreview {
    kind: ComposeAuthorityKind,
    principal: Principal,
    preview: PlannedCompose,
    scopes: Vec<Scope>,
    generation: i64,
    checked_at: i64,
    identity_expires_at: i64,
}
impl AuthenticatedComposePreview {
    /// Return ordinary preview data; the observation creates no approval or lease.
    pub fn into_preview(self) -> PlannedCompose {
        self.preview
    }
    pub fn snapshot(&self) -> ComposeAuthorityObservation<'_> {
        ComposeAuthorityObservation {
            version: 1,
            kind: self.kind,
            principal: &self.principal,
            preview: &self.preview,
            required_scopes: &self.scopes,
            generation: self.generation,
            checked_at: self.checked_at,
            identity_expires_at: self.identity_expires_at,
        }
    }
}
enum Identity<'a> {
    Session(&'a str),
    Task {
        digest: &'a str,
        peer_uid: u32,
        task: &'a str,
    },
}
fn now() -> Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|v| i64::try_from(v.as_secs()).ok())
        .ok_or(Error(ErrorCode::Unavailable))
}
impl Store {
    /// Authenticate a current read session; this API creates no approval or host effect.
    pub fn resolve_compose_session(
        &self,
        session_digest: &str,
        catalog: &ComposeCatalog,
        expected: &PlannedCompose,
        required_scopes: &[Scope],
    ) -> Result<AuthenticatedComposePreview> {
        self.resolve_compose_authority(
            Identity::Session(session_digest),
            catalog,
            expected,
            required_scopes,
            now,
        )
    }
    /// The composition root supplies actual kernel peer UID/task binding, as for P03.
    pub fn resolve_compose_task(
        &self,
        task_digest: &str,
        peer_uid: u32,
        task: &str,
        catalog: &ComposeCatalog,
        expected: &PlannedCompose,
        required_scopes: &[Scope],
    ) -> Result<AuthenticatedComposePreview> {
        self.resolve_compose_authority(
            Identity::Task {
                digest: task_digest,
                peer_uid,
                task,
            },
            catalog,
            expected,
            required_scopes,
            now,
        )
    }
    fn resolve_compose_authority(
        &self,
        identity: Identity<'_>,
        catalog: &ComposeCatalog,
        expected: &PlannedCompose,
        requirements: &[Scope],
        clock: impl Fn() -> Result<i64>,
    ) -> Result<AuthenticatedComposePreview> {
        let start = clock()?;
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Deferred)
            .map_err(durable)?;
        let principal = self.review_identity(&identity, expected, start)?;
        let preview = self.compose_plan(&principal, catalog, &expected.plan.id, start)?;
        let (revision,expires):(i64,i64)=tx.query_row(
            "SELECT revision,expires FROM compose_plans WHERE id=? AND principal=? AND canceled=0",
            params![preview.plan.id,principal.id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(durable)?;
        if revision != preview.plan.grant_revision || expires != preview.plan.expires_at {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        // Compare the bounded durable row before copying/serializing ordinary caller data.
        if expected != &preview {
            return Err(Error(ErrorCode::Conflict));
        }
        let scopes = compose_scopes(catalog, &preview.plan.selection)?;
        if requirements != scopes {
            return Err(Error(ErrorCode::Forbidden));
        }
        self.review_scopes(&identity, &principal, &scopes, start)?;
        let end = clock()?;
        if end < start {
            return Err(Error(ErrorCode::Expired));
        }
        let current = self.review_identity(&identity, &preview, end)?;
        if current != principal {
            return Err(Error(ErrorCode::Expired));
        }
        let fresh = self.compose_plan(&current, catalog, &preview.plan.id, end)?;
        if fresh != preview {
            return Err(Error(ErrorCode::Conflict));
        }
        self.review_scopes(&identity, &current, &scopes, end)?;
        let generation: i64 = tx
            .query_row("SELECT generation FROM meta WHERE id=1", [], |r| r.get(0))
            .map_err(durable)?;
        if generation != self.generation {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        let (kind, identity_expires_at) = match identity {
            Identity::Session(digest) => (
                ComposeAuthorityKind::HumanSession,
                self.session_expiry(digest)?,
            ),
            Identity::Task { digest, .. } => (
                ComposeAuthorityKind::Task,
                tx.query_row(
                    "SELECT expires FROM task_tokens WHERE digest=?",
                    [digest],
                    |r| r.get(0),
                )
                .map_err(durable)?,
            ),
        };
        let checked = AuthenticatedComposePreview {
            kind,
            principal: current,
            preview: fresh,
            scopes,
            generation,
            checked_at: end,
            identity_expires_at,
        };
        observation_limit(&checked.snapshot(), OBSERVATION_BYTES)?;
        tx.commit().map_err(durable)?;
        Ok(checked)
    }
    fn review_identity(
        &self,
        identity: &Identity<'_>,
        expected: &PlannedCompose,
        at: i64,
    ) -> Result<Principal> {
        match identity {
            Identity::Session(digest) => {
                if !limeos_domain::opaque_id(digest) {
                    return Err(Error(ErrorCode::Unauthenticated));
                }
                self.authenticate(digest, None, at)
            }
            Identity::Task {
                digest,
                peer_uid,
                task,
            } => {
                if !limeos_domain::opaque_id(digest) || !limeos_domain::identifier(task) {
                    return Err(Error(ErrorCode::Expired));
                }
                expected.plan.selection.validate()?;
                self.check_task(
                    digest,
                    *peer_uid,
                    task,
                    &Scope {
                        operation: Operation::DeploymentManage,
                        resource: format!("stack:{}", expected.plan.selection.stack),
                    },
                    at,
                )
            }
        }
    }
    fn review_scopes(
        &self,
        identity: &Identity<'_>,
        principal: &Principal,
        scopes: &[Scope],
        at: i64,
    ) -> Result<()> {
        // compose_plan already checks current role/grants for each scope. Tasks also
        // need every exact token subset, never just the ordinary stack permission.
        if let Identity::Task {
            digest,
            peer_uid,
            task,
        } = identity
        {
            for scope in scopes {
                if &self.check_task(digest, *peer_uid, task, scope, at)? != principal {
                    return Err(Error(ErrorCode::Expired));
                }
            }
        }
        Ok(())
    }
}
struct Count {
    bytes: usize,
    max: usize,
}
impl Write for Count {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.max.saturating_sub(self.bytes) {
            return Err(std::io::Error::other(
                "Compose authority observation exceeds its bound",
            ));
        }
        self.bytes += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn observation_limit(observation: &ComposeAuthorityObservation<'_>, max: usize) -> Result<()> {
    serde_json::to_writer(&mut Count { bytes: 0, max }, observation)
        .map_err(|_| Error(ErrorCode::Unavailable))
}

#[cfg(test)]
mod tests;
