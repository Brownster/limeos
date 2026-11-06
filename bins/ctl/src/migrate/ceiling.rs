//! `migrate propose-ceiling`: an offline, deny-by-default container ceiling.
//!
//! The input is `docker ps --no-trunc --format json` output that the operator
//! already collected; this tool never talks to Docker. Every `allow_*` is false,
//! so adopting the file changes nothing until the operator flips a flag.
use limeos_executor_protocol::Ceiling;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

const COMPOSE_FILES: [&str; 4] = [
    "compose.yaml",
    "compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
];
const INPUT_LIMIT: u64 = 1 << 20;

#[derive(Debug)]
pub struct Container {
    pub id: String,
    pub name: String,
}
pub struct Proposal {
    pub containers: Vec<Container>,
    pub stacks: Vec<String>,
    pub json: String,
}

fn text_of(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_owned)
}
/// Accepts JSON lines (docker's native shape) or a single JSON array.
pub fn parse_docker_ps(text: &str) -> Result<Vec<Container>, String> {
    let rows: Vec<serde_json::Value> = if text.trim_start().starts_with('[') {
        serde_json::from_str(text).map_err(|_| "docker ps input is not valid JSON")?
    } else {
        text.lines()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty())
            .map(|(i, l)| {
                serde_json::from_str(l).map_err(|_| format!("docker ps line {} is not JSON", i + 1))
            })
            .collect::<Result<_, _>>()?
    };
    let mut containers: Vec<Container> = Vec::new();
    for row in rows {
        let id = text_of(&row, "ID").ok_or("docker ps row has no ID")?;
        if !limeos_domain::opaque_id(&id) {
            return Err(
                "container ID is not a full 64-character ID; collect with `docker ps --no-trunc --format json`"
                    .into(),
            );
        }
        if containers.iter().any(|c| c.id == id) {
            return Err("duplicate container ID in docker ps input".into());
        }
        let name = text_of(&row, "Names")
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_control())
            .collect();
        containers.push(Container { id, name });
    }
    if containers.is_empty() || containers.len() > 64 {
        return Err("a ceiling lists between 1 and 64 containers".into());
    }
    Ok(containers)
}

/// Stack directories are immediate subdirectories holding a regular Compose file.
/// Symlinks are never followed.
pub fn list_stacks(root: &Path) -> Result<Vec<String>, String> {
    let meta = std::fs::symlink_metadata(root).map_err(|_| "cannot read the stacks directory")?;
    if !meta.is_dir() {
        return Err("the stacks path is not a plain directory".into());
    }
    let mut names = Vec::new();
    for entry in std::fs::read_dir(root).map_err(|_| "cannot read the stacks directory")? {
        let entry = entry.map_err(|_| "cannot read the stacks directory")?;
        let path = entry.path();
        if !std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir()) {
            continue;
        }
        let compose = COMPOSE_FILES
            .iter()
            .any(|f| std::fs::symlink_metadata(path.join(f)).is_ok_and(|m| m.is_file()));
        if let (true, Some(name)) = (compose, entry.file_name().to_str()) {
            names.push(name.to_owned());
        }
    }
    names.sort();
    Ok(names)
}

pub fn propose(
    core_uid: u32,
    containers: Vec<Container>,
    stacks: Vec<String>,
) -> Result<Proposal, String> {
    let ceiling = Ceiling {
        version: limeos_contracts::VERSION,
        core_uid,
        allow_health: false,
        allow_host_read: false,
        allow_storage_read: false,
        allow_stack_read: false,
        allow_container_health_read: false,
        allow_container_read: false,
        allow_restart: false,
        allow_start: false,
        allow_stop: false,
        allow_container_logs: false,
        allow_stack: false,
        managed_containers: containers.iter().map(|c| c.id.clone()).collect(),
        managed_stacks: stacks.clone(),
    };
    if let Some(bad) = stacks.iter().find(|s| !limeos_domain::stack_project(s)) {
        return Err(format!(
            "stack directory {bad:?} is not a valid Compose project name; rename or remove it"
        ));
    }
    if !ceiling.configuration_valid() {
        return Err("the proposed ceiling is not valid".into());
    }
    let json = serde_json::to_string_pretty(&ceiling).map_err(|_| "cannot encode the ceiling")?;
    Ok(Proposal {
        containers,
        stacks,
        json: json + "\n",
    })
}

fn read_small(path: &Path) -> Result<String, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| "cannot read the docker ps file")?;
    if !meta.is_file() || meta.len() > INPUT_LIMIT {
        return Err("the docker ps file is not a regular file of a sane size".into());
    }
    std::fs::read_to_string(path).map_err(|_| "cannot read the docker ps file".into())
}

/// Refuses to replace anything, including a dangling symlink.
pub fn write_new(path: &Path, json: &str) -> Result<(), String> {
    write_new_with_mode(path, json, 0o644)
}
pub fn write_new_with_mode(path: &Path, json: &str, mode: u32) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                "the output file already exists; nothing was overwritten".to_owned()
            } else {
                "cannot create the output file".to_owned()
            }
        })?;
    file.write_all(json.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|_| {
            let _ = std::fs::remove_file(path);
            "cannot write the output file".to_owned()
        })
}

pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let (mut ps, mut stacks, mut out, mut uid) = (None, None, None, None);
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let slot = match flag.as_str() {
            "--docker-ps" => &mut ps,
            "--stacks" => &mut stacks,
            "--out" => &mut out,
            "--core-uid" => &mut uid,
            _ => return Err("unknown propose-ceiling argument; run limeosctl --help".into()),
        };
        *slot = Some(
            it.next()
                .ok_or("missing value for a propose-ceiling flag")?,
        );
    }
    let (Some(ps), Some(stacks), Some(out), Some(uid)) = (ps, stacks, out, uid) else {
        return Err("usage: migrate propose-ceiling --docker-ps FILE --stacks DIR --core-uid UID --out FILE".into());
    };
    let uid: u32 = uid.parse().map_err(|_| "--core-uid must be a number")?;
    if std::fs::symlink_metadata(out).is_ok() {
        return Err("the output file already exists; nothing was overwritten".into());
    }
    let proposal = propose(
        uid,
        parse_docker_ps(&read_small(Path::new(ps))?)?,
        list_stacks(Path::new(stacks))?,
    )?;
    write_new(Path::new(out), &proposal.json)?;
    for c in &proposal.containers {
        println!("container {} {}", &c.id[..12], c.name);
    }
    for s in &proposal.stacks {
        println!("stack {s}");
    }
    println!(
        "wrote {out}: {} containers, {} stacks, every allow_* (incl. allow_stack) false. Review it, then install it root-owned and set only the flags you intend.",
        proposal.containers.len(),
        proposal.stacks.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: [&str; 14] = [
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
    fn id(n: usize) -> String {
        format!("{n:02x}").repeat(32)
    }
    fn docker_ps() -> String {
        NAMES
            .iter()
            .enumerate()
            .map(|(i, n)| {
                format!(
                    r#"{{"ID":"{}","Names":"{n}","Image":"x","State":"running"}}"#,
                    id(i)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_fourteen_wybie_containers_yield_a_deny_by_default_ceiling_that_check_ceiling_accepts() {
        let containers = parse_docker_ps(&docker_ps()).unwrap();
        assert_eq!(containers.len(), 14);
        let p = propose(1001, containers, vec!["media".into(), "mattermost".into()]).unwrap();
        assert!(crate::ceiling_acceptable(p.json.as_bytes(), 1001));
        assert!(!crate::ceiling_acceptable(p.json.as_bytes(), 1002));
        let value: serde_json::Value = serde_json::from_str(&p.json).unwrap();
        let object = value.as_object().unwrap();
        let allows: Vec<_> = object
            .iter()
            .filter(|(k, _)| k.starts_with("allow_"))
            .collect();
        assert_eq!(allows.len(), 11);
        assert_eq!(
            object["managed_stacks"],
            serde_json::json!(["media", "mattermost"])
        );
        assert!(
            propose(
                1001,
                parse_docker_ps(&docker_ps()).unwrap(),
                vec!["Bad Name".into()]
            )
            .is_err()
        );
        assert!(
            allows
                .iter()
                .all(|(_, v)| v == &&serde_json::Value::Bool(false))
        );
        let managed: Vec<_> = object["managed_containers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        assert_eq!(managed, (0..14).map(id).collect::<Vec<_>>());
    }

    #[test]
    fn names_are_reported_in_order_and_array_input_is_accepted() {
        let containers = parse_docker_ps(&docker_ps()).unwrap();
        let names: Vec<_> = containers.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, NAMES);
        let array = format!("[{}]", docker_ps().replace('\n', ","));
        assert_eq!(parse_docker_ps(&array).unwrap().len(), 14);
    }

    #[test]
    fn truncated_ids_duplicates_garbage_and_oversized_sets_are_refused() {
        let short = r#"{"ID":"0123456789ab","Names":"x"}"#;
        assert!(parse_docker_ps(short).unwrap_err().contains("--no-trunc"));
        let row = format!(r#"{{"ID":"{}","Names":"x"}}"#, id(1));
        assert!(parse_docker_ps(&format!("{row}\n{row}")).is_err());
        assert!(parse_docker_ps("not json").is_err());
        assert!(parse_docker_ps("").is_err());
        let many: Vec<_> = (0..65)
            .map(|i| format!(r#"{{"ID":"{}","Names":"n{i}"}}"#, id(i)))
            .collect();
        assert!(parse_docker_ps(&many.join("\n")).is_err());
    }

    #[test]
    fn stacks_need_a_real_compose_file_and_symlinks_are_not_followed() {
        let t = tempfile::tempdir().unwrap();
        for (dir, file) in [
            ("media", "compose.yaml"),
            ("mattermost", "docker-compose.yml"),
            ("empty", "notes.txt"),
        ] {
            std::fs::create_dir(t.path().join(dir)).unwrap();
            std::fs::write(t.path().join(dir).join(file), "services: {}").unwrap();
        }
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("compose.yaml"), "x").unwrap();
        std::os::unix::fs::symlink(outside.path(), t.path().join("linked")).unwrap();
        std::fs::create_dir(t.path().join("sneaky")).unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("compose.yaml"),
            t.path().join("sneaky/compose.yaml"),
        )
        .unwrap();
        assert_eq!(list_stacks(t.path()).unwrap(), ["mattermost", "media"]);
        assert!(list_stacks(&t.path().join("media/compose.yaml")).is_err());
    }

    #[test]
    fn an_existing_output_is_never_overwritten() {
        let t = tempfile::tempdir().unwrap();
        let out = t.path().join("container.json");
        write_new(&out, "first\n").unwrap();
        assert!(
            write_new(&out, "second\n")
                .unwrap_err()
                .contains("already exists")
        );
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "first\n");
        std::os::unix::fs::symlink(t.path().join("target"), t.path().join("link.json")).unwrap();
        assert!(write_new(&t.path().join("link.json"), "x").is_err());
        assert!(!t.path().join("target").exists());
        let args: Vec<String> = [
            "--docker-ps",
            "/nonexistent",
            "--stacks",
            "/nonexistent",
            "--core-uid",
            "1",
            "--out",
            out.to_str().unwrap(),
        ]
        .map(String::from)
        .into();
        assert!(
            run(&args)
                .unwrap_err()
                .to_string()
                .contains("already exists")
        );
    }
}
