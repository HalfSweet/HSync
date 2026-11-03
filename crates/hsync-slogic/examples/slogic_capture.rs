// SPDX-License-Identifier: GPL-3.0-or-later
//! Lists SLogic Combo 8, SLogic16U3 and SLogic32U3 devices, or performs a short capture when explicitly
//! requested.
//!
//! ```sh
//! cargo run -p hsync-slogic --example slogic_capture
//! cargo run -p hsync-slogic --example slogic_capture -- --capture
//! cargo run -p hsync-slogic --example slogic_capture -- --capture --channels 4 --rate 800000000 --repeat 3
//! cargo run -p hsync-slogic --example slogic_capture -- --capture --pattern emulation --samples 800000000
//! ```
//!
//! Captures default to every channel, 5 MHz, 65,536 samples and a 1.7 V threshold where adjustable.
//! Each stops after five seconds. The device stays open between repeats;
//! the example never flashes firmware and writes no files.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hsync_capture::LogicCapture;
use hsync_core::TriggerCondition;
use hsync_driver::{
    CandidateStatus, CapturePlan, Device, DeviceManager, EndReason, OptionKey, OptionValue, Packet,
};
use hsync_slogic::SlogicDriver;
use hsync_slogic::protocol::Pattern;

const SAMPLE_COUNT: u64 = 65_536;
/// The timeout of a capture that takes no time on the wire; longer ones get ten times theirs.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(5);

/// The counter bit each channel carries under Sipeed's Emulation pattern, D0 first, and whether
/// it is inverted, after `build/bench/emulation.json` of Sipeed's SLogic
/// (af106af699ce41602fdda24e16a69f9d62e1387c). The counter starts at 0 with the capture and
/// counts samples. The Combo 8, which that file does not list, sends a plain counter, each
/// channel toggling on the previous one's falling edge; that was measured on its 359f:3000
/// firmware.
fn emulation_bits(model: &str, wire: u16) -> Option<Vec<(u8, bool)>> {
    // D16 to D31 of the SLogic32U3.
    const HIGH: [(u8, bool); 16] = [
        (0, false),
        (0, true),
        (1, false),
        (1, true),
        (2, false),
        (2, true),
        (3, true),
        (3, false),
        (4, true),
        (4, false),
        (5, true),
        (5, false),
        (6, true),
        (6, false),
        (7, true),
        (7, false),
    ];
    let low = (0..16).map(|bit| (bit, bit < 3));
    match (model, wire) {
        ("SLogic16U3" | "SLogic32U3", 4 | 8 | 16) => Some(low.take(usize::from(wire)).collect()),
        ("SLogic32U3", 32) => Some(low.chain(HIGH).collect()),
        ("SLogic Combo 8", 2) => Some(vec![(1, false), (2, false)]),
        ("SLogic Combo 8", 4 | 8) => Some((0..wire).map(|bit| (bit as u8, false)).collect()),
        _ => None,
    }
}

/// Compares the cross-format blocks of an Emulation capture with the pattern.
struct EmulationCheck {
    bits: Vec<(u8, bool)>,
    /// The counter at the first sample: 0 for a capture started at once, else read from it.
    start: Option<u16>,
    pending: Vec<u8>,
    blocks: u64,
    wrong: u64,
    /// The channel and sample of the first difference.
    first_wrong: Option<(usize, u64)>,
}

impl EmulationCheck {
    fn new(bits: Vec<(u8, bool)>, start: Option<u16>) -> Self {
        Self {
            bits,
            start,
            pending: Vec::new(),
            blocks: 0,
            wrong: 0,
            first_wrong: None,
        }
    }

    /// The 16 counter bits of 64 samples counting from `base`, a word each: the 64 additions
    /// `base + j` at once, one bit at a time.
    fn counter(base: u16) -> [u64; 16] {
        // Bit `k` of j, for j in 0..64.
        const J: [u64; 6] = [
            0xaaaa_aaaa_aaaa_aaaa,
            0xcccc_cccc_cccc_cccc,
            0xf0f0_f0f0_f0f0_f0f0,
            0xff00_ff00_ff00_ff00,
            0xffff_0000_ffff_0000,
            0xffff_ffff_0000_0000,
        ];
        let mut words = [0; 16];
        let mut carry = 0;
        for (bit, word) in words.iter_mut().enumerate() {
            let j = J.get(bit).copied().unwrap_or(0);
            let base = if base >> bit & 1 == 1 { u64::MAX } else { 0 };
            *word = j ^ base ^ carry;
            carry = (j & base) | (carry & (j ^ base));
        }
        words
    }

