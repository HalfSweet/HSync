// SPDX-License-Identifier: Apache-2.0

//! `uiharu bench`: synthesizes SPI NOR traffic and measures how the pipeline keeps up.
//!
//! Each block of samples is pushed, run and its events read before the next; the latency of
//! a block is the time from its push to the last of its events being read, which is what an
//! event's delay would be under a live capture that delivers blocks of this size.

use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use uiharu_core::{PushError, SignalItem, Tick};
use uiharu_protocols::spi::synth::SpiWaveform;

use crate::Failure;
use crate::args::Args;
use crate::generate::{Rng, traffic};
use crate::ndjson::StreamWriter;
use crate::setup::{self, Setup};

/// The options of the command, beyond the shared ones.
pub const OPTIONS: &[&str] = &["samples", "chunk", "bytes", "half-period", "json"];

/// The flags among [`OPTIONS`].
pub const FLAGS: &[&str] = &["json"];

pub const USAGE: &str = "\
uiharu bench [options]

  Synthesizes SPI NOR traffic, runs it through the pipeline block by block and reports the
  throughput, the latency of a block's events and the peak memory of the process.

      --samples <N>             samples to run (default 50000000)
      --chunk <SAMPLES>         samples per block (default 65536)
      --bytes <N>               data bytes per read (default 64)
      --half-period <SAMPLES>   samples per half clock period (default 2)
      --json                    print the report as JSON
";

/// The report of a run.
#[derive(Debug, Serialize)]
pub struct Report {
    pub samples: u64,
    pub blocks: u64,
    pub chunk: u64,
    pub spi_events: u64,
    pub nor_events: u64,
    pub seconds: f64,
    pub samples_per_second: f64,
    pub events_per_second: f64,
    pub latency_p50_us: f64,
    pub latency_p95_us: f64,
    pub latency_max_us: f64,
    pub peak_rss_bytes: Option<u64>,
}

/// The peak resident set size of this process, where the platform tells.
pub fn peak_rss() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let line = status.lines().find(|line| line.starts_with("VmHWM:"))?;
        let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        return Some(kib * 1024);
    }
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()?;
        let kib: u64 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .ok()?;
        return Some(kib * 1024);
    }
    #[allow(unreachable_code)]
    None
}

fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

pub fn run(args: &Args) -> Result<(), Failure> {
    let mut known: Vec<&str> = setup::OPTIONS.to_vec();
    known.extend(OPTIONS);
    args.only(&known)?;
    let setup = Setup::from_args(args)?;
    let samples: u64 = args.parsed("samples", 50_000_000)?;
    let chunk: u64 = args.parsed("chunk", 65_536)?;
    let bytes: usize = args.parsed("bytes", 64)?;
    let half_period: u64 = args.parsed("half-period", 2)?;

    // One block's worth of traffic, replayed at successive positions.
    let mut waveform = SpiWaveform::new(setup.spi.clone(), setup.channels, half_period, Tick::ZERO);
    let mut rng = Rng::new(7);
    waveform.idle(8);
    while waveform.position().get() < chunk.max(1) {
        traffic(&mut waveform, &mut rng, 1, bytes, 8);
    }
    let unit = Arc::new(waveform.finish());

    let built = setup.build()?;
    let mut pipeline = built.pipeline;
    let source = built.source;
    let mut spi = StreamWriter::new("spi", built.spi);
    let mut nor = built.nor.map(|nor| StreamWriter::new("nor", nor));
    let mut latencies = Vec::new();
    let started = Instant::now();
    let mut start = Tick::ZERO;
    let mut blocks = 0;
    while start.get() < samples {
        let block = Arc::new(unit.rebased(start));
        start = block.end();
        blocks += 1;
        let pushed = Instant::now();
        let mut item = SignalItem::Data(block);
        loop {
            match pipeline.push(source, item) {
                Ok(()) => break,
                Err(PushError::Full(back)) => {
                    item = back;
                    pipeline.run();
                    spi.discard();
                    if let Some(nor) = &mut nor {
                        nor.discard();
                    }
                }
                Err(error) => return Err(Failure::Io(error.to_string())),
            }
        }
        pipeline.run();
        spi.discard();
        if let Some(nor) = &mut nor {
            nor.discard();
        }
        latencies.push(pushed.elapsed());
    }
    let mut item = SignalItem::End(start);
    loop {
        match pipeline.push(source, item) {
            Ok(()) => break,
            Err(PushError::Full(back)) => {
                item = back;
                pipeline.run();
                spi.discard();
                if let Some(nor) = &mut nor {
                    nor.discard();
                }
            }
            Err(error) => return Err(Failure::Io(error.to_string())),
        }
    }
    while !pipeline.is_finished() {
        pipeline.run();
        spi.discard();
        if let Some(nor) = &mut nor {
            nor.discard();
        }
    }
    let seconds = started.elapsed().as_secs_f64();
    latencies.sort();
    let events = spi.events + nor.as_ref().map_or(0, |nor| nor.events);
    let report = Report {
        samples: start.get(),
        blocks,
        chunk: unit.len(),
        spi_events: spi.events,
        nor_events: nor.as_ref().map_or(0, |nor| nor.events),
        seconds,
        samples_per_second: start.get() as f64 / seconds,
        events_per_second: events as f64 / seconds,
        latency_p50_us: percentile(&latencies, 0.5).as_secs_f64() * 1e6,
        latency_p95_us: percentile(&latencies, 0.95).as_secs_f64() * 1e6,
        latency_max_us: percentile(&latencies, 1.0).as_secs_f64() * 1e6,
        peak_rss_bytes: peak_rss(),
    };
    let mut out = std::io::stdout().lock();
    if args.flag("json") {
        serde_json::to_writer_pretty(&mut out, &report)?;
        out.write_all(b"\n")?;
    } else {
        writeln!(
            out,
            "{} samples in {} blocks of {} ({} channels), {:.2} s",
            report.samples, report.blocks, report.chunk, setup.channels, report.seconds
        )?;
        writeln!(
            out,
            "throughput: {:.1} Msamples/s, {:.0} events/s ({} spi, {} nor)",
            report.samples_per_second / 1e6,
            report.events_per_second,
            report.spi_events,
            report.nor_events
        )?;
        writeln!(
            out,
            "block latency: p50 {:.0} µs, p95 {:.0} µs, max {:.0} µs",
            report.latency_p50_us, report.latency_p95_us, report.latency_max_us
        )?;
        match report.peak_rss_bytes {
            Some(bytes) => writeln!(
                out,
                "peak memory: {:.1} MiB",
                bytes as f64 / (1024.0 * 1024.0)
            )?,
            None => writeln!(out, "peak memory: not available on this platform")?,
        }
    }
    Ok(())
}
