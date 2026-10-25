// SPDX-License-Identifier: GPL-3.0-or-later
//! SLogic U3 configuration, following Sipeed's libsigrok `slogic-dev` driver:
//! `api.c`, `slogic/slogic16u3.c`, `slogic/slogic32u3.c` and `slogic/slogic_data.c`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam_channel::Sender;
use hsync_core::{ANALOG_BASE, ChannelIndex, UiError};
use hsync_driver::{
    Acquisition, AnalogInput, CapturePlan, Choice, Device, DeviceInfo, DriverError, Label,
    OptionKey, OptionKind, OptionState, OptionValue, Packet, RateTier, TriggerCaps,
};

use crate::model::{Model, UsbSpeed};
use crate::protocol::Link;
use crate::{acquisition, adc};

/// HSync sends logic in blocks of 64 samples per enabled channel.
pub const MIN_SAMPLES: u64 = 64;
/// HSync's software capture-depth limit, shared with its other hardware drivers.
pub const MAX_SOFTWARE_DEPTH: u64 = 16 * (1 << 30);

const DEFAULT_SAMPLES: u64 = 1 << 20;
const VTH_RANGE: std::ops::RangeInclusive<f64> = 0.0..=6.0;

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
    pub enabled: u32,
    /// Bit `g` reads inputs `8g..8g + 8` as the ADC module's analog channel `g`; those
    /// inputs are not captured as logic meanwhile, whatever `enabled` says of them.
    pub analog: u8,
    /// Contiguous low channels transferred by the device, including disabled inputs.
    channels: u8,
}

impl Settings {
    /// Every input at the fastest rate the link supports, with a 1.7 V threshold.
    pub fn defaults(model: &Model, speed: UsbSpeed) -> Self {
        Self {
            samplerate: model.max_samplerate(model.channels, speed),
            limit_samples: DEFAULT_SAMPLES,
            vth: 1.7,
            enabled: model.all_channels(),
            analog: 0,
            channels: model.channels,
        }
    }

    /// The contiguous wire mode covering the highest enabled input: 4, 8, 16 or 32 channels.
    pub fn channels(&self) -> u8 {
        self.channels
    }

    /// The inputs the analog channels are read from.
    fn analog_lines(&self) -> u32 {
        (0..4)
            .filter(|group| self.analog & (1 << group) != 0)
            .fold(0, |inputs, group| inputs | 0xff << (group * adc::GROUP))
    }

    /// Bit `n` captures input `n` as logic.
    pub fn logic(&self) -> u32 {
        self.enabled & !self.analog_lines()
    }

    /// The analog groups read, in order.
    pub fn analog_groups(&self) -> Vec<u8> {
        (0..8)
            .filter(|group| self.analog & (1 << group) != 0)
            .collect()
    }

    /// Rates of wire mode `channels` with these analog channels read.
    fn samplerates(&self, model: &Model, speed: UsbSpeed, channels: u8) -> Vec<u64> {
        let rates = model.samplerates(channels, speed);
        if self.analog == 0 {
            rates
        } else {
            adc::samplerates(&rates)
        }
    }

    /// The adapter selects its mode from the highest input it captures, rather than their
    /// count: a lone D15 still travels in the 16-channel wire format. The sample rate moves
    /// to the fastest the mode allows below it, or else its slowest.
    fn reconcile_channels(&mut self, model: &Model, speed: UsbSpeed) {
        if let Some(highest) = (self.logic() | self.analog_lines()).checked_ilog2()
            && let Some(channels) = model.wire_mode(highest as u8)
        {
            self.channels = channels;
        }
        let rates = self.samplerates(model, speed, self.channels);
        if !rates.contains(&self.samplerate) {
            self.samplerate = rates
                .iter()
                .copied()
                .rfind(|&rate| rate <= self.samplerate)
                .or(rates.first().copied())
                .unwrap_or(self.samplerate);
        }
    }
}

