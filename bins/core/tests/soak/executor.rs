//! The container executor: the real `ReceiptStore` behind a handler that
//! mirrors the container arms of `bins/executor/src/main.rs` (the protocol
//! ceiling's `validate`, then one store call per request, the store epoch on
//! every receipt answer, errors as `ready: false`). Only the clock differs:
//! the daemon passes `clock()` (the system clock); the harness passes the
//! simulated one, and opens the store with `open_at` for the same reason.
use crate::{
    engine::{Fault, HOST, STACKS, World, container_id},
    support::Io,
};
use limeos_contracts::VERSION;
use limeos_domain::{Error, ErrorCode, Result, StackSnapshot};
use limeos_executor_container::{Answer, ContainerCeiling, Engine, ReceiptStore};
use limeos_executor_protocol::{Ceiling, Receipt, Request};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

/// One store open: what it read and how long it took.
#[derive(Clone, Copy, Debug)]
pub struct Opened {
    pub io: Io,
    pub elapsed: Duration,
}

pub struct Executor {
    pub dir: PathBuf,
    store: Option<ReceiptStore>,
    ceiling: Ceiling,
    restart_ceiling: ContainerCeiling,
    uid: u32,
    pub world: Arc<World>,
    /// Engine effects caused by each action ID, over the whole run.
    pub effects: HashMap<String, u64>,
}

impl Executor {
    pub fn new(dir: &Path, world: Arc<World>, uid: u32) -> Self {
        let managed_containers: Vec<String> = HOST.iter().map(|(n, _)| container_id(n)).collect();
        let managed_stacks: Vec<String> = STACKS.iter().map(|s| (*s).to_owned()).collect();
        let ceiling = Ceiling {
            version: VERSION,
            core_uid: uid,
            allow_health: true,
            allow_host_read: false,
            allow_storage_read: false,
            allow_stack_read: false,
            allow_container_health_read: false,
            allow_container_read: true,
            allow_restart: true,
            allow_start: true,
            allow_stop: true,
            allow_container_logs: true,
            allow_stack: true,
            managed_containers: managed_containers.clone(),
            managed_stacks: managed_stacks.clone(),
        };
        assert!(ceiling.configuration_valid());
        let restart_ceiling = ContainerCeiling {
            version: 1,
            core_uid: uid,
            allow_restart: true,
            allow_start: true,
            allow_stop: true,
            allow_stack: true,
            managed_containers,
            managed_stacks,
        };
        Self {
            dir: dir.to_path_buf(),
            store: None,
            ceiling,
            restart_ceiling,
            uid,
            world,
            effects: HashMap::new(),
        }
    }
    pub fn up(&self) -> bool {
        self.store.is_some()
    }
    /// Start the process: open (or create) the receipt store, as the daemon
    /// does at startup, measured. A new open draws a new epoch.
    pub fn start(&mut self, now: i64) -> Result<Opened> {
        assert!(self.store.is_none(), "one executor at a time");
        let (io, at) = (Io::now(), Instant::now());
        let store = ReceiptStore::open_at(&self.dir, now)?;
        let opened = Opened {
            io: Io::now().since(io),
            elapsed: at.elapsed(),
        };
        self.store = Some(store);
        Ok(opened)
    }
    /// Stop the process (the store closes; anything in flight is lost).
    pub fn stop(&mut self) {
        self.store = None;
    }

    /// One request, answered as `bins/executor/src/main.rs` answers it.
    pub async fn handle(&mut self, request: Request, now: i64) -> Receipt {
        let before = self.world.effects();
        let action = match &request {
            Request::ExecuteContainer { request, .. } => Some(request.action.clone()),
            Request::ExecuteStack { request, .. } => Some(request.action.clone()),
            _ => None,
        };
        let result = self.answer(request, now).await;
        if let Some(action) = action {
            *self.effects.entry(action).or_default() += self.world.effects() - before;
        }
        result.unwrap_or_else(|e| {
            let mut value = Receipt::empty();
            value.ready = false;
            value.error = Some(e.0);
            value
        })
    }

