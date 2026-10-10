//! The devices of a workspace: finding and opening them, their settings, and captures that
//! stream from them into the workspace.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::Receiver;
use hsync_capture::LogicCapture;
use hsync_core::{ChannelIndex, TriggerCondition, UiError};
use hsync_driver::{
    Acquisition, AdvancedTrigger, Candidate, CandidateStatus, CapturePlan, CapturePreset, Device,
    DeviceInfo, DeviceManager, DeviceSummary, DriverError, EndReason, OptionKey, OptionState,
    OptionValue, Packet, TriggerCaps,
};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{CaptureSource, CaptureSummary, SessionError, Verdict, Workspace};

/// How long a device may take to come back after receiving its firmware.
const RESTART_TIMEOUT: Duration = Duration::from_secs(8);
const RESTART_POLL: Duration = Duration::from_millis(250);
/// How long a driver may take to announce a capture it started.
const START_TIMEOUT: Duration = Duration::from_secs(2);
/// Least time between two progress events of a running capture.
const STATUS_INTERVAL: Duration = Duration::from_millis(50);

/// The open device as the UI presents it.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeviceState {
    pub info: DeviceInfo,
    pub options: Vec<OptionState>,
    /// Channels of the current mode.
    pub channel_count: u16,
    /// Channels that will be captured, ascending.
    pub enabled_channels: Vec<ChannelIndex>,
    /// Whether a capture is running; settings are locked meanwhile.
    pub capturing: bool,
    /// What the trigger can do with the current options.
    pub trigger: TriggerCaps,
    /// Whether captures can loop with the current options.
    pub can_loop: bool,
    /// Samples per channel the device's memory holds (Buffer mode); longer captures use RLE.
    pub hardware_depth: Option<u64>,
}

/// A capture request from the UI (DESIGN.md §7).
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CaptureRequest {
    /// Per-channel trigger conditions; all must hold at once.
    pub triggers: Vec<(ChannelIndex, TriggerCondition)>,
    /// Share of the capture before the trigger, in percent.
    pub trigger_position: u8,
    /// Start at once and ignore the trigger.
    pub instant: bool,
    /// A multi-stage or serial trigger in place of `triggers`.
    pub advanced: Option<AdvancedTrigger>,
    /// Capture until stopped, keeping the newest samples up to the sample limit.
    pub looping: bool,
    /// Keep the shown capture shown until this one has ended, as repeated captures do.
    pub hold: bool,
    /// A held-behind capture stopped early replaces the shown one all the same (DSView's
    /// "update the latest data when stopped in repeat mode"); otherwise it is dropped.
    pub keep_stopped: bool,
    /// Channel names and decoders for a capture that follows no other, in place of the
    /// device's own preset: those remembered for the device, or of a session.
    pub preset: Option<CapturePreset>,
}

/// Where a running capture is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LivePhase {
    /// Armed, waiting for the trigger.
    Waiting,
    /// Samples are arriving.
    Capturing,
    /// The capture is over.
    Ended,
}

/// Why a capture ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EndKind {
    Completed,
    Stopped,
    Detached,
    Failed,
}

/// The state of a capture streaming from a device.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LiveStatus {
    pub capture_id: u64,
    pub phase: LivePhase,
    /// Samples received per channel.
    pub samples: u64,
    /// Samples requested per channel.
    pub limit: u64,
    pub trigger_sample: Option<u64>,
    /// While waiting: whether the hardware saw the trigger and how many samples it holds.
    pub hardware_triggered: bool,
    pub hardware_samples: Option<u64>,
    /// The device lost data because the host did not keep up.
    pub overflow: bool,
    /// Samples a loop capture has dropped from its start to stay within `limit`.
    pub dropped: u64,
    pub end: Option<EndKind>,
    pub error: Option<UiError>,
}

/// What a workspace tells the UI without being asked.
#[derive(Debug, Clone)]
pub enum SessionEvent {
    /// A capture from a device progressed or ended.
    Live(LiveStatus),
    /// Decoder `index` of capture `capture_id` finished.
    Decoded { capture_id: u64, index: u32 },
}

/// Receives [`SessionEvent`]s; called from background threads.
pub type EventSink = Arc<dyn Fn(SessionEvent) + Send + Sync>;

struct OpenDevice {
    id: String,
    device: Box<dyn Device>,
}

