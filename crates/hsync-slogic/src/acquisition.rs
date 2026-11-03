// SPDX-License-Identifier: GPL-3.0-or-later
//! Capture sequencing follows Sipeed libsigrok's `slogic/slogic_usb.c` at
//! 0d728b3e1226a61738c99c3333ad16d5c2814728, the transfer ring at
//! ba92c5fe8728992574950b40810b2df43528ff5d (Copyright 2023–2025 Sipeed), and the capture
//! engine of Sipeed's SLogic 2.3.3 (`slogic/slogic_usb.c` at
//! af106af699ce41602fdda24e16a69f9d62e1387c): one thread keeps the USB transfers in flight
//! and hands their bytes through a bounded queue to another, which unpacks, triggers and
//! packs them. Reads use nusb's ordered transfer queue; all waits and teardown are bounded.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, SendTimeoutError, Sender, TrySendError};
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
/// Transfers' bytes the converter may fall behind by before the capture fails, rather than
/// the analyzer's FIFO overflowing unnoticed (`max_queue_bytes`).
const BACKLOG_BYTES: usize = 256 * 1024 * 1024;
/// How much recent streaming time a rate measurement covers at minimum.
const RATE_WINDOW: Duration = Duration::from_millis(250);
/// Keep rate checkpoints at most this often, independently of the USB transfer count.
const RATE_CHECKPOINT: Duration = Duration::from_millis(25);
/// The share of the expected rate below which the analyzer cannot have kept all its samples.
const RATE_FLOOR: f64 = 0.9;

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
        pattern: settings.pattern,
    };
    link.prepare(&configuration)?;
    let transfers = TransferPlan::new(
        settings.channels(),
        settings.samplerate,
        link.usb().transfer_budget(),
    );
    let bulk = link.usb().bulk_in(link.endpoint())?;
    let stop = Arc::new(AtomicBool::new(false));
    let (raw, backlog) =
        crossbeam_channel::bounded((BACKLOG_BYTES / transfers.size).max(MIN_IN_FLIGHT * 2));
    let stream = Stream::new(settings, &plan, settings.channels(), model.head_bytes);
    // The bytes a finite untriggered capture needs: once that many are in flight, no
    // transfer is queued again (`need_raw` of the reference).
    let needed = stream.known_remaining().map(|samples| {
        model.head_bytes as u64 + (samples * u64::from(settings.channels())).div_ceil(8)
    });
    let mut worker = Worker {
        link,
        bulk: Some(bulk),
        transfer_size: transfers.size,
        transfer_count: transfers.count,
        configuration,
        raw: Some(raw),
        needed,
        forwarded: 0,
        report: Report::new(model.name, settings),
        stop: Arc::clone(&stop),
        running,
        sink: sink.clone(),
    };
    let converter = Converter {
        stream,
        stop: Arc::clone(&stop),
        sink,
        busy: Duration::ZERO,
        cpu: Duration::ZERO,
    };
    // A capture smaller than the ring takes fewer transfers.
    for _ in 0..transfers.count {
        worker.resubmit();
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
        .name("slogic-usb".into())
        .spawn(move || {
            let started = send(
                &worker.sink,
                &worker.stop,
                Packet::Start {
                    channels,
                    samplerate,
                    analog,
                },
            );
            let converter = started.and_then(|()| {
                std::thread::Builder::new()
                    .name("slogic-convert".into())
                    .spawn(move || converter.run(&backlog))
                    .map_err(|_| EndReason::Failed(UiError::new("error.device.slogic.workerStart")))
            });
            let (received, converted) = match converter {
                Ok(converter) => {
                    let received = worker.receive(&converter);
                    let received = worker.finish(received);
                    // The converter ends once the queue is drained, or at once when stopped.
                    worker.raw.take();
                    let (converted, busy, cpu) = converter.join().unwrap_or_else(|_| {
                        (
                            EndReason::Failed(UiError::new("error.device.slogic.workerStart")),
                            Duration::ZERO,
                            Duration::ZERO,
                        )
                    });
                    worker.report.busy = busy;
                    worker.report.convert_cpu = cpu;
                    (received, converted)
                }
                Err(reason) => (worker.finish(reason), EndReason::Stopped),
            };
            let reason = outcome(received, converted);
            worker.report.log(&reason);
            worker.running.store(false, Ordering::Release);
            let _ = worker.sink.send_timeout(Packet::End(reason), CANCEL_GRACE);
        })
        .map_err(|_| DriverError::Localized(UiError::new("error.device.slogic.workerStart")))?;
    Ok(Box::new(SlogicAcquisition {
        stop,
        thread: Some(thread),
    }))
}

