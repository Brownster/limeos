//! Differential evidence for the refactor: the validator bodies as they were
//! at `9019a6ba`, kept here verbatim as reference functions, against every
//! replaced site over a generated corpus. Accept/reject (and the error kind,
//! where a site has more than one) must agree for every input.
use crate::*;

// ------------------------------------------------- reference bodies (9019a6ba)

mod old {
    use crate::PLAN_TTL_SECONDS;

    // storage.rs
    pub fn canonical_path(path: &str, under_mnt: bool) -> bool {
        path.starts_with('/')
            && path.len() <= 512
            && path.len() > 1
            && !path.chars().any(char::is_control)
            && path[1..]
                .split('/')
                .all(|part| !matches!(part, "" | "." | ".."))
            && (!under_mnt || path.starts_with("/mnt/"))
    }
    // storage_operations.rs
    pub fn canonical(path: &str, root: bool) -> bool {
        (root && path == "/")
            || (path.starts_with('/')
                && path.len() > 1
                && path.len() <= 512
                && !path.chars().any(char::is_control)
                && path[1..].split('/').all(|s| !matches!(s, "" | "." | "..")))
    }
    pub fn target_id(id: &str) -> bool {
        !id.is_empty()
            && id.len() <= 64
            && id.as_bytes()[0].is_ascii_alphanumeric()
            && id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-".contains(&c))
    }
    // storage_dependencies.rs
    pub fn container_storage_path(path: &str) -> bool {
        path.starts_with('/')
            && path.len() <= 512
            && !path.chars().any(char::is_control)
            && (path == "/" || path[1..].split('/').all(|c| !matches!(c, "" | "." | "..")))
    }
    // pools.rs
    pub fn mount_path(path: &str) -> bool {
        path.starts_with("/mnt/")
            && path.len() <= 512
            && !path.chars().any(|c| c.is_control() || c.is_whitespace())
            && !path.contains(['\\', ':', ',', '#', '*', '?', '[', ']', '='])
            && path[1..].split('/').all(|p| !matches!(p, "" | "." | ".."))
    }
    // shares.rs
    pub fn share_literal_path(path: &str) -> bool {
        path.starts_with('/')
            && (2..=512).contains(&path.len())
            && !path.contains(['\\', '%', '[', ']', '#', ';', '=', '"', '\''])
            && !path
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() && c != ' ')
            && path[1..]
                .split('/')
                .all(|p| !matches!(p, "" | "." | "..") && p.trim() == p)
    }
    // compose.rs
    pub fn compose_literal_path(value: &str) -> bool {
        value.starts_with('/')
            && value.len() <= 256
            && value != "/"
            && value
                .split('/')
                .skip(1)
                .all(|s| !s.is_empty() && s != "." && s != "..")
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/-_.".contains(&b))
    }
    // deployment/discovery.rs (live); deployment/networks.rs had the same body
    pub fn compose_path(value: &str) -> bool {
        value.starts_with('/')
            && value.len() <= 512
            && !value.contains(',')
            && !value.chars().any(char::is_control)
            && value[1..].split('/').count() <= 16
            && value[1..]
                .split('/')
                .all(|p| !matches!(p, "" | "." | "..") && p.len() <= 255)
    }
    // deployment/local_config.rs
    #[cfg(feature = "deferred")]
    pub fn local_config_path(value: &str) -> bool {
        value.len() <= 512
            && value.starts_with('/')
            && value != "/"
            && !value.contains(['$', '{', '}', '\\', '"'])
            && !value
                .chars()
                .any(|c| c.is_control() || (c.is_whitespace() && c != ' '))
            && value[1..]
                .split('/')
                .all(|s| !s.is_empty() && s != "." && s != ".." && s.trim() == s)
    }
    // deployment/quickstart.rs
    #[cfg(feature = "deferred")]
    pub fn quickstart_path(s: &str) -> bool {
        s.starts_with('/')
            && s != "/"
            && s.len() <= 512
            && !s.chars().any(char::is_control)
            && !s.contains(['$', '{', '}', '\\'])
            && s[1..]
                .split('/')
                .all(|p| !matches!(p, "" | "." | "..") && p.len() <= 255)
    }
    // deployment/container_updates.rs (inline)
    #[cfg(feature = "deferred")]
    pub fn update_config_path(path: &str) -> bool {
        path.starts_with('/')
            && path.len() <= 512
            && !path.contains([',', '$', '{', '}', '\\'])
            && !path.chars().any(char::is_control)
            && path[1..].split('/').count() <= 16
            && path[1..]
                .split('/')
                .all(|s| !matches!(s, "" | "." | "..") && s.len() <= 255)
    }
    // deployment/media_settings.rs
    #[cfg(feature = "deferred")]
    pub fn media_settings_path(s: &str) -> bool {
        let text = |s: &str, max: usize| s.len() <= max && !s.chars().any(char::is_control);
        s.len() <= 512
            && s.starts_with('/')
            && s != "/"
            && text(s, 512)
            && !s.contains(['$', '{', '}', '\\'])
            && s[1..]
                .split('/')
                .all(|p| !matches!(p, "" | "." | "..") && p.len() <= 255)
    }
    // deployment/media.rs
    #[cfg(feature = "deferred")]
    pub fn media_path(s: &str) -> bool {
        s.len() <= 512
            && s.starts_with('/')
            && s.len() > 1
            && !s.chars().any(char::is_control)
            && !["{{", "}}", "${", "$("].iter().any(|p| s.contains(p))
            && s[1..]
                .split('/')
                .all(|p| !matches!(p, "" | "." | "..") && p.len() <= 255)
    }
    /// deployment/tools.rs `Config::new`, path part: `Some("limit")`,
    /// `Some("invalid_path")` or `None` (path accepted).
    #[cfg(feature = "deferred")]
    pub fn tools_share_path(share_path: &str) -> Option<&'static str> {
        if share_path.len() > 512 {
            return Some("limit");
        }
        if !share_path.starts_with('/')
            || share_path.chars().any(char::is_control)
            || share_path[1..].split('/').any(|part| {
                matches!(part, "" | "." | "..") || part.trim() != part || part.len() > 255
            })
        {
            return Some("invalid_path");
        }
        None
    }

    // Container resources: every `strip_prefix("container:")` +
    // `opaque_id` site (operations.rs, discovery.rs, health_cohort.rs, core
    // inspect and logs, persistence polling).
    pub fn container_resource(resource: &str) -> bool {
        opaque_id_body(resource.strip_prefix("container:").unwrap_or_default())
    }
    // stack_operations.rs `StackSnapshot::project`
    pub fn stack_resource_project(resource: &str) -> Option<&str> {
        resource
            .strip_prefix("stack:")
            .filter(|p| stack_project_body(p))
    }
    // operations.rs (moved verbatim)
    pub fn opaque_id_body(value: &str) -> bool {
        value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }
    // stack_operations.rs (moved verbatim)
    pub fn stack_project_body(value: &str) -> bool {
        let first = value.as_bytes().first().copied();
        value.len() <= 64
            && first.is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            && value
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
    }
    // storage.rs, `StorageContract::validate` device id
    pub fn contract_device_id(id: &str) -> bool {
        !id.is_empty()
            && id.len() <= 64
            && (id.as_bytes()[0].is_ascii_lowercase() || id.as_bytes()[0].is_ascii_digit())
            && id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-".contains(&c))
    }
    // storage.rs, `StorageMountWaitPlan::validate` device id
    pub fn wait_device_id(id: &str) -> bool {
        !id.is_empty()
            && id.len() <= 64
            && id.as_bytes()[0].is_ascii_alphanumeric()
            && id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-".contains(&c))
    }
    // storage.rs (contract and wait plan) and protection.rs
    pub fn chosen_uuid(uuid: &str) -> bool {
        !uuid.is_empty()
            && uuid.len() <= 128
            && uuid.as_bytes()[0].is_ascii_alphanumeric()
            && uuid
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
    }
    // storage/planning.rs inventory
    pub fn inventory_uuid(uuid: &str) -> bool {
        !uuid.is_empty()
            && uuid.len() <= 128
            && uuid
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
    }

    // Plan lifetimes: the shape checks (InvalidInput) ...
    pub fn bounded(created_at: i64, expires_at: i64) -> bool {
        !(created_at < 0
            || expires_at <= created_at
            || expires_at > created_at.saturating_add(PLAN_TTL_SECONDS))
    }
    // storage_operations.rs / storage/planning.rs spelling of the same
    pub fn bounded_positive(created_at: i64, expires_at: i64) -> bool {
        created_at >= 0
            && expires_at > created_at
            && expires_at <= created_at.saturating_add(PLAN_TTL_SECONDS)
    }
    // compose.rs (`checked_add(..).ok_or(InvalidInput)?` then `==`)
    pub fn compose_exact(created_at: i64, expires_at: i64) -> bool {
        created_at >= 0
            && created_at
                .checked_add(PLAN_TTL_SECONDS)
                .is_some_and(|e| e == expires_at)
    }
    // backup_jobs.rs
    pub fn backup_exact(created_at: i64, expires_at: i64) -> bool {
        !(created_at < 0 || created_at.checked_add(PLAN_TTL_SECONDS) != Some(expires_at))
    }
    // ... and the clock check (Expired) of every plan but backup preparation
    pub fn current(created_at: i64, expires_at: i64, now: i64) -> bool {
        !(now < created_at || now >= expires_at)
    }
}

