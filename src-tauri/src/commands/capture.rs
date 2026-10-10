//! Opening captures and querying their samples.

use std::path::PathBuf;
use std::sync::Arc;

use hsync_core::{ChannelIndex, UiError};
use hsync_session::{
    BinsRequest, CaptureRequest, CaptureSummary, ChannelEdges, Direction, EventSink,
    PatternCondition, PulseInfo, SessionEvent, encode_bins,
};
use tauri::ipc::Response;
use tauri::{AppHandle, Emitter, State};

use super::{CommandResult, blocking};
use crate::state::AppState;

/// Event with the [`hsync_session::LiveStatus`] of a capture taken with a device.
pub const LIVE_EVENT: &str = "capture://live";

/// The open capture, if any.
#[tauri::command]
pub fn capture_summary(state: State<'_, AppState>) -> Option<CaptureSummary> {
    state.workspace.lock().summary()
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
    blocking(app, move |state| {
        if state.instrument.lock().is_capturing() {
            return Err(hsync_session::SessionError::Busy);
        }
        load(&mut state.workspace.lock())
    })
    .await
}

/// Renames a channel of the open capture; an empty name goes back to its number.
#[tauri::command]
pub fn capture_rename_channel(
    state: State<'_, AppState>,
    capture_id: u64,
    channel: ChannelIndex,
    name: String,
) -> CommandResult<CaptureSummary> {
    let renamed = state
        .workspace
        .lock()
        .rename_channel(capture_id, channel, &name);
    renamed.map_err(|error| UiError::from(&error))
}

/// Starts a capture on the open device. The new capture replaces the open one at once and
/// fills while [`LIVE_EVENT`]s report its progress.
#[tauri::command]
pub async fn capture_start(
    app: AppHandle,
    request: CaptureRequest,
) -> CommandResult<CaptureSummary> {
    let emitter = app.clone();
    let events: EventSink = Arc::new(move |event| {
        if let SessionEvent::Live(status) = event {
            let _ = emitter.emit(LIVE_EVENT, status);
        }
    });
    blocking(app, move |state| {
        let mut instrument = state.instrument.lock();
        let mut workspace = state.workspace.lock();
        instrument.start(&mut workspace, request, events)
    })
    .await
}

/// Stops the running capture; it ends with what it captured so far.
#[tauri::command]
pub fn capture_stop(state: State<'_, AppState>) {
    state.instrument.lock().stop();
}

/// Summarises channels over a sample range in the binary layout of viewport queries
/// (`hsync_session::encode_bins`).
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
    log::debug!(
        "bins: {} channels, samples {}..{} in {} columns",
        request.channels.len(),
        request.start,
        request.end,
        request.bins
    );
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

/// Rising and falling edges of channels within a segment, for the segment measurement.
#[tauri::command]
pub async fn capture_edges(
    state: State<'_, AppState>,
    capture_id: u64,
    from: u64,
    to: u64,
    channels: Vec<ChannelIndex>,
) -> CommandResult<Vec<ChannelEdges>> {
    let capture = state
        .workspace
        .lock()
        .capture(capture_id)
        .map_err(|error| UiError::from(&error))?;
    let counted = hsync_session::edges(&capture.read(), from, to, &channels);
    counted.map_err(|error| UiError::from(&error))
}

/// The nearest sample from `sample` on, or back from it, where the channels show a level
/// pattern (DSView's search dock).
#[tauri::command]
pub async fn capture_find(
    state: State<'_, AppState>,
    capture_id: u64,
    conditions: Vec<PatternCondition>,
    sample: u64,
    direction: Direction,
) -> CommandResult<Option<u64>> {
    let capture = state
        .workspace
        .lock()
        .capture(capture_id)
        .map_err(|error| UiError::from(&error))?;
    let forward = matches!(direction, Direction::Next);
    let found = capture.read().find_pattern(&conditions, sample, forward);
    Ok(found)
}

/// The median period of a clock channel in samples, for the group meta of I2C and SPI.
#[tauri::command]
pub fn capture_clock_period(
    state: State<'_, AppState>,
    capture_id: u64,
    channel: ChannelIndex,
) -> CommandResult<Option<f64>> {
    let capture = state
        .workspace
        .lock()
        .capture(capture_id)
        .map_err(|error| UiError::from(&error))?;
    let period = hsync_session::clock_period(&capture.read(), channel);
    period.map_err(|error| UiError::from(&error))
}
