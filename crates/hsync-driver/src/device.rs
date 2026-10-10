//! Drivers recognise devices; devices are configured and run acquisitions (DESIGN.md §4.1).

use crossbeam_channel::Sender;
use hsync_core::{ChannelIndex, TriggerCondition};
use serde::Serialize;
use ts_rs::TS;

use crate::error::DriverError;
use crate::options::{OptionKey, OptionState, OptionValue};
use crate::packet::Packet;
use crate::trigger::{AdvancedTrigger, TriggerCaps};

/// Whether a device found by a scan can be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CandidateStatus {
    /// Ready to open.
    Ready,
    /// Needs its firmware, which the driver uploads on open; it then reconnects.
    NeedsFirmware,
    /// Connected below USB 2.0 high speed, which the device cannot work with.
    SpeedTooLow,
    /// Recognised but not supported by this version (e.g. DSCope oscilloscopes).
    Unsupported,
}

/// A device found by a scan.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// Stable while the device stays connected: the driver name plus its USB location.
    pub id: String,
    pub driver: &'static str,
    pub vendor: String,
    pub model: String,
    pub status: CandidateStatus,
    /// The USB device, for USB drivers.
    pub usb: Option<nusb::DeviceInfo>,
}

impl Candidate {
    /// What the UI lists.
    pub fn summary(&self) -> DeviceSummary {
        DeviceSummary {
            id: self.id.clone(),
            driver: self.driver.to_owned(),
            vendor: self.vendor.clone(),
            model: self.model.clone(),
            status: self.status,
        }
    }
}

/// A device in the device list of the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct DeviceSummary {
    pub id: String,
    pub driver: String,
    pub vendor: String,
    pub model: String,
    pub status: CandidateStatus,
}

/// Facts about an open device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeviceInfo {
    pub id: String,
    pub driver: String,
    pub vendor: String,
    pub model: String,
    /// Firmware version, e.g. `2.0`.
    pub firmware: Option<String>,
    /// USB link speed: `high` or `super`.
    pub usb_speed: Option<String>,
}

/// One kind of device.
pub trait Driver: Send + Sync {
    /// Short name, also recorded in capture files (`DSLogic`, `virtual-demo`).
    fn name(&self) -> &'static str;

    /// Recognises a USB device of this driver.
    fn identify(&self, _usb: &nusb::DeviceInfo) -> Option<Candidate> {
        None
    }

    /// Devices that are not on USB, such as the demo device.
    fn virtual_devices(&self) -> Vec<Candidate> {
        Vec::new()
    }

    /// Opens a device. A device that first needs firmware gets it and fails with
    /// [`DriverError::Restarting`]; it reconnects under a new address shortly after.
    fn open(&self, candidate: &Candidate) -> Result<Box<dyn Device>, DriverError>;
}

/// What a capture asks of the device beyond its options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapturePlan {
    /// Simple trigger: all these conditions must hold at the same sample.
    pub triggers: Vec<(ChannelIndex, TriggerCondition)>,
    /// Share of the capture before the trigger, in percent.
    pub trigger_position: u8,
    /// Start capturing at once and ignore the trigger.
    pub instant: bool,
    /// A multi-stage or serial trigger in place of `triggers`.
    pub advanced: Option<AdvancedTrigger>,
}

/// An open device.
pub trait Device: Send {
    fn info(&self) -> DeviceInfo;

    /// Every option with its current value and allowed values.
    fn options(&self) -> Vec<OptionState>;

    /// Changes an option. Some changes alter others (a channel mode limits the sample rate).
    fn set_option(&mut self, key: OptionKey, value: &OptionValue) -> Result<(), DriverError>;

    /// Number of channels in the current mode.
    fn channel_count(&self) -> u16;

    /// What the trigger can do with the current options.
    fn trigger_caps(&self) -> TriggerCaps {
        TriggerCaps::default()
    }

    /// Channels that will be captured, ascending.
    fn enabled_channels(&self) -> Vec<ChannelIndex>;

    fn set_channel_enabled(
        &mut self,
        channel: ChannelIndex,
        enabled: bool,
    ) -> Result<(), DriverError>;

    /// Starts an acquisition that reports through `sink` until it sends [`Packet::End`].
    fn start(
        &mut self,
        plan: CapturePlan,
        sink: Sender<Packet>,
    ) -> Result<Box<dyn Acquisition>, DriverError>;
}

/// A running acquisition.
pub trait Acquisition: Send {
    /// Asks the acquisition to stop; it still sends [`Packet::End`].
    fn stop(&self);

    /// Waits until the acquisition has ended.
    fn wait(self: Box<Self>);
}
