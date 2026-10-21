// SPDX-License-Identifier: GPL-3.0-or-later
//! Sipeed SLogic16U3 driver, using the official Sipeed libsigrok `slogic-dev`
//! protocol through Rust and nusb. No libsigrok code is linked at runtime.

#![forbid(unsafe_code)]

mod acquisition;
mod assemble;
mod device;
mod driver;
pub mod protocol;
mod stream;

pub use device::{Settings, SlogicDevice, UsbSpeed};
pub use driver::SlogicDriver;

/// Driver name recorded in capture files.
pub const DRIVER_NAME: &str = "SLogic";
/// Manufacturer of the supported analyzer.
pub const VENDOR: &str = "Sipeed";
