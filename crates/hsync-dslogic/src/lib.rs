//! DreamSourceLab DSLogic driver: libsigrok4DSL's `hardware/DSL` ported to Rust and nusb
//! (DESIGN.md §4.2–§4.9).

#![forbid(unsafe_code)]

mod acquisition;
pub mod boot;
mod device;
pub mod profiles;
pub mod protocol;
pub mod setting;

pub use boot::FirmwareDir;
pub use device::{DslogicDevice, MAX_SOFTWARE_DEPTH, MIN_SAMPLES, Settings, Threshold, vth_code};

/// Driver name, also recorded in capture files.
pub const DRIVER_NAME: &str = "DSLogic";
/// Vendor of every DSLogic.
pub const VENDOR: &str = "DreamSourceLab";
