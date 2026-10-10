//! The demo driver and device (DESIGN.md §4.10).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use crossbeam_channel::Sender;
use hsync_core::ChannelIndex;
use hsync_driver::{
    Acquisition, Candidate, CandidateStatus, CapturePlan, Choice, Device, DeviceInfo, Driver,
    DriverError, Label, OptionKey, OptionKind, OptionState, OptionValue, Packet,
};

use crate::acquisition::{self, DemoParams};
use crate::pattern::{CHANNELS, Pattern, PatternKind, ReplaySource};

/// Driver name, as DSView names its demo driver.
pub const DRIVER_NAME: &str = "virtual-demo";
const DEVICE_ID: &str = "virtual-demo";

/// Sample rates the demo offers.
pub const SAMPLERATES: &[u64] = &[
    1_000,
    2_000,
    5_000,
    10_000,
    20_000,
    50_000,
    100_000,
    200_000,
    500_000,
    1_000_000,
    2_000_000,
    5_000_000,
    10_000_000,
    20_000_000,
    25_000_000,
    50_000_000,
    100_000_000,
];
const MIN_SAMPLES: u64 = 1024;
const MAX_SAMPLES: u64 = 1 << 30;

/// Offers one demo device that replays the capture at `protocol`, when given and readable, or
/// synthesises signals.
#[derive(Debug, Default)]
pub struct DemoDriver {
    protocol: Option<PathBuf>,
    source: OnceLock<Option<Arc<ReplaySource>>>,
}

impl DemoDriver {
    pub fn new(protocol: Option<PathBuf>) -> Self {
        Self {
            protocol,
            source: OnceLock::new(),
        }
    }

    fn source(&self) -> Option<Arc<ReplaySource>> {
        self.source
            .get_or_init(|| {
                let path = self.protocol.as_ref()?;
                match hsync_fileio::read_dsl_file(path) {
                    Ok(document) => ReplaySource::from_capture(&document.capture).map(Arc::new),
                    Err(error) => {
                        log::warn!("reading the demo capture {}: {error}", path.display());
                        None
                    }
                }
            })
            .clone()
    }
}

impl Driver for DemoDriver {
    fn name(&self) -> &'static str {
        DRIVER_NAME
    }

    fn virtual_devices(&self) -> Vec<Candidate> {
        vec![Candidate {
            id: DEVICE_ID.to_owned(),
            driver: DRIVER_NAME,
            vendor: "HSync".to_owned(),
            model: "Demo".to_owned(),
            status: CandidateStatus::Ready,
            usb: None,
        }]
    }

    fn open(&self, _candidate: &Candidate) -> Result<Box<dyn Device>, DriverError> {
        Ok(Box::new(DemoDevice::new(self.source())))
    }
}

/// The demo device.
#[derive(Debug)]
pub struct DemoDevice {
    source: Option<Arc<ReplaySource>>,
    pattern: PatternKind,
    samplerate: u64,
    limit_samples: u64,
    enabled: u16,
    running: Arc<AtomicBool>,
    captures: AtomicU64,
}

impl DemoDevice {
    /// A device that replays `source` (at its own rate and length) or, without one, produces
    /// random signals.
    pub fn new(source: Option<Arc<ReplaySource>>) -> Self {
        let (pattern, samplerate, limit_samples) = match &source {
            Some(source) => (
                PatternKind::Protocol,
                source.samplerate(),
                source.samples().max(MIN_SAMPLES),
            ),
            None => (PatternKind::Random, 1_000_000, 1 << 20),
        };
        Self {
            source,
            pattern,
            samplerate,
            limit_samples,
            enabled: u16::MAX,
            running: Arc::new(AtomicBool::new(false)),
            captures: AtomicU64::new(0),
        }
    }

    fn samplerates(&self) -> Vec<u64> {
        let mut rates = SAMPLERATES.to_vec();
        if let Some(source) = &self.source
            && !rates.contains(&source.samplerate())
        {
            rates.push(source.samplerate());
            rates.sort_unstable();
        }
        rates
    }

    fn ensure_idle(&self) -> Result<(), DriverError> {
        if self.running.load(Ordering::Acquire) {
            return Err(DriverError::InvalidRequest("a capture is running".into()));
        }
        Ok(())
    }
}

impl Device for DemoDevice {
    fn info(&self) -> DeviceInfo {
        DeviceInfo {
            id: DEVICE_ID.to_owned(),
            driver: DRIVER_NAME.to_owned(),
            vendor: "HSync".to_owned(),
            model: "Demo".to_owned(),
            firmware: None,
            usb_speed: None,
        }
    }