struct Running {
    capture_id: u64,
    acquisition: Arc<Mutex<Option<Box<dyn Acquisition>>>>,
    ended: Arc<std::sync::atomic::AtomicBool>,
}

/// The devices of one window.
pub struct Instrument {
    manager: DeviceManager,
    candidates: Vec<Candidate>,
    open: Option<OpenDevice>,
    running: Option<Running>,
}

impl std::fmt::Debug for Instrument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Instrument")
            .field("manager", &self.manager)
            .field("open", &self.open.as_ref().map(|open| &open.id))
            .finish_non_exhaustive()
    }
}

fn driver_error(error: &DriverError) -> SessionError {
    SessionError::Device(UiError::from(error))
}

fn now_ms() -> Option<i64> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(elapsed.as_millis()).ok()
}

impl Instrument {
    pub fn new(manager: DeviceManager) -> Self {
        Self {
            manager,
            candidates: Vec::new(),
            open: None,
            running: None,
        }
    }

    /// Scans for devices.
    pub fn scan(&mut self) -> Vec<DeviceSummary> {
        self.candidates = self.manager.scan();
        self.candidates.iter().map(Candidate::summary).collect()
    }

    /// Devices of the last scan.
    pub fn devices(&self) -> Vec<DeviceSummary> {
        self.candidates.iter().map(Candidate::summary).collect()
    }

    /// The id of the open device.
    pub fn open_id(&self) -> Option<&str> {
        self.open.as_ref().map(|open| open.id.as_str())
    }

    fn candidate(&self, id: &str) -> Option<&Candidate> {
        self.candidates.iter().find(|candidate| candidate.id == id)
    }

    /// Opens device `id` from the last scan, closing the open one. A device that first needs
    /// firmware gets it, and is opened once it is back.
    pub fn open(&mut self, id: &str) -> Result<DeviceState, SessionError> {
        if self.is_capturing() {
            return Err(SessionError::Busy);
        }
        if self.candidate(id).is_none() {
            self.scan();
        }
        let candidate = self
            .candidate(id)
            .cloned()
            .ok_or_else(|| SessionError::UnknownDevice(id.to_owned()))?;
        self.open = None;
        let device = match self.manager.open(&candidate) {
            Err(DriverError::Restarting) => self.open_after_restart(id)?,
            other => other.map_err(|error| driver_error(&error))?,
        };
        log::info!("opened device {id}");
        self.open = Some(OpenDevice {
            id: id.to_owned(),
            device,
        });
        self.state().ok_or(SessionError::NoDevice)
    }

    fn open_after_restart(&mut self, id: &str) -> Result<Box<dyn Device>, SessionError> {
        let deadline = Instant::now() + RESTART_TIMEOUT;
        while Instant::now() < deadline {
            std::thread::sleep(RESTART_POLL);
            self.scan();
            if let Some(candidate) = self.candidate(id)
                && candidate.status == CandidateStatus::Ready
            {
                let candidate = candidate.clone();
                return self
                    .manager
                    .open(&candidate)
                    .map_err(|error| driver_error(&error));
            }
        }
        Err(driver_error(&DriverError::Timeout("the device restarting")))
    }

    /// Closes the open device unless it is capturing.
    pub fn close(&mut self) -> Result<(), SessionError> {
        if self.is_capturing() {
            return Err(SessionError::Busy);
        }
        self.open = None;
        Ok(())
    }

    /// Forgets the open device if a new scan no longer finds it; returns whether it did.
    pub fn drop_if_gone(&mut self) -> bool {
        let gone = self
            .open
            .as_ref()
            .is_some_and(|open| self.candidate(&open.id).is_none());
        if gone && !self.is_capturing() {
            log::info!("the open device went away");
            self.open = None;
            return true;
        }
        false
    }

    /// Whether a capture is running.
    pub fn is_capturing(&self) -> bool {
        self.running
            .as_ref()
            .is_some_and(|running| !running.ended.load(std::sync::atomic::Ordering::Acquire))
    }

    /// The open device as the UI shows it.
    pub fn state(&self) -> Option<DeviceState> {
        let open = self.open.as_ref()?;
        Some(DeviceState {
            info: open.device.info(),
            options: open.device.options(),
            channel_count: open.device.channel_count(),
            enabled_channels: open.device.enabled_channels(),
            capturing: self.is_capturing(),
            trigger: open.device.trigger_caps(),
            can_loop: open.device.can_loop(),
            hardware_depth: open.device.hardware_depth(),
        })
    }

