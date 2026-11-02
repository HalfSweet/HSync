// SPDX-License-Identifier: GPL-3.0-or-later
//! Capture sequencing follows Sipeed libsigrok's `slogic/slogic_usb.c` at
//! 0d728b3e1226a61738c99c3333ad16d5c2814728, the transfer ring at
//! ba92c5fe8728992574950b40810b2df43528ff5d (Copyright 2023–2025 Sipeed).
//! Reads use nusb's ordered transfer queue; all waits and teardown are bounded.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{SendTimeoutError, Sender};
use hsync_core::{ANALOG_BASE, ChannelIndex, UiError};
use hsync_driver::usb::{BulkIn, UsbError};
use hsync_driver::{Acquisition, CapturePlan, DriverError, EndReason, Packet};

use crate::model::Model;
use crate::protocol::{CaptureSettings, Link, MIN_IN_FLIGHT, TransferPlan};
use crate::stream::Stream;
use crate::{Settings, adc};

const POLL: Duration = Duration::from_millis(20);
const STALL: Duration = Duration::from_secs(2);
const CANCEL_GRACE: Duration = Duration::from_secs(2);

pub(crate) fn start(
    link: Link,
    model: &Model,
    settings: &Settings,
    plan: CapturePlan,
    running: Arc<AtomicBool>,
    sink: Sender<Packet>,
) -> Result<Box<dyn Acquisition>, DriverError> {
    let configuration = CaptureSettings {
        channels: settings.channels(),
        samplerate: settings.samplerate,
        threshold: settings.vth,
        clock_mhz: (settings.analog != 0).then_some(adc::CLOCK_MHZ),
    };
    link.prepare(&configuration)?;
    let transfers = TransferPlan::new(
        settings.channels(),
        settings.samplerate,
        link.usb().transfer_budget(),
    );
    let bulk = link.usb().bulk_in(link.endpoint())?;
    let stop = Arc::new(AtomicBool::new(false));
    let mut worker = Worker {
        link,
        bulk: Some(bulk),
        transfer_size: transfers.size,
        transfer_count: transfers.count,
        configuration,
        stream: Stream::new(settings, &plan, settings.channels(), model.head_bytes),
        report: Report::new(model.name, settings),
        stop: Arc::clone(&stop),
        running,
        sink,
    };
    for _ in 0..transfers.count {
        worker.bulk().submit(transfers.size);
    }
    // Queue first, then open the gate. Worker::drop stops before cancellation
    // even when RUN, thread creation, or the first packet fails.
    worker.link.run(&configuration)?;
    worker.report.run = Instant::now();
    worker.running.store(true, Ordering::Release);
    let logic = settings.logic();
    let channels = (0..model.channels.into())
        .filter(|c| logic & (1 << c) != 0)
        .collect();
    let analog = settings
        .analog_groups()
        .into_iter()
        .map(|group| (ANALOG_BASE + ChannelIndex::from(group), adc::SCALE))
        .collect();
    let samplerate = settings.samplerate;
    let thread = std::thread::Builder::new()
        .name("slogic-acquisition".into())
        .spawn(move || {
            let reason = match worker.send(Packet::Start {
                channels,
                samplerate,
                analog,
            }) {
                Ok(()) => worker.receive(),
                Err(reason) => reason,
            };
            let reason = worker.finish(reason);
            worker.report.log(&reason);
            let _ = worker.sink.send_timeout(Packet::End(reason), CANCEL_GRACE);
        })
        .map_err(|_| DriverError::Localized(UiError::new("error.device.slogic.workerStart")))?;
    Ok(Box::new(SlogicAcquisition {
        stop,
        thread: Some(thread),
    }))
}