/// An opened SLogic U3 analyzer, configured through a claimed nusb interface.
pub struct SlogicDevice {
    link: Link,
    info: DeviceInfo,
    model: &'static Model,
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
    pub fn open(
        link: Link,
        model: &'static Model,
        speed: UsbSpeed,
        id: String,
    ) -> Result<Self, DriverError> {
        link.initialize()?;
        Ok(Self {
            link,
            info: DeviceInfo {
                id,
                driver: crate::DRIVER_NAME.to_owned(),
                vendor: crate::VENDOR.to_owned(),
                model: model.name.to_owned(),
                // The U3 capture protocol provides no firmware-version query.
                firmware: None,
                usb_speed: Some(speed.id().to_owned()),
                slow_port: speed == UsbSpeed::High,
            },
            model,
            speed,
            settings: Settings::defaults(model, speed),
            running: Arc::new(AtomicBool::new(false)),
        })
    }

    /// The settings the next capture will use.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Rates the current wire mode offers on this link.
    fn samplerates(&self) -> Vec<u64> {
        self.settings
            .samplerates(self.model, self.speed, self.settings.channels)
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
                    values: self.samplerates(),
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
                    .filter(|rate| self.samplerates().contains(rate))
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
        self.model.channels.into()
    }

    fn trigger_caps(&self) -> TriggerCaps {
        TriggerCaps::default()
    }

    fn can_loop(&self) -> bool {
        true
    }

    fn enabled_channels(&self) -> Vec<ChannelIndex> {
        let logic = self.settings.logic();
        (0..self.channel_count())
            .filter(|channel| logic & (1 << channel) != 0)
            .collect()
    }

    fn set_channel_enabled(
        &mut self,
        channel: ChannelIndex,
        enabled: bool,
    ) -> Result<(), DriverError> {
        self.ensure_idle()?;
        if channel >= self.channel_count() {
            return Err(DriverError::Localized(
                UiError::new("error.device.slogic.channel").with("channel", channel),
            ));
        }
        if enabled {
            self.settings.enabled |= 1 << channel;
        } else {
            self.settings.enabled &= !(1 << channel);
        }
        self.settings.reconcile_channels(self.model, self.speed);
        Ok(())
    }

    fn analog_inputs(&self) -> Vec<AnalogInput> {
        (0..self.model.analog_groups)
            .map(|group| AnalogInput {
                index: ANALOG_BASE + ChannelIndex::from(group),
                inputs: (0..adc::GROUP)
                    .map(|bit| ChannelIndex::from(group * adc::GROUP + bit))
                    .collect(),
                low: adc::LOW,
                high: adc::HIGH,
                enabled: self.settings.analog & (1 << group) != 0,
            })
            .collect()
    }

    fn set_analog_enabled(
        &mut self,
        index: ChannelIndex,
        enabled: bool,
    ) -> Result<(), DriverError> {
        self.ensure_idle()?;
        let group = index
            .checked_sub(ANALOG_BASE)
            .and_then(|group| u8::try_from(group).ok())
            .filter(|&group| group < self.model.analog_groups)
            .ok_or_else(|| DriverError::InvalidRequest(format!("no analog input {index}")))?;
        if enabled {
            self.settings.analog |= 1 << group;
        } else {
            self.settings.analog &= !(1 << group);
        }
        self.settings.reconcile_channels(self.model, self.speed);
        Ok(())
    }

    /// The wire modes: the inputs from D0 up to the mode's width share the link's bandwidth.
    /// Those that keep an analog channel offer only the rates the ADC module allows.
    fn rate_tiers(&self) -> Vec<RateTier> {
        self.model
            .wire_modes
            .iter()
            .map(|&(channels, _)| {
                let mut kept = self.settings.clone();
                kept.analog &= analog_within(channels);
                let samplerates = kept.samplerates(self.model, self.speed, channels);
                RateTier {
                    channels: channels.into(),
                    leading: true,
                    span: channels.into(),
                    max_samplerate: samplerates.last().copied().unwrap_or(0),
                    samplerates,
                }
            })
            .collect()
    }

