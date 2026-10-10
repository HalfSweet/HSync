//! Running a capture (`Session::OrderStart()`, `ThreadWork::DeviceRecvThread()`,
//! `ThreadRead`): sending the settings and the trigger, the transfer loop, the frames that
//! carry samples and progress, and the stop sequence.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use hsync_core::{ChannelIndex, TriggerCondition, UiError};
use hsync_driver::usb::{BulkIn, UsbError};
use hsync_driver::{Acquisition, DriverError, EndReason, Packet};

use crate::assemble::Assembler;
use crate::models::CHANNELS;
use crate::protocol::{
    CaptureSettings, EP_IN, Frame, FrameReader, Link, MCU_PACKET, cmd, frame, mcu, settings_args,
    trigger_args, u40,
};

/// Everything a capture needs, fixed when it starts.
#[derive(Debug, Clone)]
pub(crate) struct AcquisitionParams {
    pub stream: bool,
    pub rle: bool,
    pub samplerate: u64,
    pub rate_code: u8,
    /// Samples per channel to forward.
    pub limit_samples: u64,
    /// Samples per channel the device is asked for: the limit, or as many as it takes for a
    /// loop capture.
    pub depth: u64,
    /// Samples before the trigger.
    pub pre_trigger: u64,
    pub enabled: u16,
    pub vth: f64,
    pub triggers: Vec<(ChannelIndex, TriggerCondition)>,
    pub instant: bool,
    pub looping: bool,
}

impl AcquisitionParams {
    fn settings(&self) -> CaptureSettings {
        CaptureSettings {
            buffer: !self.stream,
            rle: self.rle,
            vth: self.vth,
            rate_code: self.rate_code,
            depth: self.depth,
            pre_trigger: self.pre_trigger,
        }
    }

    fn channels(&self) -> Vec<ChannelIndex> {
        (0..CHANNELS)
            .filter(|&channel| self.enabled & (1 << channel) != 0)
            .collect()
    }
}

/// Bytes of a capture transfer (`READ_TRANSFER_BUFFER_SIZE`).
pub(crate) const TRANSFER_SIZE: usize = 16 * 1024;
/// Transfers in flight unless the data comes slowly (`maxTransferSize`).
const TRANSFERS: usize = 16;
/// Transfers in flight at most (`READ_TRANSFER_LENGTH`).
const MAX_TRANSFERS: usize = 40;
/// Bits per second below which fewer transfers are kept in flight.
const SLOW_BITS: f64 = 250_000_000.0;
/// How long one wait for a transfer lasts before the stop request and the timeouts are checked.
const POLL_INTERVAL: Duration = Duration::from_millis(20);
/// How long cancelled transfers may take to come back.
const CANCEL_GRACE: Duration = Duration::from_secs(2);
/// Reads that may find data while clearing the device before a capture; ATK-Logic reads on
/// until one finds nothing.
const START_DRAIN: usize = 2000;
/// Reads that may find data while clearing the device after a capture.
const FINISH_DRAIN: usize = 1000;
/// Most samples per channel and channels the depth of a loop capture covers: ATK-Logic's
/// longest Stream capture (`rleMaxDepth = pow(2, 40) * 1000 - 1` per millisecond).
const LOOP_SAMPLES: u64 = (1 << 40) - 1;

/// The depth a loop capture asks for: the longest ATK-Logic offers with `enabled` channels.
pub(crate) fn loop_depth(enabled: u32) -> u64 {
    LOOP_SAMPLES / u64::from(enabled.max(1))
}

/// Transfers to keep in flight (`ThreadWork::DeviceRecvThread()`): 16, or fewer for captures
/// that move under 250 Mbit/s, scaled out of 40.
pub(crate) fn transfers(params: &AcquisitionParams) -> usize {
    let channels = f64::from(params.enabled.count_ones().max(1));
    let samples = params.limit_samples as f64;
    let seconds = (samples / params.samplerate.max(1) as f64).ceil().max(1.0);
    let bits = samples * channels / seconds;
    if bits < SLOW_BITS {
        ((bits / SLOW_BITS * MAX_TRANSFERS as f64).ceil() as usize).clamp(1, MAX_TRANSFERS)
    } else {
        TRANSFERS
    }
}

