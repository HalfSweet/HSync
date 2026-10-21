// SPDX-License-Identifier: GPL-3.0-or-later
//! SLogic16U3 configuration, following Sipeed's libsigrok `slogic-dev` driver:
//! `api.c`, `slogic/slogic16u3.c` and `slogic/slogic_data.c`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam_channel::Sender;
use hsync_core::{ChannelIndex, UiError};
use hsync_driver::{
    Acquisition, CapturePlan, Choice, Device, DeviceInfo, DriverError, Label, OptionKey,
    OptionKind, OptionState, OptionValue, Packet, TriggerCaps,
};

use crate::acquisition;
use crate::protocol::Link;

/// Physical digital inputs on the SLogic16U3.
pub const CHANNELS: u16 = 16;
/// HSync sends logic in blocks of 64 samples per enabled channel.
pub const MIN_SAMPLES: u64 = 64;
/// HSync's software capture-depth limit, shared with its other hardware drivers.
pub const MAX_SOFTWARE_DEPTH: u64 = 16 * (1 << 30);
/// The model's advertised rates in ascending order (`rates_16u3`).
pub const SAMPLERATES: &[u64] = &[
    5_000_000,
    8_000_000,
    10_000_000,
    16_000_000,
    20_000_000,
    25_000_000,
    32_000_000,
    40_000_000,
    50_000_000,
    80_000_000,
    100_000_000,
    160_000_000,
    200_000_000,
    400_000_000,
    800_000_000,
];

const DEFAULT_SAMPLES: u64 = 1 << 20;
const VTH_RANGE: std::ops::RangeInclusive<f64> = 0.0..=6.0;

/// The USB link's sustainable payload class for this model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbSpeed {
    /// USB 2.0: 40 MB/s of sample payload.
    High,
    /// USB 3.x: the device's 400 MB/s sample payload ceiling.
    Super,
}

impl UsbSpeed {
    /// Maximum sample rate for a 4-, 8- or 16-channel wire mode.
    pub fn max_samplerate(self, channels: u8) -> u64 {
        if !matches!(channels, 4 | 8 | 16) {
            return 0;
        }
        let bandwidth = match self {
            Self::High => 320_000_000,
            Self::Super => 3_200_000_000,
        };
        bandwidth / u64::from(channels)
    }

    fn id(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Super => "super",
        }
    }
}

/// Persistent settings for a streaming capture.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Samples per second, selected from the model's discrete rates.
    pub samplerate: u64,
    /// Samples retained per enabled channel for a finite capture.
    pub limit_samples: u64,
    /// Adjustable threshold voltage, between 0 and 6 V.
    pub vth: f64,
    /// Bit `n` selects physical channel `n` for HSync's capture.
    pub enabled: u16,
    /// Contiguous low channels transferred by the device, including disabled inputs.
    channels: u8,
}

impl Settings {
    /// All 16 inputs at the fastest rate supported by the link, with a 1.7 V threshold.
    pub fn defaults(speed: UsbSpeed) -> Self {
        Self {
            samplerate: speed.max_samplerate(16),
            limit_samples: DEFAULT_SAMPLES,
            vth: 1.7,
            enabled: u16::MAX,
            channels: 16,
        }
    }

    /// Contiguous 4-, 8- or 16-channel mode covering the highest enabled input.
    pub fn channels(&self) -> u8 {
        self.channels
    }

    /// Available rates for this link and wire mode (`slogic_rate_list()`).
    pub fn samplerates(&self, speed: UsbSpeed) -> Vec<u64> {
        let maximum = speed.max_samplerate(self.channels);
        SAMPLERATES
            .iter()
            .copied()
            .take_while(|&rate| rate <= maximum)
            .collect()
    }

    /// The adapter selects its mode from the highest enabled channel, rather than their
    /// count: a lone D15 still travels in the 16-channel wire format.
    fn reconcile_channels(&mut self, speed: UsbSpeed) {
        if self.enabled != 0 {
            self.channels = match u16::BITS - self.enabled.leading_zeros() {
                1..=4 => 4,
                5..=8 => 8,
                _ => 16,
            };
        }
        self.samplerate = self.samplerate.min(speed.max_samplerate(self.channels));
    }
}

/// An opened SLogic16U3, configured through a claimed nusb interface.
pub struct SlogicDevice {
    link: Link,
    info: DeviceInfo,
    speed: UsbSpeed,
    settings: Settings,
    running: Arc<AtomicBool>,
}

