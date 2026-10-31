//! Running a capture (`start_transfers()`, `receive_transfer()`, `receive_data2()`): setting
//! the registers up, the transfers on the data endpoint, polling the trigger while nothing
//! arrives, and stopping at the sample limit.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use hsync_core::{ChannelIndex, UiError};
use hsync_driver::usb::{BulkIn, transfers_within};
use hsync_driver::{Acquisition, DriverError, EndReason, Packet};

use crate::profiles::UsbSpeed;
use crate::protocol::{
    Control, EP_DATA, EP_REGISTER2_IN, EP_REGISTER2_OUT, VTH_PERIOD, block_size, regs,
    sample_clock, vth_duty,
};

/// The trigger conditions of a capture: bit `n` stands for channel `n`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TriggerMasks {
    pub zero: u32,
    pub one: u32,
    pub rise: u32,
    pub fall: u32,
}

impl TriggerMasks {
    fn any(&self) -> bool {
        self.zero | self.one | self.rise | self.fall != 0
    }
}

/// Everything a capture needs, fixed when it starts.
#[derive(Debug, Clone)]
pub(crate) struct AcquisitionParams {
    pub speed: UsbSpeed,
    pub stream: bool,
    pub samplerate: u64,
    /// Samples per channel, a multiple of 64.
    pub limit_samples: u64,
    /// The enabled channels, ascending, and as a mask.
    pub channels: Vec<ChannelIndex>,
    pub enabled: u32,
    pub vth: f64,
    pub filter: bool,
    pub clock_falling_edge: bool,
    /// The external trigger register's value.
    pub external_trigger: u32,
    pub triggers: TriggerMasks,
    /// Samples before the trigger.
    pub trigger_position: u32,
    /// Run on past the limit until stopped (Stream mode).
    pub looping: bool,
}

/// Blocks in flight (`num_transfers`).
const TRANSFERS: usize = 4;
/// Longest transfer HSync asks for: PXView's default `BUFSIZE`. PXView reads a whole block (up
/// to 4 MiB) at once and turns WinUSB's RAW_IO off, so WinUSB splits it; nusb reads with RAW_IO
/// on, where WinUSB fails any read longer than the pipe's maximum transfer size. The same blocks
/// stay in flight, in more transfers.
const MAX_TRANSFER: usize = 1 << 20;
/// PWM0 at PXView's 1 kHz with half duty, switched off: the registers PXView writes before
/// every capture (`PWM_CLK / pwm0_freq`).
const PWM0_PERIOD: u32 = 125_000_000 / 1000;
const PWM0_DUTY: u32 = PWM0_PERIOD / 2;
/// Bit of the control register for Stream mode.
const CONTROL_STREAM: u32 = 1 << 1;
/// Bits of the control register while a capture is set up.
const CONTROL_SETUP: u32 = 5;
/// The bit of the control register pulsed during the setup.
const CONTROL_PULSE: u32 = 1 << 4;
/// How long one wait for a transfer lasts, and how often the trigger is polled; PXView's event
/// source polls every 5 ms.
const POLL_INTERVAL: Duration = Duration::from_millis(20);
/// How long cancelled transfers may take to come back.
const CANCEL_GRACE: Duration = Duration::from_secs(2);

/// The size and number of the transfers that keep [`TRANSFERS`] blocks of `block` bytes in
/// flight. Blocks are whole 4 KiB pages, so every transfer is whole packets. Unlike PXView, the
/// transfers keep within the transport's `budget` ([`UsbTransport::transfer_budget`]): Linux
/// fails what overflows its usbfs pool, which four 4 MiB blocks do by default.
///
/// [`UsbTransport::transfer_budget`]: hsync_driver::usb::UsbTransport::transfer_budget
fn transfer_plan(block: u32, budget: Option<usize>) -> (usize, usize) {
    let block = block as usize;
    let size = block.min(MAX_TRANSFER);
    let count = (TRANSFERS * block).div_ceil(size);
    (size, transfers_within(budget, size, count))
}

