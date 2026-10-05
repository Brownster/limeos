use super::*;
use limeos_executor_protocol::{Receipt, Request};
use std::{collections::HashMap, path::PathBuf};
use tokio::net::UnixStream;
pub fn start_host() -> Cache {
    let cache = Cache::new(vec![Source::Host, Source::Storage, Source::Pools]);
    let c = cache.clone();
    tokio::spawn(async move {
        let mut previous = None;
        loop {
            let (metrics, warnings) = probes::host(&mut previous);
            c.update(ObservationBatch {
                host: Some(metrics),
                resources: Vec::new(),
                sources: vec![fresh(Source::Host, warnings)],
            });
            match probes::block_devices().await {
                Ok(resources) => c.update(ObservationBatch {
                    host: None,
                    resources,
                    sources: vec![fresh(Source::Storage, Vec::new())],
                }),
                Err(_) => c.failed(&[Source::Storage]),
            }
            match probes::read(std::path::Path::new("/proc/self/mountinfo"), 131072) {
                Ok(mounts) => c.update(ObservationBatch {
                    host: None,
                    resources: probes::pools(&mounts),
                    sources: vec![fresh(Source::Pools, Vec::new())],
                }),
                Err(_) => c.failed(&[Source::Pools]),
            }
            let duration = Duration::from_secs(if c.active() { 10 } else { 60 });
            tokio::select! {_=tokio::time::sleep(duration)=>{},_=c.notify.notified()=>{}}
            // Demand bursts never start multiple collectors or unbounded rapid probes.
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    });
    cache
}
pub fn start_docker(socket: PathBuf) -> Cache {
    let cache = Cache::new(vec![Source::Docker]);
    let c = cache.clone();
    let adapter = docker::Docker { socket };
    let event_adapter = adapter.clone();
    let event_cache = cache.clone();
    tokio::spawn(async move {
        loop {
            let _ = event_adapter.events(&event_cache.notify).await;
            event_cache.notify.notify_one();
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
    tokio::spawn(async move {
        let mut previous = HashMap::new();
        loop {
            let result = async {
                let version = adapter.version().await?;
                let mut resources = adapter.inventory(&version).await?;
                let mut warnings = Vec::new();
                let mut current = HashMap::new();
                if c.active() {
                    sample_metrics(
                        &mut resources,
                        &previous,
                        &mut current,
                        &mut warnings,
                        |id| {
                            let a = adapter.clone();
                            let v = version.clone();
                            async move { a.stats(&v, &id).await }
                        },
                    )
                    .await;
                } else {
                    warnings.push(
                        "Container resource sampling is paused until a dashboard opens".into(),
                    );
                }
                previous = current;
                resources.extend(docker::stacks(&resources));
                Ok::<_, Error>(ObservationBatch {
                    host: None,
                    resources,
                    sources: vec![fresh(Source::Docker, warnings)],
                })
            };
            match tokio::time::timeout(Duration::from_secs(15), result).await {
                Ok(Ok(batch)) => c.update(batch),
                _ => c.failed(&[Source::Docker]),
            }
            let duration = Duration::from_secs(if c.active() { 10 } else { 60 });
            tokio::select! {_=tokio::time::sleep(duration)=>{},_=c.notify.notified()=>{}}
            // Coalesce event storms without starting additional collectors.
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });
    cache
}
async fn sample_metrics<F, Fut>(
    resources: &mut [limeos_contracts::Resource],
    previous: &HashMap<String, (u64, u64, u64)>,
    current: &mut HashMap<String, (u64, u64, u64)>,
    warnings: &mut Vec<String>,
    fetch: F,
) where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<serde_json::Value>> + Send + 'static,
{
    let mut tasks = tokio::task::JoinSet::new();
    let ids: Vec<_> = resources
        .iter()
        .filter(|r| r.status == "running")
        .map(|r| r.id.trim_start_matches("container:").to_owned())
        .collect();
    let sampling = async {
        for id in ids {
            let future = fetch(id.clone());
            tasks.spawn(async move { (id, future.await) });
            if tasks.len() >= 4 {
                consume(&mut tasks, resources, previous, current, warnings).await;
            }
        }
        while !tasks.is_empty() {
            consume(&mut tasks, resources, previous, current, warnings).await;
        }
    };
    // Optional stats have their own budget. Slow stats must not consume the
    // outer inventory deadline and turn successful inventory into unavailable.
    if tokio::time::timeout(Duration::from_secs(8), sampling)
        .await
        .is_err()
    {
        tasks.abort_all();
        warnings.push("Some container metrics exceeded the sampling deadline".into());
    }
}

async fn consume(
    tasks: &mut tokio::task::JoinSet<(String, Result<serde_json::Value>)>,
    resources: &mut [limeos_contracts::Resource],
    previous: &HashMap<String, (u64, u64, u64)>,
    current: &mut HashMap<String, (u64, u64, u64)>,
    warnings: &mut Vec<String>,
) {
    if let Some(Ok((id, result))) = tasks.join_next().await {
        match result {
            Ok(v) => {
                if let Some(r) = resources
                    .iter_mut()
                    .find(|r| r.id == format!("container:{id}"))
                {
                    docker::stats(r, &v, previous.get(&id).copied());
                }
                if let Some(c) = docker::counters(&v) {
                    current.insert(id, c);
                }
            }
            Err(_) => {
                if warnings.is_empty() {
                    warnings.push("Some container metrics are unavailable".into());
                }
            }
        }
    }
}
pub fn start_core(
    host: PathBuf,
    docker: PathBuf,
    telemetry: Option<telemetry::Telemetry>,
) -> Cache {
    let cache = Cache::default();
    let c = cache.clone();
    tokio::spawn(async move {
        loop {
            let active = c.active();
            let (h, d) = tokio::join!(
                rpc(&host, Source::Host, active),
                rpc(&docker, Source::Docker, active)
            );
            match h {
                Ok(batch) => {
                    if let (Some(t), Some(m), Some(source)) = (
                        &telemetry,
                        &batch.host,
                        batch
                            .sources
                            .iter()
                            .find(|s| s.source == Source::Host && s.state == Availability::Fresh),
                    ) {
                        if let Some(at) = source.sampled_at {
                            let _ = t.record(at, m.clone());
                        }
                    }
                    c.update(batch);
                }
                Err(_) => c.failed(&[Source::Host, Source::Storage, Source::Pools]),
            }
            match d {
                Ok(batch) => c.update(batch),
                Err(_) => c.failed(&[Source::Docker]),
            }
            if telemetry.as_ref().is_some_and(|t| t.healthy()) {
                c.update(ObservationBatch {
                    host: None,
                    resources: Vec::new(),
                    sources: vec![fresh(Source::History, Vec::new())],
                });
            } else {
                c.failed(&[Source::History]);
            }
            let duration = Duration::from_secs(if active { 10 } else { 60 });
            tokio::select! {_=tokio::time::sleep(duration)=>{},_=c.notify.notified()=>{}}
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    cache
}
async fn rpc(path: &std::path::Path, source: Source, active: bool) -> Result<ObservationBatch> {
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut stream = UnixStream::connect(path)
            .await
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        limeos_contracts::write_frame(
            &mut stream,
            &Request::Observe {
                version: VERSION,
                source,
                active,
            },
        )
        .await?;
        let receipt: Receipt = limeos_contracts::read_frame(&mut stream).await?;
        if receipt.version != VERSION || !receipt.ready {
            return Err(Error(ErrorCode::Unavailable));
        }
        receipt.observations.ok_or(Error(ErrorCode::Unavailable))
    })
    .await
    .map_err(|_| Error(ErrorCode::Unavailable))?
}

#[cfg(test)]
mod tests;
