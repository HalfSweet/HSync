//! Decoded data: decoder states, lanes for the timeline and the data stream.

use hsync_decode::StreamMode;
use hsync_session::{DecoderLanes, DecoderStatus, SearchResult, StreamPage};
use tauri::State;

use super::CommandResult;
use crate::state::AppState;
use hsync_core::UiError;

/// Event sent when a decoder of the open capture finished: `{ captureId, index }`.
pub const DECODED_EVENT: &str = "decode://done";

fn decoding(
    state: &AppState,
    capture_id: u64,
) -> CommandResult<std::sync::Arc<hsync_session::DecodeSet>> {
    state
        .workspace
        .lock()
        .decoding(capture_id)
        .map_err(|error| UiError::from(&error))
}

/// The decoders of a capture and how far they are.
#[tauri::command]
pub fn decode_list(
    state: State<'_, AppState>,
    capture_id: u64,
) -> CommandResult<Vec<DecoderStatus>> {
    Ok(decoding(&state, capture_id)?.statuses())
}

/// Lanes of every finished decoder over `start..end`.
#[tauri::command]
pub fn decode_view(
    state: State<'_, AppState>,
    capture_id: u64,
    start: u64,
    end: u64,
    samples_per_pixel: f64,
) -> CommandResult<Vec<DecoderLanes>> {
    Ok(decoding(&state, capture_id)?.view(start, end, samples_per_pixel.max(1e-6)))
}

/// Rows of a decoder's data stream.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn decode_stream(
    state: State<'_, AppState>,
    capture_id: u64,
    index: u32,
    mode: StreamMode,
    segment: Option<(u64, u64)>,
    only_segment: bool,
    offset: u32,
    count: u32,
) -> CommandResult<Option<StreamPage>> {
    Ok(decoding(&state, capture_id)?.stream(index, mode, segment, only_segment, offset, count))
}

/// Position in a decoder's data stream of the first row ending after `sample`.
#[tauri::command]
pub fn decode_row_at(
    state: State<'_, AppState>,
    capture_id: u64,
    index: u32,
    mode: StreamMode,
    sample: u64,
) -> CommandResult<Option<u32>> {
    Ok(decoding(&state, capture_id)?.row_at(index, mode, sample))
}

/// Values or text matching `query` in every decoder's results, with their rows in `mode`
/// listings; nothing for a blank query.
#[tauri::command]
pub async fn decode_search(
    state: State<'_, AppState>,
    capture_id: u64,
    query: String,
    mode: StreamMode,
) -> CommandResult<Option<SearchResult>> {
    Ok(decoding(&state, capture_id)?.search(&query, mode))
}

/// Replaces the decoders of the open capture (DSView's `decoders` list) and decodes again.
#[tauri::command]
pub async fn decode_set(
    app: tauri::AppHandle,
    capture_id: u64,
    decoders: serde_json::Value,
) -> CommandResult<hsync_session::CaptureSummary> {
    super::blocking(app, move |state| {
        let capturing = state.instrument.lock().is_capturing();
        state
            .workspace
            .lock()
            .set_decoders(capture_id, decoders, capturing)
    })
    .await
}
