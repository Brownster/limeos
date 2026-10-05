use limeos_executor_container::{Engine, ReceiptStore, RestartCeiling, docker::Docker};
use limeos_executor_protocol::{Ceiling, Request};
use std::{os::unix::fs::MetadataExt, path::Path, sync::Arc};
use tokio::{
    net::UnixListener,
    sync::{Mutex, Semaphore},
};
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.iter().any(|s| matches!(s.as_str(), "--help" | "-h")) {
        println!(
            "Usage: limeos-executor POLICY SOCKET host|container|storage [RECEIPT_DIRECTORY [read-only]]\n       limeos-executor storage-check|storage-wait --plan ROOT_OWNED_JSON\n       limeos-executor storage-inventory\n       limeos-executor storage-target-plan --contract ROOT_OWNED_JSON\n       limeos-executor storage-prepare-targets --plan ROOT_OWNED_JSON\n       limeos-executor storage-target-receipt|storage-reconcile-targets --action ACTION_ID\n\nRoot operator preparation creates empty mount target directories only.\nReconciliation observes targets and updates receipts. Other storage commands inspect.\nStdout is JSON; errors go to stderr.\nExit: 0 verified, 1 unsafe/unavailable/uncertain, 2 invalid plan/usage, 3 deadline expired."
        );
        return;
    }
    if args.get(1).is_some_and(|s| s == "--version") && args.len() == 2 {
        println!("limeos-executor {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.get(1).is_some_and(|s| s.starts_with("storage-")) {
        storage_cli(&args).await;
        return;
    }
    tracing_subscriber::fmt().json().with_target(false).init();
    if run().await.is_err() {
        tracing::error!("executor configuration or runtime failure");
        std::process::exit(1);
    }
}
async fn storage_cli(args: &[String]) {
    use limeos_executor_storage::{Failure, read_plan, verify, wait};
    let result = async {
        if args.len() == 4 && args[1] == "storage-reconcile-targets" && args[2] == "--action" {
            if std::env::current_exe().map_err(|_| Failure::Unavailable)?
                != Path::new("/usr/lib/limeos/limeos-executor")
            {
                return Err(Failure::Unavailable);
            }
            let value = tokio::time::timeout(
                std::time::Duration::from_secs(12),
                limeos_executor_storage::reconcile_targets(&args[3]),
            )
            .await
            .map_err(|_| Failure::TimedOut)??;
            return serde_json::to_value(value).map_err(|_| Failure::Unavailable);
        }
        if args.len() == 4 && args[1] == "storage-target-receipt" && args[2] == "--action" {
            return serde_json::to_value(limeos_executor_storage::target_receipt(&args[3])?)
                .map_err(|_| Failure::Unavailable);
        }
        if args.len() == 4 && args[1] == "storage-target-plan" && args[2] == "--contract" {
            let contract = limeos_executor_storage::read_contract(Path::new(&args[3]))?;
            let value = tokio::time::timeout(
                std::time::Duration::from_secs(12),
                limeos_executor_storage::target_plan(&contract),
            )
            .await
            .map_err(|_| Failure::TimedOut)??;
            return serde_json::to_value(value).map_err(|_| Failure::Unavailable);
        }
        if args.len() == 4 && args[1] == "storage-prepare-targets" && args[2] == "--plan" {
            // The shadow payload is read-only, including root operator commands.
            if std::env::current_exe().map_err(|_| Failure::Unavailable)?
                != Path::new("/usr/lib/limeos/limeos-executor")
            {
                return Err(Failure::Unavailable);
            }
            let plan = limeos_executor_storage::read_target_plan(Path::new(&args[3]))?;
            let value = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                limeos_executor_storage::prepare_targets(&plan),
            )
            .await
            .map_err(|_| Failure::TimedOut)??;
            let verified = matches!(
                value.state,
                limeos_executor_storage::PreparationState::Verified
            );
            println!(
                "{}",
                serde_json::to_string(&value).map_err(|_| Failure::Unavailable)?
            );
            std::process::exit(if verified { 0 } else { 1 });
        }
        if args.len() == 2 && args[1] == "storage-inventory" {
            let value = tokio::time::timeout(
                std::time::Duration::from_secs(4),
                limeos_executor_storage::inventory(),
            )
            .await
            .map_err(|_| Failure::TimedOut)??;
            return serde_json::to_value(value).map_err(|_| Failure::Unavailable);
        }
        if args.len() != 4
            || args[2] != "--plan"
            || !matches!(args[1].as_str(), "storage-check" | "storage-wait")
        {
            return Err(Failure::InvalidPlan);
        }
        let plan = read_plan(Path::new(&args[3]))?;
        let value = if args[1] == "storage-wait" {
            wait(&plan).await
        } else {
            tokio::time::timeout(
                std::time::Duration::from_secs(u64::from(plan.timeout_seconds)),
                verify(&plan),
            )
            .await
            .map_err(|_| Failure::TimedOut)?
        }?;
        serde_json::to_value(value).map_err(|_| Failure::Unavailable)
    }
    .await;
    match result {
        Ok(snapshot) => println!(
            "{}",
            serde_json::to_string(&snapshot).expect("serializable storage snapshot")
        ),
        Err(error) => {
            eprintln!(
                "{}",
                serde_json::json!({"error": error, "message": error.message()})
            );
            std::process::exit(match error {
                Failure::InvalidPlan => 2,
                Failure::TimedOut => 3,
                _ => 1,
            });
        }
    }
}
async fn run() -> std::io::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if !(4..=6).contains(&args.len()) {
        return Err(std::io::Error::other(
            "usage: limeos-executor POLICY SOCKET host|container|storage [RECEIPT_DIRECTORY [read-only]]",
        ));
    }
    let policy = Path::new(&args[1]);
    let meta = std::fs::symlink_metadata(policy)?;
    if !meta.is_file() || meta.uid() != 0 || meta.mode() & 0o022 != 0 || meta.len() > 65536 {
        return Err(std::io::Error::other("unsafe ceiling"));
    }
    let ceiling: Ceiling =
        serde_json::from_slice(&std::fs::read(policy)?).map_err(std::io::Error::other)?;
    if !ceiling.configuration_valid()
        || (args.len() == 6 && (args[5] != "read-only" || ceiling.can_write()))
    {
        return Err(std::io::Error::other("invalid or writable shadow ceiling"));
    }
    let ceiling = Arc::new(ceiling);
    let source = match args[3].as_str() {
        "host" | "storage" => limeos_contracts::Source::Host,
        "container" => limeos_contracts::Source::Docker,
        _ => return Err(std::io::Error::other("invalid executor kind")),
    };
    let storage_reader = args[3] == "storage";
    if ceiling.allow_storage_read != storage_reader || (storage_reader && args.len() != 4) {
        return Err(std::io::Error::other(
            "storage reader requires a dedicated read-only ceiling",
        ));
    }
    if source != limeos_contracts::Source::Docker
        && (ceiling.can_write() || ceiling.allow_container_logs)
    {
        return Err(std::io::Error::other(
            "host executor cannot mutate containers",
        ));
    }
    let restart_ceiling = Arc::new(RestartCeiling {
        version: ceiling.version,
        core_uid: ceiling.core_uid,
        allow_restart: ceiling.allow_restart,
        allow_start: ceiling.allow_start,
        allow_stop: ceiling.allow_stop,
        managed_containers: ceiling.managed_containers.clone(),
    });
    let receipts = if ceiling.can_write() {
        let directory = args
            .get(4)
            .map(String::as_str)
            .unwrap_or("/var/lib/limeos/executors/container");
        Some(Arc::new(Mutex::new(
            ReceiptStore::open(Path::new(directory))
                .map_err(|_| std::io::Error::other("unsafe or unavailable receipt store"))?,
        )))
    } else {
        None
    };
    let engine = Docker::new("/var/run/docker.sock".into());
    // An independent ceiling can disable collection entirely.
    let observations = match source {
        limeos_contracts::Source::Host if ceiling.allow_host_read => {
            limeos_observations::start_host()
        }
        limeos_contracts::Source::Docker if ceiling.allow_container_read => {
            limeos_observations::start_docker("/var/run/docker.sock".into())
        }
        _ => limeos_observations::Cache::new(vec![source]),
    };
    let path = Path::new(&args[2]);
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        use std::os::unix::fs::FileTypeExt;
        if !meta.file_type().is_socket() || std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Err(std::io::Error::other("socket conflict"));
        }
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o660))?;
    let permits = Arc::new(Semaphore::new(8));
    let storage_permit = Arc::new(Mutex::new(()));
    loop {
        let (mut stream, _) = listener.accept().await?;
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            continue;
        };
        let ceiling = ceiling.clone();
        let observations = observations.clone();
        let receipts = receipts.clone();
        let restart_ceiling = restart_ceiling.clone();
        let engine = engine.clone();
        let storage_permit = storage_permit.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let result = async {
                let peer = stream
                    .peer_cred()
                    .map_err(|_| limeos_domain::Error(limeos_domain::ErrorCode::Forbidden))?;
                // Reject unauthenticated peers before parsing or allocating a frame.
                if peer.uid() != ceiling.core_uid {
                    return Err(limeos_domain::Error(limeos_domain::ErrorCode::Forbidden));
                }
                let request: Request = tokio::time::timeout(
                    limeos_contracts::RPC_DEADLINE,
                    limeos_contracts::read_frame(&mut stream),
                )
                .await
                .map_err(|_| limeos_domain::Error(limeos_domain::ErrorCode::Unavailable))??;
                let mut receipt = ceiling.validate(peer.uid(), &request)?;
                match request {
                    Request::StorageInventory { .. } if storage_reader => {
                        let _single_flight = storage_permit.try_lock().map_err(|_| {
                            limeos_domain::Error(limeos_domain::ErrorCode::Overloaded)
                        })?;
                        receipt.storage = Some(
                            tokio::time::timeout(
                                std::time::Duration::from_secs(4),
                                limeos_executor_storage::inventory(),
                            )
                            .await
                            .map_err(|_| {
                                limeos_domain::Error(limeos_domain::ErrorCode::Unavailable)
                            })?
                            .map_err(|_| {
                                limeos_domain::Error(limeos_domain::ErrorCode::Unavailable)
                            })?,
                        );
                    }
                    Request::Observe {
                        source: requested,
                        active,
                        ..
                    } => {
                        if requested != source {
                            return Err(limeos_domain::Error(limeos_domain::ErrorCode::Forbidden));
                        }
                        if active {
                            observations.demand();
                        }
                        let value = observations.snapshot();
                        receipt.observations = Some(limeos_contracts::ObservationBatch {
                            host: value.host,
                            resources: value.resources,
                            sources: value.sources,
                        });
                    }
                    Request::Inspect { resource, .. }
                        if source == limeos_contracts::Source::Docker =>
                    {
                        receipt.inspection = Some(
                            tokio::time::timeout(
                                std::time::Duration::from_secs(5),
                                engine.inspect(&resource[10..]),
                            )
                            .await
                            .map_err(|_| {
                                limeos_domain::Error(limeos_domain::ErrorCode::Unavailable)
                            })??,
                        );
                    }
                    Request::ContainerLogs {
                        resource, options, ..
                    } if source == limeos_contracts::Source::Docker => {
                        receipt.logs = Some(engine.logs(&resource, options).await?);
                    }
                    Request::Restart { request, .. }
                    | Request::ExecuteContainer { request, .. }
                        if source == limeos_contracts::Source::Docker =>
                    {
                        let store = receipts
                            .as_ref()
                            .ok_or(limeos_domain::Error(limeos_domain::ErrorCode::Forbidden))?;
                        // Do not wait behind another effect with an expiring plan.
                        let mut store = store.try_lock().map_err(|_| {
                            limeos_domain::Error(limeos_domain::ErrorCode::Overloaded)
                        })?;
                        receipt.restart = Some(
                            store
                                .execute(&engine, &restart_ceiling, peer.uid(), &request, clock)
                                .await?,
                        );
                    }
                    Request::RestartReceipt { action, digest, .. } => {
                        let store = receipts
                            .as_ref()
                            .ok_or(limeos_domain::Error(limeos_domain::ErrorCode::Forbidden))?;
                        let store = store.try_lock().map_err(|_| {
                            limeos_domain::Error(limeos_domain::ErrorCode::Overloaded)
                        })?;
                        receipt.restart = store.receipt(&action, &digest)?;
                        if let Some(value) = &receipt.restart {
                            restart_ceiling.authorize(
                                peer.uid(),
                                &value.before.resource,
                                value.operation,
                            )?;
                        }
                    }
                    Request::VerifyRestart {
                        action,
                        digest,
                        after,
                        ..
                    } => {
                        let store = receipts
                            .as_ref()
                            .ok_or(limeos_domain::Error(limeos_domain::ErrorCode::Forbidden))?;
                        let mut store = store.try_lock().map_err(|_| {
                            limeos_domain::Error(limeos_domain::ErrorCode::Overloaded)
                        })?;
                        receipt.restart = Some(
                            store
                                .verify(
                                    &engine,
                                    &restart_ceiling,
                                    peer.uid(),
                                    &action,
                                    &digest,
                                    &after,
                                )
                                .await?,
                        );
                    }
                    Request::Health { .. } => {}
                    _ => return Err(limeos_domain::Error(limeos_domain::ErrorCode::Forbidden)),
                }
                Ok::<_, limeos_domain::Error>(receipt)
            }
            .await;
            let receipt = result.unwrap_or_else(|e| {
                let mut value = limeos_executor_protocol::Receipt::empty();
                value.ready = false;
                value.error = Some(e.0);
                value
            });
            // Responses are bounded even if the caller stops reading.
            let _ = tokio::time::timeout(
                limeos_contracts::RPC_DEADLINE,
                limeos_contracts::write_frame(&mut stream, &receipt),
            )
            .await;
        });
    }
}
fn clock() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}
