use hsync_session::Workspace;
use parking_lot::Mutex;

/// State shared by all commands of the main window.
#[derive(Debug, Default)]
pub struct AppState {
    /// The open capture and its queries.
    pub workspace: Mutex<Workspace>,
}