    fn push(&mut self, data: &[u8]) {
        self.pending.extend_from_slice(data);
        let size = self.bits.len() * 8;
        let whole = self.pending.len() / size * size;
        let pending = std::mem::take(&mut self.pending);
        for block in pending[..whole].chunks_exact(size) {
            self.check(block);
        }
        self.pending = pending[whole..].to_vec();
    }

    /// Checks a block of 64 samples, a little-endian word per channel.
    fn check(&mut self, block: &[u8]) {
        let words: Vec<u64> = block
            .as_chunks::<8>()
            .0
            .iter()
            .map(|word| u64::from_le_bytes(*word))
            .collect();
        let bits = &self.bits;
        let start = *self.start.get_or_insert_with(|| {
            bits.iter()
                .zip(&words)
                .fold(0, |start, (&(bit, inverted), word)| {
                    start | u16::from((word & 1 == 1) != inverted) << bit
                })
        });
        let counter = Self::counter(start.wrapping_add((self.blocks * 64) as u16));
        for (channel, (&(bit, inverted), &got)) in self.bits.iter().zip(&words).enumerate() {
            let want = counter[usize::from(bit)];
            let want = if inverted { !want } else { want };
            if got != want {
                self.wrong += 1;
                let sample = self.blocks * 64 + u64::from((got ^ want).trailing_zeros());
                self.first_wrong.get_or_insert((channel, sample));
            }
        }
        self.blocks += 1;
    }

