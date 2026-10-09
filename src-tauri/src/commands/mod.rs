//! IPC commands, see DESIGN.md §3.4. Errors cross as [`UiError`]: an i18n key and parameters.

pub mod capture;

use hsync_core::UiError;

/// Result of a command.
pub type CommandResult<T> = Result<T, UiError>;

/// Error for a background task that panicked or was cancelled.
fn task_failed(error: impl std::fmt::Display) -> UiError {
    UiError::new("error.app.taskFailed").with("detail", error.to_string())
}
