use super::*;

/// The tree's failures as a test sees them.
#[derive(Debug, PartialEq)]
enum Seen {
    Unsupported,
    Io(Option<i32>),
    Other,
}
impl Failure for Seen {
    fn changed() -> Self {
        Seen::Other
    }
    fn collision() -> Self {
        Seen::Other
    }
    fn unsafe_root() -> Self {
        Seen::Other
    }
    fn unsupported_metadata() -> Self {
        Seen::Unsupported
    }
    fn allocation() -> Self {
        Seen::Other
    }
    fn cancelled() -> Self {
        Seen::Other
    }
    fn io(_: &'static str, source: io::Error) -> Self {
        Seen::Io(source.raw_os_error())
    }
    fn into_io(self) -> Option<io::Error> {
        None
    }
}

#[test]
fn a_filesystem_without_attribute_support_holds_no_attributes() {
    // Both trees (before-images and preparation) list attributes through
    // `listed`; ENOTSUP and EOPNOTSUPP mean the filesystem keeps none.
    for errno in [Errno::NOTSUP, Errno::OPNOTSUPP] {
        assert_eq!(listed::<Seen>(Err(errno)), Ok(0));
    }
    assert_eq!(listed::<Seen>(Ok(17)), Ok(17));
    assert_eq!(listed::<Seen>(Err(Errno::RANGE)), Err(Seen::Unsupported));
    for errno in [Errno::IO, Errno::ACCESS, Errno::BADF] {
        assert_eq!(
            listed::<Seen>(Err(errno)),
            Err(Seen::Io(Some(errno.raw_os_error())))
        );
    }
}

/// A default ACL naming another user with full access, as `setfacl -d -m
/// u:65534:rwx` writes it (`system.posix_acl_default`, version 2).
fn default_acl() -> Vec<u8> {
    let mut value = 2u32.to_le_bytes().to_vec();
    for (tag, perm, id) in [
        (0x01u16, 7u16, u32::MAX), // owner
        (0x02, 7, 65534),          // a named user
        (0x04, 0, u32::MAX),       // owning group
        (0x10, 7, u32::MAX),       // mask
        (0x20, 0, u32::MAX),       // others
    ] {
        value.extend_from_slice(&tag.to_le_bytes());
        value.extend_from_slice(&perm.to_le_bytes());
        value.extend_from_slice(&id.to_le_bytes());
    }
    value
}

#[test]
fn a_private_root_with_a_default_acl_is_refused_by_both_trees_alike() {
    // A default ACL on the root would be inherited by every attempt and
    // object created beneath it, granting access the 0700 mode does not.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(
        &root,
        <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
    )
    .unwrap();
    let directory = File::open(&root).unwrap();
    match rustix::fs::fsetxattr(
        &directory,
        "system.posix_acl_default",
        &default_acl(),
        rustix::fs::XattrFlags::empty(),
    ) {
        Ok(()) => {}
        Err(e) if e == Errno::NOTSUP || e == Errno::OPNOTSUPP => {
            eprintln!("skipped: this filesystem has no POSIX ACLs");
            return;
        }
        Err(e) => panic!("cannot set a default ACL: {e}"),
    }
    let preparation = crate::preparation::PreparationRoot::open(&root).unwrap_err();
    assert!(matches!(
        preparation.failure,
        crate::preparation::Failure::UnsupportedMetadata
    ));
    assert_eq!(
        preparation.to_string(),
        "private preparation refused: UnsupportedMetadata"
    );
    let capture = crate::before_images::BeforeImageRoot::open(&root).unwrap_err();
    assert!(matches!(
        capture.failure,
        crate::before_images::Failure::UnsupportedMetadata
    ));
    assert_eq!(
        capture.to_string(),
        "before-image capture refused: UnsupportedMetadata"
    );
    // Removing it (`setfacl -k`) admits the root again.
    rustix::fs::fremovexattr(&directory, "system.posix_acl_default").unwrap();
    crate::preparation::PreparationRoot::open(&root).unwrap();
    crate::before_images::BeforeImageRoot::open(&root).unwrap();
}
