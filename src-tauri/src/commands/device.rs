//! Finding, opening and configuring devices.

use hsync_core::ChannelIndex;
use hsync_driver::{DeviceSummary, OptionKey, OptionValue};
use hsync_session::DeviceState;
use tauri::{AppHandle, Emitter, Manager, State};

use super::{CommandResult, blocking};
use crate::state::AppState;

/// Event with the device list, sent when devices come or go.
pub const DEVICE_LIST_EVENT: &str = "device://list";
/// Event with the open device, sent when it changes without being asked (it went away).
pub const DEVICE_STATE_EVENT: &str = "device://state";

/// Scans for devices.
#[tauri::command]
pub async fn device_list(app: AppHandle) -> CommandResult<Vec<DeviceSummary>> {
    blocking(app, |state| Ok(state.instrument.lock().scan())).await
}

/// Opens a device from the list, uploading its firmware first when it needs that.
#[tauri::command]
pub async fn device_open(app: AppHandle, id: String) -> CommandResult<DeviceState> {
    blocking(app, move |state| state.instrument.lock().open(&id)).await
}

/// The open device, if any.
#[tauri::command]
pub fn device_state(state: State<'_, AppState>) -> Option<DeviceState> {
    state.instrument.lock().state()
}

/// Changes an option of the open device.
#[tauri::command]
pub async fn device_set_option(
    app: AppHandle,
    key: OptionKey,
    value: OptionValue,
) -> CommandResult<DeviceState> {
    blocking(app, move |state| {
        state.instrument.lock().set_option(key, &value)
    })
    .await
}

/// Enables or disables a channel of the open device.
#[tauri::command]
pub async fn device_set_channel(
    app: AppHandle,
    channel: ChannelIndex,
    enabled: bool,
) -> CommandResult<DeviceState> {
    blocking(app, move |state| {
        state
            .instrument
            .lock()
            .set_channel_enabled(channel, enabled)
    })
    .await
}

/// Rescans after USB devices came or went and tells the UI.
pub fn devices_changed(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut instrument = state.instrument.lock();
    let devices = instrument.scan();
    let gone = instrument.drop_if_gone();
    drop(instrument);
    log::info!("devices changed: {} found", devices.len());
    let _ = app.emit(DEVICE_LIST_EVENT, &devices);
    if gone {
        let _ = app.emit(DEVICE_STATE_EVENT, Option::<DeviceState>::None);
    }
}
