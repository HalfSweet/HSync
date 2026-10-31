//! An opened PXLogic: the open sequence (`hw_usb_open()`), its options (`config_set()`) and
//! starting captures.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam_channel::Sender;
use hsync_core::{ChannelIndex, TriggerCondition};
use hsync_driver::{
    Acquisition, CapturePlan, Choice, Device, DeviceInfo, DriverError, FirmwareDir, Label,
    OptionKey, OptionKind, OptionState, OptionValue, Packet, RateTier, TriggerCaps,
};

use crate::acquisition::{self, AcquisitionParams, TriggerMasks};
use crate::profiles::{
    ChannelMode, FIRMWARE, FIRMWARE_VERSION, FPGA_BITSTREAM, FPGA_RESET_BITSTREAM, Profile,
    UsbSpeed, find_profile,
};
use crate::protocol::{
    Control, EP_UPLOAD, channel_depth, mcu_firmware_image, regs, trigger_position, upload,
};

/// Fewest samples a capture takes.
pub const MIN_SAMPLES: u64 = 1024;
/// HSync's limit on samples per channel, as for every device (DSView's `LogicMaxSWDepth64`).
pub const MAX_SOFTWARE_DEPTH: u64 = 16 * (1 << 30);
/// Samples a capture takes at first, as on a DSLogic.
const DEFAULT_SAMPLES: u64 = 1_000_000;
/// Threshold PXView sets at first.
const DEFAULT_VTH: f64 = 2.0;
/// Range of the threshold voltage (PXView's device options).
const VTH_RANGE: std::ops::RangeInclusive<f64> = 0.0..=6.0;
/// Captures take whole groups of 64 samples (`SR_CONF_LIMIT_SAMPLES`).
const SAMPLES_ALIGN: u64 = 63;
/// Largest trigger position in Buffer mode, in percent.
const MAX_TRIGGER_PERCENT: u8 = 90;

/// What on the external trigger input fires the trigger (`enum pxlogic_extern_edge_modes`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExternalTrigger {
    /// The input is ignored.
    #[default]
    Off,
    Rise,
    High,
    Fall,
    Low,
    /// Any edge.
    Both,
}

impl ExternalTrigger {
    const ALL: [Self; 6] = [
        Self::Off,
        Self::Rise,
        Self::High,
        Self::Fall,
        Self::Low,
        Self::Both,
    ];

    /// The value of the external trigger register.
    pub fn code(self) -> u32 {
        match self {
            Self::Off => 0,
            Self::Rise => 1,
            Self::High => 2,
            Self::Fall => 3,
            Self::Low => 4,
            Self::Both => 5,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Rise => "rise",
            Self::High => "high",
            Self::Fall => "fall",
            Self::Low => "low",
            Self::Both => "both",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|trigger| trigger.id() == id)
    }
}

/// Settings that persist between captures (`struct PX_context`).
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub stream: bool,
    pub channel_mode: &'static ChannelMode,
    pub samplerate: u64,
    pub limit_samples: u64,
    pub vth: f64,
    pub filter: bool,
    pub clock_falling_edge: bool,
    pub external_trigger: ExternalTrigger,
    /// Bit `n`: channel `n` is captured.
    pub enabled: u32,
}

/// Every channel of a mode.
fn all_channels(mode: &ChannelMode) -> u32 {
    if mode.channels >= 32 {
        u32::MAX
    } else {
        (1 << mode.channels) - 1
    }
}

impl Settings {
    /// `pxlogic_dev_new()`: the profile's channel mode at its fastest rate, every channel on.
    pub fn defaults(profile: &'static Profile) -> Self {
        let mode = profile.default_mode();
        Self {
            stream: mode.stream,
            channel_mode: mode,
            samplerate: mode.max_samplerate,
            limit_samples: DEFAULT_SAMPLES,
            vth: DEFAULT_VTH,
            filter: false,
            clock_falling_edge: false,
            external_trigger: ExternalTrigger::Off,
            enabled: all_channels(mode),
        }
    }

    pub fn enabled_count(&self) -> u32 {
        self.enabled.count_ones()
    }

    /// `pxlogic_adjust_probes()` and `adjust_samplerate()`: every channel of the new mode on,
    /// the rate kept within its range.
    fn select_channel_mode(&mut self, mode: &'static ChannelMode) {
        self.channel_mode = mode;
        self.enabled = all_channels(mode);
        self.samplerate = self
            .samplerate
            .clamp(mode.min_samplerate, mode.max_samplerate);
    }