    /// The verdict over the blocks seen.
    fn verdict(&self) -> Result<String, String> {
        match self.first_wrong {
            None => Ok(format!(
                "all {} samples of {} channels match from counter {}",
                self.blocks * 64,
                self.bits.len(),
                self.start.unwrap_or(0)
            )),
            Some((channel, sample)) => Err(format!(
                "{} of {} channel blocks differ, the first at D{channel} sample {sample}",
                self.wrong,
                self.blocks * self.bits.len() as u64
            )),
        }
    }
}

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
    let mut samples = SAMPLE_COUNT;
    let mut triggers = Vec::new();
    let mut model = None;
    let mut pattern = Pattern::Normal;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--capture" => capture_requested = true,
            "--model" => {
                model = Some(args.next().ok_or("--model requires an exact model name")?);
            }
            "--channels" => {
                let count: u16 = args
                    .next()
                    .ok_or("--channels requires 2, 4, 8, 16 or 32")?
                    .parse()?;
                if !matches!(count, 2 | 4 | 8 | 16 | 32) {
                    return Err("--channels must be 2, 4, 8, 16 or 32".into());
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
            "--samples" => {
                samples = args
                    .next()
                    .ok_or("--samples requires an integer")?
                    .parse()?;
                if samples == 0 || !samples.is_multiple_of(64) {
                    return Err("--samples must be a positive multiple of 64".into());
                }
            }
            "--trigger" => {
                let spec = args
                    .next()
                    .ok_or("--trigger requires CHANNEL:high|low|rise|fall|edge")?;
                let (channel, condition) = spec
                    .split_once(':')
                    .ok_or("--trigger requires CHANNEL:CONDITION")?;
                let condition = match condition {
                    "high" => TriggerCondition::High,
                    "low" => TriggerCondition::Low,
                    "rise" => TriggerCondition::Rise,
                    "fall" => TriggerCondition::Fall,
                    "edge" => TriggerCondition::Both,
                    _ => return Err("trigger conditions are high, low, rise, fall or edge".into()),
                };
                triggers.push((channel.parse()?, condition));
            }
            "--pattern" => {
                pattern = match args.next().as_deref() {
                    Some("normal") => Pattern::Normal,
                    Some("emulation") => Pattern::Emulation,
                    _ => return Err("--pattern requires normal or emulation".into()),
                };
            }
            "--help" | "-h" => {
                println!(
                    "slogic_capture [--capture] [--model NAME] [--channels 2|4|8|16|32] [--rate HZ] [--samples N] \
                     [--trigger CH:COND]... [--pattern P] [--repeat N]"
                );
                println!("Default: list devices without opening them or starting a capture.");
                println!("--capture       Required to open the device and acquire samples.");
                println!("--model NAME    Select an exact model name when several are connected.");
                println!("--channels N    Enable D0 through D(N-1); default every channel.");
                println!("--rate HZ       Supported integer sample rate in Hz; default 5000000.");
                println!(
                    "--repeat N      Capture 1 to 10 times on the same open device; default 1."
                );
                println!(
                    "--samples N     Samples per capture, a multiple of 64; default 65536. The \
                     capture times out after 5 s plus ten times its duration."
                );
                println!(
                    "--trigger C:X    Trigger on channel C: high, low, rise, fall or edge; may be \
                     repeated, all must hold. A tenth of the samples come before it. Default: \
                     capture at once."
                );
                println!(
                    "--pattern P     normal (the inputs) or emulation (Sipeed's test pattern), \
                     whose samples are checked where the pattern is known. Default: normal."
                );
                println!("U3 thresholds are 1.7 V.");
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
        println!("No SLogic found (expected USB ID 359f:0300, 359f:3000, 359f:3031 or 359f:3032).");
    }
    if !capture_requested {
        return Ok(());
    }
    let candidate = candidates
        .iter()
        .find(|candidate| {
            candidate.status == CandidateStatus::Ready
                && model.as_ref().is_none_or(|name| *name == candidate.model)
        })
        .ok_or("no ready SLogic found")?;
    println!("opening {}", candidate.id);
    let mut device = SlogicDriver::new().open_device(candidate)?;
    device.set_pattern(pattern)?;
    println!("device: {:?}", device.info());
    let channels = channels.unwrap_or(device.channel_count());
    if channels > device.channel_count() {
        return Err(format!("this model has {} channels", device.channel_count()).into());
    }
    for channel in 0..device.channel_count() {
        device.set_channel_enabled(channel, channel < channels)?;
    }
    device.set_option(OptionKey::Samplerate, &OptionValue::Integer(samplerate))?;
    device.set_option(OptionKey::SampleLimit, &OptionValue::Integer(samples))?;
    if device
        .options()
        .iter()
        .any(|option| option.key == OptionKey::Vth)
    {
        device.set_option(OptionKey::Vth, &OptionValue::Number(1.7))?;
    }
    let bits = emulation_bits(&device.info().model, channels);
    if pattern == Pattern::Emulation && bits.is_none() {
        println!("no Emulation pattern is known for this model and channel count; not checked");
    }
    let bits = bits.filter(|_| pattern == Pattern::Emulation);
    for index in 1..=repeat {
        capture_once(
            &mut device,
            index,
            repeat,
            samples,
            samplerate,
            &triggers,
            bits.as_deref(),
        )?;
    }
    Ok(())
}

fn capture_once(
    device: &mut dyn Device,
    index: u8,
    repeat: u8,
    samples: u64,
    samplerate: u64,
    triggers: &[(u16, TriggerCondition)],
    bits: Option<&[(u8, bool)]>,
) -> Result<(), Box<dyn std::error::Error>> {
    // A capture started at once counts from 0; a triggered one from wherever it fired.
    let mut check =
        bits.map(|bits| EmulationCheck::new(bits.to_vec(), triggers.is_empty().then_some(0)));
    // How long the samples take to arrive when the host keeps pace.
    let expected = Duration::from_secs_f64(samples as f64 / samplerate as f64);
    let timeout = CAPTURE_TIMEOUT + expected * 10;
    println!("capture {index}/{repeat}");
    let (sink, packets) = crossbeam_channel::unbounded();
    let started = Instant::now();
    let acquisition = device.start(
        CapturePlan {
            instant: triggers.is_empty(),
            triggers: triggers.to_vec(),
            trigger_position: 10,
            ..CapturePlan::default()
        },
        sink,
    )?;
    let mut capture: Option<LogicCapture> = None;
    let mut deinterleaver = None;
    let mut overflow = false;
    let result = loop {
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            acquisition.stop();
            break Err(format!("capture timed out after {timeout:?}"));
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
                if let Some(check) = check.as_mut() {
                    check.push(&data);
                }
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
        "capture {index}/{repeat} end: {reason:?}; {} samples in {:.0} ms (the analyzer takes \
         {:.0} ms to sample them); overflow: {overflow}",
        capture.len(),
        started.elapsed().as_secs_f64() * 1000.0,
        expected.as_secs_f64() * 1000.0
    );
    for (channel, data) in capture.channels() {
        let edges = data.count_edges(0, capture.len());
        println!(
            "D{channel}: {} rising, {} falling edges",
            edges.rising, edges.falling
        );
    }
    if let Some(check) = &check {
        match check.verdict() {
            Ok(verdict) => println!("emulation: {verdict}"),
            Err(problem) => return Err(format!("emulation: {problem}").into()),
        }
    }
    if reason != EndReason::Completed || overflow || capture.len() != samples {
        return Err("capture did not complete with the expected sample count".into());
    }
    Ok(())
}
