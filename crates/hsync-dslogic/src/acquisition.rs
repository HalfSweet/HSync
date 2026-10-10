//! Running a capture: arming the FPGA, the bulk transfer loop with the trigger header, progress
//! and overflow polling, and the stop sequence.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use hsync_core::{ChannelIndex, UiError};
use hsync_driver::usb::BulkIn;
use hsync_driver::{Acquisition, DriverError, EndReason, Packet};

use crate::profiles::{ChannelMode, Profile, UsbSpeed, caps};
use crate::protocol::{CaptureStatus, Control, EP_DATA, ctr0, dest, reg, status, write};
use crate::setting::{
    ATOMIC_SAMPLES, ArmConfig, SAMPLES_ALIGN, SETTING_LEN, TriggerSetup, bytes_per_ms, setting,
    setting_ext32,
};

/// Everything a capture needs, fixed when it starts.
#[derive(Debug, Clone)]
pub(crate) struct AcquisitionParams {
    pub profile: &'static Profile,
    pub mode: &'static ChannelMode,
    pub samplerate: u64,
    pub limit_samples: u64,
    pub enabled: u32,
    pub filter: bool,
    pub external_clock: bool,
    pub clock_falling_edge: bool,
    pub rle: bool,
    pub instant: bool,
    pub looping: bool,
    pub buffer_upload: bool,
    pub trigger: TriggerSetup,
}

impl AcquisitionParams {
    fn arm_config(&self) -> ArmConfig<'_> {
        ArmConfig {
            profile: self.profile,
            mode: self.mode,
            samplerate: self.samplerate,
            limit_samples: self.limit_samples,
            enabled: self.enabled,
            filter: self.filter,
            external_clock: self.external_clock,
            clock_falling_edge: self.clock_falling_edge,
            rle: self.rle,
            instant: self.instant,
            looping: self.looping,
            trigger: &self.trigger,
        }
    }

    fn channels(&self) -> Vec<ChannelIndex> {
        (0..self.mode.channels)
            .filter(|&channel| self.enabled & (1 << channel) != 0)
            .collect()
    }

    fn superspeed(&self) -> bool {
        self.profile.speed == UsbSpeed::Super
    }
}

/// `check_id` of a valid trigger header.
const HEADER_CHECK_ID: u32 = 0x5555_5555;
/// Empty polls before reading progress or overflow (`MAX_EMPTY_POLL`).
const MAX_EMPTY_POLLS: u32 = 16;
/// How long one poll waits for a transfer (DSView's buffer mode poll timeout).
const POLL_INTERVAL: Duration = Duration::from_millis(20);
/// How long a stopped capture may take to hand back its transfers before they are cancelled.
const STOP_GRACE: Duration = Duration::from_millis(500);
/// Transfer size in buffer mode.
const BUFFER_TRANSFER: usize = 1 << 20;
/// Most transfers in flight (`NUM_SIMUL_TRANSFERS`).
const MAX_TRANSFERS: usize = 64;
const ARM_TIMEOUT: Duration = Duration::from_millis(1000);

/// `dsl_header_size()`: models able to run on USB 3 send a 1 KiB header at any speed.
pub(crate) fn header_size(profile: &Profile) -> usize {
    if profile.has(caps::USB30) { 1024 } else { 512 }
}

/// `get_buffer_size()` and `get_number_of_transfers()`: the size and number of data transfers.
/// DSView keeps a single transfer in flight in buffer mode except on Windows; HSync uses the
/// Windows numbers everywhere for faster uploads.
pub(crate) fn transfer_plan(params: &AcquisitionParams) -> (usize, usize) {
    let superspeed = params.superspeed();
    let align = if superspeed { 1024 } else { 512 };
    if !params.mode.stream {
        return (BUFFER_TRANSFER, if superspeed { 16 } else { 4 });
    }
    let per_ms = bytes_per_ms(params.samplerate, params.enabled.count_ones().max(1)) as usize;
    let (single_ms, total_ms) = if superspeed { (10, 40) } else { (20, 100) };
    let size = (single_ms * per_ms).div_ceil(align).max(1) * align;
    let count = ((total_ms * per_ms) as f64 / size as f64).ceil() as usize;
    (size, count.clamp(1, MAX_TRANSFERS))
}

