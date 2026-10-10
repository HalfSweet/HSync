//! ALIENTEK ATK-Logic driver (DL16, DL16 Plus): the USB protocol of the ATK-Logic application
//! (`pv/usb`, `pv/thread`, `pv/data/analysis.cpp`) ported to Rust and nusb.

#![forbid(unsafe_code)]

mod acquisition;
mod assemble;
mod device;
mod driver;
pub mod models;
pub mod protocol;

pub use device::{
    APP_VERSION, AtkLogicDevice, MAX_SOFTWARE_DEPTH, MIN_FPGA_VERSION, MIN_SAMPLES, Settings,
    UPDATE_TOOL,
};
pub use driver::AtkLogicDriver;

/// Driver name, also recorded in capture files; devices are listed under it until opened.
pub const DRIVER_NAME: &str = "ATK-Logic";
/// Vendor of every ATK-Logic.
pub const VENDOR: &str = "ALIENTEK";
