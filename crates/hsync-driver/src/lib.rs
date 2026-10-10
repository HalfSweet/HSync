//! Devices and drivers of HSync: the traits drivers implement, the option model
//! the UI renders, the packets acquisitions send, and USB access through nusb.

#![forbid(unsafe_code)]

mod device;
mod error;
mod manager;
#[cfg(feature = "mock")]
pub mod mock;
mod options;
mod packet;
mod trigger;
pub mod usb;

pub use device::{
    Acquisition, Candidate, CandidateStatus, CapturePlan, CapturePreset, Device, DeviceInfo,
    DeviceSummary, Driver,
};
pub use error::DriverError;
pub use manager::{DeviceManager, WatchHandle};
pub use options::{Choice, Label, OptionKey, OptionKind, OptionState, OptionValue};
pub use packet::{EndReason, Packet};
pub use trigger::{
    AdvancedTrigger, MAX_SERIAL_BITS, MAX_TRIGGER_STAGES, SerialTriggerSpec, TriggerCaps,
    TriggerPattern, TriggerStageSpec,
};
