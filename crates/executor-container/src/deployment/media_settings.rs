//! Supplied bytes only. No HTTP request, status, credential or source ownership.
use super::media::{Bounded, Object, decode};
use limeos_domain::{
    Error, ErrorCode, Result,
    deployment::{
        media::MediaService,
        media_settings::{
            DownloadHandling, MAX_PROFILES, Management, Naming, QualityProfile, SettingField,
        },
    },
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagementWire {
    id: u32,
    copy_using_hardlinks: bool,
    recycle_bin: String,
    use_script_import: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HandlingWire {
    id: u32,
    enable_completed_download_handling: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NamingWire {
    id: u32,
    rename_episodes: Option<bool>,
    rename_movies: Option<bool>,
    rename_tracks: Option<bool>,
    replace_illegal_characters: bool,
}
#[derive(Deserialize)]
struct ProfileWire {
    id: u32,
    name: String,
}
fn invalid<T>() -> Result<T> {
    Err(Error(ErrorCode::Unavailable))
}
pub fn management(bytes: &[u8]) -> Result<Management> {
    let Object(wire): Object<ManagementWire> = decode(bytes)?;
    let value = Management {
        id: wire.id,
        copy_using_hardlinks: wire.copy_using_hardlinks,
        recycle_bin: wire.recycle_bin,
        use_script_import: wire.use_script_import,
    };
    value
        .validate()
        .map_err(|_| Error(ErrorCode::Unavailable))?;
    Ok(value)
}
pub fn download_handling(bytes: &[u8]) -> Result<DownloadHandling> {
    let Object(wire): Object<HandlingWire> = decode(bytes)?;
    let value = DownloadHandling {
        id: wire.id,
        enabled: wire.enable_completed_download_handling,
    };
    value
        .validate()
        .map_err(|_| Error(ErrorCode::Unavailable))?;
    Ok(value)
}
pub fn naming(app: MediaService, bytes: &[u8]) -> Result<Naming> {
    let Object(wire): Object<NamingWire> = decode(bytes)?;
    let rename = match app {
        MediaService::Sonarr => wire.rename_episodes,
        MediaService::Radarr => wire.rename_movies,
        MediaService::Lidarr => wire.rename_tracks,
        _ => return invalid(),
    }
    .ok_or(Error(ErrorCode::Unavailable))?;
    let value = Naming {
        id: wire.id,
        field: SettingField::rename_field(app).map_err(|_| Error(ErrorCode::Unavailable))?,
        rename,
        replace_illegal_characters: wire.replace_illegal_characters,
    };
    value
        .validate(app)
        .map_err(|_| Error(ErrorCode::Unavailable))?;
    Ok(value)
}
pub fn quality_profiles(bytes: &[u8]) -> Result<Vec<QualityProfile>> {
    let Bounded(rows): Bounded<Object<ProfileWire>, MAX_PROFILES> = decode(bytes)?;
    let mut result: Vec<QualityProfile> = Vec::new();
    for Object(wire) in rows {
        let value = QualityProfile {
            id: wire.id,
            name: wire.name,
        };
        value
            .validate()
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        if result.iter().any(|p| p.id == value.id) {
            return invalid();
        }
        result.push(value);
    }
    Ok(result)
}
#[cfg(test)]
mod tests;
