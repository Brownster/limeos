//! One connection to the host, container or storage-reader executor, from
//! accept to answer. The daemon (`main.rs`) sets the role up and hands each
//! accepted connection to [`SharedRole::serve`]; tests drive the same
//! function over a socket pair.
//!
//! The ceiling is read twice per connection (`crate::ceiling`): once at
//! accept, so a peer that is not core is refused before any frame is read,
//! and again after the frame, and only that second read authorizes the
//! request. A request already authorized runs to its end.
use crate::{
    blocking_read::{BlockingReads, Refusal},
    ceiling::{CeilingFile, container_ceiling},
    container_requests::{ContainerRole, answer, refusal},
    read_gate::{Gate, INTERACTIVE_WAIT},
    read_supervision::Supervisor,
};
use limeos_contracts::Source;
use limeos_domain::{Error, ErrorCode};
use limeos_executor_container::docker::Docker;
use limeos_executor_protocol::Request;
use std::sync::Arc;
use tokio::net::UnixStream;

/// How long a storage inventory may take before it is abandoned.
const INVENTORY_DEADLINE: std::time::Duration = std::time::Duration::from_secs(4);

/// Everything a connection to one of the three roles needs.
pub struct SharedRole {
    pub ceilings: Arc<CeilingFile>,
    /// `Host` for the host executor and the storage reader, `Docker` for the
    /// container executor.
    pub source: Source,
    pub storage_reader: bool,
    pub observations: limeos_observations::Cache,
    /// Whether a collector for `source` was started at start. Observation
    /// answers need one; a read granted later is `Unavailable` until restart.
    pub collecting: bool,
    pub containers: Arc<ContainerRole<Docker>>,
    pub storage_permit: Gate,
    pub read_supervisor: Arc<Supervisor>,
    pub stacks: limeos_observations::stacks::Cache,
    /// Whether the stack scan task exists. A stack read granted after start
    /// has no scan behind its cache, so it is `Unavailable` until restart.
    pub stack_scan: bool,
    pub inventory_reads: Arc<BlockingReads>,
}

impl SharedRole {
    /// Answers the one request on `stream`.
    pub async fn serve(&self, mut stream: UnixStream) {
        let result = self.answer(&mut stream).await;
        let receipt = result.unwrap_or_else(refusal);
        // Responses are bounded even if the caller stops reading.
        let _ = tokio::time::timeout(
            limeos_contracts::RPC_DEADLINE,
            limeos_contracts::write_frame(&mut stream, &receipt),
        )
        .await;
    }