    fn device_mut(&mut self) -> Result<&mut Box<dyn Device>, SessionError> {
        if self.is_capturing() {
            return Err(SessionError::Busy);
        }
        self.open
            .as_mut()
            .map(|open| &mut open.device)
            .ok_or(SessionError::NoDevice)
    }

    /// Changes an option of the open device.
    pub fn set_option(
        &mut self,
        key: OptionKey,
        value: &OptionValue,
    ) -> Result<DeviceState, SessionError> {
        self.device_mut()?
            .set_option(key, value)
            .map_err(|error| driver_error(&error))?;
        self.state().ok_or(SessionError::NoDevice)
    }

    /// Enables or disables a channel of the open device.
    pub fn set_channel_enabled(
        &mut self,
        channel: ChannelIndex,
        enabled: bool,
    ) -> Result<DeviceState, SessionError> {
        self.device_mut()?
            .set_channel_enabled(channel, enabled)
            .map_err(|error| driver_error(&error))?;
        self.state().ok_or(SessionError::NoDevice)
    }

    /// Starts a capture on the open device. Its samples stream into a new capture of
    /// `workspace`, whose summary is returned; progress arrives through `events`.
    pub fn start(
        &mut self,
        workspace: &mut Workspace,
        request: CaptureRequest,
        events: EventSink,
    ) -> Result<CaptureSummary, SessionError> {
        let open = self.open.as_mut().ok_or(SessionError::NoDevice)?;
        if self
            .running
            .as_ref()
            .is_some_and(|running| !running.ended.load(std::sync::atomic::Ordering::Acquire))
        {
            return Err(SessionError::Busy);
        }
        let limit = open
            .device
            .options()
            .iter()
            .find(|option| option.key == OptionKey::SampleLimit)
            .and_then(|option| option.value.as_integer())
            .map_or(0, |limit| (limit + 1023) & !1023);
        let waits = !request.instant
            && (request.advanced.is_some()
                || request
                    .triggers
                    .iter()
                    .any(|(_, condition)| *condition != TriggerCondition::None));
        let looping = request.looping;
        let plan = CapturePlan {
            triggers: request.triggers,
            trigger_position: request.trigger_position,
            instant: request.instant,
            advanced: request.advanced,
            looping: request.looping,
        };
        let (sink, packets) = crossbeam_channel::unbounded();
        let acquisition = open
            .device
            .start(plan, sink)
            .map_err(|error| driver_error(&error))?;
        let (channels, samplerate) = match packets.recv_timeout(START_TIMEOUT) {
            Ok(Packet::Start {
                channels,
                samplerate,
            }) => (channels, samplerate),
            other => {
                acquisition.stop();
                log::warn!("the capture did not start: {other:?}");
                return Err(driver_error(&DriverError::Protocol(
                    "the capture did not start".into(),
                )));
            }
        };
        let info = open.device.info();
        let source = CaptureSource::Device {
            driver: info.driver,
            model: info.model,
        };
        let live = workspace.begin_live(
            source,
            LogicCapture::new(samplerate, channels),
            request.hold,
            request.preset.or_else(|| open.device.preset()),
        );
        let (summary, capture) = (live.summary, live.samples);
        let acquisition = Arc::new(Mutex::new(Some(acquisition)));
        let ended = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let status = LiveStatus {
            capture_id: summary.id,
            phase: if waits {
                LivePhase::Waiting
            } else {
                LivePhase::Capturing
            },
            samples: 0,
            limit,
            trigger_sample: None,
            hardware_triggered: false,
            hardware_samples: None,
            overflow: false,
            dropped: 0,
            end: None,
            error: None,
        };
        events(SessionEvent::Live(status.clone()));
        let ingest = Ingest {
            capture,
            looping,
            verdict: live.verdict,
            keep_stopped: request.keep_stopped,
            decode: Some(live.decode),
            packets,
            acquisition: Arc::clone(&acquisition),
            ended: Arc::clone(&ended),
            events,
            status,
        };
        std::thread::Builder::new()
            .name("capture-ingest".into())
            .spawn(move || ingest.run())
            .map_err(|error| driver_error(&DriverError::Protocol(error.to_string())))?;
        self.running = Some(Running {
            capture_id: summary.id,
            acquisition,
            ended,
        });
        Ok(summary)
    }

