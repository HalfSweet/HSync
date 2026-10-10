//! Devices and drivers of HSync (DESIGN.md §4): the traits drivers implement, the option model
//! the UI renders, the packets acquisitions send, and USB access through nusb.

#![forbid(unsafe_code)]

mod device;
mod error;
mod manager;
#[cfg(feature = "mock")]
pub mod mock;
mod options;
mod packet;
pub mod usb;

pub use device::{
    Acquisition, Candidate, CandidateStatus, CapturePlan, Device, DeviceInfo, DeviceSummary, Driver,
};
pub use error::DriverError;
pub use manager::{DeviceManager, WatchHandle};
pub use options::{Choice, Label, OptionKey, OptionKind, OptionState, OptionValue};
pub use packet::{EndReason, Packet};