    async fn answer(
        &self,
        stream: &mut UnixStream,
    ) -> Result<limeos_executor_protocol::Receipt, Error> {
        let peer = stream
            .peer_cred()
            .map_err(|_| Error(ErrorCode::Forbidden))?
            .uid();
        // Refuse a peer that is not core before parsing or allocating a frame.
        self.ceilings.for_peer(peer)?;
        let request: Request = tokio::time::timeout(
            limeos_contracts::RPC_DEADLINE,
            limeos_contracts::read_frame(stream),
        )
        .await
        .map_err(|_| Error(ErrorCode::Unavailable))??;
        // The ceiling as the file says now that the request is here: only
        // this read authorizes it, so holding a connection open across a
        // revocation gains nothing.
        let ceiling = self.ceilings.for_peer(peer)?;
        let mut receipt = ceiling.validate(peer, &request)?;
        let docker = self.source == Source::Docker;
        match request {
            Request::ContainerHealth { nonce, .. } if self.storage_reader => {
                let guard = self
                    .storage_permit
                    .request(INTERACTIVE_WAIT)
                    .await
                    .ok_or(Error(ErrorCode::Overloaded))?;
                let manager = self.read_supervisor.clone();
                let answer = tokio::task::spawn_blocking(move || {
                    crate::health_read::refresh(&nonce, &manager, guard)
                })
                .await
                .unwrap_or(Err(crate::health_read::Failed {
                    error: Error(ErrorCode::Unavailable),
                    failure: limeos_executor_protocol::HealthReadFailure::WorkerFailed,
                }));
                match answer {
                    Ok(cohort) => receipt.container_health = Some(cohort),
                    // The refusal says why, so core can report it.
                    Err(failed) => {
                        let mut refused = refusal(failed.error);
                        refused.health_failure = Some(failed.failure);
                        return Ok(refused);
                    }
                }
            }
            Request::StackInventory { .. } if self.storage_reader => {
                if !self.stack_scan {
                    return Err(Error(ErrorCode::Unavailable));
                }
                receipt.stack_cohort = Some(self.stacks.cohort()?);
            }
            Request::ContainerStorageInventory { .. } if docker => {
                let _single_flight = self
                    .storage_permit
                    .request(INTERACTIVE_WAIT)
                    .await
                    .ok_or(Error(ErrorCode::Overloaded))?;
                receipt.container_storage = Some(self.containers.engine.storage_inventory().await?);
            }
            Request::StorageInventory { .. } if self.storage_reader => {
                let _single_flight = self
                    .storage_permit
                    .request(INTERACTIVE_WAIT)
                    .await
                    .ok_or(Error(ErrorCode::Overloaded))?;
                self.read_supervisor
                    .ensure_not_poisoned()
                    .map_err(|_| Error(ErrorCode::Unavailable))?;
                // Sysfs, block-device and blkid reads run on their own thread
                // and runtime: a read stuck in the kernel cannot stall this
                // accept loop, and the deadline is real.
                let inventory = self
                    .inventory_reads
                    .run("storage-inventory", INVENTORY_DEADLINE, || {
                        tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .map_err(|_| limeos_executor_storage::Failure::Unavailable)?
                            .block_on(limeos_executor_storage::inventory())
                    })
                    .await
                    .map_err(|refusal| {
                        note_inventory_refusal(refusal);
                        Error(ErrorCode::Unavailable)
                    })?;
                receipt.storage = Some(inventory.map_err(|_| Error(ErrorCode::Unavailable))?);
            }
            Request::Observe {
                source: requested,
                active,
                ..
            } => {
                if requested != self.source {
                    return Err(Error(ErrorCode::Forbidden));
                }
                if !self.collecting {
                    return Err(Error(ErrorCode::Unavailable));
                }
                if active {
                    self.observations.demand();
                }
                let value = self.observations.snapshot();
                receipt.observations = Some(limeos_contracts::ObservationBatch {
                    host: value.host,
                    resources: value.resources,
                    sources: value.sources,
                });
            }
            Request::ContainerLogs {
                resource, options, ..
            } if docker => {
                receipt.logs = Some(self.containers.engine.logs(&resource, options).await?);
            }
            Request::Health { .. } => {
                if self.storage_reader {
                    crate::service::health_answer(&mut receipt, &self.inventory_reads);
                }
            }
            // Inspection, the receipt epoch and every receipt-store request;
            // anything else is refused there.
            request => {
                answer(
                    &self.containers,
                    &container_ceiling(&ceiling),
                    peer,
                    request,
                    &mut receipt,
                    &clock,
                )
                .await?
            }
        }
        Ok(receipt)
    }
}

fn note_inventory_refusal(refusal: Refusal) {
    match refusal {
        Refusal::Deadline => tracing::warn!(
            "storage inventory missed its 4 s deadline and was abandoned; a disk may be failing (its read is left to finish)"
        ),
        Refusal::Stranded => tracing::warn!(
            "storage inventory refused: an earlier read is still stuck in the kernel; inventories stay refused until it returns, and core reports the storage reader degraded"
        ),
        Refusal::Unavailable => tracing::warn!("storage inventory could not start"),
    }
}

fn clock() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}

#[cfg(test)]
mod tests;