/// Starts a capture (`Session::OrderStart()`): wakes the FPGA, clears what the MCU still holds,
/// sends the settings and, once the FPGA has had time to set the threshold, the trigger, which
/// starts the capture. Then hands the transfers to a thread.
pub(crate) fn start(
    link: Link,
    params: AcquisitionParams,
    running: Arc<AtomicBool>,
    sink: Sender<Packet>,
) -> Result<Box<dyn Acquisition>, DriverError> {
    link.mcu(mcu::RESET_STATE, &[1])?;
    link.read(MCU_PACKET)?;
    link.drain(START_DRAIN)?;
    link.command(cmd::SETTINGS, &settings_args(&params.settings()))?;
    link.pause(link.timing().gap);
    link.command(
        cmd::TRIGGER,
        &trigger_args(params.enabled, &params.triggers, params.instant),
    )?;

    let transfers = transfers(&params);
    let mut bulk = link.usb().bulk_in(EP_IN)?;
    let mut submitted = VecDeque::with_capacity(transfers);
    for _ in 0..transfers {
        bulk.submit(TRANSFER_SIZE);
        submitted.push_back(Instant::now());
    }
    log::info!(
        "capturing {} samples at {} Hz on {} channels in {} mode{}",
        params.limit_samples,
        params.samplerate,
        params.enabled.count_ones(),
        if params.stream { "Stream" } else { "Buffer" },
        if params.rle { " with RLE" } else { "" }
    );

    running.store(true, Ordering::Release);
    let channels = params.channels();
    let _ = sink.send(Packet::Start {
        channels: channels.clone(),
        samplerate: params.samplerate,
    });
    let limit = (!params.looping).then_some(params.limit_samples);
    let stop = Arc::new(AtomicBool::new(false));
    let worker = Worker {
        assembler: Assembler::new(&channels, limit),
        link,
        params,
        bulk: Some(bulk),
        submitted,
        transfers,
        sink,
        stop: Arc::clone(&stop),
        reader: FrameReader::new(),
        offset_seen: false,
    };
    let thread = std::thread::Builder::new()
        .name("atklogic-acquisition".into())
        .spawn(move || {
            let (sink, reason) = worker.run();
            running.store(false, Ordering::Release);
            let _ = sink.send(Packet::End(reason));
        })
        .map_err(|error| DriverError::Protocol(format!("starting the capture thread: {error}")))?;
    Ok(Box::new(AtkAcquisition {
        stop,
        thread: Some(thread),
    }))
}

