//! Fixed installed nonpersistent mount service with a separately guarded helper.
//! Only protected selection, genuine admission and owned helper termination can
//! reach execution/terminal receipt states. Wire facts cannot reconstruct them.
use crate::storage_mount_admission::{self, MountAdmission};
use crate::storage_mount_supervision::GatedMountHelper;
use limeos_contracts::{RPC_DEADLINE, VERSION, read_frame, write_frame};
use limeos_domain::{
    Error, ErrorCode, MountExecutionState, Result, StorageMountEffectPlan, StorageMountReceipt,
    StorageMountSelection, verify_storage_mount,
};
use limeos_executor_protocol::{Receipt, Request, StorageMountCeiling};
use limeos_executor_storage::effects::MountJournal;
use sha2::{Digest, Sha256};
use std::{
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::net::UnixListener;

const POLICY: &str = "/etc/limeos/system-policy/storage-mounts.json";
const SOCKET: &str = "/run/limeos-storage-mounts/executor.sock";
const ROLE: &str = "storage-mounts";
const OPERATION_DEADLINE: Duration = Duration::from_secs(30);

pub(super) async fn run(args: &[String]) -> std::io::Result<()> {
    if args.len() != 4
        || args[1] != POLICY
        || args[2] != SOCKET
        || args[3] != ROLE
        || !rustix::process::geteuid().is_root()
        || std::env::current_exe()? != Path::new("/usr/lib/limeos/limeos-executor")
    {
        return Err(std::io::Error::other(
            "mounts require the standard installed root service",
        ));
    }
    policy().map_err(|_| std::io::Error::other("unsafe mount ceiling"))?;
    for directory in ["/run", "/run/limeos-storage-mounts"] {
        let meta = std::fs::symlink_metadata(directory)?;
        if !meta.is_dir() || meta.uid() != 0 || meta.mode() & 0o022 != 0 {
            return Err(std::io::Error::other("unsafe mount socket directory"));
        }
    }
    if let Ok(meta) = std::fs::symlink_metadata(SOCKET) {
        if !meta.file_type().is_socket()
            || meta.uid() != 0
            || meta.mode() & 0o007 != 0
            || std::os::unix::net::UnixStream::connect(SOCKET).is_ok()
        {
            return Err(std::io::Error::other("mount socket conflict"));
        }
        std::fs::remove_file(SOCKET)?;
    }
    let listener = UnixListener::bind(SOCKET)?;
    std::fs::set_permissions(SOCKET, std::fs::Permissions::from_mode(0o660))?;
    // One operation lives in this process at a time. Genuine retained owners are
    // not moved into arbitrary spawned tasks; other clients remain bounded by
    // the socket backlog/frame deadlines, never parallel privileged helpers.
    loop {
        let (mut stream, _) = listener.accept().await?;
        let result: Result<Receipt> = async {
            let uid = stream
                .peer_cred()
                .map_err(|_| Error(ErrorCode::Forbidden))?
                .uid();
            policy()?.authorize_peer(uid)?;
            let request = tokio::time::timeout(RPC_DEADLINE, read_frame::<Request>(&mut stream))
                .await
                .map_err(|_| Error(ErrorCode::Unavailable))??;
            let deadline = Instant::now() + OPERATION_DEADLINE;
            let cancelled = || Instant::now() >= deadline;
            // This bounds cooperative async work. Synchronous kernel reads/fsync
            // additionally need independent service supervision qualification;
            // the helper has its own genuine guardian/termination contract.
            tokio::time::timeout(OPERATION_DEADLINE, dispatch(uid, request, &cancelled))
                .await
                .map_err(|_| Error(ErrorCode::Unavailable))?
        }
        .await;
        let reply = result.unwrap_or_else(|error| {
            let mut reply = Receipt::empty();
            reply.ready = false;
            reply.error = Some(error.0);
            reply
        });
        let _ = tokio::time::timeout(RPC_DEADLINE, write_frame(&mut stream, &reply)).await;
    }
}

async fn dispatch(uid: u32, request: Request, cancelled: &dyn Fn() -> bool) -> Result<Receipt> {
    let ceiling = policy()?;
    ceiling.authorize_peer(uid)?;
    let mut reply = Receipt::empty();
    match request {
        Request::StorageMountCatalog { version } if version == VERSION => {
            let mut catalog = storage_mount_admission::catalog(cancelled).await?;
            catalog.validate()?;
            // The protected configuration defines devices; the independently
            // loaded root ceiling can only narrow those exact named choices.
            let current = policy()?;
            current.authorize_peer(uid)?;
            catalog.targets.retain(|device| {
                current.managed_targets.iter().any(|target| {
                    target.target_id == device.id
                        && target.filesystem_uuid == device.filesystem_uuid
                        && target.mountpoint == device.mountpoint
                })
            });
            catalog.validate()?;
            reply.storage_mount_catalog = Some(catalog);
        }
        Request::InspectStorageMount { version, selection } if version == VERSION => {
            ceiling.authorize_selection(uid, &selection)?;
            let admission = storage_mount_admission::inspect(&selection, cancelled).await?;
            admission.revalidate(cancelled).await?;
            let current = policy()?;
            let target = current.authorize_selection(uid, &selection)?;
            let facts = admission.inspection();
            facts.validate(now()?)?;
            if facts.target_id != target.target_id
                || facts.device.filesystem_uuid != target.filesystem_uuid
                || facts.device.mountpoint != target.mountpoint
            {
                return Err(Error(ErrorCode::Conflict));
            }
            reply.storage_mount_inspection = Some(facts.clone());
        }
        Request::ExecuteStorageMount { version, plan } if version == VERSION => {
            reply.storage_mount = Some(execute(uid, &plan, cancelled).await?);
        }
        Request::StorageMountReceipt {
            version,
            action,
            digest,
        } if version == VERSION => {
            let journal = MountJournal::readonly().map_err(map_failure)?;
            reply.storage_mount = Some(
                journal
                    .lookup(&action, Some(&digest))
                    .map_err(map_failure)?
                    .ok_or(Error(ErrorCode::NotFound))?,
            );
        }
        Request::ReconcileStorageMount {
            version,
            action,
            digest,
        } if version == VERSION => {
            reply.storage_mount = Some(reconcile_accepted(uid, &action, &digest, cancelled).await?);
        }
        Request::InspectMountedStorage { version, plan } if version == VERSION => {
            plan.validate(plan.created_at)?;
            let selection = StorageMountSelection {
                target_id: plan.device.id.clone(),
            };
            ceiling.authorize_selection(uid, &selection)?;
            let journal = MountJournal::readonly().map_err(map_failure)?;
            let receipt = journal
                .lookup(&plan.action, Some(&effect_digest(&plan)?))
                .map_err(map_failure)?
                .ok_or(Error(ErrorCode::NotFound))?;
            if receipt.plan != *plan {
                return Err(Error(ErrorCode::Conflict));
            }
            if receipt.state == MountExecutionState::PreconditionChanged {
                let current = storage_mount_admission::inspect_unmounted(&plan, cancelled).await?;
                plan.check_reconciliation(&current, now()?)?;
                reply.storage_mount_inspection = Some(current);
            } else {
                reply.mounted_storage =
                    Some(storage_mount_admission::inspect_mounted(&plan, cancelled).await?);
            }
            policy()?.authorize_selection(uid, &selection)?;
        }
        _ => return Err(Error(ErrorCode::Forbidden)),
    }
    Ok(reply)
}

async fn execute(
    uid: u32,
    plan: &StorageMountEffectPlan,
    cancelled: &dyn Fn() -> bool,
) -> Result<StorageMountReceipt> {
    plan.validate(plan.created_at)?;
    let digest = effect_digest(plan)?;
    let mut journal = MountJournal::open().map_err(map_failure)?;
    // Receipt replay precedes fresh execution checks. It never reopens targets,
    // launches a helper, refreshes expiry or mints another launch permit.
    if let Some(receipt) = journal
        .lookup(&plan.action, Some(&digest))
        .map_err(map_failure)?
    {
        return Ok(receipt);
    }
    policy()?.authorize_plan(uid, plan, now()?)?;
    let selection = StorageMountSelection {
        target_id: plan.device.id.clone(),
    };
    let admission: MountAdmission = storage_mount_admission::inspect(&selection, cancelled).await?;
    admission.revalidate(cancelled).await?;
    plan.check_current(admission.inspection(), now()?)?;
    policy()?.authorize_plan(uid, plan, now()?)?;
    let prepared = journal.begin(plan, now()?).map_err(map_failure)?;
    let mut receipt = journal.arm(&prepared).map_err(map_failure)?;
    let gated = match GatedMountHelper::prepare(admission, &plan.action, &digest, cancelled).await {
        Ok(helper) => helper,
        Err(_) => return unknown(&mut journal, &receipt),
    };
    let (bound, permit) = match journal.bind_helper(&receipt, gated.identity().clone()) {
        Ok(value) => value,
        // The guard owns cleanup; a failed durable identity commit must not
        // release GO. Persisted Pending and core claims survive any failure.
        Err(error) => return Err(map_failure(error)),
    };
    receipt = bound;
    if policy()
        .and_then(|ceiling| ceiling.authorize_plan(uid, plan, now()?))
        .is_err()
        || cancelled()
    {
        drop(permit);
        drop(gated);
        return unknown(&mut journal, &receipt);
    }
    let running = match gated.release(permit, cancelled).await {
        Ok(helper) => helper,
        Err(_) => return unknown(&mut journal, &receipt),
    };
    let quiescence = match running.await_quiescence(cancelled).await {
        Ok(proof) => proof,
        Err(_) => return unknown(&mut journal, &receipt),
    };
    // Only an opaque owned-child/guardian cleanup token supplies these facts.
    // No request contains a receipt, Reaped value or no-helper boolean.
    let mut stopped = receipt.clone();
    stopped.quiescence = quiescence.wire_facts().clone();
    journal.record(&receipt, &stopped).map_err(map_failure)?;
    receipt = stopped;
    if !receipt.quiescence.succeeded() {
        return reconcile_quiescent(&mut journal, &receipt, cancelled).await;
    }
    // A real mount covers the original empty target. Do not revalidate the
    // unmounted/empty named inode after the effect. Fresh independent mounted
    // inspection binds its actual mount ID, filesystem root, UUID and flags.
    let observed = match storage_mount_admission::inspect_mounted(plan, cancelled).await {
        Ok(value) => value,
        Err(_) => return unknown(&mut journal, &receipt),
    };
    let mut accepted = receipt.clone();
    accepted.state = MountExecutionState::EffectAccepted;
    accepted.after = Some(observed);
    journal.record(&receipt, &accepted).map_err(map_failure)?;
    receipt = accepted;
    // Publish accepted effect before a separate fresh verification. A crash here
    // leaves EffectAccepted, never a fabricated successful terminal result.
    let independent = match storage_mount_admission::inspect_mounted(plan, cancelled).await {
        Ok(value) => value,
        Err(_) => return unknown(&mut journal, &receipt),
    };
    if verify_storage_mount(plan, &receipt, &independent).is_err() {
        return unknown(&mut journal, &receipt);
    }
    let mut verified = receipt.clone();
    verified.state = MountExecutionState::Verified;
    journal.record(&receipt, &verified).map_err(map_failure)?;
    Ok(verified)
}

async fn reconcile_quiescent(
    journal: &mut MountJournal,
    receipt: &StorageMountReceipt,
    cancelled: &dyn Fn() -> bool,
) -> Result<StorageMountReceipt> {
    if !receipt.quiescence.is_quiescent() {
        return unknown(journal, receipt);
    }
    let current = match storage_mount_admission::inspect_unmounted(&receipt.plan, cancelled).await {
        Ok(value) => value,
        Err(_) => return unknown(journal, receipt),
    };
    if receipt.plan.check_reconciliation(&current, now()?).is_err() {
        return unknown(journal, receipt);
    }
    let mut no_effect = receipt.clone();
    no_effect.state = MountExecutionState::PreconditionChanged;
    no_effect.after = None;
    journal.record(receipt, &no_effect).map_err(map_failure)?;
    Ok(no_effect)
}
async fn reconcile_accepted(
    uid: u32,
    action: &str,
    digest: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<StorageMountReceipt> {
    let mut journal = MountJournal::open().map_err(map_failure)?;
    let receipt = journal
        .lookup(action, Some(digest))
        .map_err(map_failure)?
        .ok_or(Error(ErrorCode::NotFound))?;
    // Terminal lookup is idempotent; Pending/Unknown/Prepared are never promoted
    // by an absent mount, a caller-supplied fact, or this narrow refinement.
    if receipt.state == MountExecutionState::Verified {
        return Ok(receipt);
    }
    if receipt.state != MountExecutionState::EffectAccepted
        || !receipt.quiescence.succeeded()
        || receipt.after.is_none()
    {
        return Err(Error(ErrorCode::Conflict));
    }
    authorize_reconciliation(uid, &receipt.plan)?;
    let independent = storage_mount_admission::inspect_mounted(&receipt.plan, cancelled).await?;
    verify_storage_mount(&receipt.plan, &receipt, &independent)?;
    authorize_reconciliation(uid, &receipt.plan)?;
    journal
        .verify_accepted(&receipt, &independent)
        .map_err(map_failure)
}
fn authorize_reconciliation(uid: u32, plan: &StorageMountEffectPlan) -> Result<()> {
    // Read/verification authority survives write-ceiling revocation. Exact
    // protected configured identity still binds this historical operation.
    plan.validate(plan.created_at)?;
    let ceiling = policy()?;
    let target = ceiling.authorize_selection(
        uid,
        &StorageMountSelection {
            target_id: plan.device.id.clone(),
        },
    )?;
    if target.filesystem_uuid != plan.device.filesystem_uuid
        || target.mountpoint != plan.device.mountpoint
    {
        return Err(Error(ErrorCode::Conflict));
    }
    Ok(())
}
fn unknown(
    journal: &mut MountJournal,
    receipt: &StorageMountReceipt,
) -> Result<StorageMountReceipt> {
    let mut next = receipt.clone();
    next.state = MountExecutionState::OutcomeUnknown;
    journal.record(receipt, &next).map_err(map_failure)?;
    Ok(next)
}
fn effect_digest(plan: &StorageMountEffectPlan) -> Result<String> {
    plan.validate(plan.created_at)?;
    let bytes = serde_json::to_vec(plan).map_err(|_| Error(ErrorCode::InvalidInput))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
fn policy() -> Result<StorageMountCeiling> {
    let ceiling: StorageMountCeiling =
        limeos_executor_storage::read_protected_json(Path::new(POLICY)).map_err(map_failure)?;
    if !ceiling.configuration_valid() {
        return Err(Error(ErrorCode::Forbidden));
    }
    Ok(ceiling)
}
fn now() -> Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|t| i64::try_from(t.as_secs()).ok())
        .ok_or(Error(ErrorCode::Unavailable))
}
fn map_failure(failure: limeos_executor_storage::Failure) -> Error {
    use limeos_executor_storage::Failure;
    Error(match failure {
        Failure::StateNotDurable => ErrorCode::StateNotDurable,
        Failure::InvalidPlan => ErrorCode::InvalidInput,
        Failure::Unavailable | Failure::TimedOut | Failure::NotReady => ErrorCode::Unavailable,
        _ => ErrorCode::Conflict,
    })
}

#[cfg(test)]
mod tests;