impl std::fmt::Debug for SlogicDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlogicDevice")
            .field("info", &self.info)
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl SlogicDevice {
    /// Stops and drains a previous capture, then resets into Sipeed's known idle state.
    pub fn open(link: Link, speed: UsbSpeed, id: String) -> Result<Self, DriverError> {
        link.initialize()?;
        Ok(Self {
            link,
            info: DeviceInfo {
                id,
                driver: crate::DRIVER_NAME.to_owned(),
                vendor: crate::VENDOR.to_owned(),
                model: "SLogic16U3".to_owned(),
                // The U3 capture protocol provides no firmware-version query.
                firmware: None,
                usb_speed: Some(speed.id().to_owned()),
                slow_port: speed == UsbSpeed::High,
            },
            speed,
            settings: Settings::defaults(speed),
            running: Arc::new(AtomicBool::new(false)),
        })
    }

    /// The settings the next capture will use.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    fn ensure_idle(&self) -> Result<(), DriverError> {
        if self.running.load(Ordering::Acquire) {
            return Err(DriverError::Localized(UiError::new(
                "error.device.slogic.captureRunning",
            )));
        }
        Ok(())
    }
}

impl Device for SlogicDevice {
    fn info(&self) -> DeviceInfo {
        self.info.clone()
    }

    fn options(&self) -> Vec<OptionState> {
        vec![
            OptionState {
                key: OptionKey::OperationMode,
                kind: OptionKind::Choice {
                    choices: vec![Choice {
                        id: "stream".to_owned(),
                        label: Label::new("device.operationMode.stream"),
                    }],
                },
                value: OptionValue::Choice("stream".to_owned()),
                enabled: false,
            },
            OptionState {
                key: OptionKey::Samplerate,
                kind: OptionKind::Integers {
                    values: self.settings.samplerates(self.speed),
                },
                value: OptionValue::Integer(self.settings.samplerate),
                enabled: true,
            },
            OptionState {
                key: OptionKey::SampleLimit,
                kind: OptionKind::Range {
                    min: MIN_SAMPLES as f64,
                    max: MAX_SOFTWARE_DEPTH as f64,
                    step: MIN_SAMPLES as f64,
                },
                value: OptionValue::Integer(self.settings.limit_samples),
                enabled: true,
            },
            OptionState {
                key: OptionKey::Vth,
                kind: OptionKind::Range {
                    min: *VTH_RANGE.start(),
                    max: *VTH_RANGE.end(),
                    step: 0.1,
                },
                value: OptionValue::Number(self.settings.vth),
                enabled: true,
            },
        ]
    }

    fn set_option(&mut self, key: OptionKey, value: &OptionValue) -> Result<(), DriverError> {
        self.ensure_idle()?;
        match key {
            OptionKey::OperationMode if value.as_choice() == Some("stream") => {}
            OptionKey::Samplerate => {
                let rate = value
                    .as_integer()
                    .filter(|rate| self.settings.samplerates(self.speed).contains(rate))
                    .ok_or_else(|| {
                        DriverError::Localized(UiError::new("error.device.slogic.samplerate"))
                    })?;
                self.settings.samplerate = rate;
            }
            OptionKey::SampleLimit => {
                let limit = value
                    .as_integer()
                    .filter(|limit| {
                        (MIN_SAMPLES..=MAX_SOFTWARE_DEPTH).contains(limit)
                            && limit.is_multiple_of(MIN_SAMPLES)
                    })
                    .ok_or_else(|| {
                        DriverError::Localized(
                            UiError::new("error.device.slogic.sampleLimit")
                                .with("min", MIN_SAMPLES)
                                .with("max", MAX_SOFTWARE_DEPTH)
                                .with("step", MIN_SAMPLES),
                        )
                    })?;
                self.settings.limit_samples = limit;
            }
            OptionKey::Vth => {
                self.settings.vth = value
                    .as_number()
                    .filter(|threshold| VTH_RANGE.contains(threshold))
                    .ok_or_else(|| {
                        DriverError::Localized(UiError::new("error.device.slogic.threshold"))
                    })?;
            }
            _ => {
                return Err(DriverError::Localized(UiError::new(
                    "error.device.slogic.option",
                )));
            }
        }
        Ok(())
    }

    fn channel_count(&self) -> u16 {
        CHANNELS
    }

    fn trigger_caps(&self) -> TriggerCaps {
        TriggerCaps::default()
    }

    fn can_loop(&self) -> bool {
        true
    }

    fn enabled_channels(&self) -> Vec<ChannelIndex> {
        (0..CHANNELS)
            .filter(|channel| self.settings.enabled & (1 << channel) != 0)
            .collect()
    }