/// `dsl_fpga_arm()` minus building the settings: the control handshake around them.
fn arm(ctl: &Control, config: &ArmConfig<'_>) -> Result<(), DriverError> {
    if config.profile.speed != UsbSpeed::Super {
        ctl.write_byte(dest::WORDWIDE, write::WORDWIDE)?;
    }
    ctl.announce_bulk((SETTING_LEN / 2) as u32)?;
    ctl.wait_status(status::SYS_CLR, "SYS_CLR before arming")?;
    ctl.bulk_write(&setting(config), ARM_TIMEOUT)?;
    if config.profile.has(caps::LA_CH32) {
        ctl.bulk_write(&setting_ext32(config), ARM_TIMEOUT)?;
    }
    ctl.write_byte(dest::INTRDY, write::INTRDY)?;
    if ctl.hw_status()? & status::GPIF_DONE == 0 {
        return Err(DriverError::Protocol(
            "the FPGA did not accept the capture settings".into(),
        ));
    }
    Ok(())
}

/// `dev_acquisition_start()`: arms the FPGA, queues the transfers, starts the capture and hands
/// the transfer loop to a thread.
pub(crate) fn start(
    ctl: Control,
    params: AcquisitionParams,
    running: Arc<AtomicBool>,
    sink: Sender<Packet>,
) -> Result<Box<dyn Acquisition>, DriverError> {
    let config = params.arm_config();
    let actual_bytes = config.actual_bytes();
    ctl.write(dest::STOP, 0, &[])?;
    arm(&ctl, &config)?;

    let header_size = header_size(params.profile);
    let (transfer_size, transfers) = transfer_plan(&params);
    let mut bulk = ctl.usb().bulk_in(EP_DATA)?;
    bulk.submit(header_size);
    for _ in 0..transfers {
        bulk.submit(transfer_size);
    }
    ctl.write(dest::START, 0, &[])?;
    log::info!(
        "capturing {} samples at {} Hz on {} channels ({} transfers of {transfer_size} bytes)",
        params.limit_samples,
        params.samplerate,
        params.enabled.count_ones(),
        transfers
    );

    running.store(true, Ordering::Release);
    let _ = sink.send(Packet::Start {
        channels: params.channels(),
        samplerate: params.samplerate,
    });
    let stop = Arc::new(AtomicBool::new(false));
    let worker = Worker {
        ctl,
        params,
        bulk,
        sink,
        stop: Arc::clone(&stop),
        header_size,
        transfer_size,
        actual_bytes,
        force_ready_sent: false,
    };
    let thread = std::thread::Builder::new()
        .name("dslogic-acquisition".into())
        .spawn(move || {
            let (sink, reason) = worker.run();
            running.store(false, Ordering::Release);
            let _ = sink.send(Packet::End(reason));
        })
        .map_err(|error| DriverError::Protocol(format!("starting the capture thread: {error}")))?;
    Ok(Box::new(DslAcquisition {
        stop,
        thread: Some(thread),
    }))
}

struct DslAcquisition {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Acquisition for DslAcquisition {
    fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    fn wait(mut self: Box<Self>) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for DslAcquisition {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

/// What the trigger header said.
struct Header {
    trigger: Option<u64>,
    remain: u64,
}

fn parse_header(data: &[u8], expected_len: usize) -> Option<Header> {
    let word = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(
            *data.get(at..at + 4)?.first_chunk::<4>()?,
        ))
    };
    if data.len() != expected_len || word(0)? != HEADER_CHECK_ID {
        return None;
    }
    let real_pos = word(4)?;
    let remain = (u64::from(word(16)?) << 32) | u64::from(word(12)?);
    let triggered = word(20)? & 1 != 0;
    log::debug!(
        "trigger header: real_pos {real_pos}, ram_saddr {:#x}, remain {remain}, status {:#x}",
        word(8)?,
        word(20)?
    );
    Some(Header {
        trigger: triggered.then_some(u64::from(real_pos)),
        remain,
    })
}

struct Worker {
    ctl: Control,
    params: AcquisitionParams,
    bulk: Box<dyn BulkIn>,
    sink: Sender<Packet>,
    stop: Arc<AtomicBool>,
    header_size: usize,
    transfer_size: usize,
    actual_bytes: u64,
    force_ready_sent: bool,
}

/// How the transfer loop is stopping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stopping {
    No,
    /// Buffer mode, stopped before the buffer filled: the device uploads what it has.
    Upload,
    /// Data is no longer wanted; waiting for the transfers to come back.
    Abort {
        since: Instant,
    },
}

impl Worker {
    /// Runs the transfer loop and the stop sequence. Returns the sink and the reason to send
    /// with [`Packet::End`].
    fn run(mut self) -> (Sender<Packet>, EndReason) {
        let reason = self.receive();
        self.finish(&reason);
        (self.sink, reason)
    }

