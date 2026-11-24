use super::*;

#[test]
fn storage_device_id_boundaries() {
    let cases: &[(String, bool)] = &[
        ("a".into(), true),
        ("0".into(), true),
        ("data-1_b".into(), true),
        ("a".repeat(64), true),
        ("a".repeat(65), false),
        (String::new(), false),
        ("-a".into(), false),
        ("_a".into(), false),
        // The wait-plan copy tested the first byte with
        // `is_ascii_alphanumeric`, the contract copy with lowercase-or-digit;
        // the alphabet refuses uppercase either way, so they agreed.
        ("Ab".into(), false),
        ("aB".into(), false),
        ("a.b".into(), false),
        ("a b".into(), false),
    ];
    for (value, expected) in cases {
        assert_eq!(storage_device_id(value), *expected, "{value:?}");
    }
}

#[test]
fn filesystem_uuid_rules_differ_only_in_the_first_byte() {
    // (value, chosen by a contract, reported by the host)
    let cases: &[(String, bool, bool)] = &[
        ("0b6f0c55-8d2a-4b6e-9d1c-3c4f5a6b7c8d".into(), true, true),
        ("ABCD-1234".into(), true, true),
        ("a".into(), true, true),
        ("a.b_c-d".into(), true, true),
        ("a".repeat(128), true, true),
        ("a".repeat(129), false, false),
        (String::new(), false, false),
        ("-abc".into(), false, true),
        (".abc".into(), false, true),
        ("_abc".into(), false, true),
        ("..".into(), false, true),
        ("a b".into(), false, false),
        ("a:b".into(), false, false),
        ("a/b".into(), false, false),
        ("a\u{e9}".into(), false, false),
    ];
    for (value, chosen, reported) in cases {
        assert_eq!(filesystem_uuid(value), *chosen, "{value:?}");
        assert_eq!(observed_filesystem_uuid(value), *reported, "{value:?}");
    }
}
