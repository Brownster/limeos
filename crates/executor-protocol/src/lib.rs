use limeos_contracts::VERSION;
use limeos_domain::{Error, ErrorCode, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Ceiling {
    pub version: u16,
    pub core_uid: u32,
    pub allow_health: bool,
}
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Health { version: u16 },
}
#[derive(Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub version: u16,
    pub ready: bool,
}
impl Ceiling {
    pub fn validate(&self, peer_uid: u32, request: &Request) -> Result<Receipt> {
        if self.version != VERSION || peer_uid != self.core_uid || !self.allow_health {
            return Err(Error(ErrorCode::Forbidden));
        }
        match request {
            Request::Health { version } if *version == VERSION => Ok(Receipt {
                version: VERSION,
                ready: true,
            }),
            _ => Err(Error(ErrorCode::InvalidInput)),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn peer_and_ceiling_are_independent_and_no_content_or_command_is_accepted() {
        let ceiling = Ceiling {
            version: VERSION,
            core_uid: 1001,
            allow_health: true,
        };
        assert!(
            ceiling
                .validate(1002, &Request::Health { version: VERSION })
                .is_err()
        );
        assert!(
            ceiling
                .validate(1001, &Request::Health { version: 99 })
                .is_err()
        );
        for extra in ["content", "command", "path", "principal"] {
            let raw = format!("{{\"operation\":\"health\",\"version\":1,\"{extra}\":\"attack\"}}");
            assert!(serde_json::from_str::<Request>(&raw).is_err());
        }
    }
}
