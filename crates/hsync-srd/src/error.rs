use hsync_core::UiError;

/// Errors of the decoder host.
#[derive(Debug, thiserror::Error)]
pub enum SrdError {
    /// Python could not be started or a decoder could not be loaded.
    #[error("python: {0}")]
    Python(String),
    /// No decoder has this id.
    #[error("unknown decoder {0}")]
    UnknownDecoder(String),
    /// A decoder declares something the host cannot use.
    #[error("decoder {decoder}: {detail}")]
    InvalidDecoder { decoder: String, detail: String },
}

impl From<pyo3::PyErr> for SrdError {
    fn from(error: pyo3::PyErr) -> Self {
        Self::Python(error.to_string())
    }
}

impl From<&SrdError> for UiError {
    fn from(error: &SrdError) -> Self {
        match error {
            SrdError::Python(detail) => {
                UiError::new("error.decode.python").with("detail", detail.as_str())
            }
            SrdError::UnknownDecoder(id) => {
                UiError::new("error.decode.unknown").with("id", id.as_str())
            }
            SrdError::InvalidDecoder { decoder, detail } => UiError::new("error.decode.invalid")
                .with("id", decoder.as_str())
                .with("detail", detail.as_str()),
        }
    }
}