    fn rate_tier(&self) -> Option<usize> {
        self.model
            .wire_modes
            .iter()
            .position(|&(channels, _)| channels == self.settings.channels)
    }

    fn select_rate_tier(
        &mut self,
        index: usize,
        channels: &[ChannelIndex],
    ) -> Result<(), DriverError> {
        let &(wire, _) = self
            .model
            .wire_modes
            .get(index)
            .ok_or_else(|| DriverError::InvalidRequest(format!("no rate tier {index}")))?;
        if !self.rate_tiers()[index].fits(channels) {
            return Err(DriverError::InvalidRequest(format!(
                "channels {channels:?} do not fit rate tier {index}"
            )));
        }
        // Analog channels beyond the tier's inputs stop being read.
        self.ensure_idle()?;
        self.settings.analog &= analog_within(wire);
        self.set_enabled_channels(channels)?;
        // With no input on, the wire mode stays where it was; take the tier's.
        if channels.is_empty() {
            self.settings.channels = wire;
            self.settings.reconcile_channels(self.model, self.speed);
        }
        Ok(())
    }

    fn start(
        &mut self,
        mut plan: CapturePlan,
        sink: Sender<Packet>,
    ) -> Result<Box<dyn Acquisition>, DriverError> {
        self.ensure_idle()?;
        if self.settings.logic() == 0 && self.settings.analog == 0 {
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
            .find(|&&(channel, _)| channel >= self.channel_count())
        {
            return Err(DriverError::Localized(
                UiError::new("error.device.slogic.triggerChannel").with("channel", channel),
            ));
        }
        // Only inputs captured as logic trigger; the analog channels do not.
        let logic = self.settings.logic();
        plan.triggers
            .retain(|&(channel, _)| logic & (1 << channel) != 0);
        acquisition::start(
            self.link.clone(),
            self.model,
            &self.settings,
            plan,
            Arc::clone(&self.running),
            sink,
        )
    }
}

/// The analog groups wholly inside the first `channels` inputs.
fn analog_within(channels: u8) -> u8 {
    ((1u16 << (channels / adc::GROUP)) - 1) as u8
}

#[cfg(test)]
mod tests {
    use hsync_core::TriggerCondition;
    use hsync_driver::AdvancedTrigger;
    use hsync_driver::mock::{MockDevice, MockTransport};
    use hsync_driver::usb::UsbError;

    use super::*;
    use crate::model::{SLOGIC16U3, SLOGIC32U3};

    /// An idle device that accepts the open/reset sequence; captures are not started here.
    struct IdleDevice;

    impl MockDevice for IdleDevice {
        fn control_out(&mut self, _: u8, _: u16, _: u16, _: &[u8]) -> Result<(), UsbError> {
            Ok(())
        }

        fn control_in(&mut self, _: u8, _: u16, _: u16, _: u16) -> Result<Vec<u8>, UsbError> {
            panic!("opening an SLogic requires no control reads");
        }

        fn bulk_out(&mut self, _: u8, _: &[u8]) -> Result<(), UsbError> {
            panic!("the SLogic U3 models use no bulk OUT endpoint");
        }

        fn bulk_in(&mut self, _: u8, _: usize) -> Option<Result<Vec<u8>, UsbError>> {
            None
        }
    }

    fn device(model: &'static Model, speed: UsbSpeed) -> SlogicDevice {
        let link = Link::new(Arc::new(MockTransport::new(IdleDevice)));
        SlogicDevice::open(link, model, speed, "test".to_owned()).expect("open")
    }

    fn select(device: &mut SlogicDevice, enabled: u32) {
        for channel in 0..device.channel_count() {
            device
                .set_channel_enabled(channel, enabled & (1 << channel) != 0)
                .expect("channel selection");
        }
    }

