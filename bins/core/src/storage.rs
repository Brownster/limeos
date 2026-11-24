use super::*;
use limeos_domain::{
    PlanApproval, PlannedStorageSetup, Principal, StorageInventory, StorageInventoryView,
    StorageSetupInput,
};
pub(super) fn container_storage_receipt(
    receipt: limeos_executor_protocol::Receipt,
    requested_at: i64,
    received_at: i64,
) -> Result<limeos_domain::ContainerStorageInventory> {
    if receipt.version != VERSION {
        return Err(Error(ErrorCode::Unavailable));
    }
    if let Some(code) = receipt.error {
        return Err(Error(code));
    }
    if !receipt.ready {
        return Err(Error(ErrorCode::Unavailable));
    }
    let inventory = receipt
        .container_storage
        .ok_or(Error(ErrorCode::Unavailable))?;
    inventory.validate(received_at)?;
    if inventory.observed_at < requested_at {
        return Err(Error(ErrorCode::Conflict));
    }
    Ok(inventory)
}
/// How long a request someone is waiting on queues for the storage read
/// gate: past the longest background hold (the 1 Hz health read holds it
/// for at most its 4 s window; a pass is usually far shorter), so a request
/// queued behind a full-window read is still served, and within the 10 s
/// HTTP handler deadline together with the 5 s read that follows.
pub(super) const INTERACTIVE_WAIT: std::time::Duration =
    crate::health_collection::WINDOW.saturating_add(std::time::Duration::from_millis(500));
const _: () = assert!(
    INTERACTIVE_WAIT.as_millis() > crate::health_collection::WINDOW.as_millis()
        && INTERACTIVE_WAIT.as_millis() + limeos_contracts::RPC_DEADLINE.as_millis()
            < limeos_api::HANDLER_DEADLINE.as_millis()
);
/// The gate, queued with a deadline. The 1 Hz background health read only
/// ever takes a free gate, so a waiting request is served next.
pub(super) async fn interactive_permit(
    gate: &Arc<Semaphore>,
    wait: std::time::Duration,
) -> Result<tokio::sync::OwnedSemaphorePermit> {
    tokio::time::timeout(wait, gate.clone().acquire_owned())
        .await
        .map_err(|_| Error(ErrorCode::Overloaded))?
        .map_err(|_| Error(ErrorCode::Unavailable))
}
/// The rule core trusts an unprivileged executor's socket by (the container
/// executor's and the storage reader's), for this process's UID.
pub(super) fn executor_owner() -> limeos_contracts::ExecutorOwner {
    limeos_contracts::ExecutorOwner::new(rustix::process::geteuid().as_raw())
}
impl Core {
    pub(super) async fn interactive_storage_read(
        &self,
    ) -> Result<tokio::sync::OwnedSemaphorePermit> {
        interactive_permit(&self.storage_readers, INTERACTIVE_WAIT).await
    }
    pub(super) async fn fresh_container_storage(
        &self,
    ) -> Result<limeos_domain::ContainerStorageInventory> {
        let _permit = self.interactive_storage_read().await?;
        let requested_at = now();
        tokio::time::timeout(limeos_contracts::RPC_DEADLINE, async {
            let mut stream = executor_owner()
                .connect(self.container_socket.as_ref())
                .await?;
            limeos_contracts::write_frame(
                &mut stream,
                &limeos_executor_protocol::Request::ContainerStorageInventory { version: VERSION },
            )
            .await?;
            let receipt: limeos_executor_protocol::Receipt =
                limeos_contracts::read_frame(&mut stream).await?;
            container_storage_receipt(receipt, requested_at, now())
        })
        .await
        .map_err(|_| Error(ErrorCode::Unavailable))?
    }
    pub(super) async fn human_container_storage(
        &self,
        token: String,
    ) -> Result<limeos_domain::ContainerStorageInventoryView> {
        self.storage_principal(token.clone(), None).await?;
        let inventory = self.fresh_container_storage().await?;
        self.storage_principal(token, None).await?;
        inventory.validate(now())?;
        let body = serde_json::to_string(&inventory).map_err(|_| Error(ErrorCode::Unavailable))?;
        Ok(limeos_domain::ContainerStorageInventoryView {
            inventory,
            digest: limeos_identity::digest(&body),
        })
    }
    pub(super) async fn storage_principal(
        &self,
        token: String,
        csrf: Option<String>,
    ) -> Result<Principal> {
        let digest = limeos_identity::digest(&token);
        let csrf = csrf.map(|s| limeos_identity::digest(&s));
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, csrf.as_deref(), now())?;
                s.authorize_storage(&principal)?;
                Ok(principal)
            })
            .await
    }
    pub(super) async fn fresh_storage(&self) -> Result<StorageInventory> {
        let _permit = self.interactive_storage_read().await?;
        tokio::time::timeout(limeos_contracts::RPC_DEADLINE, async {
            // The reader runs as its own account, as the container executor
            // does, and is trusted by the same rule: kernel credentials on
            // both ends, the socket's owner neither root nor core. Only that
            // account or root can create its socket, so no other account,
            // and not core itself, can supply disk or fstab evidence.
            let mut stream = executor_owner()
                .connect(self.storage_socket.as_ref())
                .await?;
            limeos_contracts::write_frame(
                &mut stream,
                &limeos_executor_protocol::Request::StorageInventory { version: VERSION },
            )
            .await?;
            let receipt: limeos_executor_protocol::Receipt =
                limeos_contracts::read_frame(&mut stream).await?;
            if receipt.version != VERSION || !receipt.ready {
                return Err(Error(ErrorCode::Unavailable));
            }
            if let Some(error) = receipt.error {
                return Err(Error(error));
            }
            let inventory = receipt.storage.ok_or(Error(ErrorCode::Unavailable))?;
            inventory.validate()?;
            Ok(inventory)
        })
        .await
        .map_err(|_| Error(ErrorCode::Unavailable))?
    }
    pub(super) async fn human_storage_inventory(
        &self,
        token: String,
    ) -> Result<StorageInventoryView> {
        self.storage_principal(token.clone(), None).await?;
        let inventory = self.fresh_storage().await?;
        self.storage_principal(token, None).await?;
        let body = serde_json::to_string(&inventory).map_err(|_| Error(ErrorCode::Unavailable))?;
        Ok(StorageInventoryView {
            inventory,
            digest: limeos_identity::digest(&body),
        })
    }
    pub(super) async fn human_plan_storage(
        &self,
        token: String,
        csrf: String,
        input: StorageSetupInput,
    ) -> Result<PlannedStorageSetup> {
        // Authenticated before the input is judged: an anonymous caller
        // learns nothing from its body.
        self.storage_principal(token.clone(), Some(csrf.clone()))
            .await?;
        input.validate()?;
        let inventory = self.fresh_storage().await?;
        let digest = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, Some(&csrf), now())?;
                s.plan_storage(&principal, &input, &inventory, now())
            })
            .await
    }
    pub(super) async fn human_approve_storage(
        &self,
        token: String,
        csrf: String,
        id: String,
        plan_digest: String,
    ) -> Result<PlanApproval> {
        let principal = self
            .storage_principal(token.clone(), Some(csrf.clone()))
            .await?;
        let plan_id = id.clone();
        self.db
            .call(move |s| s.storage_plan(&principal, &plan_id, now()))
            .await?;
        let inventory = self.fresh_storage().await?;
        let digest = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, Some(&csrf), now())?;
                s.approve_storage(&principal, &id, &plan_digest, &inventory, now())
            })
            .await
    }
}
