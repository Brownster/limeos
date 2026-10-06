use super::*;
use limeos_domain::{
    PlanApproval, PlannedStorageSetup, Principal, StorageInventory, StorageInventoryView,
    StorageSetupInput,
};
impl Core {
    pub(super) async fn fresh_container_storage(
        &self,
    ) -> Result<limeos_domain::ContainerStorageInventory> {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        let _permit = self
            .storage_readers
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error(ErrorCode::Overloaded))?;
        let requested_at = now();
        tokio::time::timeout(limeos_contracts::RPC_DEADLINE, async {
            let owner = std::fs::symlink_metadata(self.container_socket.as_ref())
                .map_err(|_| Error(ErrorCode::Unavailable))?;
            let core_uid = std::fs::metadata("/proc/self")
                .map_err(|_| Error(ErrorCode::Unavailable))?
                .uid();
            if !owner.file_type().is_socket() || owner.uid() == 0 || owner.uid() == core_uid {
                return Err(Error(ErrorCode::Forbidden));
            }
            let mut stream = UnixStream::connect(self.container_socket.as_ref())
                .await
                .map_err(|_| Error(ErrorCode::Unavailable))?;
            if stream
                .peer_cred()
                .map_err(|_| Error(ErrorCode::Unavailable))?
                .uid()
                != owner.uid()
            {
                return Err(Error(ErrorCode::Forbidden));
            }
            limeos_contracts::write_frame(
                &mut stream,
                &limeos_executor_protocol::Request::ContainerStorageInventory { version: VERSION },
            )
            .await?;
            let receipt: limeos_executor_protocol::Receipt =
                limeos_contracts::read_frame(&mut stream).await?;
            if receipt.version != VERSION || !receipt.ready {
                return Err(Error(ErrorCode::Unavailable));
            }
            if let Some(code) = receipt.error {
                return Err(Error(code));
            }
            let inventory = receipt
                .container_storage
                .ok_or(Error(ErrorCode::Unavailable))?;
            inventory.validate(now())?;
            if inventory.observed_at < requested_at {
                return Err(Error(ErrorCode::Conflict));
            }
            Ok(inventory)
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
        let _permit = self
            .storage_readers
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error(ErrorCode::Overloaded))?;
        tokio::time::timeout(limeos_contracts::RPC_DEADLINE, async {
            let mut stream = UnixStream::connect(self.storage_socket.as_ref())
                .await
                .map_err(|_| Error(ErrorCode::Unavailable))?;
            // Kernel credentials on both ends; an unprivileged replacement
            // socket cannot supply disk/fstab evidence to the core.
            if stream
                .peer_cred()
                .map_err(|_| Error(ErrorCode::Unavailable))?
                .uid()
                != 0
            {
                return Err(Error(ErrorCode::Forbidden));
            }
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
        input.validate()?;
        self.storage_principal(token.clone(), Some(csrf.clone()))
            .await?;
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
