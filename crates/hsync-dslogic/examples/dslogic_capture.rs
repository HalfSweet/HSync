//! Captures from the first DSLogic found and summarises what arrived: a smoke test for real
//! hardware (DESIGN.md §4.11).
//!
//! ```sh
//! cargo run -p hsync-dslogic --example dslogic_capture -- \
//!     [--buffer] [--mode CHANNEL_MODE] [--rate HZ] [--samples N] [--channels N] [--trigger CH:rise|fall|both|high|low]
//!     [--stage CH:CONDITION,...]... [--position PERCENT] [--instant] [--stop-after SECONDS] [--vth VOLTS]
//! ```
//!
//! Each `--stage` adds a stage of a multi-stage trigger (Buffer mode) whose first pattern holds
//! the listed conditions; an empty list matches at once.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hsync_capture::LogicCapture;
use hsync_core::TriggerCondition;
use hsync_driver::{
    AdvancedTrigger, CandidateStatus, CapturePlan, DeviceManager, DriverError, OptionKey,
    OptionValue, Packet, TriggerStageSpec,
};
use hsync_dslogic::{DslogicDriver, FirmwareDir};

/// Prints this crate's log messages to stderr.
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

struct Args {
    buffer: bool,
    mode: Option<String>,
    rate: Option<u64>,
    samples: Option<u64>,
    channels: Option<u16>,
    trigger: Option<(u16, TriggerCondition)>,
    stages: Vec<TriggerStageSpec>,
    position: u8,
    instant: bool,
    stop_after: Option<f64>,
    vth: Option<f64>,
}

