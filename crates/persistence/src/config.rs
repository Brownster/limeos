use limeos_domain::{Error, ErrorCode, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

pub const CONFIG_LIMIT: u64 = 64 * 1024;
// Linux O_NOFOLLOW; isolated use of the safe standard-library open API.
const NOFOLLOW: i32 = 0x20000;
const NONBLOCK: i32 = 0x800;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u16,
    pub listen: String,
    pub origin: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            listen: "127.0.0.1:8003".into(),
            origin: "https://localhost".into(),
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        let address: std::net::SocketAddr = self
            .listen
            .parse()
            .map_err(|_| Error(ErrorCode::CorruptConfiguration))?;
        // Cleartext only on loopback behind a local HTTPS proxy. No forwarding-header trust.
        if self.version != 1
            || !address.ip().is_loopback()
            || address.port() == 0
            || !self.origin.starts_with("https://")
            || self.origin.len() > 256
            || self.origin[8..].is_empty()
            || self.origin[8..].contains(['/', '\\', '@', '?', '#'])
            || self.origin.bytes().any(|byte| byte <= 0x20 || byte >= 0x7f)
        {
            return Err(Error(ErrorCode::CorruptConfiguration));
        }
        Ok(())
    }
}
pub enum ConfigRead {
    Missing,
    Corrupt,
    Valid(Config),
}
pub fn read(path: &Path) -> Result<ConfigRead> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(NOFOLLOW | NONBLOCK)
        .open(path);
    let file = match file {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ConfigRead::Missing),
        Err(_) => return Err(Error(ErrorCode::CorruptConfiguration)),
        Ok(file) => file,
    };
    if !file
        .metadata()
        .map_err(|_| Error(ErrorCode::CorruptConfiguration))?
        .is_file()
    {
        return Err(Error(ErrorCode::CorruptConfiguration));
    }
    let mut bytes = Vec::new();
    file.take(CONFIG_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error(ErrorCode::CorruptConfiguration))?;
    if bytes.len() as u64 > CONFIG_LIMIT {
        return Ok(ConfigRead::Corrupt);
    }
    Ok(match serde_json::from_slice::<Config>(&bytes) {
        Ok(config) if config.validate().is_ok() => ConfigRead::Valid(config),
        _ => ConfigRead::Corrupt,
    })
}
/// Caller owns a private managed directory. Sync both the data and directory.
/// A corrupt destination cannot be replaced by this normal write path.
pub fn write(path: &Path, config: &Config) -> Result<()> {
    config.validate()?;
    if matches!(read(path)?, ConfigRead::Corrupt) {
        return Err(Error(ErrorCode::CorruptConfiguration));
    }
    let parent = path.parent().ok_or(Error(ErrorCode::InvalidInput))?;
    let mut temp =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| Error(ErrorCode::StateNotDurable))?;
    fs::set_permissions(
        temp.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .map_err(|_| Error(ErrorCode::StateNotDurable))?;
    temp.write_all(&serde_json::to_vec(config).map_err(|_| Error(ErrorCode::InvalidInput))?)
        .map_err(|_| Error(ErrorCode::StateNotDurable))?;
    temp.as_file()
        .sync_all()
        .map_err(|_| Error(ErrorCode::StateNotDurable))?;
    temp.persist(path)
        .map_err(|_| Error(ErrorCode::StateNotDurable))?;
    File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|_| Error(ErrorCode::StateNotDurable))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corrupt_config_is_preserved_reads_do_not_write_and_atomic_write_is_valid() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("core.json");
        assert!(matches!(read(&p).unwrap(), ConfigRead::Missing));
        assert!(!p.exists());
        write(&p, &Config::default()).unwrap();
        assert!(matches!(read(&p).unwrap(), ConfigRead::Valid(_)));
        fs::write(&p, b"{broken").unwrap();
        let before = fs::metadata(&p).unwrap().modified().unwrap();
        assert!(matches!(read(&p).unwrap(), ConfigRead::Corrupt));
        assert!(write(&p, &Config::default()).is_err());
        assert_eq!(fs::read(&p).unwrap(), b"{broken");
        assert_eq!(before, fs::metadata(&p).unwrap().modified().unwrap());
    }
    #[test]
    fn public_cleartext_and_symlink_config_are_rejected() {
        assert!(
            Config {
                listen: "0.0.0.0:8003".into(),
                ..Config::default()
            }
            .validate()
            .is_err()
        );
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("link");
        std::os::unix::fs::symlink("/etc/passwd", &p).unwrap();
        assert!(read(&p).is_err());
        let fifo = d.path().join("fifo");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        assert!(read(&fifo).is_err());
        assert!(read(d.path()).is_err());
        assert!(
            Config {
                origin: "https://localhost\t".into(),
                ..Config::default()
            }
            .validate()
            .is_err()
        );
    }
}
