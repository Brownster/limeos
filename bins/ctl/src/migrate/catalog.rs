//! `migrate propose-catalog`: a PROPOSED management catalog for stack status and
//! whole-stack actions, built from Compose project labels. Read-only against Docker
//! and the stacks root. The proposal is a new file the operator reviews; only an
//! explicit `--install` also places it, root-owned, at the path the stack reader loads.
//!
//! Catalog `stack` is the stack directory's name; `project` is the Compose project
//! label. They can differ (a project name is lower-cased and sanitised from the
//! directory, or set with `-p`), and both are recorded. The catalog bounds (8 stacks,
//! 16 services each) are enforced here; whatever does not fit is refused by name.
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const INSTALLED: &str = "/etc/limeos/management-catalog.json";
const COMPOSE_FILES: [&str; 4] = [
    "compose.yaml",
    "compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
];
const MANAGED_FILE: &str = limeos_domain::COMPOSE_MANAGED_FILE;
const FILE_LIMIT: u64 = 64 * 1024;
const OUTPUT_LIMIT: usize = 32 << 20;
const INSPECT_BATCH: usize = 50;
const MAX_STACKS: usize = 8;
const MAX_SERVICES: usize = 16;
const DOCKERS: [&str; 3] = ["/usr/bin/docker", "/usr/local/bin/docker", "/bin/docker"];

/// What the labels of one container say; everything else about it is ignored.
#[derive(Debug)]
pub struct Member {
    pub project: String,
    pub service: String,
    pub one_off: bool,
    pub working_dir: Option<String>,
    pub config_files: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Registration {
    pub stack: String,
    pub project: String,
    pub working_directory: String,
    pub operator: Operator,
    pub managed: Managed,
    pub services: Vec<String>,
}
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Operator {
    pub name: String,
    pub sha256: String,
}
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Managed {
    Absent {},
    Present { sha256: String },
}
#[derive(Debug, Serialize)]
struct Catalog<'a> {
    version: u16,
    engine_id: &'a str,
    stacks: &'a [Registration],
}

pub struct Proposal {
    pub registered: Vec<Registration>,
    /// (what was refused, why), in a stable order.
    pub refused: Vec<(String, String)>,
    pub unlabelled: usize,
    pub one_offs: usize,
    pub json: String,
}

fn label<'a>(labels: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    labels.get(key)?.as_str()
}

/// Reads `docker inspect` output (one JSON array, or several concatenated by batches).
/// Containers without a Compose project label are counted by the caller, not listed.
pub fn parse_inspect(text: &str) -> Result<(Vec<Member>, usize), String> {
    let mut rows: Vec<serde_json::Value> = Vec::new();
    for batch in serde_json::Deserializer::from_str(text).into_iter::<Vec<serde_json::Value>>() {
        rows.extend(batch.map_err(|_| "docker inspect output is not a JSON array")?);
    }
    let (mut members, mut unlabelled) = (Vec::new(), 0);
    for row in rows {
        let labels = row
            .pointer("/Config/Labels")
            .filter(|l| l.is_object())
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let text = |key| label(&labels, key).map(str::to_owned);
        let (Some(project), Some(service)) = (
            text("com.docker.compose.project"),
            text("com.docker.compose.service"),
        ) else {
            unlabelled += 1;
            continue;
        };
        members.push(Member {
            project,
            service,
            one_off: label(&labels, "com.docker.compose.oneoff")
                .is_some_and(|v| v.eq_ignore_ascii_case("true")),
            working_dir: text("com.docker.compose.project.working_dir"),
            config_files: text("com.docker.compose.project.config_files"),
        });
    }
    Ok((members, unlabelled))
}

/// The catalog's own project rule (`ManagementCatalog::validate`).
fn catalog_project(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-".contains(&b))
}

