//! The decoder host of an application: where the decoders live, and what they are, read once.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::{DecoderInfo, SrdError, load_catalog, python};

/// The Python decoders of one application.
#[derive(Debug)]
pub struct DecoderHost {
    dir: PathBuf,
    /// The Python home to start with, when the app carries its own Python.
    python_home: Option<PathBuf>,
    catalog: OnceLock<Result<Vec<DecoderInfo>, String>>,
}

impl DecoderHost {
    /// Decoders in `dir`, one package each; nothing is loaded yet.
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            python_home: None,
            catalog: OnceLock::new(),
        }
    }

    /// Starts Python with the standard library under `home`, the runtime release builds bundle,
    /// instead of where the interpreter the build linked against keeps it.
    pub fn with_python_home(mut self, home: PathBuf) -> Self {
        self.python_home = Some(home);
        self
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Every decoder, read on first use (this starts Python and imports them all).
    pub fn catalog(&self) -> Result<&[DecoderInfo], SrdError> {
        self.catalog
            .get_or_init(|| {
                let started = std::time::Instant::now();
                let loaded = self
                    .python_home
                    .as_deref()
                    .map_or(Ok(()), python::start)
                    .and_then(|()| load_catalog(&self.dir))
                    .map_err(|error| error.to_string());
                match &loaded {
                    Ok(list) => log::info!(
                        "read {} protocol decoders in {:.0} ms",
                        list.len(),
                        started.elapsed().as_secs_f64() * 1e3
                    ),
                    Err(error) => log::warn!("the protocol decoders did not load: {error}"),
                }
                loaded
            })
            .as_deref()
            .map_err(|error| SrdError::Python(error.clone()))
    }

    /// The decoder with `id`, e.g. `1:i2c`.
    pub fn info(&self, id: &str) -> Result<&DecoderInfo, SrdError> {
        self.catalog()?
            .iter()
            .find(|info| info.id == id)
            .ok_or_else(|| SrdError::UnknownDecoder(id.to_owned()))
    }
}
