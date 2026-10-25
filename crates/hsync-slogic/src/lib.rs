// SPDX-License-Identifier: GPL-3.0-or-later
//! Sipeed SLogic16U3 and SLogic32U3 driver, using the official Sipeed libsigrok
//! `slogic-dev` protocol through Rust and nusb. No libsigrok code is linked at runtime.

#![forbid(unsafe_code)]

mod acquisition;
mod assemble;
mod device;
mod driver;
pub mod model;
pub mod protocol;
mod stream;

pub use device::{Settings, SlogicDevice};
pub use driver::SlogicDriver;
pub use model::{Model, SLOGIC16U3, SLOGIC32U3, UsbSpeed};

/// Driver name recorded in capture files.
pub const DRIVER_NAME: &str = "SLogic";
/// Manufacturer of the supported analyzers.
pub const VENDOR: &str = "Sipeed";