    fn set_channel_enabled(
        &mut self,
        channel: ChannelIndex,
        enabled: bool,
    ) -> Result<(), DriverError> {
        self.ensure_idle()?;
        if channel >= CHANNELS {
            return Err(DriverError::Localized(
                UiError::new("error.device.slogic.channel").with("channel", channel),
            ));
        }
        if enabled {
            self.settings.enabled |= 1 << channel;
        } else {
            self.settings.enabled &= !(1 << channel);
        }
        self.settings.reconcile_channels(self.speed);
        Ok(())
    }

    fn start(
        &mut self,
        mut plan: CapturePlan,
        sink: Sender<Packet>,
    ) -> Result<Box<dyn Acquisition>, DriverError> {
        self.ensure_idle()?;
        if self.settings.enabled == 0 {
            return Err(DriverError::Localized(UiError::new(
                "error.device.slogic.noChannels",
            )));
        }
        if plan.advanced.is_some() {
            return Err(DriverError::Localized(UiError::new(
                "error.device.slogic.triggerType",
            )));
        }
        if let Some(&(channel, _)) = plan
            .triggers
            .iter()
            .find(|&&(channel, _)| channel >= CHANNELS)
        {
            return Err(DriverError::Localized(
                UiError::new("error.device.slogic.triggerChannel").with("channel", channel),
            ));
        }
        plan.triggers
            .retain(|&(channel, _)| self.settings.enabled & (1 << channel) != 0);
        acquisition::start(
            self.link.clone(),
            &self.settings,
            plan,
            Arc::clone(&self.running),
            sink,
        )
    }
}

#[cfg(test)]
mod tests {
    use hsync_core::TriggerCondition;
    use hsync_driver::AdvancedTrigger;
    use hsync_driver::mock::{MockDevice, MockTransport};
    use hsync_driver::usb::UsbError;

    use super::*;

    /// An idle device that accepts the open/reset sequence; captures are not started here.
    struct IdleDevice;

    impl MockDevice for IdleDevice {
        fn control_out(&mut self, _: u8, _: u16, _: u16, _: &[u8]) -> Result<(), UsbError> {
            Ok(())
        }

        fn control_in(&mut self, _: u8, _: u16, _: u16, _: u16) -> Result<Vec<u8>, UsbError> {
            panic!("opening the SLogic16U3 requires no control reads");
        }

        fn bulk_out(&mut self, _: u8, _: &[u8]) -> Result<(), UsbError> {
            panic!("the SLogic16U3 uses no bulk OUT endpoint");
        }

        fn bulk_in(&mut self, _: u8, _: usize) -> Option<Result<Vec<u8>, UsbError>> {
            None
        }
    }

    fn device(speed: UsbSpeed) -> SlogicDevice {
        let link = Link::new(Arc::new(MockTransport::new(IdleDevice)));
        SlogicDevice::open(link, speed, "test".to_owned()).expect("open")
    }

    #[test]
    fn advertises_the_official_discrete_rates_for_each_link_and_channel_mode() {
        for (speed, ceilings) in [
            (UsbSpeed::Super, [800_000_000, 400_000_000, 200_000_000]),
            (UsbSpeed::High, [80_000_000, 40_000_000, 20_000_000]),
        ] {
            let mut settings = Settings::defaults(speed);
            for (channels, maximum) in [4, 8, 16].into_iter().zip(ceilings) {
                settings.enabled = u16::MAX >> (16 - channels);
                settings.reconcile_channels(speed);
                assert_eq!(settings.channels(), channels);
                assert_eq!(
                    settings.samplerates(speed),
                    SAMPLERATES
                        .iter()
                        .copied()
                        .filter(|&rate| rate <= maximum)
                        .collect::<Vec<_>>()
                );
                assert_eq!(settings.samplerates(speed).last(), Some(&maximum));
            }
        }
        assert_eq!(UsbSpeed::Super.max_samplerate(3), 0);
    }

