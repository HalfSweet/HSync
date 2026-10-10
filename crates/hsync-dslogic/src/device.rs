//! An opened DSLogic: its options (DESIGN.md §4.9), the open sequence (§4.5) and starting
//! acquisitions.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam_channel::Sender;
use hsync_core::ChannelIndex;
use hsync_driver::{
    Acquisition, CapturePlan, Choice, Device, DeviceInfo, DriverError, Label, OptionKey,
    OptionKind, OptionState, OptionValue, Packet, TriggerCaps,
};

use crate::acquisition::{self, AcquisitionParams};
use crate::boot::{self, FirmwareDir, SECURITY_EEPROM_ADDR, SECURITY_STEPS};
use crate::profiles::{CHANNEL_MODES, ChannelMode, Profile, UsbSpeed, caps};
use crate::protocol::{Control, REQUIRED_FIRMWARE_MAJOR, REQUIRED_HDL_VERSION, reg};
use crate::setting::{SAMPLES_ALIGN, TriggerSetup, channel_depth};

/// Fixed input threshold level of models without an adjustable threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Threshold {
    /// 1.8/2.5/3.3 V logic.
    V33,
    /// 5 V logic.
    V50,
}

impl Threshold {
    fn id(self) -> &'static str {
        match self {
            Self::V33 => "v33",
            Self::V50 => "v50",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        match id {
            "v33" => Some(Self::V33),
            "v50" => Some(Self::V50),
            _ => None,
        }
    }
}

/// Software limit on samples per channel (DSView's `LogicMaxSWDepth64`).
pub const MAX_SOFTWARE_DEPTH: u64 = 16 * (1 << 30);
/// Fewest samples a capture takes.
pub const MIN_SAMPLES: u64 = 1024;
/// RLE stretches the hardware buffer by up to this factor.
const RLE_DEPTH_FACTOR: u64 = 1024;

/// Settings that persist between captures (`DSL_context`).
#[derive(Debug, Clone)]
pub struct Settings {
    pub stream: bool,
    pub channel_mode: &'static ChannelMode,
    pub samplerate: u64,
    pub limit_samples: u64,
    pub threshold: Threshold,
    pub vth: f64,
    pub filter: bool,
    /// Offer captures longer than the hardware buffer, relying on run-length compression.
    pub rle: bool,
    pub external_clock: bool,
    pub clock_falling_edge: bool,
    /// When stopped in buffer mode before the buffer filled, upload what was captured.
    pub buffer_upload: bool,
    /// Bit `n`: channel `n` is captured.
    pub enabled: u32,
}

impl Settings {
    /// `DSLogic_dev_new()`.
    pub fn defaults(profile: &'static Profile) -> Self {
        let mode = &CHANNEL_MODES[usize::from(profile.default_channel_mode)];
        let mut settings = Self {
            stream: mode.stream,
            channel_mode: mode,
            samplerate: profile.default_samplerate,
            limit_samples: profile.default_sample_limit,
            threshold: Threshold::V33,
            vth: 1.0,
            filter: false,
            rle: false,
            external_clock: false,
            clock_falling_edge: false,
            buffer_upload: true,
            enabled: 0,
        };
        settings.enable_valid_channels();
        settings.adjust_samplerate(profile);
        settings
    }

    /// Enables the first channels the mode can capture at once.
    fn enable_valid_channels(&mut self) {
        let valid = u32::from(self.channel_mode.valid_channels.min(32));
        self.enabled = if valid == 32 {
            u32::MAX
        } else {
            (1 << valid) - 1
        };
    }

    /// `dsl_adjust_samplerate()`: keeps the rate inside the channel mode's range.
    fn adjust_samplerate(&mut self, profile: &Profile) {
        let rates = profile.samplerates_for(self.channel_mode);
        if let (Some(&low), Some(&high)) = (rates.first(), rates.last()) {
            self.samplerate = self.samplerate.clamp(low, high);
        }
    }

    pub fn enabled_count(&self) -> u32 {
        self.enabled.count_ones()
    }

