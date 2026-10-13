// SPDX-License-Identifier: Apache-2.0

//! `uiharu`: replays captures through a decode pipeline and exports the events as NDJSON,
//! generates captures to try it on and benchmarks it.

#![forbid(unsafe_code)]

mod args;
mod bench;
mod decode;
mod generate;
mod ndjson;
mod setup;

use std::fmt;
use std::io;
use std::process::ExitCode;

use args::{Args, UsageError};
use uiharu_core::GraphError;

const USAGE: &str = "\
uiharu — the uiharu decode engine on the command line

  uiharu decode [options] <capture>   decode a raw binary capture to NDJSON
                                      (--graph <file> for a described pipeline)
  uiharu gen [options] <output>       write a synthetic SPI NOR capture
  uiharu bench [options]              measure throughput, latency and memory
  uiharu <command> --help             the command's options

  Options shared by every command:
";

/// Why a command failed, and with which exit code.
#[derive(Debug)]
pub enum Failure {
    /// The arguments are wrong (exit code 2).
    Usage(String),
    /// The pipeline cannot be built as asked (exit code 2).
    Graph(GraphError),
    /// Reading or writing failed (exit code 1).
    Io(String),
    /// Standard output was closed; nothing to report.
    ClosedOutput,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Usage(message) | Failure::Io(message) => f.write_str(message),
            Failure::Graph(error) => write!(f, "{error}"),
            Failure::ClosedOutput => f.write_str("standard output was closed"),
        }
    }
}

impl From<UsageError> for Failure {
    fn from(error: UsageError) -> Self {
        Failure::Usage(error.0)
    }
}

impl From<GraphError> for Failure {
    fn from(error: GraphError) -> Self {
        Failure::Graph(error)
    }
}

impl From<uiharu_registry::BuildError> for Failure {
    fn from(error: uiharu_registry::BuildError) -> Self {
        match error {
            uiharu_registry::BuildError::Description(message) => Failure::Usage(message),
            uiharu_registry::BuildError::Graph(error) => Failure::Graph(error),
        }
    }
}

impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::BrokenPipe {
            Failure::ClosedOutput
        } else {
            Failure::Io(error.to_string())
        }
    }
}

impl From<serde_json::Error> for Failure {
    fn from(error: serde_json::Error) -> Self {
        Failure::from(io::Error::from(error))
    }
}

fn usage(command: Option<&str>) -> String {
    let command_usage = match command {
        Some("decode") => decode::USAGE,
        Some("gen") => generate::USAGE,
        Some("bench") => bench::USAGE,
        _ => USAGE,
    };
    format!("{command_usage}\n{}\n", setup::USAGE)
}

fn run() -> Result<(), Failure> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let command = raw.first().map(String::as_str);
    if raw.iter().any(|arg| arg == "--help" || arg == "-h") || command.is_none() {
        print!("{}", usage(command));
        return Ok(());
    }
    if command == Some("--version") {
        println!("uiharu {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let mut flags: Vec<&str> = setup::FLAGS.to_vec();
    flags.extend(decode::FLAGS);
    flags.extend(bench::FLAGS);
    let args = Args::parse(raw.iter().cloned(), &flags)?;
    match command {
        Some("decode") => decode::run(&args),
        Some("gen") => generate::run(&args),
        Some("bench") => bench::run(&args),
        Some(other) => Err(Failure::Usage(format!("unknown command {other}"))),
        None => Ok(()),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::ClosedOutput) => ExitCode::SUCCESS,
        Err(error @ (Failure::Usage(_) | Failure::Graph(_))) => {
            eprintln!("uiharu: {error}");
            eprintln!("Run `uiharu --help` for the options.");
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("uiharu: {error}");
            ExitCode::FAILURE
        }
    }
}
