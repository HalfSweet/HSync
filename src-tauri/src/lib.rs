//! HSync desktop application: Tauri commands, events and state wiring.

mod commands;
mod state;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use hsync_atklogic::AtkLogicDriver;
use hsync_demo::DemoDriver;
use hsync_driver::{DeviceManager, FirmwareDir};
use hsync_dslogic::DslogicDriver;
use hsync_pxlogic::PxlogicDriver;
use hsync_session::{EventSink, Instrument, SessionEvent, Workspace};
use hsync_slogic::SlogicDriver;
use state::AppState;
use tauri::path::BaseDirectory;
use tauri::{App, Emitter, Manager};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind};
use tauri_plugin_window_state::StateFlags;

/// Crates whose messages follow the log level of the settings; others log warnings and up.
const OWN_LOG_TARGETS: [&str; 9] = [
    "hsync_lib",
    "hsync_session",
    "hsync_driver",
    "hsync_dslogic",
    "hsync_atklogic",
    "hsync_pxlogic",
    "hsync_slogic",
    "hsync_demo",
    "hsync_uiharu",
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
    let manager = DeviceManager::new(vec![
        Arc::new(DemoDriver::new(demo)),
        Arc::new(DslogicDriver::new(FirmwareDir(firmware.clone()))),
        Arc::new(AtkLogicDriver::new()),
        Arc::new(PxlogicDriver::new(FirmwareDir(firmware))),
        Arc::new(SlogicDriver::new()),
    ]);
    let emitter = app.handle().clone();
    let events: EventSink = Arc::new(move |event| {
        let decoder = |capture_id: u64, index: u32| serde_json::json!({ "captureId": capture_id, "index": index });
        let (name, payload) = match event {
            SessionEvent::Decoded { capture_id, index } => {
                (commands::decode::DECODED_EVENT, decoder(capture_id, index))
            }
            SessionEvent::DecodeProgress { capture_id, index } => (
                commands::decode::DECODE_PROGRESS_EVENT,
                decoder(capture_id, index),
            ),
            SessionEvent::ModulesChanged => {
                (commands::decode::MODULES_EVENT, serde_json::Value::Null)
            }
            SessionEvent::Live(_) => return,
        };
        let _ = emitter.emit(name, payload);
    });
    let workspace = Workspace::with_events(events);
    app.manage(AppState::new(workspace, Instrument::new(manager.clone())));
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
        // Only the data stream window comes back where it was; the main window opens as set.
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_denylist(&[commands::window::MAIN_WINDOW])
                .with_state_flags(
                    StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED,
                )
                .build(),
        )
        .on_window_event(commands::window::on_window_event)
        .setup(setup)
        .invoke_handler(tauri::generate_handler![
            commands::app::log_configure,
            commands::app::log_file,
            commands::app::log_reveal,
            commands::app::system_memory,
            commands::capture::capture_summary,
            commands::capture::capture_open_file,
            commands::capture::capture_start,
            commands::capture::capture_stop,
            commands::capture::capture_bins,
            commands::capture::capture_analog_bins,
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
            commands::decode::decode_modules,
            commands::decode::decode_load_module,
            commands::decode::decode_unload_module,
            commands::decode::decode_row_at,
            commands::decode::decode_search,
            commands::decode::decode_coverage,
            commands::files::capture_export,
            commands::files::capture_save,
            commands::files::file_cancel,
            commands::files::text_read,
            commands::files::text_save,
            commands::waveform::waveform_inspect,
            commands::waveform::waveform_import,
            commands::waveform::waveform_discard,
            commands::decode::decode_set,
            commands::device::device_list,
            commands::device::device_open,
            commands::device::device_state,
            commands::device::device_set_option,
            commands::device::device_set_channel,
            commands::device::device_set_analog,
            commands::device::device_set_channels,
            commands::device::device_select_rate_tier,
            commands::window::stream_window_open,
            commands::window::stream_window_close,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run the HSync application");
}