struct AtkAcquisition {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Acquisition for AtkAcquisition {
    fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    fn wait(mut self: Box<Self>) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for AtkAcquisition {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

fn failed(error: DriverError) -> EndReason {
    EndReason::Failed(UiError::from(&error))
}

fn usb_failed(error: UsbError) -> EndReason {
    if error.is_disconnected() {
        EndReason::Detached
    } else {
        failed(DriverError::Usb(error))
    }
}

/// The samples of a data frame: its bytes, or in RLE captures pairs of a count and a byte to
/// repeat. A last count without its byte repeats the zero that follows it in the frame's end.
fn samples(payload: &[u8], rle: bool) -> Vec<u8> {
    let data = payload.get(2..).unwrap_or_default();
    if !rle {
        return data.to_vec();
    }
    let mut out = Vec::new();
    for pair in data.chunks(2) {
        let value = pair.get(1).copied().unwrap_or(0);
        out.resize(out.len() + usize::from(pair[0]), value);
    }
    out
}

struct Worker {
    link: Link,
    params: AcquisitionParams,
    /// The transfer queue, until the capture ends.
    bulk: Option<Box<dyn BulkIn>>,
    /// When each pending transfer was submitted, oldest first.
    submitted: VecDeque<Instant>,
    transfers: usize,
    sink: Sender<Packet>,
    stop: Arc<AtomicBool>,
    reader: FrameReader,
    assembler: Assembler,
    /// The trigger offset arrived; a second one is an error.
    offset_seen: bool,
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

    fn bulk(&mut self) -> &mut dyn BulkIn {
        let Some(bulk) = self.bulk.as_deref_mut() else {
            unreachable!("the transfers outlive the transfer loop");
        };
        bulk
    }

    fn receive(&mut self) -> EndReason {
        let mut last_frame = Instant::now();
        loop {
            if self.stop.load(Ordering::Acquire) {
                return EndReason::Stopped;
            }
            match self.bulk().next(POLL_INTERVAL) {
                Some(Ok(data)) => {
                    self.submitted.pop_front();
                    self.reader.push(&data);
                    self.bulk().submit(TRANSFER_SIZE);
                    self.submitted.push_back(Instant::now());
                }
                Some(Err(error)) => return usb_failed(error),
                None => {
                    let timeout = self.link.timing().transfer;
                    if self
                        .submitted
                        .front()
                        .is_some_and(|since| since.elapsed() >= timeout)
                        && let Err(reason) = self.cut_short()
                    {
                        return reason;
                    }
                }
            }
            while let Some(frame) = self.reader.next_frame() {
                last_frame = Instant::now();
                if let Some(reason) = self.handle(&frame) {
                    self.forward();
                    return reason;
                }
            }
            self.forward();
            if last_frame.elapsed() >= self.link.timing().silence {
                return failed(DriverError::Timeout("the capture data"));
            }
        }
    }

    /// Cuts the pending transfers short, keeping what they hold, as their timeout does in
    /// ATK-Logic: the device sends no short packets, so a transfer fills only when enough data
    /// comes. Then queues them again.
    fn cut_short(&mut self) -> Result<(), EndReason> {
        self.bulk().cancel_all();
        let deadline = Instant::now() + CANCEL_GRACE;
        while self.bulk().pending() > 0 {
            match self.bulk().next(POLL_INTERVAL) {
                Some(Ok(data)) => self.reader.push(&data),
                Some(Err(error)) => return Err(usb_failed(error)),
                None if Instant::now() >= deadline => {
                    return Err(failed(DriverError::Timeout("cancelled transfers")));
                }
                None => {}
            }
        }
        self.submitted.clear();
        for _ in 0..self.transfers {
            self.bulk().submit(TRANSFER_SIZE);
            self.submitted.push_back(Instant::now());
        }
        Ok(())
    }

    /// Sends the samples every channel is ready with.
    fn forward(&mut self) {
        let data = self.assembler.take();
        if !data.is_empty() {
            self.send(Packet::Logic(data));
        }
    }

    /// Acts on a frame; returns why the capture ends if it does.
    fn handle(&mut self, found: &Frame) -> Option<EndReason> {
        let payload = &found.payload;
        match found.kind {
            frame::DATA => {
                let Some(&channel) = payload.first() else {
                    return Some(failed(DriverError::Protocol("an empty data frame".into())));
                };
                let channel = ChannelIndex::from(channel);
                if channel >= CHANNELS {
                    return Some(failed(DriverError::Protocol(format!(
                        "samples of channel {channel}"
                    ))));
                }
                if self.assembler.has(channel) {
                    let data = samples(payload, self.params.rle);
                    self.assembler.push(channel, &data);
                }
                None
            }
            frame::OFFSET => self.offset(payload),
            frame::REPLY => match payload.get(2).copied() {
                Some(cmd::STOP) => Some(EndReason::Completed),
                // The trigger command is acknowledged with 3; anything else refuses the settings.
                Some(cmd::TRIGGER) if payload.get(3) != Some(&3) => Some(failed(
                    DriverError::InvalidRequest("the device refused the capture settings".into()),
                )),
                _ => None,
            },
            frame::PROGRESS => {
                if payload.len() < 7 {
                    return Some(failed(DriverError::Protocol(
                        "a short progress frame".into(),
                    )));
                }
                if !self.params.stream {
                    let count = u40(&payload[2..7]);
                    self.send(Packet::Progress {
                        triggered: count > self.params.pre_trigger,
                        captured: count.min(self.params.limit_samples),
                    });
                }
                None
            }
            frame::DONE => {
                // Stream mode: the device's buffer overflowed because USB did not keep up.
                if self.params.stream && payload.get(2) == Some(&1) {
                    self.send(Packet::Overflow);
                    return Some(EndReason::Failed(UiError::new("error.device.overflow")));
                }
                Some(EndReason::Completed)
            }
            _ => None,
        }
    }

    /// The trigger offset: in Stream mode where the trigger is; in Buffer mode how many samples
    /// follow it and how many bytes each channel holds, which tells how many to drop from the
    /// start, and with RLE whether the capture overran the memory.
    fn offset(&mut self, payload: &[u8]) -> Option<EndReason> {
        if std::mem::replace(&mut self.offset_seen, true) {
            return Some(failed(DriverError::Protocol(
                "a second trigger offset".into(),
            )));
        }
        let short = || {
            Some(failed(DriverError::Protocol(
                "a short trigger offset".into(),
            )))
        };
        let Some(after) = payload.get(2..7).map(u40) else {
            return short();
        };
        log::debug!("trigger offset {after}");
        if self.params.stream {
            if after > 0 && !self.params.instant {
                self.send(Packet::Trigger { sample: after });
            }
            return None;
        }
        let counts_end = 7 + 5 * usize::from(CHANNELS);
        let Some(counts) = payload.get(7..counts_end) else {
            return short();
        };
        let (pre, depth) = (self.params.pre_trigger, self.params.depth);
        for (channel, count) in counts
            .as_chunks::<5>()
            .0
            .iter()
            .map(|count| u40(count))
            .enumerate()
        {
            let channel = channel as ChannelIndex;
            if !self.assembler.has(channel) {
                continue;
            }
            // The device keeps capturing before the trigger until it fires; whatever it holds
            // beyond what was asked for comes first and is dropped. ATK-Logic's unsigned
            // arithmetic would wrap on inconsistent numbers; this keeps everything instead.
            let samples = count.saturating_mul(8);
            let skip = if count != 0 && samples > depth {
                samples.saturating_sub(pre).saturating_sub(after)
            } else {
                0
            };
            log::debug!("channel {channel}: {count} bytes, dropping {skip} samples");
            if count == 0 {
                log::warn!("the device holds no samples of channel {channel}");
                self.assembler.absent(channel);
            }
            self.assembler.skip(channel, skip);
        }
        if self.params.rle {
            let Some(&flags) = payload.get(counts_end) else {
                return short();
            };
            // ATK-Logic stops here in either case, before any samples arrive.
            if flags & 2 != 0 {
                return Some(EndReason::Failed(UiError::new("error.device.rleOverrun")));
            }
            if flags & 1 != 0 {
                return Some(EndReason::Failed(UiError::new("error.device.memoryFull")));
            }
        }
        if !self.params.instant {
            self.send(Packet::Trigger { sample: pre });
        }
        None
    }

    /// Stops reading, then stops the device and reads what it still sends, as ATK-Logic does
    /// after every capture, and lets the FPGA sleep until the next.
    fn finish(&mut self, reason: &EndReason) {
        log::info!(
            "capture ended ({reason:?}) with {} samples per channel",
            self.assembler.emitted()
        );
        if let Some(mut bulk) = self.bulk.take() {
            bulk.cancel_all();
            let deadline = Instant::now() + CANCEL_GRACE;
            while bulk.pending() > 0 && Instant::now() < deadline {
                let _ = bulk.next(POLL_INTERVAL);
            }
        }
        if matches!(reason, EndReason::Detached) {
            return;
        }
        let link = &self.link;
        if let Err(error) = link.command(cmd::STOP, &[]) {
            log::warn!("stopping the capture failed: {error}");
            return;
        }
        match link.drain(FINISH_DRAIN) {
            Ok(reads) => log::debug!("{reads} reads cleared the device"),
            Err(error) => log::debug!("clearing the device failed: {error}"),
        }
        if let Err(error) = link.mcu(mcu::RESET_STATE, &[0]) {
            log::warn!("letting the FPGA sleep failed: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(stream: bool, samplerate: u64, limit: u64, enabled: u16) -> AcquisitionParams {
        AcquisitionParams {
            stream,
            rle: false,
            samplerate,
            rate_code: 1,
            limit_samples: limit,
            depth: limit,
            pre_trigger: 0,
            enabled,
            vth: 1.6,
            triggers: Vec::new(),
            instant: false,
            looping: false,
        }
    }

    #[test]
    fn plans_transfers_by_data_rate() {
        // 16 channels at 20 MHz move 320 Mbit/s.
        assert_eq!(
            transfers(&params(true, 20_000_000, 20_000_000, 0xffff)),
            TRANSFERS
        );
        // A 1 ms capture of 16 channels at 250 MHz is 4 Mbit in its first second.
        assert_eq!(transfers(&params(false, 250_000_000, 250_000, 0xffff)), 1);
        // Just under the threshold, ATK-Logic keeps all 40 in flight: 249 Mbit in a second.
        assert_eq!(
            transfers(&params(true, 20_000_000, 15_562_500, 0xffff)),
            MAX_TRANSFERS
        );
        assert_eq!(transfers(&params(true, 20_000_000, 15_000_000, 0xffff)), 39);
    }

    #[test]
    fn expands_run_lengths() {
        assert_eq!(samples(&[3, 0, 0xaa, 0x55], false), vec![0xaa, 0x55]);
        assert_eq!(
            samples(&[3, 0, 2, 0xaa, 0, 0x11, 3, 0x0f], true),
            vec![0xaa, 0xaa, 0x0f, 0x0f, 0x0f]
        );
        assert_eq!(samples(&[3, 0, 2], true), vec![0, 0]);
    }

    #[test]
    fn loops_over_the_longest_capture() {
        assert_eq!(loop_depth(1), (1 << 40) - 1);
        assert_eq!(loop_depth(16), ((1 << 40) - 1) / 16);
    }
}