    fn send(&self, packet: Packet) {
        let _ = self.sink.send(packet);
    }

    fn force_ready(&mut self) {
        if let Err(error) = self.ctl.write_reg(reg::CTR0, ctr0::FORCE_RDY) {
            log::warn!("forcing the capture to end failed: {error}");
        }
        self.force_ready_sent = true;
    }

    fn receive(&mut self) -> EndReason {
        let mut stopping = Stopping::No;
        let mut awaiting_header = true;
        let mut forwarded: u64 = 0;
        let mut empty_polls = 0;
        let mut overflowed = false;
        loop {
            if stopping == Stopping::No && (self.stop.load(Ordering::Acquire) || overflowed) {
                if awaiting_header
                    && !self.params.mode.stream
                    && self.params.buffer_upload
                    && !overflowed
                {
                    // SR_CONF_WAIT_UPLOAD: end the capture early and upload what it holds.
                    if let Err(error) = self.ctl.write_reg(reg::CTR0, ctr0::FORCE_STOP) {
                        log::warn!("stopping the capture failed: {error}");
                    }
                    stopping = Stopping::Upload;
                } else {
                    self.force_ready();
                    stopping = Stopping::Abort {
                        since: Instant::now(),
                    };
                }
            }
            let stopped = if overflowed {
                EndReason::Failed(UiError::new("error.device.overflow"))
            } else {
                EndReason::Stopped
            };
            if let Stopping::Abort { since } = stopping
                && (since.elapsed() >= STOP_GRACE || self.bulk.pending() == 0)
            {
                return stopped;
            }

            match self.bulk.next(POLL_INTERVAL) {
                Some(Ok(data)) => {
                    empty_polls = 0;
                    if matches!(stopping, Stopping::Abort { .. }) {
                        continue;
                    }
                    if awaiting_header {
                        awaiting_header = false;
                        let Some(header) = parse_header(&data, self.header_size) else {
                            return if stopping == Stopping::No {
                                EndReason::Failed(UiError::from(&DriverError::Protocol(
                                    "invalid trigger header".into(),
                                )))
                            } else {
                                EndReason::Stopped
                            };
                        };
                        if !self.accept_header(&header, stopping == Stopping::Upload) {
                            return if stopping == Stopping::No {
                                EndReason::Failed(UiError::from(&DriverError::Protocol(
                                    "the capture holds no samples".into(),
                                )))
                            } else {
                                EndReason::Stopped
                            };
                        }
                        continue;
                    }
                    let mut data = data;
                    if !self.params.looping {
                        let remaining = self.actual_bytes.saturating_sub(forwarded);
                        data.truncate(usize::try_from(remaining).unwrap_or(usize::MAX));
                    }
                    forwarded += data.len() as u64;
                    if !data.is_empty() {
                        self.send(Packet::Logic(data));
                    }
                    if !self.params.looping && forwarded >= self.actual_bytes {
                        return if stopping == Stopping::Upload {
                            EndReason::Stopped
                        } else {
                            EndReason::Completed
                        };
                    }
                    self.bulk.submit(self.transfer_size);
                }
                Some(Err(error)) => {
                    if error.is_disconnected() {
                        return EndReason::Detached;
                    }
                    if matches!(stopping, Stopping::Abort { .. }) {
                        return stopped;
                    }
                    return EndReason::Failed(UiError::from(&DriverError::Usb(error)));
                }
                None => {
                    if matches!(stopping, Stopping::Abort { .. }) {
                        continue;
                    }
                    empty_polls += 1;
                    if empty_polls > MAX_EMPTY_POLLS {
                        empty_polls = 0;
                        if awaiting_header {
                            self.report_progress();
                        }
                        if self.params.mode.stream && self.check_overflow() {
                            self.send(Packet::Overflow);
                            overflowed = true;
                        }
                    }
                }
            }
        }
    }

    /// `receive_header()`: reports the trigger and, when the device ended the capture early,
    /// shortens it. Returns whether data follows.
    fn accept_header(&mut self, header: &Header, uploading: bool) -> bool {
        let limit = self.params.limit_samples;
        if !self.params.mode.stream && header.remain >= limit {
            return false;
        }
        if !self.params.mode.stream || uploading {
            let samples = limit.saturating_sub(header.remain) & !SAMPLES_ALIGN;
            let channels = u64::from(self.params.enabled.count_ones().max(1));
            self.actual_bytes = samples / ATOMIC_SAMPLES * channels * 8;
        }
        if let Some(sample) = header.trigger {
            self.send(Packet::Trigger { sample });
        }
        true
    }