fn plain_file(path: &Path) -> Option<u64> {
    std::fs::symlink_metadata(path)
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.len())
}
fn hash_file(path: &Path) -> Result<String, String> {
    match plain_file(path) {
        Some(len) if len <= FILE_LIMIT => {}
        Some(_) => return Err(format!("{} is larger than 64 KiB", path.display())),
        None => return Err(format!("{} is not a regular file", path.display())),
    }
    let bytes = std::fs::read(path).map_err(|_| format!("cannot read {}", path.display()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

/// One project's registration, or the reason it cannot be one.
fn register(root: &Path, project: &str, members: &[&Member]) -> Result<Registration, String> {
    let dirs: Vec<_> = members
        .iter()
        .filter_map(|m| m.working_dir.as_deref())
        .collect();
    let Some(&dir) = dirs.first() else {
        return Err("containers carry no working_dir label (not started by Compose?)".into());
    };
    if dirs.len() != members.len() || dirs.iter().any(|d| *d != dir) {
        return Err("containers disagree about the project's working directory".into());
    }
    let dir_path = Path::new(dir);
    if dir_path.parent() != Some(root) {
        return Err(format!(
            "working directory {dir} is not directly under the stacks root {}",
            root.display()
        ));
    }
    let stack = dir_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("working directory has no usable name")?;
    if !limeos_domain::compose_name(stack) {
        return Err(format!(
            "directory name {stack:?} is not a catalog stack name (lowercase letters, digits, - and _, starting with a letter, at most 48)"
        ));
    }
    if !catalog_project(project) {
        return Err(format!(
            "project name {project:?} is not valid in the catalog"
        ));
    }
    let named: Vec<_> = members
        .iter()
        .filter_map(|m| m.config_files.as_deref())
        .collect();
    let file =
        match named.first() {
            Some(&first) if named.len() == members.len() && named.iter().all(|c| *c == first) => {
                if first.contains(',') {
                    return Err(
                        "the project uses several Compose files; the catalog registers one".into(),
                    );
                }
                let path = Path::new(first);
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if path.parent() != Some(dir_path) || !COMPOSE_FILES.contains(&name) {
                    return Err(format!(
                        "Compose file {first} is not one of {} in the working directory",
                        COMPOSE_FILES.join(", ")
                    ));
                }
                name.to_owned()
            }
            Some(_) => return Err("containers disagree about the project's Compose file".into()),
            None => {
                let found: Vec<_> = COMPOSE_FILES
                    .iter()
                    .filter(|f| plain_file(&dir_path.join(f)).is_some())
                    .collect();
                match found.as_slice() {
                    [one] => (**one).to_owned(),
                    _ => return Err(
                        "no config_files label and not exactly one Compose file in the directory"
                            .into(),
                    ),
                }
            }
        };
    let sha256 = hash_file(&dir_path.join(&file))?;
    let managed_path = dir_path.join(MANAGED_FILE);
    let managed = if std::fs::symlink_metadata(&managed_path).is_ok() {
        Managed::Present {
            sha256: hash_file(&managed_path)?,
        }
    } else {
        Managed::Absent {}
    };
    let mut services: Vec<String> = members.iter().map(|m| m.service.clone()).collect();
    services.sort();
    services.dedup();
    if services.len() > MAX_SERVICES {
        return Err(format!(
            "{} services exceed the catalog limit of {MAX_SERVICES}",
            services.len()
        ));
    }
    if let Some(bad) = services.iter().find(|s| !limeos_domain::compose_name(s)) {
        return Err(format!("service name {bad:?} is not valid in the catalog"));
    }
    Ok(Registration {
        stack: stack.to_owned(),
        project: project.to_owned(),
        working_directory: dir.to_owned(),
        operator: Operator { name: file, sha256 },
        managed,
        services,
    })
}

/// `idle`: stack directories under the root that hold a Compose file, from
/// `ceiling::list_stacks`; those without any container are reported, not registered.
pub fn propose(
    engine_id: &str,
    root: &Path,
    members: &[Member],
    unlabelled: usize,
    idle: &[String],
) -> Result<Proposal, String> {
    if engine_id.is_empty()
        || engine_id.len() > 128
        || !engine_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
    {
        return Err("the Docker engine ID is not usable in the catalog".into());
    }
    let one_offs = members.iter().filter(|m| m.one_off).count();
    let mut projects: BTreeMap<&str, Vec<&Member>> = BTreeMap::new();
    for m in members.iter().filter(|m| !m.one_off) {
        projects.entry(&m.project).or_default().push(m);
    }
    let (mut registered, mut refused) = (Vec::new(), Vec::new());
    for (project, group) in &projects {
        let note = format!("{} container(s)", group.len());
        match register(root, project, group) {
            Ok(r) => registered.push(r),
            Err(why) => refused.push((format!("project {project} ({note})"), why)),
        }
    }
    registered.sort_by(|a, b| a.stack.cmp(&b.stack));
    for pair in registered.windows(2) {
        if pair[0].stack == pair[1].stack {
            return Err(format!(
                "projects {} and {} share the stack directory {}; resolve by hand",
                pair[0].project, pair[1].project, pair[0].stack
            ));
        }
    }
    let seen: Vec<_> = members
        .iter()
        .filter(|m| !m.one_off)
        .filter_map(|m| Path::new(m.working_dir.as_deref()?).file_name()?.to_str())
        .collect();
    for name in idle {
        if !seen.contains(&name.as_str()) {
            refused.push((
                format!("stack directory {name}"),
                "has a Compose file but no containers, so its services are unknown; create it with Compose and run this again".into(),
            ));
        }
    }
    while registered.len() > MAX_STACKS {
        let extra = registered.pop().expect("longer than the limit");
        refused.push((
            format!("project {} (stack {})", extra.project, extra.stack),
            format!("over the catalog limit of {MAX_STACKS} stacks"),
        ));
    }
    if registered.is_empty() {
        return Err("no stack can be registered; see the refusals above".into());
    }
    let json = serde_json::to_string_pretty(&Catalog {
        version: 1,
        engine_id,
        stacks: &registered,
    })
    .map_err(|_| "cannot encode the catalog")?;
    Ok(Proposal {
        registered,
        refused,
        unlabelled,
        one_offs,
        json: json + "\n",
    })
}

/// Read-only Docker verbs only. Output is bounded and never echoed.
fn docker(binary: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new(binary)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .output()
        .map_err(|_| "cannot run docker".to_owned())?;
    if !output.status.success() || output.stdout.len() > OUTPUT_LIMIT {
        return Err(format!("`docker {}` failed or returned too much", args[0]));
    }
    String::from_utf8(output.stdout).map_err(|_| "docker output is not text".to_owned())
}

fn collect(binary: &Path) -> Result<(String, String), String> {
    let engine = docker(binary, &["info", "--format", "{{.ID}}"])?;
    let ids = docker(binary, &["ps", "-a", "-q", "--no-trunc"])?;
    let ids: Vec<_> = ids.split_whitespace().collect();
    let mut inspected = String::new();
    for batch in ids.chunks(INSPECT_BATCH) {
        let mut args = vec!["inspect", "--type", "container"];
        args.extend(batch);
        inspected.push_str(&docker(binary, &args)?);
        inspected.push('\n');
    }
    Ok((engine.trim().to_owned(), inspected))
}

fn find_docker() -> Result<PathBuf, String> {
    DOCKERS
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
        .ok_or_else(|| {
            "docker not found in /usr/bin, /usr/local/bin or /bin; pass --docker PATH".into()
        })
}

/// Places the reviewed text where the stack reader loads it. Refuses to replace anything:
/// a temporary file is linked into place, and linking fails if the name exists.
pub fn install(path: &Path, json: &str, owner: u32) -> Result<(), String> {
    let parent = path.parent().ok_or("the install path has no directory")?;
    let meta = std::fs::symlink_metadata(parent).map_err(|_| "the install directory is missing")?;
    if !meta.is_dir() || meta.uid() != owner || meta.mode() & 0o022 != 0 {
        return Err("the install directory is not a root-owned directory closed to others".into());
    }
    if std::fs::symlink_metadata(path).is_ok() {
        return Err("the installed catalog already exists; nothing was overwritten".into());
    }
    let temporary = parent.join(".management-catalog.json.new");
    super::ceiling::write_new_with_mode(&temporary, json, 0o644)?;
    let linked = std::fs::hard_link(&temporary, path);
    let _ = std::fs::remove_file(&temporary);
    linked.map_err(|_| "cannot install the catalog (it may have appeared meanwhile)".into())
}

pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let (mut out, mut root, mut dock, mut inspect, mut engine) =
        (None, "/opt/stacks".to_owned(), None, None, None);
    let mut install_it = false;
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        if flag == "--install" {
            install_it = true;
            continue;
        }
        if !matches!(
            flag.as_str(),
            "--out" | "--stacks" | "--docker" | "--docker-inspect" | "--engine-id"
        ) {
            return Err(crate::args::Usage(
                "unknown propose-catalog argument; run limeosctl --help".into(),
            )
            .into());
        }
        let value = crate::args::value(flag, it.next())?;
        match flag.as_str() {
            "--out" => out = Some(value.clone()),
            "--stacks" => root = value.clone(),
            "--docker" => dock = Some(PathBuf::from(value)),
            "--docker-inspect" => inspect = Some(value.clone()),
            _ => engine = Some(value.clone()),
        }
    }
    let Some(out) = out else {
        return Err(crate::args::Usage(
            "usage: migrate propose-catalog --out FILE [--stacks DIR] [--install]".into(),
        )
        .into());
    };
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(
            "migrate propose-catalog is root-only so it can read Docker and the Compose files"
                .into(),
        );
    }
    if std::fs::symlink_metadata(&out).is_ok() {
        return Err("the output file already exists; nothing was overwritten".into());
    }
    let root = PathBuf::from(root.trim_end_matches('/'));
    let (engine_id, inspected) = match (inspect, engine) {
        (Some(file), Some(id)) => {
            let meta =
                std::fs::symlink_metadata(&file).map_err(|_| "cannot read the inspect file")?;
            if !meta.is_file() || meta.len() > OUTPUT_LIMIT as u64 {
                return Err("the inspect file is not a regular file of a sane size".into());
            }
            (
                id,
                std::fs::read_to_string(&file).map_err(|_| "cannot read the inspect file")?,
            )
        }
        (None, None) => collect(&match dock {
            Some(path) => path,
            None => find_docker()?,
        })?,
        _ => return Err("--docker-inspect and --engine-id go together".into()),
    };
    let (members, unlabelled) = parse_inspect(&inspected)?;
    let idle = super::ceiling::list_stacks(&root)?;
    let proposal = propose(&engine_id, &root, &members, unlabelled, &idle)?;
    super::ceiling::write_new_with_mode(Path::new(&out), &proposal.json, 0o644)?;
    for r in &proposal.registered {
        println!(
            "stack {} (project {}) {}/{} services={}{}",
            r.stack,
            r.project,
            r.working_directory,
            r.operator.name,
            r.services.join(","),
            if matches!(r.managed, Managed::Present { .. }) {
                " +limeos.override.json"
            } else {
                ""
            }
        );
    }
    for (what, why) in &proposal.refused {
        println!("refused {what}: {why}");
    }
    println!(
        "ignored: {} container(s) without a Compose project, {} one-off container(s)",
        proposal.unlabelled, proposal.one_offs
    );
    println!(
        "wrote {out}: {} stack(s) registered, {} refused.",
        proposal.registered.len(),
        proposal.refused.len()
    );
    if install_it {
        install(Path::new(INSTALLED), &proposal.json, 0)?;
        println!(
            "installed {INSTALLED} (root-owned, mode 0644). Start limeos-storage-reader.service to use it."
        );
    } else {
        println!("Review it, then: install -o root -g root -m 0644 {out} {INSTALLED}");
    }
    std::io::stdout().flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use limeos_executor_container::compose_management::catalog::ManagementCatalog;

    const MEDIA: [&str; 14] = [
        "jellyfin",
        "radarr",
        "sonarr",
        "sabnzbd",
        "transmission",
        "jackett",
        "vpn",
        "navidrome",
        "audiobookshelf",
        "rdtclient",
        "requestrr",
        "get_iplayer",
        "yt-to-jellyfin",
        "watchtower",
    ];
    const ENGINE: &str = "ABCD:EFGH:IJKL";

    fn row(project: &str, service: &str, dir: &str, files: &str, one_off: bool) -> String {
        let mut labels = serde_json::json!({
            "com.docker.compose.project": project,
            "com.docker.compose.service": service,
            "com.docker.compose.oneoff": if one_off { "True" } else { "False" },
        });
        if !dir.is_empty() {
            labels["com.docker.compose.project.working_dir"] = dir.into();
        }
        if !files.is_empty() {
            labels["com.docker.compose.project.config_files"] = files.into();
        }
        serde_json::json!({"Id": "0".repeat(64), "Config": {"Labels": labels}}).to_string()
    }
    struct Host {
        _dir: tempfile::TempDir,
        root: PathBuf,
    }
    /// wybie's shape: media (14), mattermost (3), `docker` (1, outside the root), one-offs.
    fn host() -> Host {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("stacks");
        for (name, file) in [
            ("media", "compose.yaml"),
            ("mattermost", "docker-compose.yml"),
        ] {
            std::fs::create_dir_all(root.join(name)).unwrap();
            std::fs::write(root.join(name).join(file), format!("services: {name}")).unwrap();
        }
        Host { _dir: dir, root }
    }
    fn media_row(h: &Host, service: &str, one_off: bool) -> String {
        let r = h.root.display();
        let (dir, file) = (format!("{r}/media"), format!("{r}/media/compose.yaml"));
        row("media", service, &dir, &file, one_off)
    }
    fn wybie(h: &Host) -> String {
        let r = h.root.display();
        let mut rows: Vec<String> = MEDIA.iter().map(|s| media_row(h, s, false)).collect();
        for s in ["mattermost", "postgres", "mattermost"] {
            let (dir, file) = (
                format!("{r}/mattermost"),
                format!("{r}/mattermost/docker-compose.yml"),
            );
            rows.push(row("mattermost", s, &dir, &file, false));
        }
        rows.push(row(
            "docker",
            "portainer",
            "/home/holly/docker",
            "/home/holly/docker/compose.yaml",
            false,
        ));
        rows.push(media_row(h, "radarr", true));
        rows.push(r#"{"Id":"x","Config":{"Labels":{"org.x":"y"}}}"#.into());
        rows.push(r#"{"Id":"y","Config":{"Labels":null}}"#.into());
        format!("[{}]", rows.join(","))
    }
    fn propose_from(h: &Host, text: &str) -> Result<Proposal, String> {
        let (members, unlabelled) = parse_inspect(text)?;
        let idle = super::super::ceiling::list_stacks(&h.root)?;
        propose(ENGINE, &h.root, &members, unlabelled, &idle)
    }
    fn refusal(h: &Host, project: &str, row_text: &str) -> String {
        let (members, _) = parse_inspect(&format!("[{row_text}]")).unwrap();
        register(&h.root, project, &members.iter().collect::<Vec<_>>()).unwrap_err()
    }
    fn valid(p: &Proposal) {
        let catalog: ManagementCatalog = serde_json::from_str(&p.json).unwrap();
        catalog.validate().unwrap();
    }

    #[test]
    fn wybie_shape_registers_media_and_mattermost_and_refuses_the_outsider() {
        let h = host();
        let p = propose_from(&h, &wybie(&h)).unwrap();
        let names: Vec<_> = p
            .registered
            .iter()
            .map(|r| (r.stack.as_str(), r.services.len()))
            .collect();
        assert_eq!(names, [("mattermost", 2), ("media", 14)]);
        assert_eq!(p.registered[0].operator.name, "docker-compose.yml");
        assert_eq!(p.registered[1].operator.name, "compose.yaml");
        assert_eq!(
            p.registered[1].operator.sha256,
            hex::encode(Sha256::digest(b"services: media"))
        );
        assert_eq!((p.refused.len(), p.unlabelled, p.one_offs), (1, 2, 1));
        assert!(p.refused[0].0.starts_with("project docker "));
        assert!(
            p.refused[0]
                .1
                .contains("not directly under the stacks root")
        );
        // The executor's own validator accepts exactly what was written.
        valid(&p);
        let value: serde_json::Value = serde_json::from_str(&p.json).unwrap();
        assert_eq!(value["engine_id"], ENGINE);
        assert_eq!(
            value["stacks"][1]["managed"],
            serde_json::json!({"state": "absent"})
        );
    }

    #[test]
    fn one_offs_never_count_as_services_or_stacks() {
        let h = host();
        let (only, live) = (
            media_row(&h, "migrate", true),
            media_row(&h, "radarr", false),
        );
        let p = propose_from(&h, &format!("[{only},{live}]")).unwrap();
        assert_eq!(p.registered[0].services, ["radarr"]);
        let err = propose_from(&h, &format!("[{only}]")).err().unwrap();
        assert!(err.contains("no stack can be registered"));
    }

    #[test]
    fn stack_name_and_project_name_may_differ() {
        let h = host();
        let r = h.root.display();
        std::fs::create_dir(h.root.join("my-apps")).unwrap();
        std::fs::write(h.root.join("my-apps/compose.yml"), "x").unwrap();
        let (dir, file) = (format!("{r}/my-apps"), format!("{r}/my-apps/compose.yml"));
        let p = propose_from(
            &h,
            &format!("[{}]", row("apps_prod", "web", &dir, &file, false)),
        )
        .unwrap();
        let reg = &p.registered[0];
        assert_eq!(
            (reg.stack.as_str(), reg.project.as_str()),
            ("my-apps", "apps_prod")
        );
        valid(&p);
    }

    #[test]
    fn directories_without_containers_are_reported_not_registered() {
        let h = host();
        let p = propose_from(&h, &format!("[{}]", media_row(&h, "radarr", false))).unwrap();
        assert_eq!(p.registered.len(), 1);
        assert_eq!(p.refused.len(), 1);
        assert_eq!(p.refused[0].0, "stack directory mattermost");
    }

    #[test]
    fn bounds_are_enforced_and_refusals_named() {
        let h = host();
        let r = h.root.display();
        // 17 services in one project is refused; the other stack still registers.
        let mut rows: Vec<String> = (0..17)
            .map(|i| media_row(&h, &format!("svc{i}"), false))
            .collect();
        let (dir, file) = (
            format!("{r}/mattermost"),
            format!("{r}/mattermost/docker-compose.yml"),
        );
        rows.push(row("mattermost", "app", &dir, &file, false));
        let p = propose_from(&h, &format!("[{}]", rows.join(","))).unwrap();
        assert_eq!(p.registered.len(), 1);
        assert!(p.refused[0].1.contains("17 services exceed"));
        // 10 stacks keep the first 8 by name and refuse the rest.
        let mut rows = Vec::new();
        for i in 0..10 {
            let name = format!("s{i}");
            std::fs::create_dir(h.root.join(&name)).unwrap();
            std::fs::write(h.root.join(&name).join("compose.yaml"), "x").unwrap();
            let (dir, file) = (format!("{r}/{name}"), format!("{r}/{name}/compose.yaml"));
            rows.push(row(&name, "a", &dir, &file, false));
        }
        let p = propose_from(&h, &format!("[{}]", rows.join(","))).unwrap();
        assert_eq!(p.registered.len(), 8);
        assert_eq!(p.registered[7].stack, "s7");
        assert!(
            p.refused
                .iter()
                .any(|(w, why)| w.contains("s9") && why.contains("limit of 8 stacks"))
        );
        valid(&p);
    }

    #[test]
    fn unsafe_or_unusual_sources_are_refused() {
        let h = host();
        let r = h.root.display();
        let m = format!("{r}/media");
        let cases = [
            (
                "media",
                "/tmp/media",
                "/tmp/media/compose.yaml",
                "directly under",
            ),
            (
                "media",
                &format!("{m}/sub"),
                &format!("{m}/sub/compose.yaml"),
                "directly under",
            ),
            (
                "media",
                &m,
                &format!("{m}/a.yaml,{m}/b.yaml"),
                "several Compose files",
            ),
            ("media", &m, &format!("{m}/override.yaml"), "not one of"),
            ("Media", &m, &format!("{m}/compose.yaml"), "project name"),
        ];
        for (project, dir, files, fragment) in cases {
            let why = refusal(&h, project, &row(project, "a", dir, files, false));
            assert!(why.contains(fragment), "{why}");
        }
        // No labels at all.
        let why = refusal(&h, "media", &row("media", "a", "", "", false));
        assert!(why.contains("no working_dir label"), "{why}");
        // A symlinked Compose file is never hashed.
        std::fs::remove_file(h.root.join("media/compose.yaml")).unwrap();
        std::os::unix::fs::symlink("/etc/hostname", h.root.join("media/compose.yaml")).unwrap();
        let why = refusal(
            &h,
            "media",
            &row("media", "a", &m, &format!("{m}/compose.yaml"), false),
        );
        assert!(why.contains("not a regular file"), "{why}");
        // Directory names must be catalog stack names.
        std::fs::create_dir(h.root.join("Upper")).unwrap();
        let (dir, file) = (format!("{r}/Upper"), format!("{r}/Upper/compose.yaml"));
        let why = refusal(&h, "upper", &row("upper", "a", &dir, &file, false));
        assert!(why.contains("not a catalog stack name"), "{why}");
    }

    #[test]
    fn a_managed_override_file_is_recorded_with_its_hash() {
        let h = host();
        std::fs::write(h.root.join("media").join(MANAGED_FILE), "{}").unwrap();
        let p = propose_from(&h, &format!("[{}]", media_row(&h, "radarr", false))).unwrap();
        assert_eq!(
            p.registered[0].managed,
            Managed::Present {
                sha256: hex::encode(Sha256::digest(b"{}"))
            }
        );
        valid(&p);
    }

    #[test]
    fn bad_input_and_engine_ids_are_refused() {
        let h = host();
        assert!(parse_inspect("not json").is_err());
        let (members, u) = parse_inspect(&wybie(&h)).unwrap();
        assert!(propose("", &h.root, &members, u, &[]).is_err());
        assert!(propose("bad id", &h.root, &members, u, &[]).is_err());
        // Batches from several `docker inspect` calls concatenate.
        let (members, _) = parse_inspect("[]\n[]\n").unwrap();
        assert!(members.is_empty());
    }

    #[test]
    fn install_never_replaces_and_needs_a_closed_directory() {
        let t = tempfile::tempdir().unwrap();
        let uid = rustix::process::geteuid().as_raw();
        let dir = t.path().join("etc");
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("management-catalog.json");
        install(&path, "first\n", uid).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\n");
        assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o7777, 0o644);
        assert!(
            install(&path, "second\n", uid)
                .unwrap_err()
                .contains("already exists")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\n");
        assert!(!dir.join(".management-catalog.json.new").exists());
        assert!(install(&dir.join("x.json"), "x", uid + 1).is_err());
        std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o777))
            .unwrap();
        assert!(install(&dir.join("y.json"), "y", uid).is_err());
    }
}
