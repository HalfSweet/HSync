//! The app's surroundings: diagnostics (DESIGN.md §2.5), the log level and log file the settings
//! panel controls, and the machine's memory, which large captures must fit in (§5.2).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use hsync_core::UiError;
use tauri::{AppHandle, Manager};

use super::CommandResult;

/// Whether messages go to the log file as well as to stdout.
pub static LOG_FILE: AtomicBool = AtomicBool::new(true);

/// The least severe messages kept, by the names the settings panel uses.
fn level_filter(level: &str) -> log::LevelFilter {
    match level {
        "error" => log::LevelFilter::Error,
        "warn" => log::LevelFilter::Warn,
        "debug" => log::LevelFilter::Debug,
        _ => log::LevelFilter::Info,
    }
}

/// Applies the diagnostics settings: which messages are logged, and whether the log file is
/// written. Other crates stay at warnings and above whatever the level.
#[tauri::command]
pub fn log_configure(level: String, file: bool) {
    let filter = level_filter(&level);
    log::set_max_level(filter);
    LOG_FILE.store(file, Ordering::Relaxed);
    log::info!(
        "logging {filter} and above, log file {}",
        if file { "on" } else { "off" }
    );
}

/// The file the log plugin writes: the app's name in the app's log folder.
fn log_path(app: &AppHandle) -> Result<PathBuf, UiError> {
    let folder = app
        .path()
        .app_log_dir()
        .map_err(|error| UiError::new("error.app.logFolder").with("detail", error.to_string()))?;
    Ok(folder.join(&app.package_info().name).with_extension("log"))
}

/// Where the log file is written.
#[tauri::command]
pub fn log_file(app: AppHandle) -> CommandResult<String> {
    Ok(log_path(&app)?.to_string_lossy().into_owned())
}

/// Shows the log file in the file manager, or its folder while there is no file yet.
#[tauri::command]
pub fn log_reveal(app: AppHandle) -> CommandResult<()> {
    let file = log_path(&app)?;
    let target = if file.exists() {
        file.as_path()
    } else {
        file.parent().unwrap_or(Path::new("."))
    };
    tauri_plugin_opener::reveal_item_in_dir(target)
        .map_err(|error| UiError::new("error.app.reveal").with("detail", error.to_string()))
}

/// Physical memory in bytes; captures beyond half of it get a warning, as DESIGN.md §5.2 asks.
#[tauri::command]
pub fn system_memory() -> u64 {
    let mut system = sysinfo::System::new();
    system.refresh_memory_specifics(sysinfo::MemoryRefreshKind::nothing().with_ram());
    system.total_memory()
}
