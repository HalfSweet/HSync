//! Decoded data: decoder states, lanes for the timeline, the data stream listings, the search
//! and the catalog of decoders.

use hsync_session::{
    CatalogEntry, DecoderLanes, DecoderStatus, ModuleInfo, SearchResult, StreamPage,
};
use tauri::State;

use super::CommandResult;
use crate::state::AppState;
use hsync_core::UiError;

/// Event sent when a decoder of a capture finished, or failed: `{ captureId, index }`.
pub const DECODED_EVENT: &str = "decode://done";

/// Event sent when a decoder of a capture has more to show: `{ captureId, index }`.
pub const DECODE_PROGRESS_EVENT: &str = "decode://progress";

/// Event sent when the decoder modules loaded changed without a request: a capture file
/// brought some. No payload; the interface asks for the modules and the catalog again.
pub const MODULES_EVENT: &str = "decode://modules";

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

/// The decoders of a capture, their streams and how far they are.
#[tauri::command]
pub fn decode_list(
    state: State<'_, AppState>,
    capture_id: u64,
) -> CommandResult<Vec<DecoderStatus>> {
    Ok(decoding(&state, capture_id)?.statuses())
}

/// Lanes of every decoder over `start..end`.
#[tauri::command]
pub fn decode_view(
    state: State<'_, AppState>,
    capture_id: u64,
    start: u64,
    end: u64,
    samples_per_pixel: f64,
) -> CommandResult<Vec<DecoderLanes>> {
    Ok(decoding(&state, capture_id)?.view(start..end, samples_per_pixel.max(1e-6)))
}

/// Rows of a stream's listing in the data stream.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn decode_stream(
    state: State<'_, AppState>,
    capture_id: u64,
    index: u32,
    stream: u32,
    segment: Option<(u64, u64)>,
    only_segment: bool,
    offset: u32,
    count: u32,
) -> CommandResult<Option<StreamPage>> {
    Ok(decoding(&state, capture_id)?.stream(index, stream, segment, only_segment, offset, count))
}

/// Position in a stream's listing of the first event reaching past `sample`.
#[tauri::command]
pub fn decode_row_at(
    state: State<'_, AppState>,
    capture_id: u64,
    index: u32,
    stream: u32,
    sample: u64,
) -> CommandResult<Option<u32>> {
    Ok(decoding(&state, capture_id)?.row_at(index, stream, sample))
}

/// Events matching `query` in every decoder's streams; nothing for a blank query.
#[tauri::command]
pub async fn decode_search(
    state: State<'_, AppState>,
    capture_id: u64,
    query: String,
) -> CommandResult<Option<SearchResult>> {
    Ok(decoding(&state, capture_id)?.search(&query))
}

/// The stretches of `start..end` where decoder `index` has events, those closer than
/// `resolution` samples joined; for the overview strip.
#[tauri::command]
pub fn decode_coverage(
    state: State<'_, AppState>,
    capture_id: u64,
    index: u32,
    start: u64,
    end: u64,
    resolution: u64,
) -> CommandResult<Option<Vec<[u64; 2]>>> {
    Ok(decoding(&state, capture_id)?.coverage(index, start..end, resolution.max(1)))
}

/// Every decoder the app offers, built in or loaded, with its configuration fields and
/// event kinds.
#[tauri::command]
pub fn decode_catalog(state: State<'_, AppState>) -> Vec<CatalogEntry> {
    state.workspace.lock().catalog()
}

/// The decoder modules loaded, in the order they were.
#[tauri::command]
pub fn decode_modules(state: State<'_, AppState>) -> Vec<ModuleInfo> {
    state.workspace.lock().modules()
}

/// Loads the decoder module at `path`: a decoder written in the DSL, as its IR. It stands in
/// for a decoder of its name built in or loaded before, and the open capture's decoders that
/// name it decode again with it.
#[tauri::command]
pub async fn decode_load_module(app: tauri::AppHandle, path: String) -> CommandResult<ModuleInfo> {
    super::blocking(app, move |state| {
        state
            .workspace
            .lock()
            .load_module(std::path::Path::new(&path))
    })
    .await
}

/// Forgets the decoder module `name`; the open capture's decoders that name it decode again
/// with the decoder built in. Whether there was such a module.
#[tauri::command]
pub async fn decode_unload_module(app: tauri::AppHandle, name: String) -> CommandResult<bool> {
    super::blocking(app, move |state| {
        Ok(state.workspace.lock().unload_module(&name))
    })
    .await
}

/// Replaces the decoders of a capture with `decoders` (HSync's entries) and decodes again,
/// while the capture is still being taken too.
#[tauri::command]
pub async fn decode_set(
    app: tauri::AppHandle,
    capture_id: u64,
    decoders: serde_json::Value,
) -> CommandResult<hsync_session::CaptureSummary> {
    super::blocking(app, move |state| {
        state.workspace.lock().set_decoders(capture_id, decoders)
    })
    .await
}
