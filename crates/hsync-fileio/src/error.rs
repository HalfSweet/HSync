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
    /// The file is not a VCD, FST or GHW waveform.
    #[error("not a waveform file")]
    NotWaveform,
    /// The waveform file could not be read.
    #[error("invalid waveform: {0}")]
    Waveform(String),
    /// The waveform file holds no timestamps.
    #[error("the waveform holds no samples")]
    EmptyWaveform,
    /// The import names a signal the file does not have.
    #[error("unknown signal {0}")]
    UnknownSignal(u32),
    /// The import names no signal.
    #[error("no signal chosen")]
    NoSignal,
    /// The import would hold more channels than a capture can.
    #[error("{0} channels is more than a capture holds")]
    TooManyChannels(usize),
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
    /// The archive is not an HSync capture file.
    #[error("not an hsync capture file")]
    NotHsync,
    /// A block of samples could not be decompressed.
    #[error("block {entry} does not decompress: {detail}")]
    Block { entry: String, detail: String },
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
            FileError::NotWaveform => UiError::new("error.file.notWaveform"),
            FileError::Waveform(detail) => {
                UiError::new("error.file.waveform").with("detail", detail.as_str())
            }
            FileError::EmptyWaveform => UiError::new("error.file.emptyWaveform"),
            FileError::UnknownSignal(signal) => {
                UiError::new("error.file.unknownSignal").with("signal", *signal)
            }
            FileError::NoSignal => UiError::new("error.file.noSignal"),
            FileError::TooManyChannels(count) => UiError::new("error.file.tooManyChannels")
                .with("count", *count)
                .with("limit", u16::MAX),
            FileError::Fst(error) => {
                UiError::new("error.file.io").with("detail", error.to_string())
            }
            FileError::NotHsync => UiError::new("error.file.notCapture"),
            FileError::Block { entry, .. } => {
                UiError::new("error.file.corrupt").with("detail", entry.as_str())
            }
        }
    }
}