// ------------------------------------------------------------- corpora

/// Characters every exclusion, whitespace and control rule distinguishes:
/// all of ASCII, C1 controls (NEL), Unicode whitespace (NBSP, LINE
/// SEPARATOR, IDEOGRAPHIC SPACE) and 2-, 3- and 4-byte characters.
fn alphabet() -> Vec<char> {
    let mut chars: Vec<char> = (0u8..=0x7f).map(char::from).collect();
    chars.extend([
        '\u{80}',
        '\u{85}',
        '\u{9f}',
        '\u{a0}',
        '\u{e9}',
        '\u{2028}',
        '\u{3000}',
        '\u{20ac}',
        '\u{1f600}',
        '\u{feff}',
    ]);
    chars
}

/// A string of exactly `bytes` bytes after `prefix`, filled with `fill`
/// (padded with `a` where `fill` does not divide the remainder).
fn sized(prefix: &str, fill: char, bytes: usize) -> String {
    let mut s = prefix.to_owned();
    while s.len() + fill.len_utf8() <= bytes {
        s.push(fill);
    }
    while s.len() < bytes {
        s.push('a');
    }
    s
}

fn path_corpus() -> Vec<String> {
    let mut corpus: Vec<String> = [
        "",
        "/",
        "//",
        "///",
        "a",
        "a/b",
        "./a",
        "/.",
        "/..",
        "/...",
        "/a",
        "/a/",
        "/a//b",
        "/a/./b",
        "/a/../b",
        "/a/.",
        "/a/..",
        "/./a",
        "/../a",
        "/.a",
        "/a.",
        "/ a",
        "/a ",
        "/a b",
        "/ ",
        "/mnt",
        "/mnt/",
        "/mnt/a",
        "/mnt/a/",
        "/mnt//a",
        "/mnt/./a",
        "/mnt/../a",
        "/mntx/a",
        "/mnt/a/b",
        "/srv",
        "/srv/data",
        "/data/media",
        "\\a",
        "/a\\b",
        "mnt/a",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for c in alphabet() {
        for template in [
            "/{}",
            "/a{}",
            "/a{}b",
            "/mnt/{}",
            "/mnt/a{}b",
            "/mnt/{}/x",
            "{}/a",
        ] {
            corpus.push(template.replace("{}", &c.to_string()));
        }
        // A character at the very end of a path at each byte ceiling.
        for ceiling in [250usize, 256, 512] {
            for len in [ceiling - 1, ceiling, ceiling + 1] {
                if len > c.len_utf8() + 5 {
                    let mut s = sized("/mnt/", 'a', len - c.len_utf8());
                    s.push(c);
                    corpus.push(s);
                }
            }
        }
    }
    // Byte ceilings (path 250/256/512, segment 255) with multibyte fillers,
    // so a character can straddle the boundary.
    for fill in ['a', '\u{e9}', '\u{20ac}', '\u{1f600}', ' ', '.'] {
        for ceiling in [250usize, 256, 512] {
            for len in ceiling - 4..=ceiling + 4 {
                corpus.push(sized("/mnt/", fill, len));
                corpus.push(sized("/", fill, len));
            }
        }
        for segment in 252usize..=258 {
            corpus.push(sized("/mnt/", fill, 5 + segment));
            corpus.push(format!("{}/x", sized("/mnt/", fill, 5 + segment)));
        }
    }
    // Segment counts around 16.
    for count in 14..=18 {
        corpus.push("/a".repeat(count));
        corpus.push(format!("/mnt{}", "/a".repeat(count - 1)));
    }
    // Prefix x middle x suffix combinations.
    let middles = [
        "a", "a b", " a", "a ", ".", "..", "...", "", "a,b", "a$b", "a{b}", "{{x}}",
    ];
    for prefix in ["/", "/mnt/", "/srv/"] {
        for middle in middles {
            for suffix in ["", "/", "/.", "/..", "//x", "/x", "/ "] {
                corpus.push(format!("{prefix}{middle}{suffix}"));
            }
        }
    }
    corpus.sort();
    corpus.dedup();
    corpus
}

fn id_corpus() -> Vec<String> {
    let hex = "0123456789abcdef".repeat(4);
    let mut corpus: Vec<String> = vec![String::new(), hex.clone(), hex.to_uppercase()];
    for len in [0usize, 1, 2, 47, 48, 49, 63, 64, 65, 127, 128, 129] {
        for fill in ['a', '0', 'A', '-', '.', '_', '\u{e9}'] {
            corpus.push(sized("", fill, len));
            corpus.push(sized("a", fill, len.max(1)));
        }
    }
    for c in alphabet() {
        let c = c.to_string();
        corpus.push(c.clone());
        corpus.push(format!("{c}abc"));
        corpus.push(format!("ab{c}c"));
        corpus.push(format!("{}{c}", &hex[..63]));
        corpus.push(format!("{c}{}", &hex[1..]));
        corpus.push(format!("{}{c}{}", &hex[..32], &hex[33..]));
    }
    corpus.sort();
    corpus.dedup();
    corpus
}

fn resource_corpus() -> Vec<String> {
    let mut corpus = Vec::new();
    for id in id_corpus() {
        for prefix in [
            "",
            "container:",
            "container:container:",
            "Container:",
            " container:",
            "container: ",
            "container",
            "stack:",
            "stack:stack:",
            "Stack:",
        ] {
            corpus.push(format!("{prefix}{id}"));
        }
    }
    corpus
}

// --------------------------------------------------------------- tests

fn same(name: &str, corpus: &[String], old: impl Fn(&str) -> bool, new: impl Fn(&str) -> bool) {
    let mut accepted = 0;
    for value in corpus {
        let (o, n) = (old(value), new(value));
        assert_eq!(o, n, "{name}: {value:?} (old {o}, new {n})");
        accepted += usize::from(n);
    }
    // The corpus exercises both outcomes for every site.
    assert!(
        accepted > 0 && accepted < corpus.len(),
        "{name}: {accepted}"
    );
}

#[test]
fn every_replaced_live_path_validator_agrees_with_its_old_body() {
    let corpus = path_corpus();
    assert!(corpus.len() > 2500, "{}", corpus.len());
    for under_mnt in [false, true] {
        same(
            "storage::canonical_path",
            &corpus,
            |p| old::canonical_path(p, under_mnt),
            |p| crate::storage::canonical_path(p, under_mnt),
        );
    }
    for root in [false, true] {
        same(
            "storage_operations::canonical",
            &corpus,
            |p| old::canonical(p, root),
            |p| crate::storage_operations::canonical(p, root),
        );
    }
    same(
        "container_storage_path",
        &corpus,
        old::container_storage_path,
        crate::container_storage_path,
    );
    same(
        "pools::mount_path",
        &corpus,
        old::mount_path,
        crate::pools::mount_path,
    );
    same(
        "shares::literal_path",
        &corpus,
        old::share_literal_path,
        crate::shares::literal_path,
    );
    same(
        "compose::literal_path",
        &corpus,
        old::compose_literal_path,
        crate::compose::literal_path,
    );
    same(
        "discovery::compose_path",
        &corpus,
        old::compose_path,
        crate::deployment::discovery::compose_path,
    );
}

#[cfg(feature = "deferred")]
#[test]
fn every_replaced_deferred_path_validator_agrees_with_its_old_body() {
    use crate::deployment::*;
    let corpus = path_corpus();
    same(
        "local_config::path",
        &corpus,
        old::local_config_path,
        local_config::path,
    );
    same(
        "quickstart::path",
        &corpus,
        old::quickstart_path,
        quickstart::path,
    );
    same(
        "container_updates::config_path",
        &corpus,
        old::update_config_path,
        container_updates::config_path,
    );
    same(
        "media_settings::path",
        &corpus,
        old::media_settings_path,
        media_settings::path,
    );
    same("media::path", &corpus, old::media_path, media::path);
    same("networks::path", &corpus, old::compose_path, networks::path);
    // Two error kinds: compare which one, not only whether.
    for value in &corpus {
        let new = match tools::Config::new(value, 8080) {
            Err(tools::ConfigIssue::Limit) => Some("limit"),
            Err(tools::ConfigIssue::InvalidPath) => Some("invalid_path"),
            Err(other) => panic!("{value:?}: {other:?}"),
            Ok(_) => None,
        };
        assert_eq!(old::tools_share_path(value), new, "tools: {value:?}");
    }
}

#[test]
fn every_replaced_id_validator_agrees_with_its_old_body() {
    let ids = id_corpus();
    same("opaque_id", &ids, old::opaque_id_body, opaque_id);
    same(
        "stack_project",
        &ids,
        old::stack_project_body,
        stack_project,
    );
    same(
        "target_id",
        &ids,
        old::target_id,
        crate::storage_operations::target_id,
    );
    same(
        "contract device id",
        &ids,
        old::contract_device_id,
        storage_device_id,
    );
    same(
        "wait-plan device id",
        &ids,
        old::wait_device_id,
        storage_device_id,
    );
    same("chosen uuid", &ids, old::chosen_uuid, filesystem_uuid);
    same(
        "inventory uuid",
        &ids,
        old::inventory_uuid,
        observed_filesystem_uuid,
    );
    let resources = resource_corpus();
    same(
        "container resource",
        &resources,
        old::container_resource,
        container_resource,
    );
    for value in &resources {
        assert_eq!(
            old::stack_resource_project(value),
            stack_resource_project(value),
            "stack resource: {value:?}"
        );
        // The typed forms accept exactly what the predicates accept.
        assert_eq!(
            ContainerResource::parse(value.as_str()).is_ok(),
            container_resource(value)
        );
    }
}

/// `ResolutionEvidence::observe` took the ID with a bare `strip_prefix` and
/// let `validate` refuse a non-opaque one; it now refuses at the prefix.
/// The outcome is the same for every resource: refused exactly when the old
/// path would have refused, with the same code.
#[test]
fn resolution_evidence_refuses_exactly_the_resources_it_refused_before() {
    for resource in resource_corpus() {
        let before = ContainerSnapshot {
            resource: resource.clone(),
            image: format!("sha256:{}", "b".repeat(64)),
            started_at: "2026-10-05T07:00:00Z".into(),
            running: true,
        };
        let old_refused = !resource
            .strip_prefix("container:")
            .is_some_and(old::opaque_id_body);
        let new = ResolutionEvidence::observe(ContainerAction::Restart, &[&before], &[None]);
        assert_eq!(new.is_err(), old_refused, "{resource:?}");
        if let Err(error) = new {
            assert_eq!(error, Error(ErrorCode::InvalidInput));
        }
    }
}

#[test]
fn every_replaced_lifetime_check_agrees_with_its_old_body() {
    let ttl = PLAN_TTL_SECONDS;
    let mut points = vec![
        i64::MIN,
        i64::MIN + 1,
        -ttl - 1,
        -1,
        0,
        1,
        ttl - 1,
        ttl,
        ttl + 1,
    ];
    for base in [
        100,
        i64::MAX - ttl - 1,
        i64::MAX - ttl,
        i64::MAX - 1,
        i64::MAX,
    ] {
        for d in [-ttl - 1, -ttl, -1, 0, 1, ttl - 1, ttl, ttl + 1] {
            points.push(base.saturating_add(d));
        }
    }
    points.sort();
    points.dedup();
    for &created in &points {
        for d in [-1, 0, 1, 2, ttl - 1, ttl, ttl + 1, i64::MAX] {
            let expires = created.saturating_add(d);
            let bounded = PlanLifetime::bounded(created, expires);
            assert_eq!(
                bounded.is_ok(),
                old::bounded(created, expires),
                "{created} {expires}"
            );
            assert_eq!(bounded.is_ok(), old::bounded_positive(created, expires));
            let exact = PlanLifetime::exact(created, expires);
            assert_eq!(exact.is_ok(), old::compose_exact(created, expires));
            assert_eq!(exact.is_ok(), old::backup_exact(created, expires));
            for lifetime in [bounded, exact].into_iter().flatten() {
                for now in [
                    created.saturating_sub(1),
                    created,
                    created.saturating_add(1),
                    expires.saturating_sub(1),
                    expires,
                    expires.saturating_add(1),
                ] {
                    let current = lifetime.current(now);
                    assert_eq!(current.is_ok(), old::current(created, expires, now));
                    if let Err(error) = current {
                        assert_eq!(error, Error(ErrorCode::Expired));
                    }
                }
            }
        }
    }
}
