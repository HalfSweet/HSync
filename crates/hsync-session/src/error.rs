use hsync_core::{ChannelIndex, UiError};
use hsync_fileio::FileError;

/// Errors of session operations.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// No capture is open.
    #[error("no capture is open")]
    NoCapture,
    /// The query names a capture that has since been replaced.
    #[error("the capture was replaced")]
    StaleCapture,
    /// The capture has no such channel.
    #[error("unknown channel {0}")]
    UnknownChannel(ChannelIndex),
    /// The requested sample range or resolution is invalid.
    #[error("invalid sample range")]
    InvalidRange,
    /// Loading a capture file failed.
    #[error(transparent)]
    File(#[from] FileError),
}

impl From<&SessionError> for UiError {
    fn from(error: &SessionError) -> Self {
        match error {
            SessionError::NoCapture => UiError::new("error.session.noCapture"),
            SessionError::StaleCapture => UiError::new("error.session.staleCapture"),
            SessionError::UnknownChannel(channel) => {
                UiError::new("error.session.unknownChannel").with("channel", *channel)
            }
            SessionError::InvalidRange => UiError::new("error.session.invalidRange"),
            SessionError::File(file) => file.into(),
        }
    }
}
