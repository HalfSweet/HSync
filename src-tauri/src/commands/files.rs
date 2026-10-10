//! Saving and exporting the open capture (DESIGN.md §8), with progress and cancellation.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use hsync_session::{CaptureSummary, ExportRequest, SessionError};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use super::{CommandResult, blocking};
use crate::state::AppState;

/// Sends progress through `channel` about once a percent and says whether to go on.
fn reporter<'a>(state: &'a AppState, channel: &'a Channel<f64>) -> impl FnMut(f64) -> bool + 'a {
    let mut sent = f64::NEG_INFINITY;
    move |done| {
        if done >= 1.0 || done - sent >= 0.01 {
            sent = done;
            let _ = channel.send(done);
        }
        !state.file_cancel.load(Ordering::Relaxed)
    }
}

/// Saves the open capture as a DSView `.dsl` file; it then counts as that file.
#[tauri::command]
pub async fn capture_save(
    app: AppHandle,
    capture_id: u64,
    path: PathBuf,
    on_progress: Channel<f64>,
) -> CommandResult<CaptureSummary> {
    blocking(app, move |state| {
        if state.instrument.lock().is_capturing() {
            return Err(SessionError::Busy);
        }
        state.file_cancel.store(false, Ordering::Relaxed);
        let source = state.workspace.lock().export_source(capture_id)?;
        log::info!("saving capture {capture_id} to {}", path.display());
        let session = source.save(&path, &mut reporter(state, &on_progress))?;
        state
            .workspace
            .lock()
            .finish_save(capture_id, &path, session)
    })
    .await
}

/// Writes the open capture, or part of it, in another format.
#[tauri::command]
pub async fn capture_export(
    app: AppHandle,
    capture_id: u64,
    path: PathBuf,
    request: ExportRequest,
    on_progress: Channel<f64>,
) -> CommandResult<()> {
    blocking(app, move |state| {
        if state.instrument.lock().is_capturing() {
            return Err(SessionError::Busy);
        }
        state.file_cancel.store(false, Ordering::Relaxed);
        let source = state.workspace.lock().export_source(capture_id)?;
        log::info!(
            "exporting capture {capture_id} as {:?} to {}",
            request.kind,
            path.display()
        );
        source.export(&path, &request, &mut reporter(state, &on_progress))
    })
    .await
}

/// Writes text the UI composed, such as the data stream as listed.
#[tauri::command]
pub async fn text_save(app: AppHandle, path: PathBuf, text: String) -> CommandResult<()> {
    blocking(app, move |_| {
        log::info!("writing {} bytes of text to {}", text.len(), path.display());
        hsync_session::write_text(&path, &text)
    })
    .await
}

/// Text files the UI reads (sessions) are small; anything bigger is not one.
const TEXT_LIMIT: u64 = 16 << 20;

/// Reads a text file the UI asked for, such as a session.
#[tauri::command]
pub async fn text_read(app: AppHandle, path: PathBuf) -> CommandResult<String> {
    blocking(app, move |_| hsync_session::read_text(&path, TEXT_LIMIT)).await
}

/// Stops the save or export under way; it fails as cancelled and leaves no file.
#[tauri::command]
pub fn file_cancel(state: State<'_, AppState>) {
    state.file_cancel.store(true, Ordering::Relaxed);
}