/// Why the capture ended, from how each thread ended: a failure of the USB link first, then
/// one of the conversion, then whether every requested sample arrived.
fn outcome(received: EndReason, converted: EndReason) -> EndReason {
    match (received, converted) {
        (received @ (EndReason::Failed(_) | EndReason::Detached), _) => received,
        (_, converted @ (EndReason::Failed(_) | EndReason::Completed)) => converted,
        (received, _) => received,
    }
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

/// Hands a packet to HSync, which takes them as fast as they come.
fn send(sink: &Sender<Packet>, stop: &AtomicBool, mut packet: Packet) -> Result<(), EndReason> {
    let deadline = Instant::now() + Duration::from_millis(100);
    loop {
        if stop.load(Ordering::Acquire) {
            return Err(EndReason::Stopped);
        }
        match sink.send_timeout(packet, POLL) {
            Ok(()) => return Ok(()),
            Err(SendTimeoutError::Disconnected(_)) => return Err(EndReason::Stopped),
            Err(SendTimeoutError::Timeout(_)) if Instant::now() >= deadline => {
                return Err(EndReason::Failed(UiError::new("error.device.overflow")));
            }
            Err(SendTimeoutError::Timeout(pending)) => packet = pending,
        }
    }
}

/// Recent cumulative byte counts, sampled at most every 25 ms. Retain the newest
/// checkpoint at least 250 ms old and those after it: at most twelve checkpoints,
/// even with tiny transfers. A normal stream is measured over 250–275 ms.
#[derive(Default)]
struct RateWindow {
    checkpoints: VecDeque<(Instant, u64)>,
}

impl RateWindow {
    fn record(&mut self, now: Instant, bytes: u64) {
        while self
            .checkpoints
            .get(1)
            .is_some_and(|&(at, _)| now.duration_since(at) >= RATE_WINDOW)
        {
            self.checkpoints.pop_front();
        }
        if self
            .checkpoints
            .back()
            .is_none_or(|&(at, _)| now.duration_since(at) >= RATE_CHECKPOINT)
        {
            self.checkpoints.push_back((now, bytes));
        }
    }

    fn rate(&self, now: Instant, bytes: u64) -> Option<f64> {
        let &(at, before) = self.checkpoints.front()?;
        let span = now.duration_since(at);
        (span >= RATE_WINDOW).then(|| (bytes - before) as f64 / span.as_secs_f64())
    }
}

/// What a capture moved, logged once it ends, after the `slogic-report` line of Sipeed's
/// applications: whether the USB stream kept pace, and how far the conversion fell behind.
struct Report {
    model: &'static str,
    channels: u8,
    samplerate: u64,
    /// When RUN was sent, and when the first and last data arrived.
    run: Instant,
    first: Option<Instant>,
    last: Instant,
    bytes: u64,
    /// Recent throughput for loss detection; totals above still describe the whole capture.
    recent: RateWindow,
    transfers: u64,
    refused: u64,
    /// The most transfers waiting for the converter at once.
    backlog_peak: usize,
    /// Time spent queueing transfers again, and converting their samples.
    submit: Duration,
    busy: Duration,
    /// The CPU time the converter ran, where the OS tells it: below `busy`, it was
    /// waiting for a CPU.
    convert_cpu: Duration,
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
            recent: RateWindow::default(),
            transfers: 0,
            refused: 0,
            backlog_peak: 0,
            submit: Duration::ZERO,
            busy: Duration::ZERO,
            convert_cpu: Duration::ZERO,
        }
    }

    fn data(&mut self, bytes: usize, now: Instant) {
        self.first.get_or_insert(now);
        self.last = now;
        self.bytes += bytes as u64;
        self.transfers += 1;
        self.recent.record(now, self.bytes);
    }

    /// Recent bytes per second, excluding the first transfer's unknown sampling interval.
    fn rate(&self) -> Option<f64> {
        self.recent.rate(self.last, self.bytes)
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
             backlog_peak={} submit_ms={:.1} busy_ms={:.1} convert_cpu_ms={:.1}",
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
            self.backlog_peak,
            ms(self.submit),
            ms(self.busy),
            ms(self.convert_cpu),
        );
    }
}

