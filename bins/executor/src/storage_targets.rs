//! Dedicated root service; no mount, fstab, shell or reconciliation request.
use limeos_contracts::{RPC_DEADLINE, VERSION, read_frame, write_frame};
use limeos_domain::{Error, ErrorCode, Result};
use limeos_executor_protocol::{Receipt, Request, StorageTargetCeiling};
use std::{
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::Path,
    sync::Arc,
    time::Duration,
};
use tokio::{net::UnixListener, sync::Semaphore};

const POLICY: &str = "/etc/limeos/system-policy/storage-targets.json";

pub(super) async fn run(args: &[String]) -> std::io::Result<()> {
    if args.len() != 4
        || !rustix::process::geteuid().is_root()
        || std::env::current_exe()? != Path::new("/usr/lib/limeos/limeos-executor")
        || args[1] != POLICY
        || args[2] != "/run/limeos-storage-targets/executor.sock"
    {
        return Err(std::io::Error::other(
            "target preparation requires the standard installed root service",
        ));
    }
    // Read again for every request; this first read refuses to start on a
    // bad file.
    ceiling().map_err(|refusal| std::io::Error::other(format!("target ceiling: {refusal}")))?;
    let path = Path::new(&args[2]);
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if !meta.file_type().is_socket() || std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Err(std::io::Error::other("target socket conflict"));
        }
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660))?;
    let connections = Arc::new(Semaphore::new(4));
    let operation = Arc::new(Semaphore::new(1));
    // No watchdog: preparation's fsyncs and the inventory run on this loop's
    // thread and can legitimately outlast any keep-alive interval
    // (limeos-storage-targets.service says why).
    let mut watchdog = limeos_contracts::Watchdog::disabled();
    loop {
        let mut stream =
            limeos_executor::service::next_connection(&listener, &mut watchdog).await?;
        let Ok(connection) = connections.clone().try_acquire_owned() else {
            continue;
        };
        let operation = operation.clone();
        tokio::spawn(async move {
            let _connection = connection;
            let result: Result<Receipt> = async {
                let uid = stream
                    .peer_cred()
                    .map_err(|_| Error(ErrorCode::Forbidden))?
                    .uid();
                // The ceiling as the file says now, so a revoked target is
                // refused without a restart; no earlier copy is kept.
                let ceiling = ceiling().map_err(|refusal| refusal.error())?;
                if uid != ceiling.core_uid {
                    return Err(Error(ErrorCode::Forbidden));
                }
                let request: Request = tokio::time::timeout(RPC_DEADLINE, read_frame(&mut stream))
                    .await
                    .map_err(|_| Error(ErrorCode::Unavailable))??;
                let _operation = operation
                    .try_acquire()
                    .map_err(|_| Error(ErrorCode::Overloaded))?;
                let mut value = Receipt::empty();
                match request {
                    Request::InspectStorageTargets { version, contract } if version == VERSION => {
                        ceiling.authorize_contract(uid, &contract)?;
                        value.storage_targets = Some(
                            tokio::time::timeout(
                                Duration::from_secs(12),
                                limeos_executor_storage::target_snapshot(&contract),
                            )
                            .await
                            .map_err(|_| Error(ErrorCode::Unavailable))?
                            .map_err(map_failure)?,
                        );
                    }
                    Request::PrepareStorageTargets { version, plan } if version == VERSION => {
                        ceiling.authorize_contract(uid, &plan.contract)?;
                        value.target_preparation = Some(
                            tokio::time::timeout(
                                Duration::from_secs(30),
                                limeos_executor_storage::prepare_targets(&plan),
                            )
                            .await
                            .map_err(|_| Error(ErrorCode::Unavailable))?
                            .map_err(map_failure)?,
                        );
                    }
                    Request::StorageTargetReceipt {
                        version,
                        action,
                        digest,
                    } if version == VERSION => {
                        if !limeos_domain::opaque_id(&action) || !limeos_domain::opaque_id(&digest)
                        {
                            return Err(Error(ErrorCode::InvalidInput));
                        }
                        let receipt = limeos_executor_storage::target_receipt(&action)
                            .map_err(map_failure)?;
                        if receipt.digest != digest {
                            return Err(Error(ErrorCode::Conflict));
                        }
                        value.target_preparation = Some(receipt);
                    }
                    _ => return Err(Error(ErrorCode::Forbidden)),
                }
                Ok(value)
            }
            .await;
            let receipt = result.unwrap_or_else(|e| {
                let mut value = Receipt::empty();
                value.ready = false;
                value.error = Some(e.0);
                value
            });
            let _ = tokio::time::timeout(RPC_DEADLINE, write_frame(&mut stream, &receipt)).await;
        });
    }
}
/// Why the target ceiling was refused.
#[derive(Clone, Copy, Debug)]
enum Refusal {
    Unreadable(limeos_executor_storage::Failure),
    RootCore,
    Invalid,
}
impl Refusal {
    fn error(self) -> Error {
        match self {
            Refusal::Unreadable(failure) => map_failure(failure),
            Refusal::RootCore | Refusal::Invalid => Error(ErrorCode::Forbidden),
        }
    }
}
impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Unreadable(failure) => write!(f, "unsafe or unreadable: {}", failure.message()),
            Refusal::RootCore => f.write_str(
                "names core_uid 0; core runs as its own unprivileged user, so root is never accepted",
            ),
            Refusal::Invalid => f.write_str("fails validation"),
        }
    }
}
/// The root-owned target ceiling, read through the no-follow walk.
fn ceiling() -> std::result::Result<StorageTargetCeiling, Refusal> {
    let ceiling: StorageTargetCeiling =
        limeos_executor_storage::read_protected_json(Path::new(POLICY))
            .map_err(Refusal::Unreadable)?;
    if ceiling.core_uid == 0 {
        return Err(Refusal::RootCore);
    }
    if !ceiling.configuration_valid() {
        return Err(Refusal::Invalid);
    }
    Ok(ceiling)
}
fn map_failure(f: limeos_executor_storage::Failure) -> Error {
    use limeos_executor_storage::Failure;
    Error(match f {
        Failure::StateNotDurable => ErrorCode::StateNotDurable,
        Failure::InvalidPlan => ErrorCode::InvalidInput,
        Failure::Unavailable | Failure::TimedOut | Failure::NotReady => ErrorCode::Unavailable,
        _ => ErrorCode::Conflict,
    })
}