    fn options(&self) -> Vec<OptionState> {
        let kinds = PatternKind::ALL
            .into_iter()
            .filter(|kind| *kind != PatternKind::Protocol || self.source.is_some());
        vec![
            OptionState {
                key: OptionKey::Pattern,
                kind: OptionKind::Choice {
                    choices: kinds
                        .map(|kind| Choice {
                            id: kind.id().to_owned(),
                            label: Label::new(format!("device.pattern.{}", kind.id())),
                        })
                        .collect(),
                },
                value: OptionValue::Choice(self.pattern.id().to_owned()),
                enabled: true,
            },
            OptionState {
                key: OptionKey::Samplerate,
                kind: OptionKind::Integers {
                    values: self.samplerates(),
                },
                value: OptionValue::Integer(self.samplerate),
                enabled: true,
            },
            OptionState {
                key: OptionKey::SampleLimit,
                kind: OptionKind::Range {
                    min: MIN_SAMPLES as f64,
                    max: MAX_SAMPLES as f64,
                    step: 1024.0,
                },
                value: OptionValue::Integer(self.limit_samples),
                enabled: true,
            },
        ]
    }

    fn set_option(&mut self, key: OptionKey, value: &OptionValue) -> Result<(), DriverError> {
        self.ensure_idle()?;
        match key {
            OptionKey::Pattern => {
                let kind = value
                    .as_choice()
                    .and_then(PatternKind::from_id)
                    .filter(|kind| *kind != PatternKind::Protocol || self.source.is_some())
                    .ok_or_else(|| DriverError::InvalidRequest("unknown pattern".into()))?;
                self.pattern = kind;
            }
            OptionKey::Samplerate => {
                let rate = value
                    .as_integer()
                    .filter(|rate| self.samplerates().contains(rate))
                    .ok_or_else(|| DriverError::InvalidRequest("unsupported sample rate".into()))?;
                self.samplerate = rate;
            }
            OptionKey::SampleLimit => {
                let limit = value.as_integer().ok_or_else(|| {
                    DriverError::InvalidRequest("the sample limit takes an integer".into())
                })?;
                self.limit_samples = limit.clamp(MIN_SAMPLES, MAX_SAMPLES);
            }
            other => {
                return Err(DriverError::InvalidRequest(format!(
                    "the demo device has no {other:?} option"
                )));
            }
        }
        Ok(())
    }

    fn channel_count(&self) -> u16 {
        CHANNELS
    }

    /// The demo streams like a device in Stream mode, so it loops with any options.
    fn can_loop(&self) -> bool {
        true
    }

    fn enabled_channels(&self) -> Vec<ChannelIndex> {
        (0..CHANNELS)
            .filter(|&channel| self.enabled & (1 << channel) != 0)
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
            self.enabled |= 1 << channel;
        } else {
            self.enabled &= !(1 << channel);
        }
        Ok(())
    }

    fn start(
        &mut self,
        plan: CapturePlan,
        sink: Sender<Packet>,
    ) -> Result<Box<dyn Acquisition>, DriverError> {
        self.ensure_idle()?;
        if plan.advanced.is_some() {
            // DSView offers virtual devices the simple trigger only.
            return Err(DriverError::InvalidRequest(
                "the demo device has simple triggers only".into(),
            ));
        }
        let channels = self.enabled_channels();
        if channels.is_empty() {
            return Err(DriverError::InvalidRequest("no channel is enabled".into()));
        }
        // As the hardware does: whole kilo-samples, the trigger at least one word in and never
        // past the last word.
        let samples = (self.limit_samples + 1023) & !1023;
        let triggers: Vec<(usize, hsync_core::TriggerCondition)> = if plan.instant {
            Vec::new()
        } else {
            plan.triggers
                .iter()
                .filter(|(_, condition)| *condition != hsync_core::TriggerCondition::None)
                .filter_map(|&(channel, condition)| {
                    channels
                        .iter()
                        .position(|&enabled| enabled == channel)
                        .map(|index| (index, condition))
                })
                .collect()
        };
        let requested = (f64::from(plan.trigger_position.min(100)) / 100.0 * samples as f64) as u64;
        let pre_trigger = requested.clamp(64, samples - 64) & !63;
        let seed = 0x9e37_79b9_7f4a_7c15u64
            .wrapping_mul(self.captures.fetch_add(1, Ordering::Relaxed) + 1);
        let params = DemoParams {
            pattern: Pattern::new(self.pattern, self.source.as_ref(), seed),
            channels,
            samplerate: self.samplerate,
            samples,
            triggers,
            pre_trigger,
            looping: plan.looping,
        };
        Ok(acquisition::start(params, Arc::clone(&self.running), sink))
    }
}
