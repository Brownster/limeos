//! `migrate import-users`: read the frozen app's credential env file and hand the
//! hashes to the core over its root-only socket request. Hashes are never put in
//! argv, logs or error text; failures here carry fixed messages only.
use limeos_contracts::{CoreRequest, CoreResponse, LegacyUser, VERSION};
use std::path::Path;

const ENV_LIMIT: u64 = 64 * 1024;
const USER_LIMIT: usize = 32;

fn value_of(raw: &str) -> &str {
    let v = raw.trim();
    for q in ['"', '\''] {
        if v.len() >= 2 && v.starts_with(q) && v.ends_with(q) {
            return &v[1..v.len() - 1];
        }
    }
    v
}

/// Mirrors the frozen app's `_validate_password_hash` (Werkzeug scrypt/PBKDF2).
fn valid_hash(hash: &str) -> bool {
    let parts: Vec<_> = hash.split('$').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.is_empty()) {
        return false;
    }
    let method: Vec<_> = parts[0].split(':').collect();
    let positive = |s: &&str| s.parse::<u64>().is_ok_and(|n| n > 0);
    let digest = if method.len() == 3 && method[..2] == ["pbkdf2", "sha256"] {
        positive(&method[2]).then_some(64)
    } else if method.len() == 4 && method[0] == "scrypt" {
        method[1..].iter().all(positive).then_some(128)
    } else {
        None
    };
    digest.is_some_and(|n| parts[2].len() == n && parts[2].bytes().all(|b| b.is_ascii_hexdigit()))
}

/// The same rules as the frozen `auth_utils.load_users`, applied to env-file text.
pub fn parse_env(text: &str) -> Result<Vec<LegacyUser>, &'static str> {
    let (mut users_value, mut user, mut hash, mut plaintext) = (None, None, None, None);
    for line in text.lines() {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let slot = match key.trim() {
            "PIHEALTH_USERS" => &mut users_value,
            "PIHEALTH_USER" => &mut user,
            "PIHEALTH_PASSWORD_HASH" => &mut hash,
            "PIHEALTH_PASSWORD" => &mut plaintext,
            _ => continue,
        };
        if slot.replace(value_of(value).to_owned()).is_some() {
            return Err("a credential key appears more than once in the env file");
        }
    }
    if plaintext.is_some_and(|p| !p.trim().is_empty()) {
        return Err(
            "PIHEALTH_PASSWORD plaintext configuration is not supported; remove it from the env file",
        );
    }
    let mut users: Vec<LegacyUser> = Vec::new();
    let mut add = |name: &str, hash: &str| -> Result<(), &'static str> {
        let (name, hash) = (name.trim(), hash.trim());
        if name.is_empty() || users.iter().any(|u| u.username == name) {
            return Err("the env file has an empty or duplicate username");
        }
        if !valid_hash(hash) {
            return Err("password values must be Werkzeug scrypt or PBKDF2-SHA256 hashes");
        }
        users.push(LegacyUser {
            username: name.into(),
            password_hash: hash.into(),
        });
        Ok(())
    };
    match users_value
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        Some(list) => {
            for entry in list.split(',') {
                let (name, hash) = entry
                    .split_once(':')
                    .ok_or("PIHEALTH_USERS entries must use username:password_hash")?;
                add(name, hash)?;
            }
        }
        None => {
            let (name, hash) = (
                user.as_deref().unwrap_or("").trim(),
                hash.as_deref().unwrap_or("").trim(),
            );
            if name.is_empty() || hash.is_empty() {
                return Err("the env file configures no users");
            }
            add(name, hash)?;
        }
    }
    if users.len() > USER_LIMIT {
        return Err("too many users in the env file");
    }
    Ok(users)
}

fn read_env(path: &Path) -> Result<String, &'static str> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| "cannot read the env file")?;
    if !meta.is_file() || meta.len() > ENV_LIMIT {
        return Err("the env file must be a regular file under 64 KiB");
    }
    std::fs::read_to_string(path).map_err(|_| "cannot read the env file")
}

