use limeos_api::{Backend, IssuedSession};
use limeos_contracts::{
    CoreRequest, CoreResponse, Enrollment, ErrorEnvelope, Health, Login, SessionView, VERSION,
};
use limeos_domain::{Error, ErrorCode, Result, Scope};
use limeos_persistence::{
    Database,
    config::{self, ConfigRead},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    net::{TcpListener, UnixListener, UnixStream},
    sync::Semaphore,
};
mod compose;
mod operations;

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}
#[derive(Clone)]
struct Core {
    db: Database,
    password_workers: Arc<Semaphore>,
    dummy_hash: Arc<str>,
    observations: limeos_observations::Cache,
    telemetry: Option<limeos_observations::telemetry::Telemetry>,
    container_socket: Arc<PathBuf>,
    compose_catalog: Option<Arc<limeos_domain::ComposeCatalog>>,
}
impl Core {
    async fn password(
        &self,
        request: limeos_contracts::PasswordRequest,
    ) -> Result<limeos_contracts::PasswordResponse> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let _permit = self
            .password_workers
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error(ErrorCode::Overloaded))?;
        let executable = std::env::current_exe().map_err(|_| Error(ErrorCode::Unavailable))?;
        let directory = executable.parent().ok_or(Error(ErrorCode::Unavailable))?;
        #[cfg(test)]
        let directory = if directory.ends_with("deps") {
            directory.parent().ok_or(Error(ErrorCode::Unavailable))?
        } else {
            directory
        };
        let mut child = tokio::process::Command::new(directory.join("limeos-password-worker"))
            .env_clear()
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        let bytes = serde_json::to_vec(&request).map_err(|_| Error(ErrorCode::InvalidInput))?;
        tokio::time::timeout(std::time::Duration::from_secs(8), async {
            let mut input = child.stdin.take().ok_or(Error(ErrorCode::Unavailable))?;
            input
                .write_all(&bytes)
                .await
                .map_err(|_| Error(ErrorCode::Unavailable))?;
            drop(input);
            let mut output = Vec::new();
            child
                .stdout
                .take()
                .ok_or(Error(ErrorCode::Unavailable))?
                .take(2049)
                .read_to_end(&mut output)
                .await
                .map_err(|_| Error(ErrorCode::Unavailable))?;
            if output.len() > 2048
                || !child
                    .wait()
                    .await
                    .map_err(|_| Error(ErrorCode::Unavailable))?
                    .success()
            {
                return Err(Error(ErrorCode::Unavailable));
            }
            serde_json::from_slice(&output).map_err(|_| Error(ErrorCode::Unavailable))
        })
        .await
        .map_err(|_| Error(ErrorCode::Unavailable))?
    }
    async fn hash(&self, password: String) -> Result<String> {
        if !(12..=1024).contains(&password.len()) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        self.password(limeos_contracts::PasswordRequest::Hash { password })
            .await?
            .upgraded
            .ok_or(Error(ErrorCode::Unavailable))
    }
    async fn enroll(&self, input: Enrollment) -> Result<()> {
        if !limeos_domain::identifier(&input.username) || input.token.len() != 64 {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let hash = self.hash(input.password).await?;
        let digest = limeos_identity::digest(&input.token);
        self.db
            .call(move |s| s.enroll(&digest, &input.username, &hash, now()))
            .await
    }
}
impl Backend for Core {
    async fn login(&self, input: Login) -> Result<IssuedSession> {
        if !limeos_domain::identifier(&input.username)
            || input.username.len() > 64
            || input.password.is_empty()
            || input.password.len() > 1024
        {
            return Err(Error(ErrorCode::Unauthenticated));
        }
        let record = self
            .db
            .call(move |s| s.login_record(&input.username))
            .await?;
        let encoded = record
            .as_ref()
            .map(|r| r.password_hash.clone())
            .unwrap_or_else(|| self.dummy_hash.to_string());
        let response = self
            .password(limeos_contracts::PasswordRequest::Verify {
                password: input.password,
                encoded,
            })
            .await?;
        let (valid, upgraded) = (response.valid, response.upgraded);
        if !valid {
            return Err(Error(ErrorCode::Unauthenticated));
        }
        let record = record.ok_or(Error(ErrorCode::Unauthenticated))?;
        let principal = record.principal.clone();
        let token = limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?;
        // CSRF token is deterministically derived from the session secret, and only
        // its digest is persisted. It can be recovered after a browser reconnect.
        let csrf = limeos_identity::digest(&format!("csrf:{token}"));
        let digest = limeos_identity::digest(&token);
        let csrf_digest = limeos_identity::digest(&csrf);
        let expires_at = self
            .db
            .call(move |s| {
                s.create_session(&record, upgraded.as_deref(), &digest, &csrf_digest, now())
            })
            .await?;
        Ok(IssuedSession {
            token,
            view: SessionView {
                principal,
                csrf_token: csrf,
                expires_at,
            },
        })
    }
    async fn session(&self, token: String, csrf: Option<String>) -> Result<SessionView> {
        let csrf_token = limeos_identity::digest(&format!("csrf:{token}"));
        let digest = limeos_identity::digest(&token);
        let csrf = csrf.map(|s| limeos_identity::digest(&s));
        let principal = self
            .db
            .call(move |s| s.authenticate(&digest, csrf.as_deref(), now()))
            .await?;
        // Return the durable expiry, never extend sessions on a status read.
        let digest = limeos_identity::digest(&token);
        let expires_at = self.db.call(move |s| s.session_expiry(&digest)).await?;
        Ok(SessionView {
            principal,
            csrf_token,
            expires_at,
        })
    }
    async fn logout(&self, token: String, csrf: String) -> Result<()> {
        self.session(token.clone(), Some(csrf)).await?;
        let digest = limeos_identity::digest(&token);
        self.db
            .call(move |s| s.revoke_session(&digest, now()))
            .await
    }
    async fn health(&self) -> Result<Health> {
        self.db.call(|_| Ok(())).await?;
        Ok(Health {
            version: VERSION,
            generation: self.db.generation,
            schema_version: limeos_persistence::SCHEMA_VERSION,
            ready: true,
        })
    }
    async fn observations(&self, token: String) -> Result<limeos_contracts::Overview> {
        let digest = limeos_identity::digest(&token);
        let (principal, grants) = self
            .db
            .call(move |s| {
                let p = s.authenticate(&digest, None, now())?;
                let g = s.grants(&p)?;
                Ok((p, g))
            })
            .await?;
        let allowed = |resource: &str| {
            limeos_policy::authorize(
                &principal,
                &grants,
                &Scope {
                    operation: limeos_domain::Operation::HealthRead,
                    resource: resource.into(),
                },
            )
            .is_ok()
        };
        let mut value = self.observations.snapshot();
        let host = allowed("host:system");
        if !host {
            value.host = None;
        }
        value.resources.retain(|r| allowed(&r.id));
        value.sources.retain(|s| {
            (matches!(
                s.source,
                limeos_contracts::Source::Host | limeos_contracts::Source::History
            ) && host)
                || value.resources.iter().any(|r| r.source == s.source)
                || grants.iter().any(|g| {
                    g.operation == limeos_domain::Operation::HealthRead && g.resource == "*"
                })
        });
        if !host && value.resources.is_empty() && value.sources.is_empty() {
            return Err(Error(ErrorCode::Forbidden));
        }
        self.observations.demand();
        Ok(value)
    }
    async fn history(
        &self,
        token: String,
        range: limeos_contracts::HistoryRange,
    ) -> Result<limeos_contracts::MetricHistory> {
        let digest = limeos_identity::digest(&token);
        self.db
            .call(move |s| {
                let p = s.authenticate(&digest, None, now())?;
                limeos_policy::authorize(
                    &p,
                    &s.grants(&p)?,
                    &Scope {
                        operation: limeos_domain::Operation::HealthRead,
                        resource: "host:system".into(),
                    },
                )
            })
            .await?;
        self.telemetry
            .as_ref()
            .ok_or(Error(ErrorCode::Unavailable))?
            .query(range, now())
            .await
    }
    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.observations.subscribe()
    }
    async fn plan_compose(
        &self,
        token: String,
        csrf: String,
        selection: limeos_domain::ComposeSelection,
    ) -> Result<limeos_domain::PlannedCompose> {
        let catalog = self.catalog()?;
        let digest = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, Some(&csrf), now())?;
                s.plan_compose(&principal, &catalog, &selection, now())
            })
            .await
    }
    async fn compose_plan(
        &self,
        token: String,
        id: String,
    ) -> Result<limeos_domain::PlannedCompose> {
        let catalog = self.catalog()?;
        let digest = limeos_identity::digest(&token);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, None, now())?;
                s.compose_plan(&principal, &catalog, &id, now())
            })
            .await
    }
    async fn approve_compose(
        &self,
        token: String,
        csrf: String,
        id: String,
        plan_digest: String,
    ) -> Result<limeos_domain::PlanApproval> {
        let catalog = self.catalog()?;
        let digest = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, Some(&csrf), now())?;
                s.approve_compose(&principal, &catalog, &id, &plan_digest, now())
            })
            .await
    }
    async fn cancel_compose(&self, token: String, csrf: String, id: String) -> Result<()> {
        let catalog = self.catalog()?;
        let digest = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, Some(&csrf), now())?;
                s.cancel_compose(&principal, &catalog, &id, now())
            })
            .await
    }
    async fn plan_restart(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::RestartInput,
    ) -> Result<limeos_domain::PlannedRestart> {
        self.propose(token, csrf, input.resource).await
    }
    async fn plan_container(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::ContainerInput,
    ) -> Result<limeos_domain::PlannedContainerAction> {
        self.propose_action(token, csrf, input.operation, input.resource)
            .await
    }
    async fn container_logs(
        &self,
        token: String,
        resource: String,
        options: limeos_domain::LogOptions,
    ) -> Result<limeos_domain::ContainerLogs> {
        self.human_logs(token, resource, options).await
    }
    async fn approve_container(
        &self,
        token: String,
        csrf: String,
        id: String,
        plan_digest: String,
    ) -> Result<limeos_domain::PlanApproval> {
        let digest = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, Some(&csrf), now())?;
                s.approve_container(&principal, &id, &plan_digest, now())
            })
            .await
    }
    async fn queue_container(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::QueueRestartInput,
    ) -> Result<limeos_domain::RestartJob> {
        self.submit(token, csrf, input).await
    }
    async fn restart_progress(
        &self,
        token: String,
        id: String,
        after: i64,
    ) -> Result<limeos_contracts::JobProgress> {
        let principal = self.session(token, None).await?.principal;
        self.db
            .call(move |s| {
                Ok(limeos_contracts::JobProgress {
                    job: s.container_job(&principal, &id)?,
                    events: s.container_events(&principal, &id, after)?,
                })
            })
            .await
    }
    async fn container_jobs(&self, token: String) -> Result<Vec<limeos_domain::RestartJob>> {
        let principal = self.session(token, None).await?.principal;
        self.db.call(move |s| s.container_jobs(&principal)).await
    }
    async fn cancel_restart(
        &self,
        token: String,
        csrf: String,
        id: String,
        plan: bool,
    ) -> Result<()> {
        let digest = limeos_identity::digest(&token);
        let csrf = limeos_identity::digest(&csrf);
        self.db
            .call(move |s| {
                let principal = s.authenticate(&digest, Some(&csrf), now())?;
                if plan {
                    s.cancel_container_plan(&principal, &id, now())
                } else {
                    s.cancel_container_job(&principal, &id, now())
                }
            })
            .await
    }
}
async fn handle_rpc(core: Core, mut stream: UnixStream) -> Result<()> {
    let peer = stream
        .peer_cred()
        .map_err(|_| Error(ErrorCode::Forbidden))?;
    let result: Result<CoreResponse> = async {
        let request: CoreRequest = limeos_contracts::read_frame(&mut stream).await?;
        let response = match request {
            CoreRequest::Health { version } if version == VERSION => {
                CoreResponse::Health(core.health().await?)
            }
            CoreRequest::IssueBootstrap { version } if version == VERSION && peer.uid() == 0 => {
                let token = limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?;
                let digest = limeos_identity::digest(&token);
                let expires_at = core
                    .db
                    .call(move |s| s.issue_bootstrap(&digest, now()))
                    .await?;
                CoreResponse::Bootstrap { token, expires_at }
            }
            CoreRequest::Enroll {
                version,
                enrollment,
            } if version == VERSION && peer.uid() == 0 => {
                core.enroll(enrollment).await?;
                CoreResponse::Enrolled
            }
            CoreRequest::CheckTask {
                version,
                token,
                task,
                operation,
                resource,
            } if version == VERSION => {
                let digest = limeos_identity::digest(&token);
                let uid = peer.uid();
                core.db
                    .call(move |s| {
                        s.check_task(
                            &digest,
                            uid,
                            &task,
                            &Scope {
                                operation,
                                resource,
                            },
                            now(),
                        )
                    })
                    .await?;
                CoreResponse::Authorized
            }
            CoreRequest::ProposeRestart {
                version,
                token,
                task,
                resource,
            } if version == VERSION => {
                let proposal = core
                    .propose_task(
                        token,
                        peer.uid(),
                        task,
                        limeos_domain::ContainerAction::Restart,
                        resource,
                    )
                    .await?;
                CoreResponse::RestartPlan(proposal)
            }
            CoreRequest::QueueRestart {
                version,
                token,
                task,
                input,
            } if version == VERSION => {
                if input.proposal.plan.operation != limeos_domain::ContainerAction::Restart {
                    return Err(Error(ErrorCode::InvalidInput));
                }
                let job = core.submit_task(token, peer.uid(), task, *input).await?;
                CoreResponse::RestartJob(job)
            }
            CoreRequest::ProposeContainer {
                version,
                token,
                task,
                resource,
                operation,
            } if version == VERSION => CoreResponse::ContainerPlan(
                core.propose_task(token, peer.uid(), task, operation, resource)
                    .await?,
            ),
            CoreRequest::QueueContainer {
                version,
                token,
                task,
                input,
            } if version == VERSION => {
                CoreResponse::ContainerJob(core.submit_task(token, peer.uid(), task, *input).await?)
            }
            CoreRequest::ReadContainerLogs {
                version,
                token,
                task,
                resource,
                options,
            } if version == VERSION => CoreResponse::ContainerLogs(
                core.task_logs(token, peer.uid(), task, resource, options)
                    .await?,
            ),
            CoreRequest::ProposeCompose {
                version,
                token,
                task,
                selection,
            } if version == VERSION => CoreResponse::ComposePlan(
                core.propose_compose_task(token, peer.uid(), task, selection)
                    .await?,
            ),
            _ => return Err(Error(ErrorCode::Forbidden)),
        };
        Ok(response)
    }
    .await;
    let response = result.unwrap_or_else(|error| {
        CoreResponse::Error(ErrorEnvelope {
            code: error.0,
            message: error.message().into(),
            retry: matches!(error.0, ErrorCode::Overloaded | ErrorCode::Unavailable),
            audit_id: format!("rpc-{}", now()),
        })
    });
    limeos_contracts::write_frame(&mut stream, &response).await
}
async fn rpc_server(core: Core, listener: UnixListener) -> std::io::Result<()> {
    let permits = Arc::new(Semaphore::new(16));
    loop {
        let (stream, _) = listener.accept().await?;
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            continue;
        };
        let core = core.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let _ = tokio::time::timeout(limeos_contracts::RPC_DEADLINE, handle_rpc(core, stream))
                .await;
        });
    }
}
fn options() -> Result<(PathBuf, PathBuf, PathBuf)> {
    let mut state = PathBuf::from("/var/lib/limeos/core");
    let mut config = PathBuf::from("/etc/limeos/core.json");
    let mut socket = PathBuf::from("/run/limeos-core/core.sock");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = args.next().ok_or(Error(ErrorCode::InvalidInput))?;
        match arg.as_str() {
            "--state-dir" => state = value.into(),
            "--config" => config = value.into(),
            "--socket" => socket = value.into(),
            _ => return Err(Error(ErrorCode::InvalidInput)),
        }
    }
    Ok((state, config, socket))
}
fn bind_socket(path: &Path) -> Result<UnixListener> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        use std::os::unix::fs::FileTypeExt;
        if !meta.file_type().is_socket() || std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Err(Error(ErrorCode::Conflict));
        }
        std::fs::remove_file(path).map_err(|_| Error(ErrorCode::Unavailable))?;
    }
    let listener = UnixListener::bind(path).map_err(|_| Error(ErrorCode::Unavailable))?;
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o660))
        .map_err(|_| Error(ErrorCode::Unavailable))?;
    Ok(listener)
}
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    tracing_subscriber::fmt().json().with_target(false).init();
    if let Err(error) = run().await {
        tracing::error!(code=?error.0,message=error.message(),"core stopped");
        std::process::exit(1);
    }
}
async fn run() -> Result<()> {
    let (state, config_path, socket) = options()?;
    let config = match config::read(&config_path)? {
        ConfigRead::Valid(c) => c,
        ConfigRead::Missing => config::Config::default(),
        ConfigRead::Corrupt => return Err(Error(ErrorCode::CorruptConfiguration)),
    };
    std::fs::create_dir_all(&state).map_err(|_| Error(ErrorCode::StateNotDurable))?;
    std::fs::set_permissions(&state, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .map_err(|_| Error(ErrorCode::StateNotDurable))?;
    let db = Database::open(&state.join("core.sqlite"))?;
    let telemetry =
        limeos_observations::telemetry::Telemetry::open(&state.join("metrics.sqlite")).ok();
    let observations = limeos_observations::start_core(
        config.host_socket.clone().into(),
        config.container_socket.clone().into(),
        telemetry.clone(),
    );
    let mut core = Core {
        db,
        password_workers: Arc::new(Semaphore::new(2)),
        dummy_hash: "".into(),
        observations,
        telemetry,
        container_socket: Arc::new(config.container_socket.into()),
        compose_catalog: config::read_compose_catalog(
            &config_path.with_file_name("compose-catalog.json"),
        )?
        .map(Arc::new),
    };
    core.dummy_hash = core
        .hash(limeos_identity::opaque().map_err(|_| Error(ErrorCode::Unavailable))?)
        .await?
        .into();
    let http = TcpListener::bind(&config.listen)
        .await
        .map_err(|_| Error(ErrorCode::Unavailable))?;
    let rpc = bind_socket(&socket)?;
    tracing::info!(generation = core.db.generation, "core ready");
    let app = limeos_api::router(core.clone(), config.origin);
    let dispatcher = core.clone();
    tokio::select! {
        result=limeos_api::serve(http,app)=>result.map_err(|_|Error(ErrorCode::Unavailable)),
        result=rpc_server(core,rpc)=>result.map_err(|_|Error(ErrorCode::Unavailable)),
        _=dispatcher.dispatcher()=>Err(Error(ErrorCode::Unavailable)),
        _=tokio::signal::ctrl_c()=>Ok(()),
        _=async {if let Ok(mut signal)=tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()){signal.recv().await;}}=>Ok(()),
    }
}

#[cfg(test)]
mod compose_tests;
#[cfg(test)]
mod operation_tests;
#[cfg(test)]
mod tests;