    #[test]
    fn advertises_the_official_discrete_rates_for_each_link_and_channel_mode() {
        for model in [&SLOGIC16U3, &SLOGIC32U3] {
            for speed in [UsbSpeed::High, UsbSpeed::Super, UsbSpeed::SuperPlus] {
                let mut device = device(model, speed);
                for &(channels, _) in model.wire_modes {
                    select(&mut device, u32::MAX >> (32 - channels));
                    assert_eq!(device.settings().channels(), channels);
                    let OptionKind::Integers { values } = &device.options()[1].kind else {
                        panic!("discrete sample rates");
                    };
                    assert_eq!(values, &model.samplerates(channels, speed));
                    assert_eq!(values.last(), Some(&model.max_samplerate(channels, speed)));
                }
            }
        }
    }

    #[test]
    fn channel_mode_covers_the_highest_enabled_channel_and_reconciles_rate() {
        let mut device = device(&SLOGIC16U3, UsbSpeed::Super);
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
    fn offers_the_wire_modes_as_rate_tiers() {
        for (model, speed, ceilings) in [
            (&SLOGIC16U3, UsbSpeed::Super, &[800, 400, 200][..]),
            (&SLOGIC16U3, UsbSpeed::High, &[80, 40, 20]),
            (&SLOGIC32U3, UsbSpeed::SuperPlus, &[1400, 800, 400, 200]),
            (&SLOGIC32U3, UsbSpeed::Super, &[800, 400, 200, 100]),
        ] {
            let device = device(model, speed);
            let tiers = device.rate_tiers();
            assert_eq!(
                tiers
                    .iter()
                    .map(|tier| (tier.channels, tier.leading, tier.span, tier.max_samplerate))
                    .collect::<Vec<_>>(),
                model
                    .wire_modes
                    .iter()
                    .zip(ceilings)
                    .map(|(&(channels, _), &mhz)| {
                        (channels.into(), true, channels.into(), mhz * 1_000_000)
                    })
                    .collect::<Vec<_>>(),
                "{} on {speed:?}",
                model.name
            );
            for tier in &tiers {
                assert_eq!(tier.samplerates.first(), Some(&5_000_000));
                assert_eq!(tier.samplerates.last(), Some(&tier.max_samplerate));
            }
            // Every input starts enabled, in the widest mode.
            assert_eq!(device.rate_tier(), Some(tiers.len() - 1));
        }
    }

    #[test]
    fn selects_a_rate_tier_by_its_channels() {
        let mut device = device(&SLOGIC16U3, UsbSpeed::Super);
        device
            .select_rate_tier(0, &[0, 3])
            .expect("D0 and D3 at up to 800 MHz");
        assert_eq!(device.rate_tier(), Some(0));
        assert_eq!(device.enabled_channels(), [0, 3]);
        assert_eq!(device.settings().samplerate, 200_000_000);
        device
            .set_option(OptionKey::Samplerate, &OptionValue::Integer(800_000_000))
            .expect("800 MHz");

        let before = device.settings().clone();
        for (index, channels) in [(0, vec![4]), (1, vec![0; 2]), (3, vec![0])] {
            assert!(matches!(
                device.select_rate_tier(index, &channels),
                Err(DriverError::InvalidRequest(_))
            ));
            assert_eq!(device.settings(), &before);
        }

        device.select_rate_tier(1, &[1, 7]).expect("two of D0–D7");
        assert_eq!(device.rate_tier(), Some(1));
        assert_eq!(device.settings().samplerate, 400_000_000);
        device.select_rate_tier(2, &[]).expect("none on");
        assert!(device.enabled_channels().is_empty());
        assert_eq!(device.rate_tier(), Some(2));
        assert_eq!(device.settings().samplerate, 200_000_000);
    }

    #[test]
    fn slogic32u3_reaches_1400_mhz_on_four_channels_and_200_mhz_on_all() {
        let mut device = device(&SLOGIC32U3, UsbSpeed::SuperPlus);
        assert_eq!(device.channel_count(), 32);
        assert_eq!(device.enabled_channels(), (0..32).collect::<Vec<_>>());
        assert_eq!(device.settings().channels(), 32);
        assert_eq!(device.settings().samplerate, 200_000_000);
        select(&mut device, 0b1001);
        assert_eq!(device.settings().channels(), 4);
        device
            .set_option(OptionKey::Samplerate, &OptionValue::Integer(1_400_000_000))
            .expect("1400 MHz on D0 through D3");
        device.set_channel_enabled(16, true).expect("enable D16");
        assert_eq!(device.settings().channels(), 32);
        assert_eq!(device.settings().samplerate, 200_000_000);
        select(&mut device, 1 << 31);
        assert_eq!(device.enabled_channels(), vec![31]);
        assert_eq!(device.settings().channels(), 32);
        select(&mut device, 0xffff);
        assert_eq!(device.settings().channels(), 16);
        device
            .set_option(OptionKey::Samplerate, &OptionValue::Integer(400_000_000))
            .expect("400 MHz on D0 through D15");
        assert!(
            device
                .set_option(OptionKey::Samplerate, &OptionValue::Integer(800_000_000))
                .is_err()
        );
    }

    #[test]
    fn a_slower_link_caps_the_slogic32u3_by_its_payload() {
        let five = device(&SLOGIC32U3, UsbSpeed::Super);
        assert_eq!(five.info().usb_speed.as_deref(), Some("super"));
        assert!(!five.info().slow_port);
        assert_eq!(five.settings().samplerate, 100_000_000);
        let two = device(&SLOGIC32U3, UsbSpeed::High);
        assert!(two.info().slow_port);
        assert_eq!(two.settings().samplerate, 10_000_000);
    }

    #[test]
    fn reads_the_adc_module_in_place_of_a_group_of_inputs() {
        assert!(
            device(&SLOGIC16U3, UsbSpeed::Super)
                .analog_inputs()
                .is_empty()
        );
        assert!(
            device(&SLOGIC16U3, UsbSpeed::Super)
                .set_analog_enabled(ANALOG_BASE, true)
                .is_err()
        );
        let mut device = device(&SLOGIC32U3, UsbSpeed::SuperPlus);
        assert_eq!(
            device
                .analog_inputs()
                .iter()
                .map(|input| (input.index, input.inputs.clone(), input.enabled))
                .collect::<Vec<_>>(),
            (0..4)
                .map(|group| (
                    ANALOG_BASE + group,
                    (group * 8..group * 8 + 8).collect(),
                    false
                ))
                .collect::<Vec<_>>()
        );
        select(&mut device, 0b1001);
        device
            .set_option(OptionKey::Samplerate, &OptionValue::Integer(1_400_000_000))
            .expect("1400 MHz on D0 through D3");
        device
            .set_analog_enabled(ANALOG_BASE + 3, true)
            .expect("A3 on inputs 24–31");
        // The mode carries inputs 24–31 now, at the rates of the module's clock.
        assert_eq!(device.settings().channels(), 32);
        assert_eq!(device.settings().samplerate, 200_000_000);
        let OptionKind::Integers { values } = &device.options()[1].kind else {
            panic!("discrete sample rates");
        };
        assert_eq!(values, &[100, 160, 200].map(|mhz| mhz * 1_000_000));
        assert_eq!(device.enabled_channels(), [0, 3]);
        // Inputs under an analog channel are no logic channels, and come back as they were.
        device.set_channel_enabled(24, true).expect("D24 under A3");
        assert_eq!(device.enabled_channels(), [0, 3]);
        device
            .set_analog_enabled(ANALOG_BASE + 3, false)
            .expect("A3 off");
        assert_eq!(device.enabled_channels(), [0, 3, 24]);
        assert!(device.analog_inputs().iter().all(|input| !input.enabled));
        // A rate below the module's minimum moves up to it.
        select(&mut device, 1);
        device
            .set_option(OptionKey::Samplerate, &OptionValue::Integer(5_000_000))
            .expect("5 MHz");
        device.set_analog_enabled(ANALOG_BASE, true).expect("A0");
        assert_eq!(device.settings().channels(), 8);
        assert_eq!(device.settings().samplerate, 100_000_000);
        assert!(device.enabled_channels().is_empty());
        assert!(device.set_analog_enabled(ANALOG_BASE + 4, true).is_err());
        assert!(device.set_analog_enabled(4, true).is_err());
    }

    #[test]
    fn rate_tiers_keep_the_analog_channels_within_them() {
        let mut device = device(&SLOGIC32U3, UsbSpeed::SuperPlus);
        device.set_analog_enabled(ANALOG_BASE, true).expect("A0");
        device
            .set_analog_enabled(ANALOG_BASE + 2, true)
            .expect("A2");
        let tiers = device.rate_tiers();
        // Four inputs hold no group of eight; eight hold A0, on the module's clock.
        assert_eq!(tiers[0].max_samplerate, 1_400_000_000);
        assert_eq!(tiers[1].samplerates.first(), Some(&100_000_000));
        assert_eq!(tiers[1].max_samplerate, 800_000_000);
        assert_eq!(tiers[3].max_samplerate, 200_000_000);
        device.select_rate_tier(1, &[]).expect("D0–D7");
        let enabled: Vec<_> = device
            .analog_inputs()
            .into_iter()
            .filter(|input| input.enabled)
            .map(|input| input.index)
            .collect();
        assert_eq!(enabled, [ANALOG_BASE]);
        assert_eq!(device.rate_tier(), Some(1));
        device.select_rate_tier(0, &[0]).expect("D0–D3");
        assert!(device.analog_inputs().iter().all(|input| !input.enabled));
        assert_eq!(device.enabled_channels(), [0]);
    }

    #[test]
    fn exposes_streaming_and_simple_trigger_capabilities() {
        for (model, speed, id, slow, rate) in [
            (&SLOGIC16U3, UsbSpeed::High, "high", true, 20_000_000),
            (
                &SLOGIC32U3,
                UsbSpeed::SuperPlus,
                "superPlus",
                false,
                200_000_000,
            ),
        ] {
            let device = device(model, speed);
            assert_eq!(device.info().model, model.name);
            assert_eq!(device.info().vendor, "Sipeed");
            assert_eq!(device.info().usb_speed.as_deref(), Some(id));
            assert_eq!(device.info().slow_port, slow);
            assert_eq!(device.settings().samplerate, rate);
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
    }

    #[test]
    fn rejects_unsupported_settings_without_changing_the_previous_values() {
        let mut narrow = device(&SLOGIC16U3, UsbSpeed::High);
        let before = narrow.settings().clone();
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
            assert!(narrow.set_option(key, &value).is_err(), "{key:?}");
            assert_eq!(narrow.settings(), &before);
        }
        assert!(narrow.set_channel_enabled(16, true).is_err());
        assert_eq!(narrow.settings(), &before);
        for limit in [MIN_SAMPLES, MAX_SOFTWARE_DEPTH] {
            narrow
                .set_option(OptionKey::SampleLimit, &OptionValue::Integer(limit))
                .expect("sample limit boundary");
        }
        for voltage in [0.0, 6.0] {
            narrow
                .set_option(OptionKey::Vth, &OptionValue::Number(voltage))
                .expect("threshold boundary");
        }
        let mut wide = device(&SLOGIC32U3, UsbSpeed::SuperPlus);
        let before = wide.settings().clone();
        assert!(wide.set_channel_enabled(32, true).is_err());
        assert_eq!(wide.settings(), &before);
    }

    #[test]
    fn refuses_invalid_capture_plans_and_configuration_during_capture() {
        for model in [&SLOGIC16U3, &SLOGIC32U3] {
            let mut device = device(model, UsbSpeed::Super);
            let (sink, _receiver) = crossbeam_channel::unbounded();
            let advanced = CapturePlan {
                advanced: Some(AdvancedTrigger::Stages { stages: vec![] }),
                ..CapturePlan::default()
            };
            assert!(device.start(advanced, sink.clone()).is_err());
            let outside = CapturePlan {
                triggers: vec![(model.channels.into(), TriggerCondition::Rise)],
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
            select(&mut device, 0);
            assert!(device.start(CapturePlan::default(), sink).is_err());
        }
    }
}
