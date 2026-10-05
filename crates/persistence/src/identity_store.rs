use super::*;

impl Store {
    pub fn issue_bootstrap(&mut self, digest: &str, now: i64) -> Result<i64> {
        let expires = now.checked_add(900).ok_or(Error(ErrorCode::InvalidInput))?;
        self.write(|tx| {
            let count: i64 = tx.query_row("SELECT count(*) FROM users", [], |r| r.get(0)).map_err(durable)?;
            if count != 0 { return Err(Error(ErrorCode::Conflict)); }
            tx.execute("INSERT INTO bootstrap VALUES (1,?,?) ON CONFLICT(id) DO UPDATE SET digest=excluded.digest,expires=excluded.expires", params![digest,expires]).map_err(durable)?;
            event(tx,None,None,EventKind::BootstrapIssued,now)?; Ok(expires)
        })
    }
    pub fn enroll(
        &mut self,
        token_digest: &str,
        username: &str,
        hash: &str,
        now: i64,
    ) -> Result<()> {
        if !limeos_domain::identifier(username)
            || username.len() > 64
            || !hash.starts_with("$argon2id$")
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let id = limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?;
        self.write(|tx| {
            let count: i64 = tx
                .query_row("SELECT count(*) FROM users", [], |r| r.get(0))
                .map_err(durable)?;
            if count != 0
                || tx
                    .execute(
                        "DELETE FROM bootstrap WHERE id=1 AND digest=? AND expires>?",
                        params![token_digest, now],
                    )
                    .map_err(durable)?
                    != 1
            {
                return Err(Error(ErrorCode::Expired));
            }
            tx.execute(
                "INSERT INTO users VALUES (?,?,?,'\"administrator\"',1)",
                params![id, username, hash],
            )
            .map_err(durable)?;
            for op in [
                Operation::HealthRead,
                Operation::MediaRequest,
                Operation::ContainerManage,
                Operation::StorageManage,
                Operation::IdentityManage,
            ] {
                tx.execute(
                    "INSERT INTO grants VALUES (?,?, '*')",
                    params![id, json(&op)?],
                )
                .map_err(durable)?;
            }
            event(tx, Some(&id), None, EventKind::AdministratorEnrolled, now)
        })
    }
    /// P06's importer supplies only validated hashes, never plaintext, and reads
    /// original files without changing them. New identities preserve admin access.
    pub fn import_legacy_user(
        &mut self,
        username: &str,
        encoded: &str,
        now: i64,
    ) -> Result<Principal> {
        if !limeos_domain::identifier(username)
            || username.len() > 64
            || !limeos_identity::supported_legacy(encoded)
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let id = limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?;
        self.write(|tx| {
            let exists: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM users WHERE username=?)",
                    [username],
                    |r| r.get(0),
                )
                .map_err(durable)?;
            if exists {
                return Err(Error(ErrorCode::Conflict));
            }
            tx.execute(
                "INSERT INTO users VALUES (?,?,?, ?,1)",
                params![id, username, encoded, json(&Role::Administrator)?],
            )
            .map_err(durable)?;
            for op in [
                Operation::HealthRead,
                Operation::MediaRequest,
                Operation::ContainerManage,
                Operation::StorageManage,
                Operation::IdentityManage,
            ] {
                tx.execute(
                    "INSERT INTO grants VALUES (?,?, '*')",
                    params![id, json(&op)?],
                )
                .map_err(durable)?;
            }
            event(tx, Some(&id), None, EventKind::LegacyIdentityImported, now)?;
            Ok(Principal {
                id,
                role: Role::Administrator,
                grant_revision: 1,
            })
        })
    }
    pub fn login_record(&self, username: &str) -> Result<Option<LoginRecord>> {
        let row: Option<(String, String, String, i64)> = self
            .conn
            .query_row(
                "SELECT id,password_hash,role,grant_revision FROM users WHERE username=?",
                [username],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(durable)?;
        row.map(|(id, password_hash, role, grant_revision)| {
            Ok(LoginRecord {
                principal: Principal {
                    id,
                    role: parse(&role)?,
                    grant_revision,
                },
                password_hash,
            })
        })
        .transpose()
    }
    pub fn create_session(
        &mut self,
        record: &LoginRecord,
        upgraded: Option<&str>,
        digest: &str,
        csrf_digest: &str,
        now: i64,
    ) -> Result<i64> {
        let expires = now + 8 * 3600;
        self.write(|tx| {
            let changed = tx.execute("UPDATE users SET password_hash=coalesce(?,password_hash) WHERE id=? AND password_hash=? AND grant_revision=?", params![upgraded,record.principal.id,record.password_hash,record.principal.grant_revision]).map_err(durable)?;
            if changed != 1 { return Err(Error(ErrorCode::Expired)); }
            tx.execute("DELETE FROM sessions WHERE expires<=?", [now]).map_err(durable)?;
            let count: i64 = tx.query_row("SELECT count(*) FROM sessions", [], |r| r.get(0)).map_err(durable)?;
            if count >= 8192 { return Err(Error(ErrorCode::Overloaded)); }
            let user_count: i64 = tx.query_row("SELECT count(*) FROM sessions WHERE principal=?", [&record.principal.id], |r| r.get(0)).map_err(durable)?;
            if user_count >= 16 { tx.execute("DELETE FROM sessions WHERE digest=(SELECT digest FROM sessions WHERE principal=? ORDER BY created LIMIT 1)", [&record.principal.id]).map_err(durable)?; }
            tx.execute("INSERT INTO sessions VALUES (?,?,?,?,?,?)", params![digest,record.principal.id,csrf_digest,record.principal.grant_revision,expires,now]).map_err(durable)?;
            event(tx,Some(&record.principal.id),None,EventKind::SessionCreated,now)?; Ok(expires)
        })
    }
    pub fn authenticate(&self, digest: &str, csrf: Option<&str>, now: i64) -> Result<Principal> {
        let row: Option<(String,String,i64,String)> = self.conn.query_row("SELECT u.id,u.role,u.grant_revision,s.csrf_digest FROM sessions s JOIN users u ON u.id=s.principal WHERE s.digest=? AND s.expires>? AND s.revision=u.grant_revision",params![digest,now],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(durable)?;
        let (id, role, grant_revision, csrf_digest) =
            row.ok_or(Error(ErrorCode::Unauthenticated))?;
        if csrf.is_some_and(|c| !limeos_identity::constant_eq(c, &csrf_digest)) {
            return Err(Error(ErrorCode::Forbidden));
        }
        Ok(Principal {
            id,
            role: parse(&role)?,
            grant_revision,
        })
    }
    pub fn session_expiry(&self, digest: &str) -> Result<i64> {
        self.conn
            .query_row(
                "SELECT expires FROM sessions WHERE digest=?",
                [digest],
                |r| r.get(0),
            )
            .optional()
            .map_err(durable)?
            .ok_or(Error(ErrorCode::Unauthenticated))
    }
    pub fn revoke_session(&mut self, digest: &str, now: i64) -> Result<()> {
        self.write(|tx| {
            tx.execute("DELETE FROM sessions WHERE digest=?", [digest])
                .map_err(durable)?;
            event(tx, None, None, EventKind::SessionRevoked, now)
        })
    }
    pub fn grants(&self, principal: &Principal) -> Result<Vec<Scope>> {
        let identity: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT grant_revision,role FROM users WHERE id=?",
                [&principal.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(durable)?;
        if identity != Some((principal.grant_revision, json(&principal.role)?)) {
            return Err(Error(ErrorCode::Expired));
        }
        let mut stmt = self
            .conn
            .prepare("SELECT operation,resource FROM grants WHERE principal=?")
            .map_err(durable)?;
        let rows = stmt
            .query_map([&principal.id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(durable)?;
        rows.map(|r| {
            let (op, resource) = r.map_err(durable)?;
            Ok(Scope {
                operation: parse(&op)?,
                resource,
            })
        })
        .collect()
    }
    pub fn revise_grants(
        &mut self,
        principal: &str,
        role: Role,
        scopes: &[Scope],
        now: i64,
    ) -> Result<()> {
        if scopes.len() > 32 {
            return Err(Error(ErrorCode::InvalidInput));
        }
        self.write(|tx| {
            if tx
                .execute(
                    "UPDATE users SET role=?,grant_revision=grant_revision+1 WHERE id=?",
                    params![json(&role)?, principal],
                )
                .map_err(durable)?
                != 1
            {
                return Err(Error(ErrorCode::NotFound));
            }
            tx.execute("DELETE FROM grants WHERE principal=?", [principal])
                .map_err(durable)?;
            for s in scopes {
                tx.execute(
                    "INSERT INTO grants VALUES (?,?,?)",
                    params![principal, json(&s.operation)?, s.resource],
                )
                .map_err(durable)?;
            }
            event(tx, Some(principal), None, EventKind::GrantsRevised, now)
        })
    }
    pub fn issue_task(
        &mut self,
        principal: &Principal,
        grant: TaskGrant<'_>,
        digest: &str,
        now: i64,
    ) -> Result<()> {
        let TaskGrant {
            service_uid,
            task,
            scopes,
            expires,
        } = grant;
        if !limeos_domain::identifier(task) || expires <= now || expires > now + 3600 {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let grants = self.grants(principal)?;
        limeos_policy::task_subset(&grants, scopes)?;
        for s in scopes {
            limeos_policy::authorize(principal, &grants, s)?;
        }
        let generation = self.generation;
        self.write(|tx| {
            tx.execute(
                "DELETE FROM task_tokens WHERE expires<=? OR revoked=1 OR generation<>?",
                params![now, generation],
            )
            .map_err(durable)?;
            let count: i64 = tx
                .query_row("SELECT count(*) FROM task_tokens", [], |r| r.get(0))
                .map_err(durable)?;
            if count >= 8192 {
                return Err(Error(ErrorCode::Overloaded));
            }
            tx.execute(
                "INSERT INTO task_tokens VALUES (?,?,?,?,?,?,?,?,0)",
                params![
                    digest,
                    principal.id,
                    service_uid,
                    task,
                    json(&scopes)?,
                    principal.grant_revision,
                    generation,
                    expires
                ],
            )
            .map_err(durable)?;
            event(tx, Some(&principal.id), None, EventKind::TaskIssued, now)
        })
    }
    pub fn check_task(
        &self,
        digest: &str,
        peer_uid: u32,
        task: &str,
        scope: &Scope,
        now: i64,
    ) -> Result<()> {
        let scopes: Option<String> = self.conn.query_row("SELECT t.scopes FROM task_tokens t JOIN users u ON u.id=t.principal WHERE t.digest=? AND t.service_uid=? AND t.task=? AND t.generation=? AND t.revision=u.grant_revision AND t.expires>? AND t.revoked=0",params![digest,peer_uid,task,self.generation,now],|r|r.get(0)).optional().map_err(durable)?;
        let scopes: Vec<Scope> = parse(&scopes.ok_or(Error(ErrorCode::Expired))?)?;
        if !scopes.contains(scope) {
            return Err(Error(ErrorCode::Forbidden));
        }
        Ok(())
    }
    pub fn revoke_task(&mut self, digest: &str, now: i64) -> Result<()> {
        self.write(|tx| {
            tx.execute("UPDATE task_tokens SET revoked=1 WHERE digest=?", [digest])
                .map_err(durable)?;
            event(tx, None, None, EventKind::TaskRevoked, now)
        })
    }
}
