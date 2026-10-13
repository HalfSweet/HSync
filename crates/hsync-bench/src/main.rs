//! Decoding speed of the uiharu engine against HSync's native decoders on the same captures.
//!
//! Every case runs in a child process of its own, under `/usr/bin/time` where there is one, so
//! that its peak memory is the decoder's and not another's. The parent prints a table, or JSON
//! with `--json`; `--case` is what the children run.
//!
//! ```sh
//! cargo run --release -p hsync-bench                      # every decoder on every workload
//! cargo run --release -p hsync-bench -- --repeats 256 --runs 5 --json
//! cargo run --release -p hsync-bench -- --case uiharu-spi:demo
//! ```
//!
//! `--compare` runs no benchmark: it decodes the demo capture's I²C, UART, CAN FD and SPI with
//! uiharu and with the native decoders and reports where they disagree (see `compare`).

mod compare;
mod decoders;
mod workload;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde::{Deserialize, Serialize};

use decoders::{Decoder, Outcome};
use workload::{Source, Workload};

const USAGE: &str = "\
hsync-bench [options]

  Measures uiharu's SPI and SPI → SPI NOR decoders and hsync-decode's native SPI decoder on
  DSView's protocol demo repeated many times and on synthetic SPI NOR traffic, each case in a
  child process.

      --repeats <N>            times the demo capture is repeated (default 128)
      --samples <N>            samples of synthetic traffic (default 50000000)
      --runs <N>               runs per case; the median is reported (default 3)
      --chunk <SAMPLES>        samples per block fed to uiharu (default 65536)
      --bytes <N>              data bytes per synthetic read (default 64)
      --half-period <SAMPLES>  samples per half clock period of the synthetic traffic (default 2)
      --only <DECODER>         run this decoder only; may be repeated
      --workload <NAME>        run this workload only (demo, nor); may be repeated
      --json                   print the results as JSON
      --compare                cross-check uiharu against the native decoders on the demo
      --case <DECODER:WORKLOAD>  run one case in this process and print one JSON line
";

#[derive(Debug, Clone)]
struct Options {
    repeats: u64,
    samples: u64,
    runs: usize,
    chunk: u64,
    bytes: usize,
    half_period: u64,
    decoders: Vec<Decoder>,
    workloads: Vec<Workload>,
    json: bool,
    compare: bool,
    case: Option<(Decoder, Workload)>,
}

impl Options {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut options = Options {
            repeats: 128,
            samples: 50_000_000,
            runs: 3,
            chunk: 65_536,
            bytes: 64,
            half_period: 2,
            decoders: Vec::new(),
            workloads: Vec::new(),
            json: false,
            compare: false,
            case: None,
        };
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            if arg == "--json" {
                options.json = true;
                continue;
            }
            if arg == "--compare" {
                options.compare = true;
                continue;
            }
            if arg == "--help" || arg == "-h" {
                return Err(String::new());
            }
            let name = arg
                .strip_prefix("--")
                .ok_or_else(|| format!("unexpected argument {arg}"))?;
            let value = args
                .next()
                .ok_or_else(|| format!("--{name} needs a value"))?;
            let number = |what: &str| -> Result<u64, String> {
                value
                    .parse()
                    .map_err(|error| format!("--{what} {value}: {error}"))
            };
            match name {
                "repeats" => options.repeats = number(name)?.max(1),
                "samples" => options.samples = number(name)?.max(1),
                "runs" => options.runs = number(name)?.max(1) as usize,
                "chunk" => options.chunk = number(name)?.max(1),
                "bytes" => options.bytes = number(name)? as usize,
                "half-period" => options.half_period = number(name)?.max(1),
                "only" => options.decoders.push(
                    Decoder::parse(&value)
                        .ok_or_else(|| format!("--only {value}: no such decoder"))?,
                ),
                "workload" => options.workloads.push(
                    Workload::parse(&value)
                        .ok_or_else(|| format!("--workload {value}: no such workload"))?,
                ),
                "case" => {
                    let (decoder, workload) = value
                        .split_once(':')
                        .ok_or_else(|| format!("--case {value}: expected decoder:workload"))?;
                    options.case = Some((
                        Decoder::parse(decoder)
                            .ok_or_else(|| format!("--case {value}: no such decoder"))?,
                        Workload::parse(workload)
                            .ok_or_else(|| format!("--case {value}: no such workload"))?,
                    ));
                }
                other => return Err(format!("unknown option --{other}")),
            }
        }
        if options.decoders.is_empty() {
            options.decoders = Decoder::ALL.to_vec();
        }
        if options.workloads.is_empty() {
            options.workloads = Workload::ALL.to_vec();
        }
        Ok(options)
    }

    /// The options a child needs to build the same workload.
    fn forwarded(&self) -> Vec<String> {
        vec![
            "--repeats".to_owned(),
            self.repeats.to_string(),
            "--samples".to_owned(),
            self.samples.to_string(),
            "--chunk".to_owned(),
            self.chunk.to_string(),
            "--bytes".to_owned(),
            self.bytes.to_string(),
            "--half-period".to_owned(),
            self.half_period.to_string(),
        ]
    }
}

