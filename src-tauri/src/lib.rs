//! HSync desktop application: Tauri commands, events and state wiring.

mod commands;
mod state;

use state::AppState;

/// Builds and runs the Tauri application.
///
/// # Panics
///
/// Panics when the Tauri runtime fails to start, which leaves nothing to recover.
#[allow(clippy::expect_used)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::capture::capture_summary,
            commands::capture::capture_open_demo,
            commands::capture::capture_open_file,
            commands::capture::capture_bins,
            commands::capture::capture_pulse,
            commands::capture::capture_edge,
            commands::capture::capture_clock_period,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run the HSync application");
}
