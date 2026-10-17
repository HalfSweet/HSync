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
    /// No device is open.
    #[error("no device is open")]
    NoDevice,
    /// No device has this id.
    #[error("unknown device {0}")]
    UnknownDevice(String),
    /// A capture is running.
    #[error("a capture is running")]
    Busy,
    /// The device or its driver failed.
    #[error("device error {} {:?}", .0.code, .0.params)]
    Device(UiError),
    /// An export would hold no channel.
    #[error("nothing to export")]
    NothingToExport,
    /// No waveform file was read for the import.
    #[error("no waveform to import")]
    NoWaveform,
    /// A decoder module could not be loaded.
    #[error("the decoder module {path} cannot be loaded: {detail}")]
    Module {
        /// The file.
        path: String,
        /// What is wrong with it.
        detail: String,
    },
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
            SessionError::NoDevice => UiError::new("error.session.noDevice"),
            SessionError::UnknownDevice(id) => {
                UiError::new("error.session.unknownDevice").with("id", id.as_str())
            }
            SessionError::Busy => UiError::new("error.session.busy"),
            SessionError::Device(error) => error.clone(),
            SessionError::NothingToExport => UiError::new("error.session.nothingToExport"),
            SessionError::NoWaveform => UiError::new("error.session.noWaveform"),
            SessionError::Module { path, detail } => UiError::new("error.decode.module")
                .with("path", path.as_str())
                .with("detail", detail.as_str()),
        }
    }
}