/// The CPU time the calling thread has run, where Linux tells it.
fn thread_cpu() -> Option<Duration> {
    let stat = std::fs::read_to_string("/proc/thread-self/schedstat").ok()?;
    let nanos = stat.split_whitespace().next()?.parse().ok()?;
    Some(Duration::from_nanos(nanos))
}

/// Turns the transfers' bytes into HSync's packets, on a thread of its own.
struct Converter {
    stream: Stream,
    stop: Arc<AtomicBool>,
    sink: Sender<Packet>,
    busy: Duration,
    cpu: Duration,
}

impl Converter {
    /// Converts until the limit is reached, the capture is stopped, or the USB thread hangs
    /// up; returns how it ended, the time spent converting and the CPU time it took.
    fn run(mut self, backlog: &Receiver<Vec<u8>>) -> (EndReason, Duration, Duration) {
        let cpu = thread_cpu();
        let reason = self.convert(backlog);
        if let (Some(start), Some(end)) = (cpu, thread_cpu()) {
            self.cpu = end.saturating_sub(start);
        }
        (reason, self.busy, self.cpu)
    }

    fn convert(&mut self, backlog: &Receiver<Vec<u8>>) -> EndReason {
        loop {
            if self.stop.load(Ordering::Acquire) {
                return EndReason::Stopped;
            }
            let bytes = match backlog.recv_timeout(POLL) {
                Ok(bytes) => bytes,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return EndReason::Stopped,
            };
            let started = Instant::now();
            if let Some(reason) = self
                .stream
                .push(&bytes)
                .into_iter()
                .find_map(|packet| send(&self.sink, &self.stop, packet).err())
            {
                return reason;
            }
            self.busy += started.elapsed();
            if self.stream.done() {
                return EndReason::Completed;
            }
        }
    }
}

struct Worker {
    link: Link,
    bulk: Option<Box<dyn BulkIn>>,
    transfer_size: usize,
    transfer_count: usize,
    configuration: CaptureSettings,
    /// The transfers' bytes, in order, for the converter.
    raw: Option<Sender<Vec<u8>>>,
    /// The bytes the capture needs, if it is finite and untriggered, and those forwarded.
    needed: Option<u64>,
    forwarded: u64,
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

    /// Whether the converter has every byte the capture needs.
    fn has_enough(&self) -> bool {
        self.needed.is_some_and(|needed| self.forwarded >= needed)
    }

    /// Queues a transfer again, unless those in flight already carry what the capture needs.
    fn resubmit(&mut self) {
        let size = self.transfer_size;
        let in_flight = self.bulk().pending() as u64 * size as u64;
        if self
            .needed
            .is_none_or(|needed| self.forwarded + in_flight < needed)
        {
            self.bulk().submit(size);
        }
    }

    /// Hands a transfer's bytes to the converter without ever waiting for it: the USB
    /// transfers must stay in flight.
    fn forward(&mut self, bytes: Vec<u8>) -> Result<(), EndReason> {
        let raw = self
            .raw
            .as_ref()
            .expect("the queue outlives the receive loop");
        self.report.backlog_peak = self.report.backlog_peak.max(raw.len() + 1);
        match raw.try_send(bytes) {
            Ok(()) => Ok(()),
            // The converter fell too far behind; carrying on would lose samples.
            Err(TrySendError::Full(_)) => {
                Err(EndReason::Failed(UiError::new("error.device.overflow")))
            }
            // The converter has ended, and says why.
            Err(TrySendError::Disconnected(_)) => Err(EndReason::Stopped),
        }
    }

    /// Reports once that the stream runs slower than the analyzer samples: its FIFO then
    /// overflows and drops samples, which the stream cannot show.
    fn watch_rate(&mut self, warned: &mut bool) {
        if *warned {
            return;
        }
        if let Some(rate) = self.report.rate()
            && rate < self.report.expected() * RATE_FLOOR
        {
            *warned = true;
            log::warn!(
                "the SLogic stream runs at {:.1} of {:.1} MB/s; samples were lost",
                rate / 1e6,
                self.report.expected() / 1e6
            );
            let _ = send(&self.sink, &self.stop, Packet::Overflow);
        }
    }

