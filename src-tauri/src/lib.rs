//! HSync desktop application: Tauri commands, events and state wiring.

mod commands;
mod state;

use std::sync::Arc;
use std::time::Duration;

use hsync_demo::DemoDriver;
use hsync_driver::DeviceManager;
use hsync_dslogic::{DslogicDriver, FirmwareDir};
use hsync_session::Instrument;
use state::AppState;
use tauri::path::BaseDirectory;
use tauri::{App, Manager};
use tauri_plugin_log::{Target, TargetKind};

/// Crates whose debug messages development builds keep.
const OWN_LOG_TARGETS: [&str; 5] = [
    "hsync_lib",
    "hsync_session",
    "hsync_driver",
    "hsync_dslogic",
    "hsync_demo",
];

/// Quiet time after a USB change before rescanning; devices re-enumerate in bursts.
const HOTPLUG_SETTLE: Duration = Duration::from_millis(300);

/// Registers the drivers, the shared state and the USB hotplug watch.
fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let resolver = app.path();
    let demo = resolver
        .resolve("demo/protocol.dsl", BaseDirectory::Resource)
        .ok();
    let firmware = resolver.resolve("firmware", BaseDirectory::Resource)?;
    let manager = DeviceManager::new(vec![
        Arc::new(DemoDriver::new(demo)),
        Arc::new(DslogicDriver::new(FirmwareDir(firmware))),
    ]);
    app.manage(AppState::new(Instrument::new(manager.clone())));
    let handle = app.handle().clone();
    let watch = manager.watch(HOTPLUG_SETTLE, move || {
        commands::device::devices_changed(&handle);
    });
    *app.state::<AppState>().hotplug.lock() = watch;
    Ok(())
}

/// Builds and runs the Tauri application.
///
/// # Panics
///
/// Panics when the Tauri runtime fails to start, which leaves nothing to recover.
#[allow(clippy::expect_used)]
pub fn run() {
    let detail = if cfg!(debug_assertions) {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };
    let mut logger = tauri_plugin_log::Builder::new().level(log::LevelFilter::Info);
    for target in OWN_LOG_TARGETS {
        logger = logger.level_for(target, detail);
    }
    tauri::Builder::default()
        .plugin(
            logger
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::LogDir { file_name: None }),
                ])
                .build(),
        )
        .setup(setup)
        .invoke_handler(tauri::generate_handler![
            commands::capture::capture_summary,
            commands::capture::capture_open_demo,
            commands::capture::capture_open_file,
            commands::capture::capture_start,
            commands::capture::capture_stop,
            commands::capture::capture_bins,
            commands::capture::capture_pulse,
            commands::capture::capture_edge,
            commands::capture::capture_clock_period,
            commands::device::device_list,
            commands::device::device_open,
            commands::device::device_state,
            commands::device::device_set_option,
            commands::device::device_set_channel,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run the HSync application");
}
