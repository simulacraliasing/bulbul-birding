pub mod account;
pub mod client;

use serde_json::{Value, json};
use std::fmt;

/// Only explicit, credential-free messages are exposed to CLI users.
#[derive(Debug)]
pub struct ToolError {
    message: String,
    pub code: &'static str,
    details: Option<Value>,
}

impl ToolError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::coded("operation_failed", message)
    }

    pub fn coded(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code,
            details: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn as_json(&self) -> Value {
        let mut value = json!({"error":self.message, "code":self.code});
        if let Some(details) = &self.details {
            value["details"] = details.clone();
        }
        value
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ToolError {}

pub type Result<T> = std::result::Result<T, ToolError>;