    fn receive(&mut self, converter: &JoinHandle<(EndReason, Duration, Duration)>) -> EndReason {
        let mut last_data = Instant::now();
        let mut received = false;
        let mut retried = false;
        let mut slow = false;
        loop {
            if self.stop.load(Ordering::Acquire) {
                return EndReason::Stopped;
            }
            // Once all input is queued, stop USB and let the converter drain it. Also
            // observe every converter exit, including send failures and panics; waiting
            // for another USB completion cannot reveal those once the ring is empty.
            // The joined converter supplies the final result through outcome().
            if self.has_enough() || converter.is_finished() {
                return EndReason::Stopped;
            }
            match self.bulk().next(POLL) {
                Some(Ok(bytes)) if !bytes.is_empty() => {
                    last_data = Instant::now();
                    received = true;
                    self.report.data(bytes.len(), last_data);
                    self.forwarded += bytes.len() as u64;
                    // Requeue before anything else so USB can keep filling the ring.
                    let started = Instant::now();
                    self.resubmit();
                    self.report.submit += started.elapsed();
                    if let Err(reason) = self.forward(bytes) {
                        return reason;
                    }
                    self.watch_rate(&mut slow);
                }
                Some(Ok(_)) => self.resubmit(),
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
            self.resubmit();
        }
        // Nothing arrived before, so the converter still waits for the first head.
        self.link.run(&self.configuration)
    }

    fn finish(&mut self, mut reason: EndReason) -> EndReason {
        let Some(mut bulk) = self.bulk.take() else {
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
        reason
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.finish(EndReason::Stopped);
        // Also covers startup failures before the receive/converter join path runs.
        self.running.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SLOGIC32U3, UsbSpeed};

    #[test]
    fn recent_rate_detects_a_slow_interval_after_a_long_healthy_stream() {
        let mut settings = Settings::defaults(&SLOGIC32U3, UsbSpeed::SuperPlus);
        settings.samplerate = 200_000_000;
        let mut report = Report::new(SLOGIC32U3.name, &settings);
        let start = report.run;
        // A minute at 800 MB/s must not hide a later 300 ms interval at 400 MB/s.
        for tick in 0..=2400 {
            report.data(20_000_000, start + Duration::from_millis(tick * 25));
        }
        assert_eq!(report.rate(), Some(800_000_000.0));
        for tick in 2401..=2412 {
            report.data(10_000_000, start + Duration::from_millis(tick * 25));
        }
        assert_eq!(report.rate(), Some(400_000_000.0));
        assert!(report.rate().expect("a full window") < report.expected() * RATE_FLOOR);
    }

    #[test]
    fn rate_excludes_the_first_transfer_and_waits_for_a_full_window() {
        let settings = Settings::defaults(&SLOGIC32U3, UsbSpeed::SuperPlus);
        let mut report = Report::new(SLOGIC32U3.name, &settings);
        let start = report.run + Duration::from_secs(2);
        report.data(1 << 20, start);
        report.data(124_000, start + Duration::from_millis(124));
        assert_eq!(report.rate(), None);
        report.data(126_000, start + RATE_WINDOW);
        assert_eq!(report.rate(), Some(1_000_000.0));
        assert_eq!(report.bytes, (1 << 20) + 250_000);
    }

    #[test]
    fn rate_history_stays_bounded_with_frequent_transfers_and_a_stall() {
        let mut window = RateWindow::default();
        let start = Instant::now();
        for tick in 0..=10_000 {
            window.record(start + Duration::from_millis(tick), tick * 1000);
            assert!(window.checkpoints.len() <= 12);
        }
        let resumed = start + Duration::from_secs(12);
        window.record(resumed, 10_001_000);
        assert!(window.rate(resumed, 10_001_000).expect("a full window") < 900_000.0);
        for tick in 1..=300 {
            window.record(
                resumed + Duration::from_millis(tick),
                10_001_000 + tick * 1000,
            );
        }
        assert_eq!(
            window.rate(resumed + Duration::from_millis(300), 10_301_000),
            Some(1_000_000.0)
        );
    }

    #[test]
    fn a_failed_link_outranks_the_conversion_which_outranks_a_stop() {
        let failure = || EndReason::Failed(UiError::new("error.device.overflow"));
        assert_eq!(outcome(failure(), EndReason::Completed), failure());
        assert_eq!(
            outcome(EndReason::Detached, EndReason::Stopped),
            EndReason::Detached
        );
        assert_eq!(outcome(EndReason::Stopped, failure()), failure());
        // The USB thread hears of the limit after the converter reached it.
        assert_eq!(
            outcome(EndReason::Stopped, EndReason::Completed),
            EndReason::Completed
        );
        assert_eq!(
            outcome(EndReason::Completed, EndReason::Completed),
            EndReason::Completed
        );
        assert_eq!(
            outcome(EndReason::Stopped, EndReason::Stopped),
            EndReason::Stopped
        );
    }
}
