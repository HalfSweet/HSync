//! The `sigrokdecode` module decoders import, written in Rust.

use std::path::Path;

use pyo3::prelude::*;
use pyo3::types::PyModule;

/// `OUTPUT_ANN` and the other output types of `register()`.
pub const OUTPUT_ANN: i32 = 0;
pub const OUTPUT_PYTHON: i32 = 1;
pub const OUTPUT_BINARY: i32 = 2;
pub const OUTPUT_META: i32 = 3;
/// The metadata key of the sample rate (`SRD_CONF_SAMPLERATE`).
pub const SRD_CONF_SAMPLERATE: i32 = 10_000;

/// The base class of every decoder.
#[pyclass(subclass, module = "sigrokdecode", name = "Decoder")]
#[derive(Debug, Default)]
pub struct Decoder {}

#[pymethods]
impl Decoder {
    #[new]
    #[pyo3(signature = (*_args, **_kwargs))]
    fn new(
        _args: &Bound<'_, pyo3::types::PyTuple>,
        _kwargs: Option<&Bound<'_, pyo3::types::PyDict>>,
    ) -> Self {
        Self::default()
    }
}

/// Puts `sigrokdecode` in `sys.modules` and `dir` on the import path, once.
pub fn install(py: Python<'_>, dir: &Path) -> PyResult<()> {
    let sys = py.import("sys")?;
    // The decoders live among the application's resources: leave no bytecode beside them, and
    // keep the old escape-sequence warnings of some of them quiet.
    sys.setattr("dont_write_bytecode", true)?;
    let filter = pyo3::types::PyDict::new(py);
    filter.set_item(
        "category",
        py.get_type::<pyo3::exceptions::PySyntaxWarning>(),
    )?;
    py.import("warnings")?
        .call_method("filterwarnings", ("ignore",), Some(&filter))?;
    let modules = sys.getattr("modules")?;
    if !modules.contains("sigrokdecode")? {
        let module = PyModule::new(py, "sigrokdecode")?;
        module.add_class::<Decoder>()?;
        module.add("OUTPUT_ANN", OUTPUT_ANN)?;
        module.add("OUTPUT_PYTHON", OUTPUT_PYTHON)?;
        module.add("OUTPUT_BINARY", OUTPUT_BINARY)?;
        module.add("OUTPUT_META", OUTPUT_META)?;
        module.add("SRD_CONF_SAMPLERATE", SRD_CONF_SAMPLERATE)?;
        modules.set_item("sigrokdecode", module)?;
    }
    let path = sys.getattr("path")?;
    let dir = dir.to_string_lossy().into_owned();
    if !path.contains(&dir)? {
        path.call_method1("insert", (0, dir))?;
    }
    Ok(())
}
