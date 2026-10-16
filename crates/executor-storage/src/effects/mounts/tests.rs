use super::*;
use std::os::fd::AsFd;

fn ordinary_file() -> tempfile::NamedTempFile {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let scratch = repository.join(".cache/p04-a1-invocation-fixtures");
    std::fs::create_dir_all(&scratch).unwrap();
    tempfile::Builder::new()
        .prefix("ordinary-")
        .tempfile_in(scratch)
        .unwrap()
}

#[test]
fn ordinary_file_is_not_a_mount_source_or_a_device_proof() {
    let file = ordinary_file();
    assert!(matches!(
        MountInvocation::checked_descriptors(file.as_fd(), file.as_fd(), file.as_fd()),
        Err(Failure::UnsafePath)
    ));
}

#[test]
fn fixed_arguments_reference_original_parent_fds_and_disable_external_helpers() {
    // Private argument fixture only; deliberately bypasses the production
    // constructor and never launches a helper or claims physical authority.
    let file = ordinary_file();
    let fd = file.as_fd();
    let parent = std::process::id();
    let path = descriptor_name(parent, fd);
    let invocation = MountInvocation {
        source: fd,
        target: fd,
        image: fd,
        parent,
        program_path: path.clone(),
        source_path: path.clone(),
        target_path: path.clone(),
    };
    assert_eq!(invocation.parent_pid(), parent);
    assert_eq!(invocation.argument_zero(), "/usr/bin/mount");
    assert_eq!(
        invocation.arguments(),
        [
            "--no-canonicalize",
            "--no-mtab",
            "--internal-only",
            "--types",
            "ext4",
            "--options",
            "rw,nosuid,nodev",
            "--source",
            path.as_str(),
            "--target",
            path.as_str()
        ]
    );
    assert_eq!(invocation.environment(), [("LC_ALL", "C")]);
    assert!(!invocation.program_path().contains("/proc/self/"));
    assert_eq!(MOUNT_HELPER_DEADLINE, Duration::from_secs(3));
    assert_eq!(MOUNT_HELPER_OUTPUT_BYTES, 8192);
}
