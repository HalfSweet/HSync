//! Importing HDL waveforms (VCD, FST, GHW): the file is read first, so the import dialog can
//! list its signals, then the chosen signals are laid onto a capture.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use hsync_session::{CaptureSummary, ImportRequest, SessionError, Waveform, WaveformSummary};
use tauri::AppHandle;
use tauri::ipc::Channel;

use super::files::reporter;
use super::{CommandResult, blocking};

/// A waveform file read and waiting for the import dialog's choice of signals.
#[derive(Debug)]
pub struct PendingWaveform {
    pub path: PathBuf,
    pub waveform: Waveform,
}

/// Reads a waveform file and keeps it for [`waveform_import`]; reports how far the reading got
/// and returns what the file holds. Reading cannot be cancelled.
#[tauri::command]
pub async fn waveform_inspect(
    app: AppHandle,
    path: PathBuf,
    on_progress: Channel<f64>,
) -> CommandResult<WaveformSummary> {
    blocking(app, move |state| {
        if state.instrument.lock().is_capturing() {
            return Err(SessionError::Busy);
        }
        log::info!("reading waveform {}", path.display());
        let mut sent = f64::NEG_INFINITY;
        let mut report = |done: f64| {
            if done >= 1.0 || done - sent >= 0.01 {
                sent = done;
                let _ = on_progress.send(done);
            }
        };
        let waveform = hsync_session::read_waveform_file(&path, &mut report)?;
        let summary = waveform.summary();
        log::info!(
            "waveform holds {} items on a {} Hz grid of {} samples",
            summary.items.len(),
            summary.samplerate,
            summary.sample_count
        );
        *state.waveform.lock() = Some(PendingWaveform { path, waveform });
        Ok(summary)
    })
    .await
}

/// Lays the chosen signals of the waveform read last onto a capture, which becomes the open
/// capture; the file is forgotten once imported.
#[tauri::command]
pub async fn waveform_import(
    app: AppHandle,
    request: ImportRequest,
    on_progress: Channel<f64>,
) -> CommandResult<CaptureSummary> {
    blocking(app, move |state| {
        if state.instrument.lock().is_capturing() {
            return Err(SessionError::Busy);
        }
        state.file_cancel.store(false, Ordering::Relaxed);
        let mut pending = state.waveform.lock();
        let Some(PendingWaveform { path, waveform }) = pending.as_mut() else {
            return Err(SessionError::NoWaveform);
        };
        log::info!(
            "importing {} signals of {}",
            request.signals.len(),
            path.display()
        );
        let imported = waveform.import(&request, &mut reporter(state, &on_progress))?;
        let summary = state.workspace.lock().open_imported(path, imported)?;
        *pending = None;
        Ok(summary)
    })
    .await
}

/// Forgets the waveform read last: the import dialog was closed without importing.
#[tauri::command]
pub async fn waveform_discard(app: AppHandle) -> CommandResult<()> {
    blocking(app, move |state| {
        *state.waveform.lock() = None;
        Ok(())
    })
    .await
}
