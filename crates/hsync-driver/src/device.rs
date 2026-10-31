//! Drivers recognise devices; devices are configured and run acquisitions.

use std::collections::BTreeMap;

use crossbeam_channel::Sender;
use hsync_core::{ChannelIndex, TriggerCondition};
use serde::{Deserialize, Serialize};
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
    /// USB link speed: `high`, `super` or `superPlus` (10 Gbps).
    pub usb_speed: Option<String>,
    /// A USB 3.0 device on a USB 2.0 port, where it streams far slower; DSView warns about it.
    pub slow_port: bool,
}

/// One step of how a device trades channels for sample rate in its current mode: with at most
/// `channels` channels it reaches `max_samplerate` (DSLogic and PXLogic channel modes, the
/// USB bandwidth of the ATK-Logic and SLogic16U3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RateTier {
    /// Channels it captures at most.
    pub channels: u16,
    /// Whether they must be the first `channels` (D0 upwards) rather than any of the device's.
    pub leading: bool,
    /// Whether the interface names a leading tier by its channel count ("8 ch"), as the device's
    /// maker names its channel modes, rather than by its range of channels ("0–7"). A tier that
    /// is not leading always goes by its count.
    pub named_by_count: bool,
    /// Channels it offers, D0 upwards, of which `channels` may be enabled at once: `channels`
    /// itself when leading, else the device's channels or fewer (a DSLogic's 3-channel mode
    /// takes any 3 of D0–D7).
    pub span: u16,
    /// Its fastest sample rate, Hz.
    pub max_samplerate: u64,
    /// Its sample rates, ascending, Hz.
    pub samplerates: Vec<u64>,
}

impl RateTier {
    /// Whether exactly `channels` may be enabled in this tier.
    pub fn fits(&self, channels: &[ChannelIndex]) -> bool {
        let mut seen = std::collections::BTreeSet::new();
        channels.len() <= usize::from(self.channels)
            && channels
                .iter()
                .all(|&channel| channel < self.span && seen.insert(channel))
    }
}

/// An analog channel a device can capture in place of some of its logic inputs, as an ADC
/// module on a group of a logic analyzer's inputs is read.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AnalogInput {
    /// Its channel, from [`hsync_core::ANALOG_BASE`] up.
    pub index: ChannelIndex,
    /// The logic inputs it is read from, which are not captured as logic while it is.
    pub inputs: Vec<ChannelIndex>,
    /// The volts of its lowest code.
    pub low: f32,
    /// The volts of its highest code.
    pub high: f32,
    /// Whether the next capture reads it.
    pub enabled: bool,
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
    /// Capture until stopped rather than up to the sample limit (DSView's loop mode); the
    /// host keeps only the newest samples.
    pub looping: bool,
}

