//! An opened ATK-Logic: the connect sequence (`ConnectDevice::CheckDeviceCreanInfo()`), its
//! options, and starting captures.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam_channel::Sender;
use hsync_core::ChannelIndex;
use hsync_driver::{
    Acquisition, CapturePlan, Choice, Device, DeviceInfo, DriverError, Label, OptionKey,
    OptionKind, OptionState, OptionValue, Packet, RateTier, TriggerCaps,
};

use crate::acquisition::{self, AcquisitionParams};
use crate::models::{self, CHANNELS, RLE_FACTOR};
use crate::protocol::{self, Link, MCU_PACKET, McuVersion, cmd, frame, mcu, pwm};

/// Fewest samples a capture takes.
pub const MIN_SAMPLES: u64 = 1024;
/// HSync's limit on samples per channel, as for every device (DSView's `LogicMaxSWDepth64`).
pub const MAX_SOFTWARE_DEPTH: u64 = 16 * (1 << 30);
/// Oldest FPGA logic the protocol works with (`FPGA_MIN_VERSION_NUM`).
pub const MIN_FPGA_VERSION: u32 = 115;
/// The ATK-Logic release this port follows (`APP_VERSION_NUM`, 1.0.6.6); a device that asks for
/// a newer one may speak a newer protocol.
pub const APP_VERSION: u32 = 1066;
/// The application that updates the firmware.
pub const UPDATE_TOOL: &str = "ATK-Logic";

/// Connection attempts before giving up (`maxCount` plus the first).
const ATTEMPTS: u32 = 6;
/// Reads that may find the MCU in a bad state before its firmware is deemed broken; ATK-Logic
/// then forces the bootloader to update it.
const BAD_READS: u32 = 120;
/// Reads after which a busy device is sent a device info request to settle it.
const BUSY_NUDGE: u32 = 100;
/// Reads after which the pauses between them start.
const BUSY_SLOW: u32 = 900;
/// Reads after which a busy device is left as it is.
const BUSY_READS: u32 = 1100;
/// Reads that may find data after a reset before the connection goes on regardless.
const RESET_DRAIN: usize = 2000;
/// Samples ATK-Logic captures at first: 500 ms at 20 MHz, rounded up to whole KiB as the
/// acquisition settings do.
const DEFAULT_SAMPLES: u64 = 10_000_384;
/// Threshold ATK-Logic sets at first, for 3.3 V CMOS.
const DEFAULT_VTH: f64 = 1.6;
/// Range of the threshold voltage.
const VTH_RANGE: std::ops::RangeInclusive<f64> = -5.0..=5.0;

/// Settings that persist between captures (`SSettings.settingData`).
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub stream: bool,
    pub samplerate: u64,
    pub limit_samples: u64,
    /// Threshold voltage.
    pub vth: f64,
    /// Bit `n`: channel `n` is captured.
    pub enabled: u16,
}

impl Settings {
    /// What ATK-Logic starts with: Stream mode, every channel at the fastest rate it allows.
    pub fn defaults(plus: bool) -> Self {
        let mut settings = Self {
            stream: true,
            samplerate: models::SAMPLERATES[9],
            limit_samples: DEFAULT_SAMPLES,
            vth: DEFAULT_VTH,
            enabled: u16::MAX,
        };
        settings.adjust_samplerate(plus);
        settings
    }

    pub fn enabled_count(&self) -> u32 {
        self.enabled.count_ones()
    }

    /// The rates available with the current mode and channels.
    pub fn samplerates(&self, plus: bool) -> Vec<u64> {
        models::samplerates(plus, self.stream, self.enabled_count())
    }

    /// Keeps the rate available: one beyond the list falls back to its fastest, as ATK-Logic's
    /// rate list keeps its position or ends up on its last entry.
    fn adjust_samplerate(&mut self, plus: bool) {
        let rates = self.samplerates(plus);
        if !rates.contains(&self.samplerate)
            && let Some(&fastest) = rates.last()
        {
            self.samplerate = fastest;
        }
    }

    /// How the rates narrow as channels are added: one tier per run of channel counts with
    /// the same rate list, named after the most channels of the run.
    pub fn rate_tiers(&self, plus: bool) -> Vec<RateTier> {
        let mut tiers: Vec<RateTier> = Vec::new();
        for channels in 1..=CHANNELS {
            let samplerates = models::samplerates(plus, self.stream, u32::from(channels));
            match tiers.last_mut() {
                Some(tier) if tier.samplerates == samplerates => tier.channels = channels,
                _ => tiers.push(RateTier {
                    channels,
                    leading: false,
                    named_by_count: false,
                    span: CHANNELS,
                    max_samplerate: samplerates.last().copied().unwrap_or(0),
                    samplerates,
                }),
            }
        }
        tiers
    }

