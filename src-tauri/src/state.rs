use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use hsync_driver::WatchHandle;
use hsync_session::{Instrument, Workspace};
use hsync_srd::DecoderHost;
use parking_lot::Mutex;

/// State shared by all commands of the main window. Commands that need both lock the
/// instrument before the workspace.
#[derive(Debug)]
pub struct AppState {
    /// The open capture and its queries.
    pub workspace: Mutex<Workspace>,
    /// Devices and the running capture.
    pub instrument: Mutex<Instrument>,
    /// Keeps the USB hotplug watch alive.
    pub hotplug: Mutex<Option<WatchHandle>>,
    /// Asks the save or export under way to stop.
    pub file_cancel: AtomicBool,
    /// The bundled Python protocol decoders.
    pub decoders: Arc<DecoderHost>,
}

impl AppState {
    pub fn new(workspace: Workspace, instrument: Instrument, decoders: Arc<DecoderHost>) -> Self {
        Self {
            workspace: Mutex::new(workspace),
            instrument: Mutex::new(instrument),
            hotplug: Mutex::new(None),
            file_cancel: AtomicBool::new(false),
            decoders,
        }
    }
}