/// One case as a child reports it and as the table shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CaseResult {
    decoder: String,
    workload: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    skipped: Option<String>,
    #[serde(default)]
    samples: u64,
    #[serde(default)]
    words: u64,
    #[serde(default)]
    events: u64,
    #[serde(default)]
    seconds: f64,
    #[serde(default)]
    rss_before: Option<u64>,
    #[serde(default)]
    rss_after: Option<u64>,
    /// The process's peak resident set, read from `/usr/bin/time` by the parent.
    #[serde(default)]
    peak_rss: Option<u64>,
    #[serde(default)]
    runs: usize,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Runs one case in this process and prints its JSON line.
fn run_case(decoder: Decoder, workload: Workload, options: &Options) -> Result<(), String> {
    let root = repo_root();
    let source = match workload {
        Workload::Demo => Source::demo(&root, options.repeats.max(1))?,
        Workload::Nor => Source::nor(options.samples.max(1), options.bytes, options.half_period),
    };
    let mut result = CaseResult {
        decoder: decoder.name().to_owned(),
        workload: workload.name().to_owned(),
        skipped: None,
        samples: 0,
        words: 0,
        events: 0,
        seconds: 0.0,
        rss_before: None,
        rss_after: None,
        peak_rss: None,
        runs: 1,
    };
    match decoders::run(decoder, &source, options.chunk)? {
        Outcome::Measured(measured) => {
            result.samples = measured.samples;
            result.words = measured.words;
            result.events = measured.events;
            result.seconds = measured.seconds;
            result.rss_before = measured.rss_before;
            result.rss_after = measured.rss_after;
        }
        Outcome::Skipped(reason) => result.skipped = Some(reason),
    }
    let line = serde_json::to_string(&result).map_err(|error| error.to_string())?;
    println!("{line}");
    Ok(())
}

/// The peak resident set `/usr/bin/time` printed: bytes on macOS, kilobytes from GNU time.
fn parse_time_rss(stderr: &str) -> Option<u64> {
    stderr.lines().find_map(|line| {
        let lower = line.to_ascii_lowercase();
        if !lower.contains("maximum resident set size") {
            return None;
        }
        let number: u64 = line.split_whitespace().find_map(|word| word.parse().ok())?;
        Some(if lower.contains("kbytes") {
            number * 1024
        } else {
            number
        })
    })
}

