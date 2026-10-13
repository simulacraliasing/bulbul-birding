pub mod account;
pub mod client;

use std::fmt;

/// Only explicit, credential-free messages are exposed to CLI users.
#[derive(Debug)]
pub struct ToolError(pub String);

impl ToolError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ToolError {}

pub type Result<T> = std::result::Result<T, ToolError>;