    /// Most samples per channel a capture may request now.
    pub fn max_samples(&self, profile: &Profile) -> u64 {
        if self.stream {
            MAX_SOFTWARE_DEPTH
        } else {
            let depth = channel_depth(profile, self.enabled_count().max(1));
            if self.rle {
                (depth * RLE_DEPTH_FACTOR).min(MAX_SOFTWARE_DEPTH)
            } else {
                depth
            }
        }
    }
}

/// An opened DSLogic.
pub struct DslogicDevice {
    ctl: Control,
    profile: &'static Profile,
    info: DeviceInfo,
    firmware: FirmwareDir,
    settings: Settings,
    running: Arc<AtomicBool>,
}

impl std::fmt::Debug for DslogicDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DslogicDevice")
            .field("model", &self.profile.model)
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

/// `VTH` register value for a threshold voltage.
pub fn vth_code(profile: &Profile, vth: f64) -> u8 {
    let scale = if profile.has(caps::MAX25_VTH) {
        1.0 / 2.0
    } else {
        1.5 / 2.5
    };
    (vth / 3.3 * scale * 255.0) as u8
}

impl DslogicDevice {
    /// `dsl_dev_open()` and `dev_open()`: checks the firmware, configures the FPGA when it is
    /// blank, runs the security handshake and sets the threshold and clock.
    pub fn open(
        ctl: Control,
        profile: &'static Profile,
        firmware: FirmwareDir,
        id: String,
    ) -> Result<Self, DriverError> {
        log::debug!("{}: reading the firmware version", profile.model);
        let (major, minor) = ctl.firmware_version()?;
        log::debug!("{}: firmware {major}.{minor}", profile.model);
        if major != REQUIRED_FIRMWARE_MAJOR {
            return Err(DriverError::FirmwareIncompatible(format!(
                "{major}.{minor}"
            )));
        }
        let settings = Settings::defaults(profile);
        let status = ctl.hw_status()?;
        log::debug!("{}: hardware status {status:#04x}", profile.model);
        if status & crate::protocol::status::FPGA_DONE == 0 {
            let bitstream = firmware.load(bitstream_name(profile, settings.threshold))?;
            boot::configure_fpga(&ctl, &bitstream)?;
        } else {
            boot::clear_control(&ctl)?;
            let hdl = ctl.hdl_version()?;
            log::debug!("{}: FPGA logic version {hdl:#04x}", profile.model);
            if hdl != REQUIRED_HDL_VERSION {
                return Err(DriverError::FirmwareIncompatible(format!("HDL {hdl:#04x}")));
            }
        }

        boot::clear_control(&ctl)?;
        let eeprom = ctl.read_nvm(SECURITY_EEPROM_ADDR, (SECURITY_STEPS * 2) as u8)?;
        if profile.has(caps::SECURITY) {
            let mut words = [0u16; SECURITY_STEPS];
            for (word, bytes) in words.iter_mut().zip(eeprom.as_chunks::<2>().0) {
                *word = u16::from_le_bytes(*bytes);
            }
            let passed = boot::security_check(&ctl, &words)?;
            log::info!(
                "{} security check {}",
                profile.model,
                if passed { "passed" } else { "failed" }
            );
        }

        ctl.write_reg(reg::VTH, vth_code(profile, settings.vth))?;
        if profile.has(caps::ADF4360) {
            boot::init_clock(&ctl)?;
        }

        let info = DeviceInfo {
            id,
            driver: crate::DRIVER_NAME.to_owned(),
            vendor: crate::VENDOR.to_owned(),
            model: profile.model.to_owned(),
            firmware: Some(format!("{major}.{minor}")),
            usb_speed: Some(
                match profile.speed {
                    UsbSpeed::High => "high",
                    UsbSpeed::Super => "super",
                }
                .to_owned(),
            ),
            slow_port: profile.on_slow_port(),
        };
        log::info!("opened {} with firmware {major}.{minor}", profile.model);
        Ok(Self {
            ctl,
            profile,
            info,
            firmware,
            settings,
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

    fn select_operation_mode(&mut self, stream: bool) -> Result<(), DriverError> {
        if !stream && !self.profile.has(caps::BUF) {
            return Err(DriverError::InvalidRequest(format!(
                "{} has no buffer mode",
                self.profile.model
            )));
        }
        if stream == self.settings.stream {
            return Ok(());
        }
        let mode = self
            .profile
            .channel_modes(stream)
            .next()
            .ok_or_else(|| DriverError::InvalidRequest("no channel mode".into()))?;
        self.settings.stream = stream;
        self.select_channel_mode(mode);
        Ok(())
    }

    fn select_channel_mode(&mut self, mode: &'static ChannelMode) {
        self.settings.channel_mode = mode;
        self.settings.enable_valid_channels();
        self.settings.adjust_samplerate(self.profile);
    }

    fn set_threshold(&mut self, threshold: Threshold) -> Result<(), DriverError> {
        if threshold == self.settings.threshold {
            return Ok(());
        }
        let bitstream = self
            .firmware
            .load(bitstream_name(self.profile, threshold))?;
        boot::configure_fpga(&self.ctl, &bitstream)?;
        self.settings.threshold = threshold;
        Ok(())
    }

    fn channel_mode_choices(&self) -> Vec<Choice> {
        self.profile
            .channel_modes(self.settings.stream)
            .map(|mode| Choice {
                id: mode.id.to_string(),
                label: channel_mode_label(mode),
            })
            .collect()
    }
}

/// The bitstream for a threshold level.
fn bitstream_name(profile: &Profile, threshold: Threshold) -> &'static str {
    match threshold {
        Threshold::V33 => profile.fpga_bit33,
        Threshold::V50 => profile.fpga_bit50,
    }
}

fn channel_mode_label(mode: &ChannelMode) -> Label {
    if mode.stream {
        Label::new("device.channelMode.stream")
            .with("count", mode.valid_channels)
            .with("rate", mode.max_samplerate)
    } else {
        Label::new("device.channelMode.buffer")
            .with("last", mode.channels - 1)
            .with("rate", mode.max_samplerate)
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

impl Device for DslogicDevice {
    fn info(&self) -> DeviceInfo {
        self.info.clone()
    }

    fn options(&self) -> Vec<OptionState> {
        let settings = &self.settings;
        let buffer = !settings.stream;
        let mut options = vec![OptionState {
            key: OptionKey::OperationMode,
            kind: OptionKind::Choice {
                choices: if self.profile.has(caps::BUF) {
                    vec![
                        choice("buffer", "device.operationMode.buffer"),
                        choice("stream", "device.operationMode.stream"),
                    ]
                } else {
                    vec![choice("stream", "device.operationMode.stream")]
                },
            },
            value: OptionValue::Choice(if buffer { "buffer" } else { "stream" }.to_owned()),
            enabled: self.profile.has(caps::BUF),
        }];
        options.push(OptionState {
            key: OptionKey::ChannelMode,
            kind: OptionKind::Choice {
                choices: self.channel_mode_choices(),
            },
            value: OptionValue::Choice(settings.channel_mode.id.to_string()),
            enabled: true,
        });
        options.push(OptionState {
            key: OptionKey::Samplerate,
            kind: OptionKind::Integers {
                values: self.profile.samplerates_for(settings.channel_mode),
            },
            value: OptionValue::Integer(settings.samplerate),
            enabled: !settings.external_clock,
        });
        options.push(OptionState {
            key: OptionKey::SampleLimit,
            kind: OptionKind::Range {
                min: MIN_SAMPLES as f64,
                max: settings.max_samples(self.profile) as f64,
                step: (SAMPLES_ALIGN + 1) as f64,
            },
            value: OptionValue::Integer(settings.limit_samples),
            enabled: true,
        });
        if self.profile.has(caps::VTH) {
            options.push(OptionState {
                key: OptionKey::Vth,
                kind: OptionKind::Range {
                    min: 0.0,
                    max: 5.0,
                    step: 0.1,
                },
                value: OptionValue::Number(settings.vth),
                enabled: true,
            });
        } else {
            options.push(OptionState {
                key: OptionKey::Threshold,
                kind: OptionKind::Choice {
                    choices: vec![
                        choice("v33", "device.threshold.v33"),
                        choice("v50", "device.threshold.v50"),
                    ],
                },
                value: OptionValue::Choice(settings.threshold.id().to_owned()),
                enabled: true,
            });
        }
        options.extend([
            OptionState {
                key: OptionKey::Filter,
                kind: OptionKind::Bool,
                value: OptionValue::Bool(settings.filter),
                enabled: true,
            },
            OptionState {
                key: OptionKey::Rle,
                kind: OptionKind::Bool,
                value: OptionValue::Bool(settings.rle),
                enabled: buffer,
            },
            OptionState {
                key: OptionKey::ExternalClock,
                kind: OptionKind::Bool,
                value: OptionValue::Bool(settings.external_clock),
                enabled: true,
            },
            OptionState {
                key: OptionKey::ClockFallingEdge,
                kind: OptionKind::Bool,
                value: OptionValue::Bool(settings.clock_falling_edge),
                enabled: settings.external_clock,
            },
            OptionState {
                key: OptionKey::BufferOption,
                kind: OptionKind::Choice {
                    choices: vec![
                        choice("stop", "device.bufferOption.stop"),
                        choice("upload", "device.bufferOption.upload"),
                    ],
                },
                value: OptionValue::Choice(
                    if settings.buffer_upload {
                        "upload"
                    } else {
                        "stop"
                    }
                    .to_owned(),
                ),
                enabled: buffer,
            },
        ]);
        options
    }

    fn set_option(&mut self, key: OptionKey, value: &OptionValue) -> Result<(), DriverError> {
        self.ensure_idle()?;
        match key {
            OptionKey::OperationMode => match expect_choice(key, value)? {
                "buffer" => self.select_operation_mode(false)?,
                "stream" => self.select_operation_mode(true)?,
                other => {
                    return Err(DriverError::InvalidRequest(format!(
                        "unknown operation mode {other}"
                    )));
                }
            },
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
                self.select_channel_mode(mode);
            }
            OptionKey::Samplerate => {
                let rate = value.as_integer().ok_or_else(|| {
                    DriverError::InvalidRequest("the sample rate takes an integer".into())
                })?;
                if !self
                    .profile
                    .samplerates_for(self.settings.channel_mode)
                    .contains(&rate)
                {
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
                self.settings.limit_samples = limit.max(MIN_SAMPLES);
            }
            OptionKey::Threshold => {
                let threshold = Threshold::from_id(expect_choice(key, value)?)
                    .ok_or_else(|| DriverError::InvalidRequest("unknown threshold".into()))?;
                self.set_threshold(threshold)?;
            }
            OptionKey::Vth => {
                let vth = value
                    .as_number()
                    .filter(|vth| (0.0..=5.0).contains(vth))
                    .ok_or_else(|| {
                        DriverError::InvalidRequest("the threshold takes 0 to 5 V".into())
                    })?;
                self.ctl.write_reg(reg::VTH, vth_code(self.profile, vth))?;
                self.settings.vth = vth;
            }
            OptionKey::Filter => self.settings.filter = expect_bool(key, value)?,
            OptionKey::Rle => self.settings.rle = expect_bool(key, value)?,
            OptionKey::ExternalClock => self.settings.external_clock = expect_bool(key, value)?,
            OptionKey::ClockFallingEdge => {
                self.settings.clock_falling_edge = expect_bool(key, value)?;
            }
            OptionKey::BufferOption => {
                self.settings.buffer_upload = match expect_choice(key, value)? {
                    "upload" => true,
                    "stop" => false,
                    other => {
                        return Err(DriverError::InvalidRequest(format!(
                            "unknown buffer option {other}"
                        )));
                    }
                };
            }
            OptionKey::Pattern => {
                return Err(DriverError::InvalidRequest(
                    "DSLogic devices have no pattern option".into(),
                ));
            }
        }
        Ok(())
    }

    fn channel_count(&self) -> u16 {
        self.settings.channel_mode.channels
    }

    /// DSView only loops in Stream mode (`SigSession::set_device_mode` falls back to single
    /// captures otherwise): Buffer mode captures into the device's memory, which cannot run on.
    fn can_loop(&self) -> bool {
        self.settings.stream
    }

    /// DSView marks the durations beyond this depth "(RLE)" (`SamplingBar::update_sample_count_selector`).
    fn hardware_depth(&self) -> Option<u64> {
        (!self.settings.stream)
            .then(|| channel_depth(self.profile, self.settings.enabled_count().max(1)))
    }

    /// DSView's trigger dock limits (`TriggerDock::device_updated`): in Stream mode only simple
    /// triggers at 1 %; in Buffer mode up to 90 % of the capture, less when the capture is longer
    /// than the hardware holds, and multi-stage and serial triggers.
    fn trigger_caps(&self) -> TriggerCaps {
        if self.settings.stream {
            return TriggerCaps {
                max_position: 1,
                advanced: false,
            };
        }
        let depth = channel_depth(self.profile, self.settings.enabled_count());
        let limit = self.settings.limit_samples.max(1);
        // DSView divides in integers before its ceil(), so this rounds down.
        let max_position = if depth >= limit {
            90
        } else {
            (depth * 90 / limit).clamp(1, 90) as u8
        };
        TriggerCaps {
            max_position,
            advanced: true,
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
        let bit = 1u32 << channel;
        if enabled {
            if self.settings.enabled & bit == 0
                && self.settings.enabled_count()
                    >= u32::from(self.settings.channel_mode.valid_channels)
            {
                return Err(DriverError::InvalidRequest(format!(
                    "this channel mode captures at most {} channels",
                    self.settings.channel_mode.valid_channels
                )));
            }
            self.settings.enabled |= bit;
        } else {
            self.settings.enabled &= !bit;
        }
        Ok(())
    }

    fn start(
        &mut self,
        plan: CapturePlan,
        sink: Sender<Packet>,
    ) -> Result<Box<dyn Acquisition>, DriverError> {
        self.ensure_idle()?;
        if self.settings.enabled_count() == 0 {
            return Err(DriverError::InvalidRequest("no channel is enabled".into()));
        }
        let max = self.settings.max_samples(self.profile);
        let mut limit = self.settings.limit_samples.min(max);
        if limit < self.settings.limit_samples {
            log::warn!(
                "sample limit {} exceeds {max} in this mode",
                self.settings.limit_samples
            );
        }
        limit = limit.max(MIN_SAMPLES);
        let depth = channel_depth(self.profile, self.settings.enabled_count());
        let caps = self.trigger_caps();
        if let Some(advanced) = &plan.advanced {
            if !caps.advanced {
                return Err(DriverError::InvalidRequest(
                    "multi-stage and serial triggers need Buffer mode".into(),
                ));
            }
            advanced
                .check(self.channel_count())
                .map_err(DriverError::InvalidRequest)?;
        }
        if plan.looping && !self.can_loop() {
            return Err(DriverError::InvalidRequest(
                "loop captures need Stream mode".into(),
            ));
        }
        // The trigger dock's slider keeps the position within 1 % and the limit.
        let position = plan.trigger_position.clamp(1, caps.max_position);
        let trigger = match &plan.advanced {
            Some(advanced) if !plan.instant => TriggerSetup::advanced(advanced, position),
            _ => TriggerSetup::simple(&plan.triggers, position, plan.instant),
        };
        let params = AcquisitionParams {
            profile: self.profile,
            mode: self.settings.channel_mode,
            samplerate: self.settings.samplerate,
            limit_samples: limit,
            enabled: self.settings.enabled,
            filter: self.settings.filter,
            external_clock: self.settings.external_clock,
            clock_falling_edge: self.settings.clock_falling_edge,
            rle: !self.settings.stream && self.settings.rle && limit > depth,
            instant: plan.instant,
            looping: plan.looping,
            buffer_upload: self.settings.buffer_upload,
            trigger,
        };
        acquisition::start(self.ctl.clone(), params, Arc::clone(&self.running), sink)
    }
}
