//! RW-005 footprint experiment, not a production API or executor.
use std::{error::Error, net::SocketAddr, path::PathBuf, thread, time::Instant};

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const DOCKER_BYTES_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Clone, Serialize)]
struct Inventory {
    total: usize,
    running: usize,
    response_bytes: usize,
}

#[derive(Deserialize)]
struct Container {
    #[serde(rename = "State")]
    state: String,
}

#[derive(Clone)]
struct AppState {
    database: mpsc::Sender<oneshot::Sender<std::result::Result<i64, String>>>,
    inventory: Inventory,
    // Retain the initialized Rustls client/pool for the whole measurement.
    _docker_client: reqwest::Client,
}

fn main() {
    let started = Instant::now();
    let result = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(2)
        .enable_all()
        .build()
        .map_err(Into::into)
        .and_then(|runtime| runtime.block_on(run(started)));
    if let Err(error) = result {
        eprintln!("footprint spike: {error}");
        std::process::exit(1);
    }
}

async fn run(started: Instant) -> Result<()> {
    let listen: SocketAddr = std::env::var("LIMEOS_SPIKE_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:0".into())
        .parse()?;
    if !listen.ip().is_loopback() {
        return Err("this unauthenticated experiment must bind to loopback".into());
    }
    let state_dir = PathBuf::from(std::env::var("LIMEOS_SPIKE_STATE_DIR")?);
    // The harness supplies a new private temporary directory. Never import old state.
    if !state_dir.is_absolute() || !state_dir.is_dir() {
        return Err("LIMEOS_SPIKE_STATE_DIR must be an existing absolute directory".into());
    }
    let socket = std::env::var("LIMEOS_SPIKE_DOCKER_SOCKET")
        .unwrap_or_else(|_| "/var/run/docker.sock".into());
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .unix_socket(socket)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(2))
        .build()?;

    // One fixed read, including stopped containers; fail instead of measuring an empty stub.
    let inventory = read_docker(&client).await?;
    let database = start_database(state_dir.join("spike.sqlite")).await?;
    let app_state = AppState {
        database,
        inventory: inventory.clone(),
        _docker_client: client,
    };
    let app = Router::new()
        .route("/health/ready", get(ready))
        .route("/api/v1/overview", get(overview))
        .with_state(app_state);
    let listener = tokio::net::TcpListener::bind(listen).await?;
    println!(
        "{}",
        json!({
            "event": "ready",
            "pid": std::process::id(),
            "listen": listener.local_addr()?.to_string(),
            "initialized_ms": started.elapsed().as_secs_f64() * 1000.0,
            "inventory": inventory,
            "runtime_workers": 2,
            "database_workers": 1,
        })
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

async fn read_docker(client: &reqwest::Client) -> Result<Inventory> {
    let mut response = client
        .get("http://localhost/containers/json?all=1")
        .send()
        .await?
        .error_for_status()?;
    if response
        .content_length()
        .is_some_and(|bytes| bytes > DOCKER_BYTES_LIMIT as u64)
    {
        return Err("Docker inventory exceeds byte limit".into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if chunk.len() > DOCKER_BYTES_LIMIT.saturating_sub(body.len()) {
            return Err("Docker inventory exceeds byte limit".into());
        }
        body.extend_from_slice(&chunk);
    }
    let containers: Vec<Container> = serde_json::from_slice(&body)?;
    Ok(Inventory {
        total: containers.len(),
        running: containers.iter().filter(|c| c.state == "running").count(),
        response_bytes: body.len(),
    })
}

async fn start_database(
    path: PathBuf,
) -> Result<mpsc::Sender<oneshot::Sender<std::result::Result<i64, String>>>> {
    let (send, mut receive) =
        mpsc::channel::<oneshot::Sender<std::result::Result<i64, String>>>(32);
    let (initialized, wait) = oneshot::channel();
    thread::Builder::new()
        .name("spike-sqlite".into())
        .spawn(move || {
            let connection = Connection::open(path).and_then(|connection| {
                connection.execute_batch(
                    "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=FULL;
                 CREATE TABLE spike_metadata (id INTEGER PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO spike_metadata VALUES (1, 'RW-005');",
                )?;
                Ok(connection)
            });
            let connection = match connection {
                Ok(connection) => {
                    let _ = initialized.send(Ok::<(), String>(()));
                    connection
                }
                Err(error) => {
                    let _ = initialized.send(Err(error.to_string()));
                    return;
                }
            };
            while let Some(reply) = receive.blocking_recv() {
                let result = connection
                    .query_row("SELECT COUNT(*) FROM spike_metadata", [], |row| row.get(0))
                    .map_err(|error| error.to_string());
                let _ = reply.send(result);
            }
        })?;
    wait.await?
        .map_err(|error| -> Box<dyn Error + Send + Sync> { error.into() })?;
    Ok(send)
}

async fn ready() -> Json<Value> {
    Json(json!({"ready": true, "pid": std::process::id()}))
}

async fn overview(
    State(state): State<AppState>,
) -> std::result::Result<Json<Value>, (StatusCode, &'static str)> {
    let (reply, response) = oneshot::channel();
    state.database.try_send(reply).map_err(|_| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "database queue unavailable",
        )
    })?;
    let rows = tokio::time::timeout(std::time::Duration::from_secs(2), response)
        .await
        .map_err(|_| (StatusCode::GATEWAY_TIMEOUT, "database deadline"))?
        .map_err(|_| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "database worker unavailable",
            )
        })?
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "database read failed"))?;
    Ok(Json(
        json!({"containers": state.inventory, "sqlite_rows": rows}),
    ))
}

async fn shutdown() {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("install SIGTERM handler");
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .expect("install SIGINT handler");
    tokio::select! {
        _ = terminate.recv() => {},
        _ = interrupt.recv() => {},
    }
}