/// Channel names and decoders a device's captures start with, such as those of the capture
/// the demo device replays, or those the UI remembered for the device.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CapturePreset {
    /// Names of channels, e.g. `SDA`; channels left out are named by their number.
    pub channel_names: BTreeMap<ChannelIndex, String>,
    /// Decoder stacks in DSView's `decoders` format; an empty list means none.
    #[ts(type = "unknown[] | null")]
    pub decoders: Option<serde_json::Value>,
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

    /// Whether a capture can run until stopped with the current options, keeping only its
    /// newest samples ([`CapturePlan::looping`]).
    fn can_loop(&self) -> bool {
        false
    }

    /// Samples per channel the device's own memory holds with the current options, when it
    /// captures there first (Buffer mode); longer captures need RLE compression.
    fn hardware_depth(&self) -> Option<u64> {
        None
    }

    /// Channel names and decoders for a capture that follows no other, with the current
    /// options; DSView sets up the decoders of the demo device's patterns this way.
    fn preset(&self) -> Option<CapturePreset> {
        None
    }

    /// Channels that will be captured, ascending.
    fn enabled_channels(&self) -> Vec<ChannelIndex>;

    fn set_channel_enabled(
        &mut self,
        channel: ChannelIndex,
        enabled: bool,
    ) -> Result<(), DriverError>;

    /// Enables exactly `channels` and disables the others. Devices whose mode caps how many
    /// channels it captures check the request first, so that a rejected one changes nothing.
    fn set_enabled_channels(&mut self, channels: &[ChannelIndex]) -> Result<(), DriverError> {
        let count = self.channel_count();
        if let Some(channel) = channels.iter().find(|&&channel| channel >= count) {
            return Err(DriverError::InvalidRequest(format!(
                "channel {channel} is out of range"
            )));
        }
        for channel in 0..count {
            if !channels.contains(&channel) {
                self.set_channel_enabled(channel, false)?;
            }
        }
        for &channel in channels {
            self.set_channel_enabled(channel, true)?;
        }
        Ok(())
    }

    /// Analog channels the device can capture in place of some of its logic inputs.
    fn analog_inputs(&self) -> Vec<AnalogInput> {
        Vec::new()
    }

    /// Reads analog input `index` in place of its logic inputs, or those inputs as logic again.
    /// The logic channels enabled among them come back with them.
    fn set_analog_enabled(
        &mut self,
        index: ChannelIndex,
        _enabled: bool,
    ) -> Result<(), DriverError> {
        Err(DriverError::InvalidRequest(format!(
            "no analog input {index}"
        )))
    }

    /// How fewer channels allow faster sample rates in the current mode, fewest channels
    /// first; empty when the channels do not bound the sample rate.
    fn rate_tiers(&self) -> Vec<RateTier> {
        Vec::new()
    }

    /// Index in [`Self::rate_tiers`] of the tier the device is in.
    fn rate_tier(&self) -> Option<usize> {
        None
    }

    /// Moves to rate tier `index` with exactly `channels` enabled, which must fit it. The
    /// sample rate stays where the tier allows it and is brought into the tier otherwise.
    /// By default the tier follows from the channels alone, as with a USB bandwidth limit;
    /// devices with channel modes select the mode first.
    fn select_rate_tier(
        &mut self,
        index: usize,
        channels: &[ChannelIndex],
    ) -> Result<(), DriverError> {
        let tiers = self.rate_tiers();
        let tier = tiers
            .get(index)
            .ok_or_else(|| DriverError::InvalidRequest(format!("no rate tier {index}")))?;
        if !tier.fits(channels) {
            return Err(DriverError::InvalidRequest(format!(
                "channels {channels:?} do not fit rate tier {index}"
            )));
        }
        self.set_enabled_channels(channels)
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Eight channels whose rate follows how many are on, as a USB bandwidth limit does.
    struct Bandwidth {
        enabled: Vec<ChannelIndex>,
    }

    impl Device for Bandwidth {
        fn info(&self) -> DeviceInfo {
            DeviceInfo {
                id: "test".into(),
                driver: "test".into(),
                vendor: "test".into(),
                model: "test".into(),
                firmware: None,
                usb_speed: None,
                slow_port: false,
            }
        }

        fn options(&self) -> Vec<OptionState> {
            Vec::new()
        }

        fn set_option(&mut self, _: OptionKey, _: &OptionValue) -> Result<(), DriverError> {
            Ok(())
        }

        fn channel_count(&self) -> u16 {
            8
        }

        fn enabled_channels(&self) -> Vec<ChannelIndex> {
            self.enabled.clone()
        }

        fn set_channel_enabled(
            &mut self,
            channel: ChannelIndex,
            enabled: bool,
        ) -> Result<(), DriverError> {
            self.enabled.retain(|&other| other != channel);
            if enabled {
                self.enabled.push(channel);
                self.enabled.sort_unstable();
            }
            Ok(())
        }

        fn rate_tiers(&self) -> Vec<RateTier> {
            [(2, 100), (8, 25)]
                .into_iter()
                .map(|(channels, max_samplerate)| RateTier {
                    channels,
                    leading: false,
                    named_by_count: false,
                    span: 8,
                    max_samplerate,
                    samplerates: vec![max_samplerate],
                })
                .collect()
        }

        fn start(
            &mut self,
            _: CapturePlan,
            _: Sender<Packet>,
        ) -> Result<Box<dyn Acquisition>, DriverError> {
            Err(DriverError::Unsupported)
        }
    }

    #[test]
    fn a_tier_fits_channels_within_its_count_and_span() {
        let tier = RateTier {
            channels: 3,
            leading: false,
            named_by_count: false,
            span: 8,
            max_samplerate: 1,
            samplerates: vec![1],
        };
        assert!(tier.fits(&[]));
        assert!(tier.fits(&[7, 0, 3]));
        assert!(!tier.fits(&[0, 1, 2, 3]));
        assert!(!tier.fits(&[8]));
        assert!(!tier.fits(&[1, 1]));
    }

    #[test]
    fn sets_exactly_the_channels_asked_for() {
        let mut device = Bandwidth {
            enabled: vec![0, 1, 2],
        };
        device.set_enabled_channels(&[5, 1]).expect("two channels");
        assert_eq!(device.enabled_channels(), [1, 5]);
        assert!(device.set_enabled_channels(&[1, 8]).is_err());
        assert_eq!(device.enabled_channels(), [1, 5]);
    }

    #[test]
    fn selects_a_tier_that_follows_from_the_channels() {
        let mut device = Bandwidth {
            enabled: (0..8).collect(),
        };
        device.select_rate_tier(0, &[3, 6]).expect("two channels");
        assert_eq!(device.enabled_channels(), [3, 6]);
        for (index, channels) in [(0, vec![0, 1, 2]), (2, vec![0])] {
            assert!(matches!(
                device.select_rate_tier(index, &channels),
                Err(DriverError::InvalidRequest(_))
            ));
            assert_eq!(device.enabled_channels(), [3, 6]);
        }
    }
}
