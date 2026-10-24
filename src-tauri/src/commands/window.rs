//! The data stream's window of its own (DESIGN.md §9): one at most, opened beside the main
//! window and closed with it. Its content sets its own title, in the interface's language.

use hsync_core::UiError;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent};

use super::CommandResult;

/// The main window's label, as `tauri.conf.json` names it.
pub const MAIN_WINDOW: &str = "main";
/// The data stream window's label.
pub const STREAM_WINDOW: &str = "stream";
/// Sent to the main window when the data stream window has closed.
pub const STREAM_CLOSED_EVENT: &str = "window://stream-closed";

/// What the data stream window loads: the interface, told to show the data stream alone.
const STREAM_PAGE: &str = "index.html?view=stream";

fn window_error(error: &tauri::Error) -> UiError {
    UiError::new("error.window.stream").with("detail", error.to_string())
}

/// Opens the data stream window, or brings it to the front when it is open already. The
/// window-state plugin puts it back where it was last.
#[tauri::command]
pub fn stream_window_open(app: AppHandle) -> CommandResult<()> {
    if let Some(window) = app.get_webview_window(STREAM_WINDOW) {
        window.unminimize().map_err(|error| window_error(&error))?;
        window.show().map_err(|error| window_error(&error))?;
        return window.set_focus().map_err(|error| window_error(&error));
    }
    WebviewWindowBuilder::new(&app, STREAM_WINDOW, WebviewUrl::App(STREAM_PAGE.into()))
        .title("HSync")
        .inner_size(520.0, 820.0)
        .min_inner_size(360.0, 400.0)
        .build()
        .map(|_| ())
        .map_err(|error| window_error(&error))
}

/// Closes the data stream window, if it is open.
#[tauri::command]
pub fn stream_window_close(app: AppHandle) -> CommandResult<()> {
    match app.get_webview_window(STREAM_WINDOW) {
        Some(window) => window.close().map_err(|error| window_error(&error)),
        None => Ok(()),
    }
}

/// Closes the data stream window with the main one, which would otherwise keep the app
/// running, and tells the main window when the data stream window has gone.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if !matches!(event, WindowEvent::Destroyed) {
        return;
    }
    let app = window.app_handle();
    match window.label() {
        MAIN_WINDOW => {
            if let Some(stream) = app.get_webview_window(STREAM_WINDOW) {
                let _ = stream.close();
            }
        }
        STREAM_WINDOW => {
            let _ = app.emit_to(MAIN_WINDOW, STREAM_CLOSED_EVENT, ());
        }
        _ => {}
    }
}
