//! Supplied bytes only; no acquisition, HTTP status, credential or source authority.
use super::media::{Bounded, Object, decode};
use limeos_domain::{
    Error, ErrorCode, Result,
    deployment::indexer_apps::{
        App, Application, MAX_APPLICATIONS, MAX_FIELDS, MAX_IDS, Settings, SyncLevel, validate_ids,
    },
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Wire {
    id: u32,
    name: String,
    implementation: String,
    sync_level: Option<String>,
    tags: Option<Bounded<u32, MAX_IDS>>,
    fields: Bounded<Object<Field>, MAX_FIELDS>,
}
#[derive(Deserialize)]
struct Field {
    name: String,
    value: Option<Value>,
}
fn unavailable() -> Error {
    Error(ErrorCode::Unavailable)
}
fn ids(value: Option<Value>) -> Result<Option<Vec<u32>>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let Bounded(values): Bounded<u32, MAX_IDS> =
                serde_json::from_value(value).map_err(|_| unavailable())?;
            validate_ids(&values).map_err(|_| unavailable())?;
            Ok(Some(values))
        }
    }
}
pub fn applications(bytes: &[u8]) -> Result<Vec<Application>> {
    let Bounded(rows): Bounded<Object<Wire>, MAX_APPLICATIONS> = decode(bytes)?;
    let mut result: Vec<Application> = Vec::new();
    for Object(wire) in rows {
        let kind = App::from_implementation(&wire.implementation);
        let mut categories = None;
        let mut anime_categories = None;
        let mut seen_categories = false;
        let mut seen_anime = false;
        for Object(field) in wire.fields.0 {
            if field.name.is_empty()
                || field.name.len() > 128
                || field.name.chars().any(char::is_control)
            {
                return Err(unavailable());
            }
            if field.name.eq_ignore_ascii_case("syncCategories") {
                if seen_categories || field.name != "syncCategories" {
                    return Err(unavailable());
                }
                seen_categories = true;
                categories = ids(field.value)?;
            } else if kind == Some(App::Sonarr)
                && field.name.eq_ignore_ascii_case("animeSyncCategories")
            {
                if seen_anime || field.name != "animeSyncCategories" {
                    return Err(unavailable());
                }
                seen_anime = true;
                anime_categories = ids(field.value)?;
            }
        }
        if let Some(s) = &wire.sync_level {
            if s.len() > 128 || s.chars().any(char::is_control) {
                return Err(unavailable());
            }
        }
        let sync_level = match wire.sync_level.as_deref() {
            Some("disabled") => Some(SyncLevel::Disabled),
            Some("addOnly") => Some(SyncLevel::AddOnly),
            Some("fullSync") => Some(SyncLevel::FullSync),
            _ => None,
        };
        let tags = wire.tags.map(|Bounded(tags)| tags);
        if let Some(tags) = &tags {
            validate_ids(tags).map_err(|_| unavailable())?;
        }
        let settings = match (kind, sync_level, categories, tags) {
            (Some(app), Some(sync_level), Some(categories), Some(tags))
                if app != App::Sonarr || anime_categories.is_some() =>
            {
                Some(Settings {
                    sync_level,
                    categories,
                    anime_categories,
                    tags,
                })
            }
            _ => None,
        };
        let application = Application {
            id: wire.id,
            name: wire.name,
            implementation: wire.implementation,
            settings,
        };
        application.validate().map_err(|_| unavailable())?;
        if result.iter().any(|a| a.id == application.id) {
            return Err(unavailable());
        }
        result.push(application);
    }
    Ok(result)
}
#[cfg(test)]
mod tests;
