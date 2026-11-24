use super::*;

/// Every boundary of the absolute-path rule, including the spellings the
/// replaced validators disagreed on (noted per row).
#[test]
fn absolute_path_accepts_exactly_the_canonical_absolute_spellings() {
    let long = format!("/{}", "a".repeat(MAX_PATH_BYTES - 1));
    let too_long = format!("/{}", "a".repeat(MAX_PATH_BYTES));
    let cases: &[(&str, bool)] = &[
        // The root: a valid path; storage locations refuse it by context.
        ("/", true),
        ("/a", true),
        ("/mnt/data/media", true),
        // Dot-prefixed names are ordinary components.
        ("/a/.b", true),
        ("/a/..b", true),
        ("/a/...", true),
        ("/.hidden", true),
        // Characters the core rule leaves to context rules.
        ("/a b", true),
        ("/a,b", true),
        ("/a:b", true),
        ("/a\\b", true),
        ("/caf\u{e9}", true),
        (&long, true),
        (&too_long, false),
        ("", false),
        ("a", false),
        ("a/b", false),
        ("./a", false),
        // Empty components. The core configuration check accepted `//`
        // (only `/../` was refused) and `Path::components` hides it.
        ("//", false),
        ("/a//b", false),
        ("/a/", false),
        // `.` and `..` anywhere. `Path::components` drops an inner `.`, so a
        // check built on it accepted `/a/./b`.
        ("/.", false),
        ("/./a", false),
        ("/a/./b", false),
        ("/a/.", false),
        ("/..", false),
        ("/../a", false),
        ("/a/../b", false),
        ("/a/..", false),
        // Control characters, including NUL, DEL and C1 controls.
        ("/a\0", false),
        ("/a\nb", false),
        ("/a\tb", false),
        ("/a\u{7f}", false),
        ("/a\u{85}", false),
    ];
    for (value, expected) in cases {
        assert_eq!(absolute_path(value), *expected, "{value:?}");
        assert_eq!(AbsolutePath::parse(*value).is_ok(), *expected, "{value:?}");
    }
}

#[test]
fn storage_paths_refuse_the_root_and_require_a_component_beneath_mnt_when_asked() {
    let cases: &[(&str, bool, bool)] = &[
        ("/", false, false),
        ("/mnt", false, true),
        ("/mnt/", false, false),
        ("/mnt/a", true, true),
        ("/mnt/a/b", true, true),
        ("/mntx/a", false, true),
        ("/srv/a", false, true),
        ("/mnt//a", false, false),
        ("/mnt/../etc", false, false),
    ];
    for (value, under_mnt, anywhere) in cases {
        assert_eq!(
            storage_path(value, true),
            *under_mnt,
            "{value:?} under /mnt"
        );
        assert_eq!(storage_path(value, false), *anywhere, "{value:?}");
    }
}

#[test]
fn beneath_compares_whole_components() {
    let path = |value: &str| AbsolutePath::parse(value).unwrap();
    assert!(path("/mnt/a").is_beneath("/mnt"));
    assert!(path("/mnt/a/b").is_beneath("/mnt/a"));
    assert!(!path("/mnt").is_beneath("/mnt"));
    assert!(!path("/mntx/a").is_beneath("/mnt"));
    assert!(!path("/mn").is_beneath("/mnt"));
    assert!(path("/a").is_beneath("/"));
    assert!(!path("/").is_beneath("/"));
    assert!(path("/").is_root());
    assert_eq!(path("/").components().count(), 0);
    assert_eq!(path("/a/b").components().collect::<Vec<_>>(), ["a", "b"]);
}

#[test]
fn absolute_path_round_trips_as_a_bare_string_and_refuses_an_invalid_one() {
    let path: AbsolutePath = serde_json::from_str(r#""/mnt/data""#).unwrap();
    assert_eq!(path, "/mnt/data");
    assert_eq!(serde_json::to_string(&path).unwrap(), r#""/mnt/data""#);
    assert_eq!(format!("{path}"), "/mnt/data");
    assert_eq!(format!("{path:?}"), format!("{:?}", "/mnt/data"));
    for invalid in [r#""/mnt/../etc""#, r#""relative""#, r#""""#, "7", "null"] {
        assert!(
            serde_json::from_str::<AbsolutePath>(invalid).is_err(),
            "{invalid}"
        );
    }
    assert_eq!(
        AbsolutePath::parse("x").unwrap_err(),
        Error(crate::ErrorCode::InvalidInput)
    );
}

#[test]
fn schema_and_typescript_type_are_those_of_a_string() {
    use schemars::schema_for;
    use ts_rs::TS;
    assert_eq!(
        serde_json::to_value(schema_for!(AbsolutePath)).unwrap(),
        serde_json::to_value(schema_for!(String)).unwrap()
    );
    assert_eq!(AbsolutePath::name(), String::name());
    assert_eq!(AbsolutePath::inline(), "string");
}
