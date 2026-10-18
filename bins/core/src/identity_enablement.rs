use super::*;
use std::time::Instant;

fn live(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        return Err(Error(ErrorCode::Expired));
    }
    Ok(())
}

impl Core {
    pub(super) async fn human_enable_observe_health(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::ObserveHealthEnable,
        deadline: Instant,
    ) -> Result<limeos_contracts::ObserveHealthEnabled> {
        live(deadline)?;
        if input.password.is_empty() || input.password.len() > 1024 {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let session = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        let first_session = session.clone();
        let first_csrf = csrf.clone();
        let record = self
            .db
            .call(move |store| {
                live(deadline)?;
                store.observe_health_reauthentication(&first_session, &first_csrf, now())
            })
            .await?;
        let response = self
            .password(limeos_contracts::PasswordRequest::Verify {
                password: input.password,
                encoded: record.password_hash.clone(),
            })
            .await?;
        if !response.valid {
            return Err(Error(ErrorCode::Unauthenticated));
        }
        live(deadline)?;
        let original_revision = record.principal.grant_revision;
        let principal = self
            .db
            .call(move |store| {
                live(deadline)?;
                store.enable_observe_health(
                    &session,
                    &csrf,
                    &record,
                    response.upgraded.as_deref(),
                    now,
                    deadline,
                )
            })
            .await?;
        Ok(limeos_contracts::ObserveHealthEnabled {
            enabled: true,
            reauthenticate: principal.grant_revision != original_revision,
        })
    }
}

#[cfg(test)]
mod tests;
