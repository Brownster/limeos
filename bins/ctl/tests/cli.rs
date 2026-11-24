//! The installed binary's exit status tells a script what happened: 0 done,
//! 1 refused or failed, 2 an invalid command line on which nothing was done.
use std::process::{Command, Output};

const JOB: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

fn ctl(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_limeosctl"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn help_and_version_in_command_position_succeed() {
    let version = ctl(&["--version"]);
    assert_eq!(version.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&version.stdout).starts_with("limeosctl "));
    for args in [
        &["--help"][..],
        &["-h"],
        &["job", "--help"],
        &["job", "resolve", "--help"],
        &["job", "list", "--help"],
        &["job", "cancel", "-h"],
        &["migrate", "-h"],
        &["migrate", "scan", "--help"],
    ] {
        let out = ctl(args);
        assert_eq!(out.status.code(), Some(0), "{args:?}");
        assert!(
            String::from_utf8_lossy(&out.stdout)
                .to_lowercase()
                .contains("usage"),
            "{args:?}"
        );
    }
}

#[test]
fn a_flag_where_a_value_belongs_is_a_usage_error_not_a_version_print() {
    for args in [
        &[
            "job",
            "resolve",
            JOB,
            "--admin",
            "a",
            "--outcome",
            "verified-done",
            "--reason",
            "--version",
        ][..],
        &[
            "job",
            "resolve",
            JOB,
            "--admin",
            "a",
            "--outcome",
            "verified-done",
            "--help",
        ],
        &["--socket", "--version", "status"],
        &["--socket", "/nonexistent/core.sock", "status", "--help"],
        &["migrate", "scan", "--root", "--help"],
        &["migrate", "propose-ceiling", "--out", "-h"],
        &["migrate", "import-users", "--env-file", "--version"],
        &["status", "extra"],
        &["frobnicate"],
        &["job", "list", "--state", "--version"],
        &["job", "list", "--limit", "--help"],
        &["job", "list", "--limit", "65"],
        &["job", "list", "--state", "queued", "--help"],
        &["job", "cancel", JOB, "--admin", "--version"],
        &["job", "cancel", JOB, "--admin", "a", "--help"],
        &["job", "cancel", JOB],
    ] {
        let out = ctl(args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(!stdout.contains("limeosctl 0."), "{args:?}: {stdout}");
        assert!(!stdout.contains("usage"), "{args:?}: {stdout}");
    }
}

#[test]
fn a_well_formed_request_that_cannot_be_served_is_a_refusal() {
    let out = ctl(&["--socket", "/nonexistent/core.sock", "status"]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn job_list_and_cancel_refuse_a_non_root_caller_before_contacting_core() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    for args in [
        &["--socket", "/nonexistent/core.sock", "job", "list"][..],
        &[
            "--socket",
            "/nonexistent/core.sock",
            "job",
            "cancel",
            JOB,
            "--admin",
            "a",
        ],
    ] {
        let out = ctl(args);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("run it as root"), "{args:?}: {stderr}");
    }
}
