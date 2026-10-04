#[test]
fn password_worker_is_bounded_and_has_no_command_or_actor_protocol() {
    let binary = env!("CARGO_BIN_EXE_limeos-password-worker");
    for input in [
        "x".repeat(16385),
        "{\"mode\":\"hash\",\"password\":\"long-enough-password\",\"command\":\"uname\"}".into(),
    ] {
        use std::io::Write;
        let mut child = std::process::Command::new(binary)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}
