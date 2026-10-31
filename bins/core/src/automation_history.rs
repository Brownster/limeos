//! Owned observation history through the sole Store worker; no producer/send.
use super::*;
use limeos_domain::automation::{
    AcknowledgeNotification, AlertState, AutomationPage, HealthPolicyView, Notification,
    ReplaceHealthPolicy,
};
use limeos_persistence::Store;
use std::time::Instant;

fn live(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        Err(Error(ErrorCode::Expired))
    } else {
        Ok(())
    }
}

impl Core {
    pub(super) async fn human_health_policy(
        &self,
        token: String,
        deadline: Instant,
    ) -> Result<HealthPolicyView> {
        let session = limeos_identity::digest(&token);
        self.history_store(deadline, move |store| {
            store.observe_health_policy(&session, now, deadline)
        })
        .await
    }
    pub(super) async fn human_health_policy_replace(
        &self,
        token: String,
        csrf: String,
        input: ReplaceHealthPolicy,
        deadline: Instant,
    ) -> Result<HealthPolicyView> {
        let session = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.history_store(deadline, move |store| {
            store.observe_health_policy_replace(&session, &csrf, &input, now, deadline)
        })
        .await
    }
    async fn history_store<T: Send + 'static>(
        &self,
        deadline: Instant,
        work: impl FnOnce(&mut Store) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        live(deadline)?;
        let value = tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            self.db.call(move |store| {
                live(deadline)?;
                work(store)
            }),
        )
        .await
        .map_err(|_| Error(ErrorCode::Expired))??;
        live(deadline)?;
        Ok(value)
    }

    pub(super) async fn human_alert_page(
        &self,
        token: String,
        after: Option<String>,
        deadline: Instant,
    ) -> Result<AutomationPage<AlertState>> {
        let session = limeos_identity::digest(&token);
        self.history_store(deadline, move |store| {
            store.observe_health_alert_page(&session, after.as_deref(), now, deadline)
        })
        .await
    }

    pub(super) async fn human_notification_page(
        &self,
        token: String,
        after: Option<String>,
        deadline: Instant,
    ) -> Result<AutomationPage<Notification>> {
        let session = limeos_identity::digest(&token);
        self.history_store(deadline, move |store| {
            store.observe_health_notification_page(&session, after.as_deref(), now, deadline)
        })
        .await
    }

    pub(super) async fn human_notification_acknowledge(
        &self,
        token: String,
        csrf: String,
        id: String,
        input: AcknowledgeNotification,
        deadline: Instant,
    ) -> Result<Notification> {
        let session = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.history_store(deadline, move |store| {
            store.observe_health_notification_acknowledge(
                &session, &csrf, &id, &input, now, deadline,
            )
        })
        .await
    }
}

#[cfg(test)]
mod tests;