/// Writes the registers of a capture, in `start_transfers()`'s order. PXView goes on when a
/// write fails; HSync gives up, as the capture would run on half its settings.
fn set_up(ctl: &Control, params: &AcquisitionParams, block: u32) -> Result<(), DriverError> {
    let channels = params.channels.len() as u32;
    let stream = if params.stream { CONTROL_STREAM } else { 0 };
    // The device sends a block more than the capture, so the last transfer fills.
    let bytes = params.limit_samples * u64::from(channels) / 8 + u64::from(block);
    let (clock, divider) = sample_clock(params.samplerate);

    ctl.write(regs::PWM0_ENABLE, 0)?;
    ctl.write(regs::PWM0_PERIOD, PWM0_PERIOD - 1)?;
    ctl.write(regs::PWM0_DUTY, PWM0_DUTY - 1)?;
    ctl.write(regs::PWM0_ENABLE, 0)?;
    ctl.write(regs::PWM1_ENABLE, 0)?;
    ctl.write(regs::BLOCK_START, 0)?;
    ctl.clear_halt(EP_DATA);
    ctl.clear_halt(EP_REGISTER2_OUT);
    ctl.clear_halt(EP_REGISTER2_IN);
    ctl.write(regs::VTH_PERIOD, VTH_PERIOD)?;
    ctl.write(regs::VTH_DUTY, vth_duty(params.vth))?;
    ctl.write(regs::CHANNELS, 0)?;
    ctl.write(regs::CONTROL, CONTROL_SETUP | stream)?;
    ctl.write(regs::CONTROL, CONTROL_SETUP | stream | CONTROL_PULSE)?;
    ctl.write(regs::CONTROL, CONTROL_SETUP | stream)?;
    ctl.write(regs::HOLD, u32::MAX)?;
    ctl.write(regs::BLOCK, block)?;
    ctl.write(regs::SYS_BLOCK, block)?;
    ctl.write(regs::LIMIT_LOW, bytes as u32)?;
    ctl.write(regs::LIMIT_HIGH, (bytes >> 32) as u32)?;
    ctl.write(regs::EXTERNAL_TRIGGER, params.external_trigger)?;
    ctl.write(regs::TRIGGER_OUT, 0)?;
    ctl.write(
        regs::CLOCK,
        clock | (u32::from(params.clock_falling_edge) << 3),
    )?;
    ctl.write(regs::DIVIDER, divider)?;
    ctl.write(regs::CHANNEL_COUNT, channels)?;
    ctl.write(regs::TRIGGER_POSITION, params.trigger_position)?;
    ctl.write(regs::BLOCK_START, 0)?;
    let read_back = ctl.read(regs::DIVIDER)?;
    log::debug!("sample clock {clock}, divider {divider} (reads {read_back})");
    ctl.write(regs::CHANNELS, params.enabled)?;
    ctl.write(regs::CONTROL, stream | (u32::from(params.filter) << 3))?;
    ctl.write(regs::TRIGGER_ZERO, params.triggers.zero)?;
    ctl.write(regs::TRIGGER_ONE, params.triggers.one)?;
    ctl.write(regs::TRIGGER_RISE, params.triggers.rise)?;
    ctl.write(regs::TRIGGER_FALL, params.triggers.fall)?;
    ctl.write(regs::HOLD, 0)?;
    Ok(())
}

/// `hw_dev_acquisition_start()`: sets the capture up, which starts it, and hands the
/// transfers to a thread.
pub(crate) fn start(
    ctl: Control,
    params: AcquisitionParams,
    running: Arc<AtomicBool>,
    sink: Sender<Packet>,
) -> Result<Box<dyn Acquisition>, DriverError> {
    let block = block_size(
        params.samplerate,
        params.channels.len() as u32,
        params.speed,
    );
    set_up(&ctl, &params, block)?;
    let (transfer, count) = transfer_plan(block, ctl.usb().transfer_budget());
    let mut bulk = ctl.usb().bulk_in(EP_DATA)?;
    for _ in 0..count {
        bulk.submit(transfer);
    }
    log::info!(
        "capturing {} samples at {} Hz on {} channels in {} mode (blocks of {block} bytes, \
         {count} transfers of {transfer} bytes)",
        params.limit_samples,
        params.samplerate,
        params.channels.len(),
        if params.stream { "Stream" } else { "Buffer" }
    );

    running.store(true, Ordering::Release);
    let _ = sink.send(Packet::Start {
        channels: params.channels.clone(),
        samplerate: params.samplerate,
        analog: Vec::new(),
    });
    let stop = Arc::new(AtomicBool::new(false));
    let worker = Worker {
        ctl,
        params,
        bulk,
        transfer,
        sink,
        stop: Arc::clone(&stop),
    };
    let thread = std::thread::Builder::new()
        .name("pxlogic-acquisition".into())
        .spawn(move || {
            let (sink, reason) = worker.run();
            running.store(false, Ordering::Release);
            let _ = sink.send(Packet::End(reason));
        })
        .map_err(|error| DriverError::Protocol(format!("starting the capture thread: {error}")))?;
    Ok(Box::new(PxAcquisition {
        stop,
        thread: Some(thread),
    }))
}