fn parse() -> Result<Args, String> {
    let mut args = Args {
        buffer: false,
        mode: None,
        rate: None,
        samples: None,
        channels: None,
        trigger: None,
        stages: Vec::new(),
        position: 50,
        instant: false,
        stop_after: None,
        vth: None,
    };
    let mut words = std::env::args().skip(1);
    while let Some(word) = words.next() {
        let mut value = || words.next().ok_or(format!("{word} needs a value"));
        match word.as_str() {
            "--buffer" => args.buffer = true,
            "--instant" => args.instant = true,
            "--mode" => args.mode = Some(value()?),
            "--rate" => args.rate = Some(value()?.parse().map_err(|e| format!("{e}"))?),
            "--samples" => args.samples = Some(value()?.parse().map_err(|e| format!("{e}"))?),
            "--channels" => args.channels = Some(value()?.parse().map_err(|e| format!("{e}"))?),
            "--position" => args.position = value()?.parse().map_err(|e| format!("{e}"))?,
            "--stop-after" => args.stop_after = Some(value()?.parse().map_err(|e| format!("{e}"))?),
            "--vth" => args.vth = Some(value()?.parse().map_err(|e| format!("{e}"))?),
            "--trigger" => args.trigger = Some(channel_condition(&value()?)?),
            "--stage" => {
                let text = value()?;
                let pattern0 = text
                    .split(',')
                    .filter(|part| !part.is_empty())
                    .map(channel_condition)
                    .collect::<Result<_, _>>()?;
                args.stages.push(TriggerStageSpec {
                    pattern0,
                    pattern1: Vec::new(),
                    both: true,
                    invert0: false,
                    invert1: false,
                    count: 1,
                    contiguous: false,
                });
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(args)
}

/// `CH:CONDITION`, e.g. `3:rise`.
fn channel_condition(text: &str) -> Result<(u16, TriggerCondition), String> {
    let (channel, condition) = text.split_once(':').ok_or("CH:CONDITION")?;
    let condition = match condition {
        "rise" => TriggerCondition::Rise,
        "fall" => TriggerCondition::Fall,
        "both" => TriggerCondition::Both,
        "high" => TriggerCondition::High,
        "low" => TriggerCondition::Low,
        other => return Err(format!("unknown condition {other}")),
    };
    Ok((channel.parse().map_err(|e| format!("{e}"))?, condition))
}

fn set(
    device: &mut dyn hsync_driver::Device,
    key: OptionKey,
    value: OptionValue,
) -> Result<(), DriverError> {
    println!("set {key:?} = {value:?}");
    device.set_option(key, &value)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse()?;
    log::set_logger(&StderrLog)?;
    log::set_max_level(log::LevelFilter::Debug);
    let firmware =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources/firmware");
    let manager = DeviceManager::new(vec![Arc::new(DslogicDriver::new(FirmwareDir(firmware)))]);
    let mut candidates = manager.scan();
    if let Some(candidate) = candidates
        .iter()
        .find(|c| c.status == CandidateStatus::NeedsFirmware)
    {
        println!("uploading firmware to {} …", candidate.model);
        match manager.open(candidate) {
            Err(DriverError::Restarting) => std::thread::sleep(Duration::from_secs(3)),
            other => println!("unexpected: {:?}", other.err()),
        }
        candidates = manager.scan();
    }
    let candidate = candidates
        .iter()
        .find(|c| c.status == CandidateStatus::Ready)
        .ok_or("no DSLogic found")?;
    println!("opening {} ({})", candidate.model, candidate.id);
    let opened = Instant::now();
    let mut device = manager.open(candidate)?;
    println!(
        "opened in {:.0} ms: {:?}",
        opened.elapsed().as_secs_f64() * 1e3,
        device.info()
    );

    if args.buffer {
        set(
            device.as_mut(),
            OptionKey::OperationMode,
            OptionValue::Choice("buffer".into()),
        )?;
    }
    if let Some(mode) = &args.mode {
        set(
            device.as_mut(),
            OptionKey::ChannelMode,
            OptionValue::Choice(mode.clone()),
        )?;
    }
    if let Some(vth) = args.vth {
        set(device.as_mut(), OptionKey::Vth, OptionValue::Number(vth))?;
    }
    if let Some(rate) = args.rate {
        set(
            device.as_mut(),
            OptionKey::Samplerate,
            OptionValue::Integer(rate),
        )?;
    }
    if let Some(samples) = args.samples {
        set(
            device.as_mut(),
            OptionKey::SampleLimit,
            OptionValue::Integer(samples),
        )?;
    }
    if let Some(count) = args.channels {
        for channel in count..device.channel_count() {
            device.set_channel_enabled(channel, false)?;
        }
    }
    for option in device.options() {
        println!(
            "  {:?} = {:?}{}",
            option.key,
            option.value,
            if option.enabled { "" } else { " (disabled)" }
        );
    }

    let plan = CapturePlan {
        triggers: args.trigger.into_iter().collect(),
        trigger_position: args.position,
        instant: args.instant,
        advanced: (!args.stages.is_empty()).then(|| AdvancedTrigger::Stages {
            stages: args.stages.clone(),
        }),
        looping: false,
    };
    let (sink, packets) = crossbeam_channel::unbounded();
    let started = Instant::now();
    let acquisition = device.start(plan, sink)?;
    let mut capture: Option<LogicCapture> = None;
    let mut deinterleaver = None;
    let mut bytes = 0usize;
    let mut first_data = None;
    let mut stopped = false;
    let reason = loop {
        if let Some(after) = args.stop_after
            && !stopped
            && started.elapsed().as_secs_f64() >= after
        {
            println!("stopping after {after} s");
            acquisition.stop();
            stopped = true;
        }
        let Ok(packet) = packets.recv_timeout(Duration::from_millis(100)) else {
            continue;
        };
        match packet {
            Packet::Start {
                channels,
                samplerate,
            } => {
                println!("start: {} channels at {samplerate} Hz", channels.len());
                let new = LogicCapture::new(samplerate, channels);
                deinterleaver = Some(new.deinterleaver());
                capture = Some(new);
            }
            Packet::Trigger { sample } => {
                println!(
                    "trigger at sample {sample} after {:.0} ms",
                    started.elapsed().as_secs_f64() * 1e3
                );
                if let Some(capture) = capture.as_mut() {
                    capture.set_trigger(Some(sample), None);
                }
            }
            Packet::Logic(data) => {
                first_data.get_or_insert_with(Instant::now);
                bytes += data.len();
                if let (Some(capture), Some(deinterleaver)) =
                    (capture.as_mut(), deinterleaver.as_mut())
                {
                    capture.append_cross(deinterleaver, &data);
                }
            }
            Packet::Progress {
                triggered,
                captured,
            } => {
                println!("progress: triggered {triggered}, {captured} samples in the device");
            }
            Packet::Overflow => println!("overflow!"),
            Packet::End(reason) => break reason,
        }
    };
    acquisition.wait();
    let elapsed = started.elapsed().as_secs_f64();
    let capture = capture.ok_or("no start packet")?;
    println!(
        "end: {reason:?} after {:.0} ms, {bytes} bytes ({:.1} MB/s once data flowed), {} samples, trigger {:?}",
        elapsed * 1e3,
        first_data.map_or(0.0, |at| bytes as f64
            / at.elapsed().as_secs_f64().max(1e-9)
            / 1e6),
        capture.len(),
        capture.trigger_sample()
    );
    for (index, data) in capture.channels() {
        let edges = data.count_edges(0, capture.len());
        println!(
            "  D{index:<2} first {} last {} rising {:>8} falling {:>8}",
            u8::from(data.sample(0)),
            u8::from(!capture.is_empty() && data.sample(capture.len() - 1)),
            edges.rising,
            edges.falling
        );
    }
    Ok(())
}