    /// Samples per channel the memory holds.
    pub fn memory_depth(&self, profile: &Profile) -> u64 {
        channel_depth(profile.memory_bits, self.enabled_count())
    }

    /// Most samples per channel a capture may request now.
    pub fn max_samples(&self, profile: &Profile) -> u64 {
        if self.stream {
            MAX_SOFTWARE_DEPTH
        } else {
            self.memory_depth(profile)
        }
    }
}

/// An opened PXLogic.
pub struct PxlogicDevice {
    ctl: Control,
    profile: &'static Profile,
    info: DeviceInfo,
    settings: Settings,
    running: Arc<AtomicBool>,
}

impl std::fmt::Debug for PxlogicDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PxlogicDevice")
            .field("model", &self.profile.model)
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl PxlogicDevice {
    /// `hw_usb_open()` once the interface is claimed: reads which model the device is, writes
    /// the MCU firmware into its flash when it runs another version (the device then restarts
    /// and fails the opening with [`DriverError::Restarting`]), then loads the FPGA.
    pub fn open(
        ctl: Control,
        scanned: &'static Profile,
        firmware: &FirmwareDir,
        id: String,
    ) -> Result<Self, DriverError> {
        let profile = match ctl.read(regs::LOGIC_MODE) {
            Ok(mode) => {
                find_profile(scanned.vid, scanned.pid, scanned.speed, mode).unwrap_or(scanned)
            }
            Err(error) => {
                log::debug!("reading the logic mode failed: {error}");
                scanned
            }
        };
        let version = ctl.read(regs::FIRMWARE_VERSION)?;
        log::debug!(
            "{}: firmware {version:#010x}, wanted {FIRMWARE_VERSION:#010x}",
            profile.model
        );
        if version != FIRMWARE_VERSION {
            let image = mcu_firmware_image(&firmware.load(FIRMWARE)?).ok_or_else(|| {
                DriverError::FirmwareIncompatible(format!("{FIRMWARE} does not fit"))
            })?;
            ctl.clear_halt(EP_UPLOAD);
            ctl.upload(upload::MCU_FIRMWARE_BASE, &image, upload::MCU_FLASH)?;
            log::info!(
                "wrote {FIRMWARE} into {} running {version:#010x}, restarting it",
                profile.model
            );
            // The reset takes the device off the bus before it may answer.
            if let Err(error) = ctl.write(regs::RESET, 0) {
                log::debug!("resetting after the firmware update: {error}");
            }
            return Err(DriverError::Restarting);
        }
        // PXView goes on when a bitstream fails to load; HSync stops, as the FPGA would not
        // capture.
        for name in [FPGA_RESET_BITSTREAM, FPGA_BITSTREAM] {
            let bitstream = firmware.load(name)?;
            ctl.clear_halt(EP_UPLOAD);
            ctl.upload(0, &bitstream, upload::FPGA)?;
        }
        log::info!("opened {} with firmware {version:#010x}", profile.model);
        let info = DeviceInfo {
            id,
            driver: crate::DRIVER_NAME.to_owned(),
            vendor: crate::VENDOR.to_owned(),
            model: profile.model.to_owned(),
            firmware: Some(format!("{version:08x}")),
            usb_speed: Some(
                match profile.speed {
                    UsbSpeed::High => "high",
                    UsbSpeed::Super => "super",
                }
                .to_owned(),
            ),
            // Every PXLogic is a USB 3.0 device.
            slow_port: profile.speed == UsbSpeed::High,
        };
        Ok(Self {
            ctl,
            profile,
            info,
            settings: Settings::defaults(profile),
            running: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    fn ensure_idle(&self) -> Result<(), DriverError> {
        if self.running.load(Ordering::Acquire) {
            return Err(DriverError::InvalidRequest("a capture is running".into()));
        }
        Ok(())
    }

    /// The channel modes of the operation mode as rate tiers, fewest channels first.
    fn tier_modes(&self) -> Vec<&'static ChannelMode> {
        let mut modes: Vec<_> = self.profile.channel_modes(self.settings.stream).collect();
        modes.sort_by_key(|mode| mode.channels);
        modes
    }

    fn channel_mode_choices(&self) -> Vec<Choice> {
        self.profile
            .channel_modes(self.settings.stream)
            .map(|mode| Choice {
                id: mode.id.to_string(),
                label: Label::new("device.channelMode.buffer")
                    .with("last", mode.channels - 1)
                    .with("rate", mode.max_samplerate),
            })
            .collect()
    }
}

fn choice(id: &str, key: &str) -> Choice {
    Choice {
        id: id.to_owned(),
        label: Label::new(key),
    }
}

fn expect_bool(key: OptionKey, value: &OptionValue) -> Result<bool, DriverError> {
    value
        .as_bool()
        .ok_or_else(|| DriverError::InvalidRequest(format!("{key:?} takes a boolean")))
}

fn expect_choice(key: OptionKey, value: &OptionValue) -> Result<&str, DriverError> {
    value
        .as_choice()
        .ok_or_else(|| DriverError::InvalidRequest(format!("{key:?} takes a choice")))
}

/// The trigger masks of simple conditions on enabled channels (`set_trigger()`).
fn trigger_masks(conditions: &[(ChannelIndex, TriggerCondition)], enabled: u32) -> TriggerMasks {
    let mut masks = TriggerMasks::default();
    for &(channel, condition) in conditions {
        let Some(bit) = 1u32.checked_shl(u32::from(channel)) else {
            continue;
        };
        if enabled & bit == 0 {
            continue;
        }
        match condition {
            TriggerCondition::None => {}
            TriggerCondition::High => masks.one |= bit,
            TriggerCondition::Low => masks.zero |= bit,
            TriggerCondition::Fall => masks.fall |= bit,
            TriggerCondition::Rise => masks.rise |= bit,
            TriggerCondition::Both => {
                masks.fall |= bit;
                masks.rise |= bit;
            }
        }
    }
    masks
}

impl Device for PxlogicDevice {
    fn info(&self) -> DeviceInfo {
        self.info.clone()
    }

    fn options(&self) -> Vec<OptionState> {
        let settings = &self.settings;
        vec![
            OptionState {
                key: OptionKey::OperationMode,
                kind: OptionKind::Choice {
                    choices: vec![
                        choice("buffer", "device.operationMode.buffer"),
                        choice("stream", "device.operationMode.stream"),
                    ],
                },
                value: OptionValue::Choice(
                    if settings.stream { "stream" } else { "buffer" }.to_owned(),
                ),
                enabled: true,
            },
            OptionState {
                key: OptionKey::ChannelMode,
                kind: OptionKind::Choice {
                    choices: self.channel_mode_choices(),
                },
                value: OptionValue::Choice(settings.channel_mode.id.to_string()),
                enabled: true,
            },
            OptionState {
                key: OptionKey::Samplerate,
                kind: OptionKind::Integers {
                    values: settings.channel_mode.samplerates(),
                },
                value: OptionValue::Integer(settings.samplerate),
                enabled: true,
            },
            OptionState {
                key: OptionKey::SampleLimit,
                kind: OptionKind::Range {
                    min: MIN_SAMPLES as f64,
                    max: settings.max_samples(self.profile) as f64,
                    step: MIN_SAMPLES as f64,
                },
                value: OptionValue::Integer(settings.limit_samples),
                enabled: true,
            },
            OptionState {
                key: OptionKey::Vth,
                kind: OptionKind::Range {
                    min: *VTH_RANGE.start(),
                    max: *VTH_RANGE.end(),
                    step: 0.1,
                },
                value: OptionValue::Number(settings.vth),
                enabled: true,
            },
            OptionState {
                key: OptionKey::Filter,
                kind: OptionKind::Bool,
                value: OptionValue::Bool(settings.filter),
                enabled: true,
            },
            OptionState {
                key: OptionKey::ClockFallingEdge,
                kind: OptionKind::Bool,
                value: OptionValue::Bool(settings.clock_falling_edge),
                enabled: true,
            },
            OptionState {
                key: OptionKey::ExternalTrigger,
                kind: OptionKind::Choice {
                    choices: ExternalTrigger::ALL
                        .into_iter()
                        .map(|trigger| {
                            choice(
                                trigger.id(),
                                &format!("device.externalTrigger.{}", trigger.id()),
                            )
                        })
                        .collect(),
                },
                value: OptionValue::Choice(settings.external_trigger.id().to_owned()),
                enabled: true,
            },
        ]
    }

    fn set_option(&mut self, key: OptionKey, value: &OptionValue) -> Result<(), DriverError> {
        self.ensure_idle()?;
        match key {
            OptionKey::OperationMode => {
                let stream = match expect_choice(key, value)? {
                    "stream" => true,
                    "buffer" => false,
                    other => {
                        return Err(DriverError::InvalidRequest(format!(
                            "unknown operation mode {other}"
                        )));
                    }
                };
                if stream != self.settings.stream {
                    let mode = self
                        .profile
                        .channel_modes(stream)
                        .next()
                        .ok_or_else(|| DriverError::InvalidRequest("no channel mode".into()))?;
                    self.settings.stream = stream;
                    self.settings.select_channel_mode(mode);
                }
            }
            OptionKey::ChannelMode => {
                let id: u8 = expect_choice(key, value)?
                    .parse()
                    .map_err(|_| DriverError::InvalidRequest("channel mode id".into()))?;
                let mode = self
                    .profile
                    .channel_modes(self.settings.stream)
                    .find(|mode| mode.id == id)
                    .ok_or_else(|| {
                        DriverError::InvalidRequest(format!("channel mode {id} is not available"))
                    })?;
                self.settings.select_channel_mode(mode);
            }
            OptionKey::Samplerate => {
                let rate = value.as_integer().ok_or_else(|| {
                    DriverError::InvalidRequest("the sample rate takes an integer".into())
                })?;
                if !self.settings.channel_mode.samplerates().contains(&rate) {
                    return Err(DriverError::InvalidRequest(format!(
                        "{rate} Hz is not available in this channel mode"
                    )));
                }
                self.settings.samplerate = rate;
            }
            OptionKey::SampleLimit => {
                let limit = value.as_integer().ok_or_else(|| {
                    DriverError::InvalidRequest("the sample limit takes an integer".into())
                })?;
                self.settings.limit_samples =
                    (limit.max(MIN_SAMPLES).saturating_add(SAMPLES_ALIGN)) & !SAMPLES_ALIGN;
            }
            OptionKey::Vth => {
                self.settings.vth = value
                    .as_number()
                    .filter(|vth| VTH_RANGE.contains(vth))
                    .ok_or_else(|| {
                        DriverError::InvalidRequest("the threshold takes 0 to 6 V".into())
                    })?;
            }
            OptionKey::Filter => self.settings.filter = expect_bool(key, value)?,
            OptionKey::ClockFallingEdge => {
                self.settings.clock_falling_edge = expect_bool(key, value)?;
            }
            OptionKey::ExternalTrigger => {
                self.settings.external_trigger =
                    ExternalTrigger::from_id(expect_choice(key, value)?).ok_or_else(|| {
                        DriverError::InvalidRequest("unknown external trigger".into())
                    })?;
            }
            other => {
                return Err(DriverError::InvalidRequest(format!(
                    "PXLogic devices have no {other:?} option"
                )));
            }
        }
        Ok(())
    }

    fn channel_count(&self) -> u16 {
        self.settings.channel_mode.channels
    }

    /// Stream mode runs on until stopped (`SR_CONF_LOOP_MODE`).
    fn can_loop(&self) -> bool {
        self.settings.stream
    }

    fn hardware_depth(&self) -> Option<u64> {
        (!self.settings.stream).then(|| self.settings.memory_depth(self.profile))
    }

    /// Simple triggers only; PXView pins the position to 1 % in Stream mode.
    fn trigger_caps(&self) -> TriggerCaps {
        TriggerCaps {
            max_position: if self.settings.stream {
                1
            } else {
                MAX_TRIGGER_PERCENT
            },
            advanced: false,
            advanced_other_mode: false,
        }
    }

    fn enabled_channels(&self) -> Vec<ChannelIndex> {
        (0..self.channel_count())
            .filter(|&channel| self.settings.enabled & (1 << channel) != 0)
            .collect()
    }

    fn set_channel_enabled(
        &mut self,
        channel: ChannelIndex,
        enabled: bool,
    ) -> Result<(), DriverError> {
        self.ensure_idle()?;
        if channel >= self.channel_count() {
            return Err(DriverError::InvalidRequest(format!(
                "channel {channel} does not exist in this mode"
            )));
        }
        if enabled {
            self.settings.enabled |= 1 << channel;
        } else {
            self.settings.enabled &= !(1 << channel);
        }
        Ok(())
    }

    fn rate_tiers(&self) -> Vec<RateTier> {
        self.tier_modes()
            .into_iter()
            .map(|mode| RateTier {
                channels: mode.channels,
                leading: true,
                named_by_count: false,
                span: mode.channels,
                max_samplerate: mode.max_samplerate,
                samplerates: mode.samplerates(),
            })
            .collect()
    }

    fn rate_tier(&self) -> Option<usize> {
        self.tier_modes()
            .iter()
            .position(|mode| mode.id == self.settings.channel_mode.id)
    }

    fn select_rate_tier(
        &mut self,
        index: usize,
        channels: &[ChannelIndex],
    ) -> Result<(), DriverError> {
        self.ensure_idle()?;
        let mode = *self
            .tier_modes()
            .get(index)
            .ok_or_else(|| DriverError::InvalidRequest(format!("no rate tier {index}")))?;
        let mut mask = 0u32;
        for &channel in channels {
            if channel >= mode.channels {
                return Err(DriverError::InvalidRequest(format!(
                    "channel {channel} does not exist in this mode"
                )));
            }
            mask |= 1 << channel;
        }
        self.settings.select_channel_mode(mode);
        self.settings.enabled = mask;
        Ok(())
    }

    fn start(
        &mut self,
        plan: CapturePlan,
        sink: Sender<Packet>,
    ) -> Result<Box<dyn Acquisition>, DriverError> {
        self.ensure_idle()?;
        let settings = &self.settings;
        if settings.enabled == 0 {
            return Err(DriverError::InvalidRequest("no channel is enabled".into()));
        }
        if plan.advanced.is_some() {
            return Err(DriverError::InvalidRequest(
                "PXLogic devices have simple triggers only".into(),
            ));
        }
        if plan.looping && !self.can_loop() {
            return Err(DriverError::InvalidRequest(
                "loop captures need Stream mode".into(),
            ));
        }
        let max = settings.max_samples(self.profile);
        if settings.limit_samples > max {
            log::warn!(
                "sample limit {} exceeds {max} in this mode",
                settings.limit_samples
            );
        }
        let limit = settings.limit_samples.min(max) & !SAMPLES_ALIGN;
        let position = plan
            .trigger_position
            .clamp(1, self.trigger_caps().max_position);
        // An instant capture ignores every trigger, the external input's too; PXView has none.
        let (triggers, external_trigger) = if plan.instant {
            (TriggerMasks::default(), ExternalTrigger::Off)
        } else {
            (
                trigger_masks(&plan.triggers, settings.enabled),
                settings.external_trigger,
            )
        };
        let params = AcquisitionParams {
            speed: self.profile.speed,
            stream: settings.stream,
            samplerate: settings.samplerate,
            limit_samples: limit,
            channels: self.enabled_channels(),
            enabled: settings.enabled,
            vth: settings.vth,
            filter: settings.filter,
            clock_falling_edge: settings.clock_falling_edge,
            external_trigger: external_trigger.code(),
            triggers,
            trigger_position: trigger_position(
                position,
                limit,
                settings.memory_depth(self.profile),
                settings.stream,
            ),
            looping: plan.looping,
        };
        acquisition::start(self.ctl.clone(), params, Arc::clone(&self.running), sink)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_conditions_of_enabled_channels() {
        let masks = trigger_masks(
            &[
                (0, TriggerCondition::Rise),
                (1, TriggerCondition::Both),
                (2, TriggerCondition::High),
                (3, TriggerCondition::Low),
                (4, TriggerCondition::Fall),
                (5, TriggerCondition::Rise),
                (40, TriggerCondition::Rise),
            ],
            0b1_1111,
        );
        assert_eq!(
            masks,
            TriggerMasks {
                zero: 0b1000,
                one: 0b100,
                rise: 0b11,
                fall: 0b1_0010,
            }
        );
    }

    #[test]
    fn switches_modes_as_pxview_does() {
        let profile = find_profile(0x16c0, 0x05dc, UsbSpeed::Super, 1).expect("16 Pro");
        let mut settings = Settings::defaults(profile);
        assert_eq!(settings.channel_mode.id, 2);
        assert_eq!(settings.samplerate, 500_000_000);
        assert_eq!(settings.enabled, 0xffff);
        settings.select_channel_mode(&crate::profiles::CHANNEL_MODES[5]);
        assert_eq!(settings.samplerate, 125_000_000);
        assert_eq!(settings.enabled, 0xffff);
        settings.select_channel_mode(&crate::profiles::CHANNEL_MODES[8]);
        assert_eq!(settings.samplerate, 125_000_000);
        assert_eq!(settings.enabled, 0b11);
    }
}
