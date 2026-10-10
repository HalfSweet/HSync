use hsync_driver::WatchHandle;
use hsync_session::{Instrument, Workspace};
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
}

impl AppState {
    pub fn new(instrument: Instrument) -> Self {
        Self {
            workspace: Mutex::new(Workspace::new()),
            instrument: Mutex::new(instrument),
            hotplug: Mutex::new(None),
        }
    }
}
