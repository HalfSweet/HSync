use std::sync::atomic::AtomicBool;

use hsync_driver::WatchHandle;
use hsync_session::{Instrument, Workspace};
use parking_lot::Mutex;

use crate::commands::waveform::PendingWaveform;

/// State shared by all commands of the main window. Commands that need more than one lock take
/// them in the order of the fields: the instrument before the waveform before the workspace.
#[derive(Debug)]
pub struct AppState {
    /// Devices and the running capture.
    pub instrument: Mutex<Instrument>,
    /// A waveform file read for the import dialog, until it is imported or dismissed.
    pub waveform: Mutex<Option<PendingWaveform>>,
    /// The open capture and its queries.
    pub workspace: Mutex<Workspace>,
    /// Keeps the USB hotplug watch alive.
    pub hotplug: Mutex<Option<WatchHandle>>,
    /// Asks the save, export or import under way to stop.
    pub file_cancel: AtomicBool,
}

impl AppState {
    pub fn new(workspace: Workspace, instrument: Instrument) -> Self {
        Self {
            instrument: Mutex::new(instrument),
            waveform: Mutex::new(None),
            workspace: Mutex::new(workspace),
            hotplug: Mutex::new(None),
            file_cancel: AtomicBool::new(false),
        }
    }
}