    #[test]
    fn channel_mode_covers_the_highest_enabled_channel_and_reconciles_rate() {
        let mut device = device(UsbSpeed::Super);
        for channel in 4..16 {
            device.set_channel_enabled(channel, false).expect("disable");
        }
        assert_eq!(device.settings().channels(), 4);
        assert_eq!(device.channel_count(), 16);
        device
            .set_option(OptionKey::Samplerate, &OptionValue::Integer(800_000_000))
            .expect("800 MHz on D0 through D3");
        device.set_channel_enabled(7, true).expect("enable D7");
        assert_eq!(device.settings().channels(), 8);
        assert_eq!(device.settings().samplerate, 400_000_000);
        device.set_channel_enabled(15, true).expect("enable D15");
        for channel in 0..15 {
            device.set_channel_enabled(channel, false).expect("disable");
        }
        assert_eq!(device.enabled_channels(), vec![15]);
        assert_eq!(device.settings().channels(), 16);
        assert_eq!(device.settings().samplerate, 200_000_000);
        device.set_channel_enabled(15, false).expect("disable last");
        assert_eq!(device.settings().channels(), 16);
        device.set_channel_enabled(0, true).expect("enable D0");
        assert_eq!(device.settings().channels(), 4);
    }

    #[test]
    fn exposes_streaming_and_simple_trigger_capabilities() {
        let device = device(UsbSpeed::High);
        assert_eq!(device.info().model, "SLogic16U3");
        assert_eq!(device.info().usb_speed.as_deref(), Some("high"));
        assert!(device.info().slow_port);
        assert_eq!(device.settings().samplerate, 20_000_000);
        assert_eq!(device.settings().vth, 1.7);
        assert_eq!(device.hardware_depth(), None);
        assert!(device.can_loop());
        assert_eq!(device.trigger_caps(), TriggerCaps::default());
        let options = device.options();
        let mode = &options[0];
        assert_eq!(mode.key, OptionKey::OperationMode);
        assert_eq!(mode.value, OptionValue::Choice("stream".to_owned()));
        assert!(!mode.enabled);
    }

    #[test]
    fn rejects_unsupported_settings_without_changing_the_previous_values() {
        let mut device = device(UsbSpeed::High);
        let before = device.settings().clone();
        for (key, value) in [
            (OptionKey::Samplerate, OptionValue::Integer(200_000_000)),
            (OptionKey::Samplerate, OptionValue::Integer(12_000_000)),
            (OptionKey::Samplerate, OptionValue::Number(5_000_000.0)),
            (OptionKey::SampleLimit, OptionValue::Integer(0)),
            (OptionKey::SampleLimit, OptionValue::Integer(65)),
            (OptionKey::SampleLimit, OptionValue::Integer(u64::MAX)),
            (OptionKey::Vth, OptionValue::Number(-0.1)),
            (OptionKey::Vth, OptionValue::Number(6.1)),
            (OptionKey::Vth, OptionValue::Number(f64::NAN)),
            (OptionKey::Vth, OptionValue::Number(f64::INFINITY)),
            (
                OptionKey::OperationMode,
                OptionValue::Choice("buffer".into()),
            ),
            (OptionKey::ExternalClock, OptionValue::Bool(true)),
        ] {
            assert!(device.set_option(key, &value).is_err(), "{key:?}");
            assert_eq!(device.settings(), &before);
        }
        assert!(device.set_channel_enabled(16, true).is_err());
        assert_eq!(device.settings(), &before);
        for limit in [MIN_SAMPLES, MAX_SOFTWARE_DEPTH] {
            device
                .set_option(OptionKey::SampleLimit, &OptionValue::Integer(limit))
                .expect("sample limit boundary");
        }
        for voltage in [0.0, 6.0] {
            device
                .set_option(OptionKey::Vth, &OptionValue::Number(voltage))
                .expect("threshold boundary");
        }
    }

    #[test]
    fn refuses_invalid_capture_plans_and_configuration_during_capture() {
        let mut device = device(UsbSpeed::Super);
        let (sink, _receiver) = crossbeam_channel::unbounded();
        let advanced = CapturePlan {
            advanced: Some(AdvancedTrigger::Stages { stages: vec![] }),
            ..CapturePlan::default()
        };
        assert!(device.start(advanced, sink.clone()).is_err());
        let outside = CapturePlan {
            triggers: vec![(16, TriggerCondition::Rise)],
            ..CapturePlan::default()
        };
        assert!(device.start(outside, sink.clone()).is_err());
        device.running.store(true, Ordering::Release);
        assert!(device.set_channel_enabled(0, false).is_err());
        assert!(
            device
                .set_option(OptionKey::Samplerate, &OptionValue::Integer(5_000_000))
                .is_err()
        );
        assert!(device.start(CapturePlan::default(), sink.clone()).is_err());
        device.running.store(false, Ordering::Release);
        for channel in 0..16 {
            device.set_channel_enabled(channel, false).expect("disable");
        }
        assert!(device.start(CapturePlan::default(), sink).is_err());
    }
}
