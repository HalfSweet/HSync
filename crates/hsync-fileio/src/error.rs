use hsync_core::UiError;

/// Errors reading or writing capture files.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    /// The file could not be read or written.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    /// The file is not a valid zip archive.
    #[error("invalid archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    /// A required archive entry is missing.
    #[error("missing archive entry {0}")]
    MissingEntry(String),
    /// A line of the header could not be understood.
    #[error("invalid header line: {0}")]
    InvalidHeader(String),
    /// The header declares a format version this build cannot read.
    #[error("unsupported file version {0}")]
    UnsupportedVersion(u32),
    /// The capture holds oscilloscope or data acquisition data.
    #[error("unsupported device mode {0}")]
    UnsupportedMode(i32),
    /// A logic block does not hold whole 64-sample words.
    #[error("block {entry} has {len} bytes, which is not a multiple of 8")]
    MisalignedBlock {
        /// Archive entry of the block.
        entry: String,
        /// Its length in bytes.
        len: u64,
    },
    /// The user cancelled the write.
    #[error("cancelled")]
    Cancelled,
    /// Writing an FST file failed.
    #[error("fst: {0}")]
    Fst(#[from] fst_writer::FstWriteError),
    /// An embedded JSON document is malformed.
    #[error("invalid json in {entry}: {source}")]
    Json {
        /// Archive entry of the document.
        entry: String,
        /// Parser error.
        source: serde_json::Error,
    },
}

impl From<&FileError> for UiError {
    fn from(error: &FileError) -> Self {
        match error {
            FileError::Io(io) => UiError::new("error.file.io").with("detail", io.to_string()),
            FileError::Zip(_) => UiError::new("error.file.notCapture"),
            FileError::MissingEntry(entry) => {
                UiError::new("error.file.corrupt").with("detail", entry.as_str())
            }
            FileError::InvalidHeader(line) => {
                UiError::new("error.file.corrupt").with("detail", line.as_str())
            }
            FileError::UnsupportedVersion(version) => {
                UiError::new("error.file.unsupportedVersion").with("version", *version)
            }
            FileError::UnsupportedMode(_) => UiError::new("error.file.unsupportedMode"),
            FileError::MisalignedBlock { entry, .. } => {
                UiError::new("error.file.corrupt").with("detail", entry.as_str())
            }
            FileError::Json { entry, .. } => {
                UiError::new("error.file.corrupt").with("detail", entry.as_str())
            }
            FileError::Cancelled => UiError::new("error.file.cancelled"),
            FileError::Fst(error) => {
                UiError::new("error.file.io").with("detail", error.to_string())
            }
        }
    }
}
