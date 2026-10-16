use super::*;

#[tokio::test(flavor = "current_thread")]
async fn uninstalled_or_alternate_service_cannot_activate_mounts() {
    // Refuses before opening policy, binding sockets or touching managed targets,
    // including when a source fixture happens to execute with effective UID 0.
    for args in [
        vec!["limeos-executor".into()],
        vec![
            "limeos-executor".into(),
            POLICY.into(),
            SOCKET.into(),
            "storage".into(),
        ],
        vec![
            "limeos-executor".into(),
            "/operator/policy".into(),
            SOCKET.into(),
            ROLE.into(),
        ],
        vec![
            "limeos-executor".into(),
            POLICY.into(),
            SOCKET.into(),
            ROLE.into(),
        ],
    ] {
        assert!(run(&args).await.is_err());
    }
}
