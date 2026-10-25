// SPDX-License-Identifier: GPL-3.0-or-later
//! Lists SLogic16U3 and SLogic32U3 devices, or performs a short capture when explicitly
//! requested.
//!
//! ```sh
//! cargo run -p hsync-slogic --example slogic_capture
//! cargo run -p hsync-slogic --example slogic_capture -- --capture
//! cargo run -p hsync-slogic --example slogic_capture -- --capture --channels 4 --rate 800000000 --repeat 3
//! ```
//!
//! Captures default to every channel, 5 MHz, 65,536 samples and a 1.7 V threshold.
//! Each stops after five seconds. The device stays open between repeats;
//! the example never flashes firmware and writes no files.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hsync_capture::LogicCapture;
use hsync_driver::{
    CandidateStatus, CapturePlan, Device, DeviceManager, EndReason, OptionKey, OptionValue, Packet,
};
use hsync_slogic::SlogicDriver;

const SAMPLE_COUNT: u64 = 65_536;
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(5);

/// Makes USB discovery and driver errors visible without an application logger.
struct StderrLog;

impl log::Log for StderrLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.target().starts_with("hsync")
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("[{}] {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut capture_requested = false;
    let mut channels = None;
    let mut samplerate = 5_000_000u64;
    let mut repeat = 1u8;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--capture" => capture_requested = true,
            "--channels" => {
                let count: u16 = args
                    .next()
                    .ok_or("--channels requires 4, 8, 16 or 32")?
                    .parse()?;
                if !matches!(count, 4 | 8 | 16 | 32) {
                    return Err("--channels must be 4, 8, 16 or 32".into());
                }
                channels = Some(count);
            }
            "--rate" => {
                samplerate = args
                    .next()
                    .ok_or("--rate requires an integer in Hz")?
                    .parse()?;
                if samplerate == 0 {
                    return Err("--rate must be positive".into());
                }
            }
            "--repeat" => {
                repeat = args
                    .next()
                    .ok_or("--repeat requires an integer from 1 to 10")?
                    .parse()?;
                if !(1..=10).contains(&repeat) {
                    return Err("--repeat must be between 1 and 10".into());
                }
            }
            "--help" | "-h" => {
                println!(
                    "slogic_capture [--capture] [--channels 4|8|16|32] [--rate HZ] [--repeat N]"
                );
                println!("Default: list devices without opening them or starting a capture.");
                println!("--capture       Required to open the device and acquire samples.");
                println!("--channels N    Enable D0 through D(N-1); default every channel.");
                println!("--rate HZ       Supported integer sample rate in Hz; default 5000000.");
                println!(
                    "--repeat N      Capture 1 to 10 times on the same open device; default 1."
                );
                println!("Every capture uses 65,536 samples, 1.7 V and a 5 s timeout.");
                println!(
                    "No files are saved; unsupported sample rates are rejected by the driver."
                );
                return Ok(());
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    log::set_logger(&StderrLog)?;
    log::set_max_level(log::LevelFilter::Debug);

    let manager = DeviceManager::new(vec![Arc::new(SlogicDriver::new())]);
    let candidates = manager.scan();
    for candidate in &candidates {
        println!(
            "{} {}: {} ({:?})",
            candidate.vendor, candidate.model, candidate.id, candidate.status
        );
    }
    if candidates.is_empty() {
        println!("No SLogic found (expected USB ID 359f:3031 or 359f:3032).");
    }
    if !capture_requested {
        return Ok(());
    }
    let candidate = candidates
        .iter()
        .find(|candidate| candidate.status == CandidateStatus::Ready)
        .ok_or("no ready SLogic found")?;
    println!("opening {}", candidate.id);
    let mut device = manager.open(candidate)?;
    println!("device: {:?}", device.info());
    let channels = channels.unwrap_or(device.channel_count());
    if channels > device.channel_count() {
        return Err(format!("this model has {} channels", device.channel_count()).into());
    }
    for channel in 0..device.channel_count() {
        device.set_channel_enabled(channel, channel < channels)?;
    }
    device.set_option(OptionKey::Samplerate, &OptionValue::Integer(samplerate))?;
    device.set_option(OptionKey::SampleLimit, &OptionValue::Integer(SAMPLE_COUNT))?;
    device.set_option(OptionKey::Vth, &OptionValue::Number(1.7))?;
    for index in 1..=repeat {
        capture_once(device.as_mut(), index, repeat)?;
    }
    Ok(())
}

fn capture_once(
    device: &mut dyn Device,
    index: u8,
    repeat: u8,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("capture {index}/{repeat}");
    let (sink, packets) = crossbeam_channel::unbounded();
    let started = Instant::now();
    let acquisition = device.start(
        CapturePlan {
            instant: true,
            ..CapturePlan::default()
        },
        sink,
    )?;
    let mut capture: Option<LogicCapture> = None;
    let mut deinterleaver = None;
    let mut overflow = false;
    let result = loop {
        let remaining = CAPTURE_TIMEOUT.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            acquisition.stop();
            break Err("capture timed out after five seconds".to_owned());
        }
        match packets.recv_timeout(remaining.min(Duration::from_millis(100))) {
            Ok(Packet::Start {
                channels,
                samplerate,
                ..
            }) => {
                println!("start: {} channels at {samplerate} Hz", channels.len());
                let new = LogicCapture::new(samplerate, channels);
                deinterleaver = Some(new.deinterleaver());
                capture = Some(new);
            }
            Ok(Packet::Logic(data)) => {
                if let (Some(capture), Some(deinterleaver)) =
                    (capture.as_mut(), deinterleaver.as_mut())
                {
                    capture.append_cross(deinterleaver, &data);
                }
            }
            Ok(Packet::Trigger { sample }) => println!("trigger: sample {sample}"),
            Ok(Packet::Overflow) => overflow = true,
            Ok(Packet::Progress { .. } | Packet::Analog(_)) => {}
            Ok(Packet::End(reason)) => break Ok(reason),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                acquisition.stop();
                break Err("capture channel closed without an end packet".to_owned());
            }
        }
    };
    acquisition.wait();
    let reason = result?;
    let capture = capture.ok_or("no capture start packet received")?;
    println!(
        "capture {index}/{repeat} end: {reason:?}; {} samples in {:.0} ms; overflow: {overflow}",
        capture.len(),
        started.elapsed().as_secs_f64() * 1000.0
    );
    for (channel, data) in capture.channels() {
        let edges = data.count_edges(0, capture.len());
        println!(
            "D{channel}: {} rising, {} falling edges",
            edges.rising, edges.falling
        );
    }
    if reason != EndReason::Completed || overflow || capture.len() != SAMPLE_COUNT {
        return Err("capture did not complete with the expected sample count".into());
    }
    Ok(())
}
