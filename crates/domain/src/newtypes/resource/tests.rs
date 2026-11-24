use super::*;

const ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn opaque_id_is_exactly_64_lowercase_hex_digits() {
    let cases: &[(String, bool)] = &[
        (ID.into(), true),
        ("a".repeat(64), true),
        ("0".repeat(64), true),
        ("a".repeat(63), false),
        ("a".repeat(65), false),
        (String::new(), false),
        // Uppercase: the observation adapter's own check (`is_ascii_hexdigit`)
        // accepts it; every plan, lock and executor check refuses it.
        ("A".repeat(64), false),
        (format!("{}A", &ID[..63]), false),
        (format!("{}g", &ID[..63]), false),
        (format!("{} ", &ID[..63]), false),
    ];
    for (value, expected) in cases {
        assert_eq!(opaque_id(value), *expected, "{value:?}");
        assert_eq!(ContainerId::parse(value.as_str()).is_ok(), *expected);
    }
}

#[test]
fn container_resource_is_the_prefix_once_then_an_opaque_id() {
    let cases: &[(String, Option<&str>)] = &[
        (format!("container:{ID}"), Some(ID)),
        // A second prefix: `trim_start_matches` would strip both.
        (format!("container:container:{ID}"), None),
        (format!("Container:{ID}"), None),
        (format!(" container:{ID}"), None),
        (format!("container: {ID}"), None),
        (format!("container:{}", ID.to_uppercase()), None),
        (format!("container:{}", &ID[..63]), None),
        (format!("stack:{ID}"), None),
        (ID.into(), None),
        ("container:".into(), None),
        (String::new(), None),
    ];
    for (value, id) in cases {
        assert_eq!(container_resource_id(value), *id, "{value:?}");
        assert_eq!(container_resource(value), id.is_some(), "{value:?}");
        let parsed = ContainerResource::parse(value.as_str());
        assert_eq!(parsed.as_ref().ok().map(|r| r.id()), *id, "{value:?}");
    }
    let id = ContainerId::parse(ID).unwrap();
    assert_eq!(id.resource(), format!("container:{ID}"));
    assert_eq!(id.resource().id(), ID);
}

#[test]
fn stack_project_boundaries() {
    let cases: &[(String, bool)] = &[
        ("a".into(), true),
        ("media".into(), true),
        // A leading digit: accepted for projects, refused by the catalog
        // stack-name rule (`compose_name`), a different concept.
        ("0media".into(), true),
        ("my_stack-2".into(), true),
        ("a".repeat(64), true),
        ("a".repeat(65), false),
        (String::new(), false),
        ("-a".into(), false),
        ("_a".into(), false),
        ("Media".into(), false),
        ("a.b".into(), false),
        ("a:b".into(), false),
        ("a b".into(), false),
        ("a/b".into(), false),
    ];
    for (value, expected) in cases {
        assert_eq!(stack_project(value), *expected, "{value:?}");
        assert_eq!(StackProject::parse(value.as_str()).is_ok(), *expected);
        assert_eq!(
            stack_resource_project(&format!("stack:{value}")).is_some(),
            *expected,
            "{value:?}"
        );
    }
    assert_eq!(
        StackProject::parse("media").unwrap().resource(),
        "stack:media"
    );
    assert_eq!(stack_resource_project("stack:stack:media"), None);
    assert_eq!(stack_resource_project("media"), None);
}

#[test]
fn resources_decode_only_when_valid_and_encode_as_the_bare_string() {
    let wire = format!(r#""container:{ID}""#);
    let resource: ContainerResource = serde_json::from_str(&wire).unwrap();
    assert_eq!(serde_json::to_string(&resource).unwrap(), wire);
    assert!(serde_json::from_str::<ContainerResource>(r#""container:abcd""#).is_err());
    assert!(serde_json::from_str::<StackProject>(r#""Media""#).is_err());
    let project: StackProject = serde_json::from_str(r#""media""#).unwrap();
    assert_eq!(serde_json::to_string(&project).unwrap(), r#""media""#);
}
