include!("/home/marc/Documents/github/lime-os/.worktrees/p04-stack-discovery/crates/api/src/lib.rs");
#[path = "/home/marc/Documents/github/lime-os/.worktrees/p04-stack-discovery/crates/api/src/deployment.rs"]
pub mod deployment;
pub fn proposed_routes<B: deployment::DeploymentBackend>(backend: B, origin: String) -> Router {
deployment::routes().with_state(App { backend, origin: origin.into(), limiter: Arc::new(Mutex::new(Limiter::default())), readers: Arc::new(Semaphore::new(2)) })
}