    /// Samples per channel the device's memory holds (Buffer mode).
    pub fn memory_depth(&self, plus: bool) -> u64 {
        models::memory_depth(plus, self.enabled_count())
    }

    /// Most samples per channel a capture may request now: in Buffer mode, the memory stretched
    /// by RLE compression.
    pub fn max_samples(&self, plus: bool) -> u64 {
        if self.stream {
            MAX_SOFTWARE_DEPTH
        } else {
            (self.memory_depth(plus) * RLE_FACTOR).min(MAX_SOFTWARE_DEPTH)
        }
    }
}

/// An opened ATK-Logic.
pub struct AtkLogicDevice {
    link: Link,
    info: DeviceInfo,
    /// A Plus model (`level` 1).
    plus: bool,
    settings: Settings,
    running: Arc<AtomicBool>,
}

impl std::fmt::Debug for AtkLogicDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AtkLogicDevice")
            .field("model", &self.info.model)
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

/// What the connect sequence learned.
struct Identity {
    mcu: McuVersion,
    info: protocol::DeviceInfo,
}

impl AtkLogicDevice {
    /// Connects to the device as ATK-Logic does (`ConnectDevice::CheckDeviceCreanInfo()`): asks
    /// the MCU for its version, resets the FPGA, asks the FPGA what the device is, then
    /// switches the waveform outputs off and lets the FPGA sleep until a capture.
    pub fn open(link: Link, id: String) -> Result<Self, DriverError> {
        link.pause(link.timing().settle);
        let identity = connect(&link)?;
        let Identity { mcu, info } = identity;
        // `CloseAllPWM()`, then `SetResetState(0)` and a read of whatever it answers.
        link.command(cmd::PWM, &[pwm::PWM1_OFF])?;
        link.command(cmd::PWM, &[pwm::PWM0_OFF])?;
        link.pause(link.timing().gap);
        link.mcu(mcu::RESET_STATE, &[0])?;
        link.read(MCU_PACKET)?;

        let plus = mcu.level == 1;
        let model = if plus {
            format!("{} Plus", info.name)
        } else {
            info.name.clone()
        };
        log::info!(
            "opened {model}: MCU firmware {}.{}, FPGA {}, hardware {}, USB {}",
            mcu.major,
            mcu.minor,
            info.fpga_version(),
            mcu.hardware,
            info.usb
        );
        let info = DeviceInfo {
            id,
            driver: crate::DRIVER_NAME.to_owned(),
            vendor: crate::VENDOR.to_owned(),
            model,
            firmware: Some(format!(
                "{}.{} / {}.{:02}",
                mcu.major, mcu.minor, info.fpga.0, info.fpga.1
            )),
            usb_speed: match info.usb {
                3 => Some("super".to_owned()),
                2 => Some("high".to_owned()),
                _ => None,
            },
            slow_port: false,
        };
        Ok(Self {
            link,
            info,
            plus,
            settings: Settings::defaults(plus),
            running: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Whether the device is a Plus model.
    pub fn is_plus(&self) -> bool {
        self.plus
    }

    fn ensure_idle(&self) -> Result<(), DriverError> {
        if self.running.load(Ordering::Acquire) {
            return Err(DriverError::InvalidRequest("a capture is running".into()));
        }
        Ok(())
    }
}

/// The attempts of the connect sequence.
fn connect(link: &Link) -> Result<Identity, DriverError> {
    let mut mcu: Option<McuVersion> = None;
    let mut info: Option<protocol::DeviceInfo> = None;
    for attempt in 1..=ATTEMPTS {
        link.mcu(mcu::VERSION, &[])?;
        settle_mcu(link)?;
        if mcu.is_none() {
            link.mcu(mcu::VERSION, &[])?;
            match McuVersion::parse(&link.read(MCU_PACKET)?) {
                Some(version) if version.bootloader => {
                    return Err(DriverError::FirmwareUpdate(UPDATE_TOOL));
                }
                Some(version) => {
                    log::debug!(
                        "MCU firmware {}.{}, hardware {}, level {}",
                        version.major,
                        version.minor,
                        version.hardware,
                        version.level
                    );
                    link.mcu(mcu::RESET_STATE, &[0])?;
                    link.mcu(mcu::RESET_STATE, &[1])?;
                    link.drain(RESET_DRAIN)?;
                    mcu = Some(version);
                }
                None => log::debug!("attempt {attempt}: no MCU version"),
            }
        }
        if mcu.is_some() {
            link.command(cmd::DEVICE_INFO, &[])?;
            if let Some(found) = read_info(link)? {
                check_info(&found)?;
                info = Some(found);
            }
        }
        if let (Some(mcu), Some(info)) = (mcu, &info)
            && mcu.hardware != 0
            && !info.name.is_empty()
            && matches!(info.usb, 2 | 3)
        {
            return Ok(Identity {
                mcu,
                info: info.clone(),
            });
        }
        log::debug!("attempt {attempt} to connect did not finish");
        link.pause(link.timing().settle);
    }
    Err(DriverError::Timeout("connecting to the device"))
}

/// Reads what the device still sends after asking for the MCU version, giving up on a busy
/// device after a while, and telling a broken MCU firmware by answers that start and end with
/// bytes above 200.
fn settle_mcu(link: &Link) -> Result<(), DriverError> {
    let (mut bad, mut busy) = (0, 0);
    loop {
        let data = link.read(MCU_PACKET)?;
        let Some(&first) = data.first() else {
            return Ok(());
        };
        let last = data.get(MCU_PACKET - 1).copied().unwrap_or(0);
        if first > 200 && last > 200 {
            bad += 1;
            link.pause(link.timing().poll);
            if bad > BAD_READS {
                // ATK-Logic forces the bootloader here to reinstall the firmware, which only
                // ATK-Logic can do; HSync leaves the device as it is and says so.
                return Err(DriverError::FirmwareUpdate(UPDATE_TOOL));
            }
        } else {
            busy += 1;
            if busy == BUSY_NUDGE {
                link.command(cmd::DEVICE_INFO, &[])?;
            }
            if busy > BUSY_SLOW {
                link.pause(link.timing().poll);
            }
            if busy > BUSY_READS {
                return Ok(());
            }
        }
    }
}

/// The device information in the answer to [`cmd::DEVICE_INFO`], if it came.
fn read_info(link: &Link) -> Result<Option<protocol::DeviceInfo>, DriverError> {
    let data = link.read(protocol::BLOCK)?;
    let mut reader = protocol::FrameReader::new();
    reader.push(&data);
    while let Some(found) = reader.next_frame() {
        if found.kind == frame::INFO {
            return Ok(protocol::DeviceInfo::parse(&found.payload));
        }
    }
    Ok(None)
}

/// Refuses devices the protocol cannot drive.
fn check_info(info: &protocol::DeviceInfo) -> Result<(), DriverError> {
    if !info.ok {
        return Err(DriverError::Fault);
    }
    if info.fpga_version() < MIN_FPGA_VERSION {
        return Err(DriverError::FirmwareUpdate(UPDATE_TOOL));
    }
    if info.min_app_version() > APP_VERSION {
        return Err(DriverError::FirmwareIncompatible(format!(
            "FPGA {}.{:02}",
            info.fpga.0, info.fpga.1
        )));
    }
    Ok(())
}

impl Drop for AtkLogicDevice {
    /// Lets go of the device as ATK-Logic does when it closes (`FramelessWindow::windowClose()`):
    /// wakes the FPGA, switches the waveform outputs off, says goodbye and lets it sleep.
    fn drop(&mut self) {
        if self.running.load(Ordering::Acquire) {
            return;
        }
        let link = &self.link;
        let result = link.mcu(mcu::RESET_STATE, &[1]).and_then(|()| {
            link.pause(link.timing().gap);
            link.command(cmd::PWM, &[pwm::PWM1_OFF])?;
            link.command(cmd::PWM, &[pwm::PWM0_OFF])?;
            link.command(cmd::EXIT, &[])?;
            link.pause(link.timing().gap);
            link.mcu(mcu::RESET_STATE, &[0])
        });
        if let Err(error) = result {
            log::debug!("closing {} failed: {error}", self.info.model);
        }
    }
}

fn expect_choice(key: OptionKey, value: &OptionValue) -> Result<&str, DriverError> {
    value
        .as_choice()
        .ok_or_else(|| DriverError::InvalidRequest(format!("{key:?} takes a choice")))
}

fn choice(id: &str, key: &str) -> Choice {
    Choice {
        id: id.to_owned(),
        label: Label::new(key),
    }
}

impl Device for AtkLogicDevice {
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
                key: OptionKey::Samplerate,
                kind: OptionKind::Integers {
                    values: settings.samplerates(self.plus),
                },
                value: OptionValue::Integer(settings.samplerate),
                enabled: true,
            },
            OptionState {
                key: OptionKey::SampleLimit,
                kind: OptionKind::Range {
                    min: MIN_SAMPLES as f64,
                    max: settings.max_samples(self.plus) as f64,
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
        ]
    }

    fn set_option(&mut self, key: OptionKey, value: &OptionValue) -> Result<(), DriverError> {
        self.ensure_idle()?;
        match key {
            OptionKey::OperationMode => {
                self.settings.stream = match expect_choice(key, value)? {
                    "stream" => true,
                    "buffer" => false,
                    other => {
                        return Err(DriverError::InvalidRequest(format!(
                            "unknown operation mode {other}"
                        )));
                    }
                };
                self.settings.adjust_samplerate(self.plus);
            }
            OptionKey::Samplerate => {
                let rate = value.as_integer().ok_or_else(|| {
                    DriverError::InvalidRequest("the sample rate takes an integer".into())
                })?;
                if !self.settings.samplerates(self.plus).contains(&rate) {
                    return Err(DriverError::InvalidRequest(format!(
                        "{rate} Hz is not available with these channels in this mode"
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
            OptionKey::Vth => {
                self.settings.vth = value
                    .as_number()
                    .filter(|vth| VTH_RANGE.contains(vth))
                    .ok_or_else(|| {
                        DriverError::InvalidRequest("the threshold takes -5 to 5 V".into())
                    })?;
            }
            other => {
                return Err(DriverError::InvalidRequest(format!(
                    "ATK-Logic devices have no {other:?} option"
                )));
            }
        }
        Ok(())
    }

    fn channel_count(&self) -> u16 {
        CHANNELS
    }

    /// Stream mode can run until stopped; Buffer mode captures into the device's memory.
    fn can_loop(&self) -> bool {
        self.settings.stream
    }

    /// ATK-Logic marks the durations beyond this depth with `*` and compresses them.
    fn hardware_depth(&self) -> Option<u64> {
        (!self.settings.stream).then(|| self.settings.memory_depth(self.plus))
    }

    /// Simple triggers only. Stream mode keeps a fixed share before the trigger, so the
    /// position cannot be chosen there; in Buffer mode ATK-Logic's slider goes from 1 to 90 %.
    fn trigger_caps(&self) -> TriggerCaps {
        TriggerCaps {
            max_position: if self.settings.stream { 1 } else { 90 },
            advanced: false,
            advanced_other_mode: false,
        }
    }

    fn enabled_channels(&self) -> Vec<ChannelIndex> {
        (0..CHANNELS)
            .filter(|&channel| self.settings.enabled & (1 << channel) != 0)
            .collect()
    }

    fn set_channel_enabled(
        &mut self,
        channel: ChannelIndex,
        enabled: bool,
    ) -> Result<(), DriverError> {
        self.ensure_idle()?;
        if channel >= CHANNELS {
            return Err(DriverError::InvalidRequest(format!(
                "channel {channel} does not exist"
            )));
        }
        if enabled {
            self.settings.enabled |= 1 << channel;
        } else {
            self.settings.enabled &= !(1 << channel);
        }
        self.settings.adjust_samplerate(self.plus);
        Ok(())
    }

    /// The rates follow from how many channels are on, which USB carries in Stream mode and
    /// the DL16 Plus samples at 1 GHz only up to 8 of in Buffer mode.
    fn rate_tiers(&self) -> Vec<RateTier> {
        self.settings.rate_tiers(self.plus)
    }

    fn rate_tier(&self) -> Option<usize> {
        let enabled = self.settings.enabled_count().max(1);
        self.rate_tiers()
            .iter()
            .position(|tier| u32::from(tier.channels) >= enabled)
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
                "ATK-Logic devices have simple triggers only".into(),
            ));
        }
        if plan.looping && !self.can_loop() {
            return Err(DriverError::InvalidRequest(
                "loop captures need Stream mode".into(),
            ));
        }
        let rate_code = models::rate_code(settings.samplerate).ok_or_else(|| {
            DriverError::InvalidRequest(format!("{} Hz is not available", settings.samplerate))
        })?;
        let max = settings.max_samples(self.plus);
        if settings.limit_samples > max {
            log::warn!(
                "sample limit {} exceeds {max} in this mode",
                settings.limit_samples
            );
        }
        let limit = settings
            .limit_samples
            .min(max)
            .max(MIN_SAMPLES)
            .next_multiple_of(MIN_SAMPLES);
        let depth = if plan.looping {
            acquisition::loop_depth(settings.enabled_count())
        } else {
            limit
        };
        let position = plan
            .trigger_position
            .clamp(1, self.trigger_caps().max_position);
        let params = AcquisitionParams {
            stream: settings.stream,
            rle: !settings.stream && limit > settings.memory_depth(self.plus),
            samplerate: settings.samplerate,
            rate_code,
            limit_samples: limit,
            depth,
            pre_trigger: depth / 100 * u64::from(position),
            enabled: settings.enabled,
            vth: settings.vth,
            triggers: plan.triggers,
            instant: plan.instant,
            looping: plan.looping,
        };
        acquisition::start(self.link.clone(), params, Arc::clone(&self.running), sink)
    }
}
