//! Starting the embedded interpreter from the Python home release builds bundle.

use std::ffi::{CStr, c_char};
use std::mem::MaybeUninit;
use std::path::Path;

use pyo3::ffi;

use crate::SrdError;

/// Starts Python with the standard library under `home`, isolated from the environment
/// (`PYTHONHOME`, `PYTHONPATH`, the user's site-packages). Does nothing if Python runs already.
///
/// pyo3 would otherwise start Python on first use with the defaults, which look for the standard
/// library where the interpreter the build linked against keeps it; that is not on the machines
/// a release build runs on. Call this before anything else in the process uses Python.
pub(crate) fn start(home: &Path) -> Result<(), SrdError> {
    // SAFETY: `config` is set up by `PyConfig_InitIsolatedConfig` before any other use and cleared
    // once Python has copied what it needs; `set_home` only reads `home` during the call. The
    // `Py_IsInitialized` check keeps this from starting Python twice; callers run it before any
    // other use of Python in the process, so nothing races the start.
    unsafe {
        if ffi::Py_IsInitialized() != 0 {
            return Ok(());
        }
        let mut storage = MaybeUninit::<ffi::PyConfig>::uninit();
        let config = storage.as_mut_ptr();
        ffi::PyConfig_InitIsolatedConfig(config);
        // The bundle is read-only, and signed on macOS: no bytecode next to the sources.
        (*config).write_bytecode = 0;
        let status = match set_home(config, home) {
            Ok(status) if ffi::PyStatus_Exception(status) == 0 => {
                ffi::Py_InitializeFromConfig(config)
            }
            Ok(status) => status,
            Err(error) => {
                ffi::PyConfig_Clear(config);
                return Err(error);
            }
        };
        ffi::PyConfig_Clear(config);
        if ffi::PyStatus_Exception(status) != 0 {
            return Err(SrdError::Python(message(&status)));
        }
        // Let go of the GIL as pyo3 does after starting Python, so any thread can take it.
        ffi::PyEval_SaveThread();
    }
    Ok(())
}

/// Sets `config.home`. Windows takes the path as UTF-16, so any path works.
///
/// # Safety
///
/// `config` must point to an initialised `PyConfig`.
#[cfg(windows)]
unsafe fn set_home(config: *mut ffi::PyConfig, home: &Path) -> Result<ffi::PyStatus, SrdError> {
    use std::os::windows::ffi::OsStrExt;

    let wide: Vec<u16> = home.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: the caller's `config` is initialised; `wide` is NUL-terminated and outlives the call.
    Ok(unsafe { ffi::PyConfig_SetString(config, &raw mut (*config).home, wide.as_ptr()) })
}

/// Sets `config.home`. Python decodes the bytes as it decodes file names, so they round-trip.
///
/// # Safety
///
/// `config` must point to an initialised `PyConfig`.
#[cfg(not(windows))]
unsafe fn set_home(config: *mut ffi::PyConfig, home: &Path) -> Result<ffi::PyStatus, SrdError> {
    use std::os::unix::ffi::OsStrExt;

    let bytes = std::ffi::CString::new(home.as_os_str().as_bytes())
        .map_err(|_| SrdError::Python(format!("{}: NUL in the path", home.display())))?;
    // SAFETY: the caller's `config` is initialised; `bytes` is NUL-terminated and outlives the call.
    Ok(unsafe { ffi::PyConfig_SetBytesString(config, &raw mut (*config).home, bytes.as_ptr()) })
}

/// What a failed start reports: CPython's message and the function it came from.
fn message(status: &ffi::PyStatus) -> String {
    let text = |pointer: *const c_char| {
        // SAFETY: a `PyStatus` holds NULL or static, NUL-terminated strings.
        (!pointer.is_null()).then(|| unsafe { CStr::from_ptr(pointer) }.to_string_lossy())
    };
    match (text(status.func), text(status.err_msg)) {
        (Some(func), Some(message)) => format!("{func}: {message}"),
        (None, Some(message)) => message.into_owned(),
        _ => format!("Python exited with {}", status.exitcode),
    }
}
