//! ALIENTEK ATK-Logic driver (DL16, DL16 Plus): the USB protocol of the ATK-Logic application
//! (`pv/usb`, `pv/thread`, `pv/data/analysis.cpp`) ported to Rust and nusb.

#![forbid(unsafe_code)]

pub mod models;
pub mod protocol;

/// Driver name, also recorded in capture files; devices are listed under it until opened.
pub const DRIVER_NAME: &str = "ATK-Logic";
/// Vendor of every ATK-Logic.
pub const VENDOR: &str = "ALIENTEK";