struct SlogicAcquisition {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Acquisition for SlogicAcquisition {
    fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    fn wait(mut self: Box<Self>) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for SlogicAcquisition {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

fn failed(error: DriverError) -> EndReason {
    match &error {
        DriverError::Usb(error) if error.is_disconnected() => EndReason::Detached,
        _ => EndReason::Failed(UiError::from(&error)),
    }
}

/// What a capture moved, logged once it ends, after the `slogic-report` line of Sipeed's
/// applications: whether the USB stream kept pace, and how much of its time the conversion
/// took.
struct Report {
    model: &'static str,
    channels: u8,
    samplerate: u64,
    /// When RUN was sent, and when the first and last data arrived.
    run: Instant,
    first: Option<Instant>,
    last: Instant,
    bytes: u64,
    transfers: u64,
    refused: u64,
    /// Time spent queueing transfers again, and converting and sending their samples.
    submit: Duration,
    busy: Duration,
}

impl Report {
    fn new(model: &'static str, settings: &Settings) -> Self {
        let now = Instant::now();
        Self {
            model,
            channels: settings.channels(),
            samplerate: settings.samplerate,
            run: now,
            first: None,
            last: now,
            bytes: 0,
            transfers: 0,
            refused: 0,
            submit: Duration::ZERO,
            busy: Duration::ZERO,
        }
    }

    fn data(&mut self, bytes: usize) {
        let now = Instant::now();
        self.first.get_or_insert(now);
        self.last = now;
        self.bytes += bytes as u64;
        self.transfers += 1;
    }

    /// Bytes per second the analyzer sends.
    fn expected(&self) -> f64 {
        self.samplerate as f64 * f64::from(self.channels) / 8.0
    }

    fn log(&self, reason: &EndReason) {
        let ms = |duration: Duration| duration.as_secs_f64() * 1000.0;
        let span = self.first.map_or(Duration::ZERO, |first| self.last - first);
        log::info!(
            "slogic-report model={} ch={} rate_mhz={} end={reason:?} bytes={} transfers={} \
             refused={} cold_ms={:.1} span_ms={:.1} mbps={:.1} exp_mbps={:.1} wall_ms={:.1} \
             submit_ms={:.1} busy_ms={:.1}",
            self.model,
            self.channels,
            self.samplerate / 1_000_000,
            self.bytes,
            self.transfers,
            self.refused,
            self.first.map_or(f64::NAN, |first| ms(first - self.run)),
            ms(span),
            if span.is_zero() {
                f64::NAN
            } else {
                self.bytes as f64 / span.as_secs_f64() / 1e6
            },
            self.expected() / 1e6,
            ms(self.run.elapsed()),
            ms(self.submit),
            ms(self.busy),
        );
    }
}

struct Worker {
    link: Link,
    bulk: Option<Box<dyn BulkIn>>,
    transfer_size: usize,
    transfer_count: usize,
    configuration: CaptureSettings,
    stream: Stream,
    report: Report,
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    sink: Sender<Packet>,
}

impl Worker {
    fn bulk(&mut self) -> &mut dyn BulkIn {
        self.bulk
            .as_deref_mut()
            .expect("the queue outlives the receive loop")
    }

    fn send(&self, mut packet: Packet) -> Result<(), EndReason> {
        let deadline = Instant::now() + Duration::from_millis(100);
        loop {
            if self.stop.load(Ordering::Acquire) {
                return Err(EndReason::Stopped);
            }
            match self.sink.send_timeout(packet, POLL) {
                Ok(()) => return Ok(()),
                Err(SendTimeoutError::Disconnected(_)) => return Err(EndReason::Stopped),
                Err(SendTimeoutError::Timeout(_)) if Instant::now() >= deadline => {
                    return Err(EndReason::Failed(UiError::new("error.device.overflow")));
                }
                Err(SendTimeoutError::Timeout(pending)) => packet = pending,
            }
        }
    }

    fn receive(&mut self) -> EndReason {
        let mut last_data = Instant::now();
        let mut received = false;
        let mut retried = false;
        loop {
            if self.stop.load(Ordering::Acquire) {
                return EndReason::Stopped;
            }
            match self.bulk().next(POLL) {
                Some(Ok(bytes)) if !bytes.is_empty() => {
                    last_data = Instant::now();
                    received = true;
                    self.report.data(bytes.len());
                    let size = self.transfer_size;
                    // Requeue before conversion so USB can keep filling the ring.
                    let started = Instant::now();
                    self.bulk().submit(size);
                    self.report.submit += started.elapsed();
                    for packet in self.stream.push(&bytes) {
                        if let Err(reason) = self.send(packet) {
                            return reason;
                        }
                    }
                    self.report.busy += started.elapsed();
                    if self.stream.done() {
                        return EndReason::Completed;
                    }
                }
                Some(Ok(_)) => {
                    let size = self.transfer_size;
                    self.bulk().submit(size);
                }
                Some(Err(UsbError::NoMemory)) if self.transfer_count > MIN_IN_FLIGHT => {
                    self.drop_refused();
                }
                Some(Err(error)) => return failed(DriverError::Usb(error)),
                None => {}
            }
            // A software trigger can wait indefinitely as long as samples keep
            // arriving. A silent stream is a transport failure, never a trigger wait.
            if last_data.elapsed() >= STALL {
                if !received && !retried {
                    // Current firmware can swallow the first RUN. Like the
                    // reference, re-arm once, only before any data arrived.
                    retried = true;
                    if let Err(error) = self.rearm() {
                        return failed(error);
                    }
                    last_data = Instant::now();
                    continue;
                }
                return failed(DriverError::Localized(UiError::new(
                    "error.device.slogic.captureData",
                )));
            }
        }
    }

    /// Leaves out of the ring a request the system refused memory for ([`UsbError::NoMemory`]).
    /// It never reached the device, so no data is missing. The reference keeps the requests it
    /// could submit as long as [`MIN_IN_FLIGHT`] remain; nusb reports the refusal in the
    /// request's turn, so the ring shrinks then.
    fn drop_refused(&mut self) {
        self.transfer_count -= 1;
        self.report.refused += 1;
        log::warn!(
            "the system refused memory for a USB transfer; {} transfers of {} bytes stay in flight",
            self.transfer_count,
            self.transfer_size
        );
    }

    fn rearm(&mut self) -> Result<(), DriverError> {
        self.link.stop_retry(3)?;
        self.bulk().cancel_all();
        let deadline = Instant::now() + CANCEL_GRACE;
        while self.bulk().pending() > 0 {
            if Instant::now() >= deadline {
                return Err(DriverError::Localized(UiError::new(
                    "error.device.slogic.cancelTransfers",
                )));
            }
            match self.bulk().next(POLL) {
                Some(Err(UsbError::NoMemory)) if self.transfer_count > MIN_IN_FLIGHT => {
                    self.drop_refused();
                }
                Some(Err(error)) => return Err(error.into()),
                _ => {}
            }
        }
        // Drop the endpoint claim before prepare clears its halt.
        self.bulk.take();
        self.link.drain()?;
        self.link.prepare(&self.configuration)?;
        self.bulk = Some(self.link.usb().bulk_in(self.link.endpoint())?);
        for _ in 0..self.transfer_count {
            let size = self.transfer_size;
            self.bulk().submit(size);
        }
        // Nothing arrived before, so the stream still waits for the first head.
        self.link.run(&self.configuration)
    }

    fn finish(&mut self, mut reason: EndReason) -> EndReason {
        let Some(mut bulk) = self.bulk.take() else {
            self.running.store(false, Ordering::Release);
            return reason;
        };
        if !matches!(reason, EndReason::Detached)
            && let Err(error) = self.link.stop_retry(3)
            && matches!(reason, EndReason::Completed | EndReason::Stopped)
        {
            reason = failed(error);
        }
        // SLogic can wedge if requests are cancelled while its gate is RUN.
        // Always STOP above, including on user cancellation and error paths.
        bulk.cancel_all();
        let deadline = Instant::now() + CANCEL_GRACE;
        while bulk.pending() > 0 && Instant::now() < deadline {
            let _ = bulk.next(POLL);
        }
        if bulk.pending() > 0 && matches!(reason, EndReason::Completed | EndReason::Stopped) {
            reason = failed(DriverError::Localized(UiError::new(
                "error.device.slogic.cancelTransfers",
            )));
        }
        drop(bulk);
        // Combo 8 has no STOP command: after cancelling the queue its bounded
        // endpoint drain is the entire teardown, as in Sipeed's slogic_usb.c.
        if !matches!(reason, EndReason::Detached)
            && let Err(error) = self.link.drain()
            && matches!(reason, EndReason::Completed | EndReason::Stopped)
        {
            reason = failed(error);
        }
        self.running.store(false, Ordering::Release);
        reason
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.finish(EndReason::Stopped);
    }
}