struct PxAcquisition {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Acquisition for PxAcquisition {
    fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    fn wait(mut self: Box<Self>) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for PxAcquisition {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

struct Worker {
    ctl: Control,
    params: AcquisitionParams,
    bulk: Box<dyn BulkIn>,
    /// Bytes each transfer asks for.
    transfer: usize,
    sink: Sender<Packet>,
    stop: Arc<AtomicBool>,
}

impl Worker {
    /// Runs the transfers until the capture ends, then cancels what is in flight; PXView sends
    /// the device nothing when a capture ends.
    fn run(mut self) -> (Sender<Packet>, EndReason) {
        let reason = self.receive();
        self.bulk.cancel_all();
        let deadline = Instant::now() + CANCEL_GRACE;
        while self.bulk.pending() > 0 && Instant::now() < deadline {
            let _ = self.bulk.next(POLL_INTERVAL);
        }
        (self.sink, reason)
    }

    fn send(&self, packet: Packet) {
        let _ = self.sink.send(packet);
    }

    fn receive(&mut self) -> EndReason {
        // The capture in bytes, 8 a channel for every 64 samples; counted in bytes, as a
        // transfer need not end on a group of samples.
        let limit = self.params.limit_samples * self.params.channels.len() as u64 / 8;
        // A loop capture in Stream mode keeps no count and runs until stopped.
        let counting = !(self.params.stream && self.params.looping);
        let mut received: u64 = 0;
        let mut triggered = false;
        let mut last_poll = Instant::now();
        loop {
            if self.stop.load(Ordering::Acquire) {
                return EndReason::Stopped;
            }
            match self.bulk.next(POLL_INTERVAL) {
                Some(Ok(mut data)) => {
                    if counting {
                        data.truncate(usize::try_from(limit - received).unwrap_or(usize::MAX));
                        received += data.len() as u64;
                    }
                    if !data.is_empty() {
                        self.send(Packet::Logic(data));
                    }
                    if counting && received >= limit {
                        return EndReason::Completed;
                    }
                    self.bulk.submit(self.transfer);
                }
                Some(Err(error)) => {
                    return if error.is_disconnected() {
                        EndReason::Detached
                    } else {
                        EndReason::Failed(UiError::from(&DriverError::Usb(error)))
                    };
                }
                None => {}
            }
            // `receive_data2()`: until samples are counted, read where the capture is.
            if received == 0 && !triggered && last_poll.elapsed() >= POLL_INTERVAL {
                last_poll = Instant::now();
                triggered = self.poll_trigger();
            }
        }
    }

    /// Reads the capture state, reports the progress and, once the trigger fired, where;
    /// returns whether it fired.
    fn poll_trigger(&self) -> bool {
        let state = match self.ctl.state() {
            Ok(state) => state,
            Err(error) => {
                log::debug!("reading the capture state failed: {error}");
                return false;
            }
        };
        let position = u64::from(self.params.trigger_position);
        self.send(Packet::Progress {
            triggered: state.triggered,
            captured: state.samples.min(position),
        });
        if state.triggered
            && !(self.params.stream && self.params.looping)
            && self.params.triggers.any()
        {
            self.send(Packet::Trigger {
                sample: u64::from(state.trigger_position),
            });
        }
        state.triggered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_blocks_in_transfers_of_at_most_1_mib() {
        // USB 3.0 at the fastest rates: 4 MiB blocks.
        assert_eq!(transfer_plan(4 << 20, None), (1 << 20, 16));
        // 13 channels: blocks of no whole number of MiB.
        assert_eq!(transfer_plan(4_153_344, None), (1 << 20, 16));
        // USB 2.0, and slow rates: blocks under 1 MiB are read whole.
        assert_eq!(transfer_plan(602_112, None), (602_112, 4));
        assert_eq!(transfer_plan(4096, None), (4096, 4));
    }

    #[test]
    fn keeps_the_transfers_within_the_usbfs_budget() {
        // Four 4 MiB blocks overflow Linux's default usbfs pool; its budget holds three.
        assert_eq!(transfer_plan(4 << 20, Some(12 << 20)), (1 << 20, 12));
        assert_eq!(transfer_plan(602_112, Some(12 << 20)), (602_112, 4));
    }
}
