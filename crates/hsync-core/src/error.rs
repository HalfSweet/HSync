use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;
use ts_rs::TS;

/// An error as presented to the user: a stable i18n key and its message parameters.
///
/// The backend never produces natural language (DESIGN.md §10.4); the frontend translates
/// `code` with `params`.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct UiError {
    /// i18n key, e.g. `error.file.unsupportedVersion`.
    pub code: String,
    /// Named parameters of the message.
    #[ts(type = "Record<string, string | number | boolean>")]
    pub params: BTreeMap<String, serde_json::Value>,
}

impl UiError {
    /// Creates an error without parameters.
    pub fn new(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            params: BTreeMap::new(),
        }
    }

    /// Adds a message parameter.
    #[must_use]
    pub fn with(mut self, name: &str, value: impl Into<serde_json::Value>) -> Self {
        self.params.insert(name.to_owned(), value.into());
        self
    }
}

impl fmt::Display for UiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.code)?;
        for (name, value) in &self.params {
            write!(f, " {name}={value}")?;
        }
        Ok(())
    }
}

impl std::error::Error for UiError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_code_and_params() {
        let error = UiError::new("error.file.unsupportedVersion").with("version", 7);
        let json = serde_json::to_value(&error).unwrap_or_default();
        assert_eq!(
            json,
            serde_json::json!({ "code": "error.file.unsupportedVersion", "params": { "version": 7 } })
        );
        assert_eq!(error.to_string(), "error.file.unsupportedVersion version=7");
    }
}
