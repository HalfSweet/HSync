//! PXLogic driver (PX-Logic 32, 16 Pro, 16 Plus, 16 Base): the `pxlogic` driver of PXView's
//! libsigrok (`src/hardware/pxlogic`) ported to Rust and nusb.

#![forbid(unsafe_code)]

mod acquisition;
mod device;
pub mod profiles;
pub mod protocol;

pub use device::{ExternalTrigger, MAX_SOFTWARE_DEPTH, MIN_SAMPLES, PxlogicDevice, Settings};

/// Driver name, also recorded in capture files; devices whose model is not yet known are
/// listed under it.
pub const DRIVER_NAME: &str = "PXLogic";
/// Vendor of every PXLogic.
pub const VENDOR: &str = "PXLogic";
