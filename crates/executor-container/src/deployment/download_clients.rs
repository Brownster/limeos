//! Supplied-response decoder. Selected metadata is not source or HTTP evidence.
use super::media::{Bounded, Object, decode};
use limeos_domain::{
    Error, ErrorCode, Result,
    deployment::{
        download_clients::{Category, CategoryField, Client, MAX_CLIENTS, MAX_FIELDS},
        media::MediaService,
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
    enable: bool,
    remove_completed_downloads: bool,
    remove_failed_downloads: bool,
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
pub fn clients(app: MediaService, bytes: &[u8]) -> Result<Vec<Client>> {
    let category_field = CategoryField::for_app(app).map_err(|_| unavailable())?;
    let Bounded(rows): Bounded<Object<Wire>, MAX_CLIENTS> = decode(bytes)?;
    let mut result: Vec<Client> = Vec::new();
    for Object(wire) in rows {
        let mut category = Category::Unknown;
        let mut seen = false;
        for Object(field) in wire.fields.0 {
            if field.name.is_empty()
                || field.name.len() > 128
                || field.name.chars().any(char::is_control)
            {
                return Err(unavailable());
            }
            if field.name.eq_ignore_ascii_case(category_field.wire_name()) {
                if seen || field.name != category_field.wire_name() {
                    return Err(unavailable());
                }
                seen = true;
                category = match field.value {
                    Some(Value::String(value)) => Category::Known(value),
                    None | Some(Value::Null) => Category::Unknown,
                    _ => return Err(unavailable()),
                };
            }
        }
        let client = Client {
            id: wire.id,
            name: wire.name,
            implementation: wire.implementation,
            category_field,
            category,
            enabled: wire.enable,
            remove_completed: wire.remove_completed_downloads,
            remove_failed: wire.remove_failed_downloads,
        };
        client.validate(app).map_err(|_| unavailable())?;
        if result.iter().any(|c| c.id == client.id) {
            return Err(unavailable());
        }
        result.push(client);
    }
    Ok(result)
}
#[cfg(test)]
mod tests;
