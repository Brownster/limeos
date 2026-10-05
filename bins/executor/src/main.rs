use limeos_executor_protocol::{Ceiling, Request};
use std::{os::unix::fs::MetadataExt, path::Path, sync::Arc};
use tokio::{net::UnixListener, sync::Semaphore};
#[tokio::main(flavor = "current_thread")]
async fn main() {
    tracing_subscriber::fmt().json().with_target(false).init();
    if run().await.is_err() {
        tracing::error!("executor configuration or runtime failure");
        std::process::exit(1);
    }
}
async fn run() -> std::io::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err(std::io::Error::other(
            "usage: limeos-executor POLICY SOCKET host|container",
        ));
    }
    let policy = Path::new(&args[1]);
    let meta = std::fs::symlink_metadata(policy)?;
    if !meta.is_file() || meta.uid() != 0 || meta.mode() & 0o022 != 0 || meta.len() > 65536 {
        return Err(std::io::Error::other("unsafe ceiling"));
    }
    let ceiling: Ceiling =
        serde_json::from_slice(&std::fs::read(policy)?).map_err(std::io::Error::other)?;
    let ceiling = Arc::new(ceiling);
    let source = match args[3].as_str() {
        "host" => limeos_contracts::Source::Host,
        "container" => limeos_contracts::Source::Docker,
        _ => return Err(std::io::Error::other("invalid executor kind")),
    };
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
    loop {
        let (mut stream, _) = listener.accept().await?;
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            continue;
        };
        let ceiling = ceiling.clone();
        let observations = observations.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let _ = tokio::time::timeout(limeos_contracts::RPC_DEADLINE, async {
                let peer = stream
                    .peer_cred()
                    .map_err(|_| limeos_domain::Error(limeos_domain::ErrorCode::Forbidden))?;
                // Reject unauthenticated peers before parsing or allocating a frame.
                if peer.uid() != ceiling.core_uid {
                    return Err(limeos_domain::Error(limeos_domain::ErrorCode::Forbidden));
                }
                let request: Request = limeos_contracts::read_frame(&mut stream).await?;
                let mut receipt = ceiling.validate(peer.uid(), &request)?;
                if let Request::Observe {
                    source: requested,
                    active,
                    ..
                } = request
                {
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
                limeos_contracts::write_frame(&mut stream, &receipt).await
            })
            .await;
        });
    }
}