    /// Reads how far the hardware buffer has filled while waiting for the trigger.
    fn report_progress(&self) {
        match self.ctl.read(dest::I2C_STATUS, 0, 4) {
            Ok(data) => {
                if let Some(status) = CaptureStatus::parse(&data) {
                    let limit = self.params.limit_samples;
                    let captured = if status.triggered {
                        limit.saturating_sub(status.count)
                    } else {
                        status.count.min(limit)
                    };
                    self.send(Packet::Progress {
                        triggered: status.triggered,
                        captured,
                    });
                }
            }
            Err(error) => log::debug!("reading the capture progress failed: {error}"),
        }
    }

    fn check_overflow(&self) -> bool {
        match self.ctl.hw_status() {
            Ok(value) => value & status::SYS_OVERFLOW != 0,
            Err(error) => {
                log::debug!("reading the hardware status failed: {error}");
                false
            }
        }
    }

    /// Cancels what is still in flight and leaves the device as DSView does: a capture that
    /// ended on its own gets `FORCE_RDY`; a stopped one, whose `FORCE_RDY` went out already,
    /// gets `STOP`.
    fn finish(&mut self, reason: &EndReason) {
        self.bulk.cancel_all();
        let deadline = Instant::now() + STOP_GRACE;
        while self.bulk.pending() > 0 && Instant::now() < deadline {
            let _ = self.bulk.next(POLL_INTERVAL);
        }
        if matches!(reason, EndReason::Detached) {
            return;
        }
        if self.force_ready_sent {
            if let Err(error) = self.ctl.write(dest::STOP, 0, &[]) {
                log::warn!("stopping the acquisition failed: {error}");
            }
            let _ = self.ctl.read_reg(reg::HW_STATUS);
        } else {
            self.force_ready();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::{CHANNEL_MODES, VENDOR_ID, find_profile};

    fn params(
        pid: u16,
        speed: UsbSpeed,
        mode: usize,
        samplerate: u64,
        enabled: u32,
    ) -> AcquisitionParams {
        AcquisitionParams {
            profile: find_profile(VENDOR_ID, pid, speed).expect("profile"),
            mode: &CHANNEL_MODES[mode],
            samplerate,
            limit_samples: 1 << 20,
            enabled,
            filter: false,
            external_clock: false,
            clock_falling_edge: false,
            rle: false,
            instant: false,
            looping: false,
            buffer_upload: true,
            trigger: TriggerSetup::default(),
        }
    }

    #[test]
    fn plans_stream_transfers_by_bandwidth() {
        // 12 channels at 25 MHz: 37 500 bytes per ms, 20 ms per transfer on USB 2.
        let plan = transfer_plan(&params(0x0020, UsbSpeed::High, 1, 25_000_000, 0x0fff));
        assert_eq!(plan, (750_080, 5));
        let slow = transfer_plan(&params(0x0020, UsbSpeed::High, 0, 10, 1));
        assert_eq!(slow, (512, 1));
        let fast = transfer_plan(&params(0x002A, UsbSpeed::Super, 16, 125_000_000, 0xffff));
        assert_eq!(fast, (2_500_608, 4));
    }

    #[test]
    fn plans_buffer_uploads() {
        assert_eq!(
            transfer_plan(&params(0x0020, UsbSpeed::High, 20, 100_000_000, 0xffff)),
            (1 << 20, 4)
        );
        assert_eq!(
            transfer_plan(&params(0x002C, UsbSpeed::Super, 23, 250_000_000, u32::MAX)),
            (1 << 20, 16)
        );
    }

    #[test]
    fn parses_trigger_headers() {
        let mut data = vec![0u8; 512];
        data[0..4].copy_from_slice(&HEADER_CHECK_ID.to_le_bytes());
        data[4..8].copy_from_slice(&1234u32.to_le_bytes());
        data[12..16].copy_from_slice(&5u32.to_le_bytes());
        data[16..20].copy_from_slice(&1u32.to_le_bytes());
        data[20..24].copy_from_slice(&1u32.to_le_bytes());
        let header = parse_header(&data, 512).expect("header");
        assert_eq!(header.trigger, Some(1234));
        assert_eq!(header.remain, (1 << 32) | 5);
        assert!(parse_header(&data, 1024).is_none());
        data[0] = 0;
        assert!(parse_header(&data, 512).is_none());
    }
}