/// Runs one case in a child process and reads its line.
fn run_child(
    exe: &Path,
    decoder: Decoder,
    workload: Workload,
    options: &Options,
) -> Result<CaseResult, String> {
    let time = Path::new("/usr/bin/time");
    let mut command = if time.exists() {
        let mut command = Command::new(time);
        command.arg(if cfg!(target_os = "macos") {
            "-l"
        } else {
            "-v"
        });
        command.arg(exe);
        command
    } else {
        Command::new(exe)
    };
    command
        .arg("--case")
        .arg(format!("{}:{}", decoder.name(), workload.name()))
        .args(options.forwarded());
    let output = command
        .output()
        .map_err(|error| format!("cannot run {}: {error}", exe.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let line = stdout
        .lines()
        .rev()
        .find(|line| line.starts_with('{'))
        .ok_or_else(|| {
            let tail: Vec<&str> = stderr.lines().rev().take(5).collect();
            format!(
                "the child exited with {} and no result: {}",
                output.status,
                tail.into_iter().rev().collect::<Vec<_>>().join(" | ")
            )
        })?;
    let mut result: CaseResult =
        serde_json::from_str(line).map_err(|error| format!("bad result line: {error}"))?;
    result.peak_rss = parse_time_rss(&stderr);
    Ok(result)
}

/// The run with the median time, carrying the largest peak memory seen.
fn summarize(mut runs: Vec<CaseResult>) -> Option<CaseResult> {
    if runs.is_empty() {
        return None;
    }
    let count = runs.len();
    let peak = runs.iter().filter_map(|run| run.peak_rss).max();
    runs.sort_by(|a, b| a.seconds.total_cmp(&b.seconds));
    let mut median = runs.swap_remove(count / 2);
    median.peak_rss = peak;
    median.runs = count;
    Some(median)
}

fn mib(bytes: Option<u64>) -> String {
    bytes.map_or("-".to_owned(), |bytes| {
        format!("{:.1}", bytes as f64 / (1024.0 * 1024.0))
    })
}

fn print_table(results: &[CaseResult], out: &mut impl Write) -> std::io::Result<()> {
    writeln!(
        out,
        "{:<18} {:<8} {:>11} {:>11} {:>10} {:>11} {:>9} {:>9} {:>9}",
        "decoder",
        "workload",
        "samples",
        "Msamples/s",
        "kwords/s",
        "kevents/s",
        "ms",
        "peak MiB",
        "+MiB"
    )?;
    for result in results {
        if let Some(reason) = &result.skipped {
            writeln!(
                out,
                "{:<18} {:<8} skipped: {reason}",
                result.decoder, result.workload
            )?;
            continue;
        }
        let delta = match (result.rss_before, result.rss_after) {
            (Some(before), Some(after)) => {
                format!("{:+.1}", (after as f64 - before as f64) / (1024.0 * 1024.0))
            }
            _ => "-".to_owned(),
        };
        writeln!(
            out,
            "{:<18} {:<8} {:>11} {:>11.1} {:>10.1} {:>11.1} {:>9.1} {:>9} {:>9}",
            result.decoder,
            result.workload,
            result.samples,
            result.samples as f64 / result.seconds / 1e6,
            result.words as f64 / result.seconds / 1e3,
            result.events as f64 / result.seconds / 1e3,
            result.seconds * 1e3,
            mib(result.peak_rss),
            delta
        )?;
    }
    // uiharu against the others, by samples per second.
    let rate = |decoder: &str, workload: &str| {
        results
            .iter()
            .find(|r| r.decoder == decoder && r.workload == workload && r.skipped.is_none())
            .map(|r| r.samples as f64 / r.seconds)
    };
    for workload in Workload::ALL {
        let Some(uiharu) = rate("uiharu-spi", workload.name()) else {
            continue;
        };
        let mut ratios = Vec::new();
        for other in ["hsync-decode-spi", "sigrok-cli-spi"] {
            if let Some(rate) = rate(other, workload.name()) {
                ratios.push(format!("{:.2}x {other}", uiharu / rate));
            }
        }
        if !ratios.is_empty() {
            writeln!(
                out,
                "{}: uiharu-spi is {}",
                workload.name(),
                ratios.join(", ")
            )?;
        }
    }
    Ok(())
}

/// Cross-checks uiharu against the native decoders on the demo capture; fails when they
/// disagree.
fn run_compare(options: &Options) -> Result<(), String> {
    let reports = compare::demo(&repo_root(), options.chunk)?;
    let mut out = std::io::stdout().lock();
    for report in &reports {
        write!(out, "{}", report.render()).map_err(|error| error.to_string())?;
    }
    let disagreeing: Vec<&str> = reports
        .iter()
        .filter(|report| !report.agrees())
        .map(|report| report.bus)
        .collect();
    if disagreeing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "the engines disagree on {}",
            disagreeing.join(", ")
        ))
    }
}

fn run() -> Result<(), String> {
    let options = Options::parse(std::env::args().skip(1))?;
    if options.compare {
        return run_compare(&options);
    }
    if let Some((decoder, workload)) = options.case {
        return run_case(decoder, workload, &options);
    }
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut results = Vec::new();
    for workload in &options.workloads {
        for decoder in &options.decoders {
            let mut runs = Vec::new();
            for _ in 0..options.runs {
                match run_child(&exe, *decoder, *workload, &options) {
                    Ok(result) => {
                        let skipped = result.skipped.is_some();
                        runs.push(result);
                        if skipped {
                            break;
                        }
                    }
                    Err(error) => {
                        eprintln!("{} on {}: {error}", decoder.name(), workload.name());
                        break;
                    }
                }
            }
            if let Some(summary) = summarize(runs) {
                if !options.json {
                    eprintln!(
                        "{:<18} {:<8} {}",
                        summary.decoder,
                        summary.workload,
                        summary.skipped.as_deref().map_or_else(
                            || format!(
                                "{:.1} Msamples/s",
                                summary.samples as f64 / summary.seconds / 1e6
                            ),
                            |reason| format!("skipped: {reason}")
                        )
                    );
                }
                results.push(summary);
            }
        }
    }
    let mut out = std::io::stdout().lock();
    if options.json {
        serde_json::to_writer_pretty(&mut out, &results).map_err(|error| error.to_string())?;
        writeln!(out).map_err(|error| error.to_string())?;
    } else {
        print_table(&results, &mut out).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) if message.is_empty() => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("hsync-bench: {message}");
            ExitCode::from(2)
        }
    }
}
