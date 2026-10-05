use super::*;

#[tokio::test(start_paused = true)]
async fn slow_optional_metrics_cannot_discard_a_successful_inventory() {
    use limeos_contracts::{Resource, ResourceKind};
    let mut resources: Vec<_> = (0..64)
        .map(|id| {
            Resource::new(
                format!("container:{id:064x}"),
                ResourceKind::Container,
                format!("service-{id}"),
                "running".into(),
                Source::Docker,
            )
        })
        .collect();
    let start = tokio::time::Instant::now();
    let mut current = HashMap::new();
    let mut warnings = Vec::new();
    sample_metrics(
        &mut resources,
        &HashMap::new(),
        &mut current,
        &mut warnings,
        |_| async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            Err(Error(limeos_domain::ErrorCode::Unavailable))
        },
    )
    .await;
    assert!(
        start.elapsed() <= Duration::from_secs(8),
        "optional stats exceeded their budget: {:?}",
        start.elapsed()
    );
    assert_eq!(resources.len(), 64);
    assert!(
        resources.iter().all(|r| r.status == "running"
            && r.cpu_percent.is_none()
            && r.memory_percent.is_none())
    );
    assert!(current.is_empty());
    assert!(!warnings.is_empty());
}
