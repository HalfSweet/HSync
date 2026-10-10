//! HSync desktop application: Tauri commands, events and state wiring.

mod commands;
mod state;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use hsync_demo::DemoDriver;
use hsync_driver::DeviceManager;
use hsync_dslogic::{DslogicDriver, FirmwareDir};
use hsync_session::{EventSink, Instrument, SessionEvent, Workspace};
use state::AppState;
use tauri::path::BaseDirectory;
use tauri::{App, Emitter, Manager};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind};

/// Crates whose messages follow the log level of the settings; others log warnings and up.
const OWN_LOG_TARGETS: [&str; 6] = [
    "hsync_lib",
    "hsync_session",
    "hsync_driver",
    "hsync_dslogic",
    "hsync_demo",
    "hsync_srd",
];

/// Size at which the log file starts over, keeping the previous ones.
const LOG_FILE_BYTES: u128 = 4 << 20;

/// Quiet time after a USB change before rescanning; devices re-enumerate in bursts.
const HOTPLUG_SETTLE: Duration = Duration::from_millis(300);

/// Registers the drivers, the shared state and the USB hotplug watch.
fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    // Until the settings say otherwise (`log_configure`): information, debugging in development.
    log::set_max_level(if cfg!(debug_assertions) {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    });
    let resolver = app.path();
    let demo = resolver
        .resolve("demo/protocol.dsl", BaseDirectory::Resource)
        .ok();
    let firmware = resolver.resolve("firmware", BaseDirectory::Resource)?;
    let decoders =
        hsync_srd::DecoderHost::new(resolver.resolve("decoders", BaseDirectory::Resource)?);
    // Release builds carry their own Python; development builds use the one they
    // linked against.
    let python = resolver.resolve("python", BaseDirectory::Resource)?;
    let decoders = Arc::new(if python.is_dir() {
        decoders.with_python_home(python)
    } else {
        decoders
    });
    // Reading the decoders starts Python and imports them all; do it before anyone asks.
    let preload = Arc::clone(&decoders);
    std::thread::Builder::new()
        .name("decoder-catalog".into())
        .spawn(move || {
            let _ = preload.catalog();
        })?;
    let manager = DeviceManager::new(vec![
        Arc::new(DemoDriver::new(demo)),
        Arc::new(DslogicDriver::new(FirmwareDir(firmware))),
    ]);
    let emitter = app.handle().clone();
    let events: EventSink = Arc::new(move |event| {
        if let SessionEvent::Decoded { capture_id, index } = event {
            let _ = emitter.emit(
                commands::decode::DECODED_EVENT,
                serde_json::json!({ "captureId": capture_id, "index": index }),
            );
        }
    });
    let mut workspace = Workspace::with_events(events);
    workspace.set_decoder_host(Arc::clone(&decoders));
    app.manage(AppState::new(
        workspace,
        Instrument::new(manager.clone()),
        decoders,
    ));
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
    // Everything of our own passes here; the level set at run time decides what is logged.
    let mut logger = tauri_plugin_log::Builder::new().level(log::LevelFilter::Warn);
    for target in OWN_LOG_TARGETS {
        logger = logger.level_for(target, log::LevelFilter::Trace);
    }
    tauri::Builder::default()
        .plugin(
            logger
                .max_file_size(LOG_FILE_BYTES)
                .rotation_strategy(RotationStrategy::KeepSome(2))
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::LogDir { file_name: None })
                        .filter(|_| commands::app::LOG_FILE.load(Ordering::Relaxed)),
                ])
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(setup)
        .invoke_handler(tauri::generate_handler![
            commands::app::log_configure,
            commands::app::log_file,
            commands::app::log_reveal,
            commands::app::system_memory,
            commands::capture::capture_summary,
            commands::capture::capture_open_demo,
            commands::capture::capture_open_file,
            commands::capture::capture_start,
            commands::capture::capture_stop,
            commands::capture::capture_bins,
            commands::capture::capture_pulse,
            commands::capture::capture_edge,
            commands::capture::capture_edges,
            commands::capture::capture_find,
            commands::capture::capture_rename_channel,
            commands::capture::capture_clock_period,
            commands::decode::decode_list,
            commands::decode::decode_view,
            commands::decode::decode_stream,
            commands::decode::decode_catalog,
            commands::decode::decode_row_at,
            commands::decode::decode_search,
            commands::files::capture_export,
            commands::files::capture_save,
            commands::files::file_cancel,
            commands::files::text_read,
            commands::files::text_save,
            commands::decode::decode_set,
            commands::device::device_list,
            commands::device::device_open,
            commands::device::device_state,
            commands::device::device_set_option,
            commands::device::device_set_channel,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run the HSync application");
}
