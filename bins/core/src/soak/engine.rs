//! The simulated Docker Engine: the reference host's containers as state
//! behind `limeos_executor_container::Engine`, the receipt store's own Engine
//! seam (the crate's unit tests implement it the same way). It is stateful
//! where the HTTP fixture in `docker/tests.rs` is not: a restart gives a
//! container a new start time from the simulated clock, a stop keeps it, and
//! the harness can restart a container behind LimeOS's back (an update tool, a
//! crash loop) or make the next effect fail or hang. The executor's request
//! handling (`limeos_executor::container_requests`) reaches it through
//! [`Docker`], as the daemon reaches the real Engine through its client.
use super::support::{Clock, START, rfc3339};
use limeos_domain::{
    ContainerAction, ContainerSnapshot, Error, ErrorCode, Result,
    deployment::discovery::{
        CatalogFacts, Declaration, DiscoveryCohort, EngineFacts, FileStatus, Membership,
        ProjectIdentity,
    },
};
use std::{
    collections::BTreeMap,
    future::Future,
    sync::{Arc, Mutex},
};

/// The reference host (wybie): 18 containers, 15 of them in four Compose
/// stacks, the rest standalone.
pub const HOST: [(&str, Option<&str>); 18] = [
    ("jellyfin", Some("media")),
    ("sonarr", Some("media")),
    ("radarr", Some("media")),
    ("prowlarr", Some("media")),
    ("qbittorrent", Some("media")),
    ("bazarr", Some("media")),
    ("immich-server", Some("photos")),
    ("immich-ml", Some("photos")),
    ("immich-redis", Some("photos")),
    ("immich-db", Some("photos")),
    ("homeassistant", Some("home")),
    ("mosquitto", Some("home")),
    ("zigbee2mqtt", Some("home")),
    ("nextcloud", Some("cloud")),
    ("nextcloud-db", Some("cloud")),
    ("pihole", None),
    ("syncthing", None),
    ("uptime-kuma", None),
];
pub const STACKS: [&str; 4] = ["media", "photos", "home", "cloud"];

/// What the next Engine effect does instead of succeeding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fault {
    #[default]
    None,
    /// The Engine answers an error; `applied` says whether the container
    /// changed anyway (a 500 after the daemon acted).
    Fail { applied: bool },
    /// The Engine never answers and nothing changes.
    Hang,
}

struct Container {
    image: String,
    started: (i64, u32),
    running: bool,
    project: Option<&'static str>,
}
struct Inner {
    containers: BTreeMap<String, Container>,
    fault: Fault,
    effects: u64,
}
pub struct World {
    clock: Clock,
    inner: Mutex<Inner>,
    /// Notified when an effect hangs: the request is parked in the Engine.
    pub hung: tokio::sync::Notify,
}

pub fn container_id(name: &str) -> String {
    limeos_identity::digest(&format!("soak-container:{name}"))
}

