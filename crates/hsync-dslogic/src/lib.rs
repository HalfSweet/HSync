//! DreamSourceLab DSLogic driver: libsigrok4DSL's `hardware/DSL` ported to Rust and nusb
//! (DESIGN.md §4.2–§4.9).

#![forbid(unsafe_code)]

pub mod boot;
pub mod profiles;
pub mod protocol;
pub mod setting;

pub use boot::FirmwareDir;
