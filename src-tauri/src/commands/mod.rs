//! IPC commands, see DESIGN.md §3.4. Errors cross as [`UiError`]: an i18n key and parameters.

pub mod capture;
pub mod decode;
pub mod device;

use hsync_core::UiError;
use hsync_session::SessionError;
use tauri::{AppHandle, Manager};

use crate::state::AppState;

/// Result of a command.
pub type CommandResult<T> = Result<T, UiError>;

/// Error for a background task that panicked or was cancelled.
fn task_failed(error: impl std::fmt::Display) -> UiError {
    UiError::new("error.app.taskFailed").with("detail", error.to_string())
}

/// Runs `task` on the blocking thread pool: device I/O and file loading must not stall the
/// command threads.
async fn blocking<T: Send + 'static>(
    app: AppHandle,
    task: impl FnOnce(&AppState) -> Result<T, SessionError> + Send + 'static,
) -> CommandResult<T> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        task(&state).map_err(|error| {
            log::warn!("command failed: {error}");
            UiError::from(&error)
        })
    })
    .await
    .map_err(task_failed)?
}
