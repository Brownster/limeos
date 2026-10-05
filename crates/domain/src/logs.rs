use crate::{Error, ErrorCode, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

// Leave space for worst-case JSON escaping inside the 64 KiB executor frame.
pub const CONTAINER_LOG_BYTES: usize = 24 * 1024;
pub const CONTAINER_LOG_TAIL: u16 = 200;
fn default_tail() -> u16 {
    100
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct LogOptions {
    #[serde(default = "default_tail")]
    #[schemars(range(min = 1, max = 200))]
    pub tail: u16,
}
impl LogOptions {
    pub fn validate(self) -> Result<()> {
        if self.tail == 0 || self.tail > CONTAINER_LOG_TAIL {
            return Err(Error(ErrorCode::InvalidInput));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ContainerLogs {
    pub resource: String,
    #[schemars(length(max = 24576))]
    pub text: String,
    pub truncated: bool,
}