impl World {
    pub fn new(clock: Clock) -> Self {
        let containers = HOST
            .iter()
            .map(|(name, project)| {
                (
                    container_id(name),
                    Container {
                        image: limeos_identity::digest(&format!("soak-image:{name}")),
                        started: (START - 30 * super::support::DAY, 0),
                        running: true,
                        project: *project,
                    },
                )
            })
            .collect();
        Self {
            clock,
            inner: Mutex::new(Inner {
                containers,
                fault: Fault::None,
                effects: 0,
            }),
            hung: tokio::sync::Notify::new(),
        }
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn ids(&self) -> Vec<String> {
        self.lock().containers.keys().cloned().collect()
    }
    /// Arms the next Engine effect (restart, start or stop).
    pub fn arm(&self, fault: Fault) {
        self.lock().fault = fault;
    }
    /// Engine effects that changed a container, in all.
    pub fn effects(&self) -> u64 {
        self.lock().effects
    }
    pub fn running(&self, id: &str) -> bool {
        self.lock().containers.get(id).is_some_and(|c| c.running)
    }
    pub fn snapshot(&self, id: &str) -> Option<ContainerSnapshot> {
        let inner = self.lock();
        inner.containers.get(id).map(|c| snapshot(id, c))
    }
    /// A restart LimeOS did not ask for: the container gets a new start time.
    pub fn external_restart(&self, id: &str) {
        let now = self.clock.now();
        if let Some(c) = self.lock().containers.get_mut(id) {
            c.started = later(c.started, now);
            c.running = true;
        }
    }
    /// The inventory cohort the storage reader would read from this Engine
    /// now (`Core::propose_stack` takes a stack's members from it): every
    /// Compose project registered in the catalog as a stack of its own name,
    /// one service per container, and the Engine's membership.
    pub fn cohort(&self, now: i64) -> DiscoveryCohort {
        let inner = self.lock();
        let identity = |project: &str| ProjectIdentity {
            name: project.into(),
            working_directory: format!("/opt/{project}"),
            config_files: vec![format!("/opt/{project}/compose.yaml")],
        };
        let members = HOST
            .iter()
            .map(|(name, project)| {
                let id = container_id(name);
                let container = snapshot(&id, &inner.containers[&id]);
                match project {
                    Some(project) => Membership::Compose {
                        container,
                        project: identity(project),
                        service: (*name).into(),
                        one_off: false,
                    },
                    None => Membership::Standalone { container },
                }
            })
            .collect();
        let stacks = STACKS
            .iter()
            .map(|project| Declaration {
                stack: (*project).into(),
                project: (*project).into(),
                working_directory: identity(project).working_directory,
                config_files: identity(project).config_files,
                services: HOST
                    .iter()
                    .filter(|(_, p)| *p == Some(*project))
                    .map(|(name, _)| (*name).into())
                    .collect(),
                files: FileStatus::NotInspected,
            })
            .collect();
        DiscoveryCohort::new(
            Some(CatalogFacts {
                engine_id: "soak-engine".into(),
                sampled_at: now,
                stacks,
            }),
            Some(EngineFacts {
                engine_id: "soak-engine".into(),
                sampled_at: now,
                members,
                truncated: 0,
                omitted: Vec::new(),
                hidden_projects: Vec::new(),
                unattributed: 0,
            }),
        )
        .expect("the reference host's inventory cohort")
    }
    fn effect(&self, id: &str, action: ContainerAction) -> (Result<()>, bool) {
        let now = self.clock.now();
        let mut inner = self.lock();
        let fault = std::mem::take(&mut inner.fault);
        if fault == Fault::Hang {
            self.hung.notify_one();
            return (Ok(()), true);
        }
        let Some(c) = inner.containers.get_mut(id) else {
            return (Err(Error(ErrorCode::NotFound)), false);
        };
        let apply = !matches!(fault, Fault::Fail { applied: false });
        if apply {
            match action {
                ContainerAction::Restart => {
                    c.started = later(c.started, now);
                    c.running = true;
                }
                // 304 on a running container: nothing changes.
                ContainerAction::Start if !c.running => {
                    c.started = later(c.started, now);
                    c.running = true;
                }
                ContainerAction::Start => {}
                ContainerAction::Stop => c.running = false,
            }
            inner.effects += 1;
        }
        match fault {
            Fault::Fail { .. } => (Err(Error(ErrorCode::Unavailable)), false),
            _ => (Ok(()), false),
        }
    }
    fn effect_future(&self, id: &str, action: ContainerAction) -> impl Future<Output = Result<()>> {
        let (result, hang) = self.effect(id, action);
        async move {
            if hang {
                std::future::pending::<()>().await;
            }
            result
        }
    }
}

/// A start time strictly after `previous` and not before `now`.
fn later(previous: (i64, u32), now: i64) -> (i64, u32) {
    if (now, 0) > previous {
        (now, 0)
    } else {
        (previous.0, previous.1 + 1)
    }
}

fn snapshot(id: &str, c: &Container) -> ContainerSnapshot {
    ContainerSnapshot {
        resource: format!("container:{id}"),
        image: format!("sha256:{}", c.image),
        started_at: rfc3339(c.started.0, c.started.1),
        running: c.running,
    }
}

/// The simulated Engine as the executor's request handling holds it.
#[derive(Clone)]
pub struct Docker(pub Arc<World>);

impl limeos_executor_container::Engine for Docker {
    fn inspect(&self, id: &str) -> impl Future<Output = Result<ContainerSnapshot>> + Send {
        std::future::ready(self.0.snapshot(id).ok_or(Error(ErrorCode::NotFound)))
    }
    fn restart(&self, id: &str) -> impl Future<Output = Result<()>> + Send {
        self.0.effect_future(id, ContainerAction::Restart)
    }
    fn start(&self, id: &str) -> impl Future<Output = Result<()>> + Send {
        self.0.effect_future(id, ContainerAction::Start)
    }
    fn stop(&self, id: &str) -> impl Future<Output = Result<()>> + Send {
        self.0.effect_future(id, ContainerAction::Stop)
    }
    fn project_members(
        &self,
        project: &str,
    ) -> impl Future<Output = Result<Vec<ContainerSnapshot>>> + Send {
        let inner = self.0.lock();
        let members = inner
            .containers
            .iter()
            .filter(|(_, c)| c.project == Some(project))
            .map(|(id, c)| snapshot(id, c))
            .collect::<Vec<_>>();
        std::future::ready(if members.is_empty() {
            Err(Error(ErrorCode::NotFound))
        } else {
            Ok(members)
        })
    }
}