pub async fn run(socket: &str, args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let usage = || crate::args::Usage("usage: migrate import-users --env-file PATH".into());
    let [flag, path] = args else {
        return Err(usage().into());
    };
    if flag != "--env-file" || path.starts_with('-') {
        return Err(usage().into());
    }
    if rustix::process::geteuid().as_raw() != 0 {
        return Err("migrate import-users is root-only".into());
    }
    let users = parse_env(&read_env(Path::new(path))?)?;
    let names: Vec<_> = users.iter().map(|u| u.username.clone()).collect();
    let response = crate::rpc(
        socket,
        &CoreRequest::ImportLegacyUsers {
            version: VERSION,
            users,
        },
    )
    .await?;
    let CoreResponse::LegacyUsersImported {
        imported,
        skipped_existing,
    } = response
    else {
        return Err("unexpected response from the core".into());
    };
    println!("users: {}", names.join(", "));
    println!("imported: {imported}");
    println!(
        "skipped existing: {} ({})",
        skipped_existing.len(),
        skipped_existing.join(", ")
    );
    if imported == 0 {
        println!("nothing changed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRYPT: &str = "scrypt:32768:8:1$vdYT4OKICpm89RVk$04c86b2538e3f92ba1e4d2b9a2423d3f75bda2c7be27d67f30cda40d7638089134fee1f549b2968273fe31fbf30da2b9b3f0a4d5560077b0376cfca0ab2f2397";
    const PBKDF2: &str = "pbkdf2:sha256:1000000$IgjABokZwwl2gggv$686315920b24fb807abeb0a436ce6385be3f3026e35d934ffbaf15be1a0e7ce1";

    fn pairs(users: Vec<LegacyUser>) -> Vec<(String, String)> {
        users
            .into_iter()
            .map(|u| (u.username, u.password_hash))
            .collect()
    }

    #[test]
    fn both_hash_families_parse_from_the_users_list_with_unrelated_keys_ignored() {
        let text = format!(
            "# comment\nDISK_PATH=/mnt/a\nPIHEALTH_USERS=holly:{SCRYPT}, bob : {PBKDF2}\nDISK_PATH_2=/mnt/b\n"
        );
        assert_eq!(
            pairs(parse_env(&text).unwrap()),
            [
                ("holly".to_owned(), SCRYPT.to_owned()),
                ("bob".to_owned(), PBKDF2.to_owned())
            ]
        );
    }

    #[test]
    fn single_user_form_quotes_and_export_prefix_follow_the_frozen_loader() {
        let text = format!("export PIHEALTH_USER=\"holly\"\nPIHEALTH_PASSWORD_HASH='{SCRYPT}'\n");
        assert_eq!(
            pairs(parse_env(&text).unwrap()),
            [("holly".to_owned(), SCRYPT.to_owned())]
        );
        // PIHEALTH_USERS wins when both forms exist, as in load_users.
        let both = format!(
            "PIHEALTH_USER=x\nPIHEALTH_PASSWORD_HASH={PBKDF2}\nPIHEALTH_USERS=holly:{SCRYPT}\n"
        );
        assert_eq!(parse_env(&both).unwrap()[0].username, "holly");
        assert!(parse_env("PIHEALTH_USER=holly\n").is_err());
    }

    #[test]
    fn duplicates_empties_plaintext_and_malformed_hashes_are_refused_without_echoing_secrets() {
        let cases = [
            format!("PIHEALTH_USERS=holly:{SCRYPT},holly:{PBKDF2}"),
            format!("PIHEALTH_USERS=:{SCRYPT}"),
            "PIHEALTH_USERS=holly".to_owned(),
            "PIHEALTH_USERS=holly:plaintextpassword".to_owned(),
            "PIHEALTH_USERS=holly:pbkdf2:sha256:0$salt$00".to_owned(),
            format!(
                "PIHEALTH_USERS=holly:{}",
                SCRYPT.replace("scrypt:32768", "scrypt:0")
            ),
            format!("PIHEALTH_USERS=holly:{}", &SCRYPT[..SCRYPT.len() - 1]),
            format!("PIHEALTH_USERS=holly:{SCRYPT}\nPIHEALTH_PASSWORD=hunter2hunter2"),
            format!("PIHEALTH_USERS=holly:{SCRYPT}\nPIHEALTH_USERS=bob:{PBKDF2}"),
            String::new(),
        ];
        for case in cases {
            let error = parse_env(&case)
                .err()
                .unwrap_or_else(|| panic!("accepted: {case}"));
            assert!(
                !error.contains("hunter2") && !error.contains("vdYT4O") && !error.contains("holly")
            );
        }
        // A blank plaintext key is ignored, matching `.strip()` in the frozen loader.
        assert!(
            parse_env(&format!(
                "PIHEALTH_USERS=holly:{SCRYPT}\nPIHEALTH_PASSWORD=  \n"
            ))
            .is_ok()
        );
    }

    #[test]
    fn a_partly_bad_list_yields_no_users_at_all() {
        let text = format!("PIHEALTH_USERS=holly:{SCRYPT},bob:nothash");
        assert!(parse_env(&text).is_err());
    }

    #[test]
    fn the_env_reader_refuses_symlinks_directories_and_oversize_files() {
        let t = tempfile::tempdir().unwrap();
        let real = t.path().join("credentials.env");
        std::fs::write(&real, "A=1").unwrap();
        assert!(read_env(&real).is_ok());
        std::os::unix::fs::symlink(&real, t.path().join("link.env")).unwrap();
        assert!(read_env(&t.path().join("link.env")).is_err());
        assert!(read_env(t.path()).is_err());
        std::fs::write(&real, vec![b'x'; 70_000]).unwrap();
        assert!(read_env(&real).is_err());
    }
}