    /// Asks the running capture to stop; it ends with [`EndKind::Stopped`].
    pub fn stop(&self) {
        if let Some(running) = &self.running
            && let Some(acquisition) = running.acquisition.lock().as_ref()
        {
            log::info!("stopping capture {}", running.capture_id);
            acquisition.stop();
        }
    }
}

/// Moves the packets of a running capture into its samples.
struct Ingest {
    capture: Arc<RwLock<LogicCapture>>,
    /// Keep only the newest `status.limit` samples.
    looping: bool,
    /// For a capture taken behind a held one: where to say whether it replaces it.
    verdict: Option<Verdict>,
    /// Replace the held capture even when stopped early.
    keep_stopped: bool,
    /// Decodes the capture once it is complete.
    decode: Option<Box<dyn FnOnce() + Send>>,
    packets: Receiver<Packet>,
    acquisition: Arc<Mutex<Option<Box<dyn Acquisition>>>>,
    ended: Arc<std::sync::atomic::AtomicBool>,
    events: EventSink,
    status: LiveStatus,
}

impl Ingest {
    fn emit(&self) {
        (self.events)(SessionEvent::Live(self.status.clone()));
    }

    fn run(mut self) {
        let mut deinterleaver = self.capture.read().deinterleaver();
        let mut last_emit = Instant::now();
        let reason = loop {
            let Ok(packet) = self.packets.recv() else {
                break EndReason::Failed(UiError::new("error.device.lost"));
            };
            match packet {
                Packet::Start { .. } => {}
                Packet::Logic(bytes) => {
                    let mut capture = self.capture.write();
                    capture.append_cross(&mut deinterleaver, &bytes);
                    if self.looping && self.status.limit > 0 {
                        // Whole words, as the samples arrive.
                        let excess = capture.len().saturating_sub(self.status.limit) / 64 * 64;
                        if excess > 0 {
                            capture.drop_front(excess);
                            self.status.dropped += excess;
                            if self.status.trigger_sample.is_some() {
                                self.status.trigger_sample = capture.trigger_sample();
                            }
                        }
                    }
                    self.status.samples = capture.len();
                    drop(capture);
                    let first = self.status.phase == LivePhase::Waiting;
                    self.status.phase = LivePhase::Capturing;
                    if first || last_emit.elapsed() >= STATUS_INTERVAL {
                        last_emit = Instant::now();
                        self.emit();
                    }
                }
                Packet::Trigger { sample } => {
                    // Counted from the start of the capture, before anything was dropped.
                    let sample = sample.checked_sub(self.status.dropped);
                    self.capture.write().set_trigger(sample, now_ms());
                    self.status.trigger_sample = sample;
                    self.status.phase = LivePhase::Capturing;
                    self.emit();
                }
                Packet::Progress {
                    triggered,
                    captured,
                } => {
                    self.status.hardware_triggered = triggered;
                    self.status.hardware_samples = Some(captured);
                    self.emit();
                }
                Packet::Overflow => {
                    self.status.overflow = true;
                    self.emit();
                }
                Packet::End(reason) => break reason,
            }
        };
        if let Some(acquisition) = self.acquisition.lock().take() {
            acquisition.wait();
        }
        if self.status.dropped > 0 {
            // Saving reads whole chunks, which must line up with the samples again.
            self.capture.write().compact();
        }
        let (end, error) = match reason {
            EndReason::Completed => (EndKind::Completed, None),
            EndReason::Stopped => (EndKind::Stopped, None),
            EndReason::Detached => (
                EndKind::Detached,
                Some(UiError::new("error.device.detached")),
            ),
            EndReason::Failed(error) => (EndKind::Failed, Some(error)),
        };
        log::info!(
            "capture {} ended ({end:?}) with {} samples",
            self.status.capture_id,
            self.status.samples
        );
        self.status.samples = self.capture.read().len();
        self.status.phase = LivePhase::Ended;
        self.status.end = Some(end);
        self.status.error = error;
        // Decided before anyone hears of the end, so the workspace already shows the outcome.
        let shown = end != EndKind::Stopped || self.keep_stopped;
        if let Some(verdict) = &self.verdict {
            let _ = verdict.set(shown);
        }
        self.ended.store(true, std::sync::atomic::Ordering::Release);
        self.emit();
        if (self.verdict.is_none() || shown)
            && let Some(decode) = self.decode.take()
        {
            decode();
        }
    }
}
