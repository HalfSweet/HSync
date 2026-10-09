//! Opening captures and querying their samples.

use std::path::PathBuf;

use hsync_core::{ChannelIndex, UiError};
use hsync_session::{BinsRequest, CaptureSummary, Direction, PulseInfo, encode_bins};
use tauri::ipc::Response;
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager, State};

use super::{CommandResult, task_failed};
use crate::state::AppState;

/// Location of the demo capture among the bundled resources.
const DEMO_CAPTURE: &str = "demo/protocol.dsl";

/// The open capture, if any.
#[tauri::command]
pub fn capture_summary(state: State<'_, AppState>) -> Option<CaptureSummary> {
    state.workspace.lock().summary()
}

/// Opens the bundled demo capture.
#[tauri::command]
pub async fn capture_open_demo(app: AppHandle) -> CommandResult<CaptureSummary> {
    let path = app
        .path()
        .resolve(DEMO_CAPTURE, BaseDirectory::Resource)
        .map_err(|error| {
            UiError::new("error.app.resourceMissing").with("detail", error.to_string())
        })?;
    open(app, move |workspace| workspace.open_demo(&path)).await
}

/// Opens a `.dsl` capture file.
#[tauri::command]
pub async fn capture_open_file(app: AppHandle, path: PathBuf) -> CommandResult<CaptureSummary> {
    open(app, move |workspace| workspace.open_file(&path)).await
}

async fn open(
    app: AppHandle,
    load: impl FnOnce(
        &mut hsync_session::Workspace,
    ) -> Result<CaptureSummary, hsync_session::SessionError>
    + Send
    + 'static,
) -> CommandResult<CaptureSummary> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let mut workspace = state.workspace.lock();
        load(&mut workspace).map_err(|error| UiError::from(&error))
    })
    .await
    .map_err(task_failed)?
}

/// Summarises channels over a sample range in the binary layout of DESIGN.md §5.4.
#[tauri::command]
pub async fn capture_bins(
    state: State<'_, AppState>,
    request: BinsRequest,
) -> CommandResult<Response> {
    let capture = state
        .workspace
        .lock()
        .capture(request.capture_id)
        .map_err(|error| UiError::from(&error))?;
    let bytes = encode_bins(&capture.read(), &request).map_err(|error| UiError::from(&error))?;
    Ok(Response::new(bytes))
}

/// The pulse of a channel around a sample, for the hover read-out.
#[tauri::command]
pub fn capture_pulse(
    state: State<'_, AppState>,
    capture_id: u64,
    channel: ChannelIndex,
    sample: u64,
) -> CommandResult<Option<PulseInfo>> {
    let capture = state
        .workspace
        .lock()
        .capture(capture_id)
        .map_err(|error| UiError::from(&error))?;
    let pulse = hsync_session::pulse(&capture.read(), channel, sample);
    pulse.map_err(|error| UiError::from(&error))
}

/// The nearest edge of a channel before or after a sample.
#[tauri::command]
pub fn capture_edge(
    state: State<'_, AppState>,
    capture_id: u64,
    channel: ChannelIndex,
    sample: u64,
    direction: Direction,
) -> CommandResult<Option<u64>> {
    let capture = state
        .workspace
        .lock()
        .capture(capture_id)
        .map_err(|error| UiError::from(&error))?;
    let edge = hsync_session::edge(&capture.read(), channel, sample, direction);
    edge.map_err(|error| UiError::from(&error))
}