    /// The executor is killed while an execute is inside the Engine call:
    /// the next effect hangs, the request is driven until it is parked there
    /// (its `prepared` receipt committed, with any pruning that insert did),
    /// then dropped, and the process stops. Returns whether the request got
    /// that far; one refused before the Engine leaves nothing in flight.
    pub async fn kill_inside_engine_call(&mut self, request: Request, now: i64) -> bool {
        self.world.arm(Fault::Hang);
        let interrupted = tokio::select! {
            biased;
            _ = self.handle(request, now) => false,
            _ = yield_twice() => true,
        };
        // A request refused before the Engine leaves the fault armed.
        self.world.arm(Fault::None);
        self.stop();
        interrupted
    }

    async fn answer(&mut self, request: Request, now: i64) -> Result<Receipt> {
        let mut receipt = self.ceiling.validate(self.uid, &request)?;
        let (uid, ceiling) = (self.uid, &self.restart_ceiling);
        let world = &*self.world;
        let store = self.store.as_mut().ok_or(Error(ErrorCode::Unavailable))?;
        match request {
            Request::ReceiptEpoch { .. } => receipt.epoch = Some(store.epoch().to_owned()),
            Request::Inspect { resource, .. } => {
                receipt.inspection = Some(world.inspect(&resource[10..]).await?);
            }
            Request::InspectStack { project, .. } => {
                let members = world.project_members(&project).await?;
                receipt.stack_members = Some(StackSnapshot::new(&project, members)?);
            }
            Request::ExecuteContainer { request, .. } => {
                receipt.restart = Some(store.execute(world, ceiling, uid, &request, || now).await?);
                receipt.epoch = Some(store.epoch().to_owned());
            }
            Request::RestartReceipt {
                action,
                digest,
                plan,
                close_if_absent,
                ..
            } => {
                let lookup =
                    store.lookup(ceiling, uid, &action, &digest, &plan, close_if_absent, now)?;
                copy(&mut receipt, Answer::restart_lookup(lookup));
                receipt.epoch = Some(store.epoch().to_owned());
            }
            Request::VerifyRestart {
                action,
                digest,
                after,
                ..
            } => {
                receipt.restart = Some(
                    store
                        .verify(world, ceiling, uid, &action, &digest, &after)
                        .await?,
                );
                receipt.epoch = Some(store.epoch().to_owned());
            }
            Request::ExecuteStack { request, .. } => {
                receipt.stack = Some(
                    store
                        .execute_stack(world, ceiling, uid, &request, || now)
                        .await?,
                );
                receipt.epoch = Some(store.epoch().to_owned());
            }
            Request::StackReceipt {
                action,
                digest,
                plan,
                close_if_absent,
                ..
            } => {
                let lookup = store.lookup_stack(
                    ceiling,
                    uid,
                    &action,
                    &digest,
                    &plan,
                    close_if_absent,
                    now,
                )?;
                copy(&mut receipt, Answer::stack_lookup(lookup));
                receipt.epoch = Some(store.epoch().to_owned());
            }
            Request::VerifyStack {
                action,
                digest,
                after,
                ..
            } => {
                receipt.stack = Some(
                    store
                        .verify_stack(world, ceiling, uid, &action, &digest, &after)
                        .await?,
                );
                receipt.epoch = Some(store.epoch().to_owned());
            }
            Request::ResolveAction {
                kind,
                action,
                digest,
                evidence_digest,
                declared,
                resolved_by,
                plan,
                disputed,
                ..
            } => {
                let outcome = store
                    .resolve_action(
                        world,
                        ceiling,
                        uid,
                        kind,
                        &action,
                        &digest,
                        &evidence_digest,
                        declared,
                        &resolved_by,
                        &plan,
                        disputed,
                        now,
                    )
                    .await?;
                copy(&mut receipt, Answer::resolve(outcome));
                receipt.epoch = Some(store.epoch().to_owned());
            }
            _ => return Err(Error(ErrorCode::Forbidden)),
        }
        Ok(receipt)
    }
}

/// As the daemon's `answer`: the receipt-store fields, field by field.
fn copy(receipt: &mut Receipt, answer: Answer) {
    let Answer {
        restart,
        stack,
        resolution,
        resolution_refusal,
        absence,
        closed,
    } = answer;
    receipt.restart = restart;
    receipt.stack = stack;
    receipt.resolution = resolution;
    receipt.resolution_refusal = resolution_refusal;
    receipt.absence = absence;
    receipt.closed = closed;
}

/// Ready on its second poll: lets a request run until it first parks.
async fn yield_twice() {
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
}
