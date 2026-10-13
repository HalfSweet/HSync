// SPDX-License-Identifier: Apache-2.0

//! `uiharu decode`: replays a raw binary capture through a pipeline and writes NDJSON.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::sync::Arc;

use uiharu_core::{LogicBlock, PushError, SignalItem, Tick};

use crate::Failure;
use crate::args::{Args, UsageError};
use crate::ndjson::Writer;
use crate::setup::{self, Setup};
use uiharu_registry::Description;

/// The options of the command, beyond the shared ones.
pub const OPTIONS: &[&str] = &["graph", "chunk", "stream", "progress", "stats"];

/// The flags among [`OPTIONS`].
pub const FLAGS: &[&str] = &["progress", "stats"];

pub const USAGE: &str = "\
uiharu decode [options] <capture>

  Replays a raw binary capture (sample by sample, ⌈channels / 8⌉ bytes each, bit i of byte j
  being channel 8 j + i; `-` for standard input) through the pipeline and writes every event
  as one line of JSON on standard output.

      --graph <file>            the pipeline described in a JSON file (source, nodes,
                                connections, exposed outputs; see the README); the bus options
                                below are not used then
      --chunk <SAMPLES>         samples per block fed to the pipeline (default 65536)
      --stream <name>           write only this exposed output; may be repeated (default: all)
      --progress                also write progress lines
      --stats                   print the queue statistics on standard error at the end
";

/// Fills `buffer` as far as the reader goes; the bytes read.
fn read_full(reader: &mut impl Read, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

/// The exposed outputs: each written, or only read when it was not asked for.
struct Output {
    out: Box<dyn Write>,
    progress: bool,
    writers: Vec<(Writer, bool)>,
}

impl Output {
    fn drain(&mut self) -> io::Result<()> {
        for (writer, written) in &mut self.writers {
            if *written {
                writer.drain(&mut *self.out, self.progress)?;
            } else {
                writer.discard();
            }
        }
        Ok(())
    }

    fn is_ended(&self) -> bool {
        self.writers.iter().all(|(writer, _)| writer.is_ended())
    }
}

/// The pipeline the arguments ask for: the described one, or the shared bus options' one.
fn description(args: &Args) -> Result<Description, Failure> {
    match args.value("graph") {
        Some(path) => {
            let file = File::open(path).map_err(|error| Failure::Io(format!("{path}: {error}")))?;
            serde_json::from_reader(BufReader::new(file))
                .map_err(|error| Failure::Usage(format!("{path}: {error}")))
        }
        None => Ok(Setup::from_args(args)?.description()),
    }
}

pub fn run(args: &Args) -> Result<(), Failure> {
    let mut known: Vec<&str> = setup::OPTIONS.to_vec();
    known.extend(OPTIONS);
    args.only(&known)?;
    let path = args.positional().get(1).ok_or_else(|| {
        UsageError("decode needs the capture to read, or - for standard input".into())
    })?;
    let chunk: u64 = args.parsed("chunk", 65_536)?;
    if chunk == 0 {
        return Err(UsageError("--chunk 0: a block holds at least one sample".into()).into());
    }
    let description = description(args)?;
    let streams: Vec<&str> = args.values("stream").collect();
    for stream in &streams {
        if !description.expose.contains_key(*stream) {
            let known: Vec<&str> = description.expose.keys().map(String::as_str).collect();
            return Err(UsageError(format!(
                "--stream {stream}: the pipeline exposes {}",
                known.join(", ")
            ))
            .into());
        }
    }
    let wanted = |name: &str| streams.is_empty() || streams.contains(&name);

    let built = uiharu_registry::build(&description)?;
    let mut pipeline = built.pipeline;
    let source = built.source;
    let channels = built.channels;
    let mut output = Output {
        out: Box::new(BufWriter::new(io::stdout().lock())),
        progress: args.flag("progress"),
        writers: built
            .streams
            .into_iter()
            .map(|stream| {
                let written = wanted(&stream.name);
                (Writer::new(stream.reader), written)
            })
            .collect(),
    };

    let mut reader: Box<dyn Read> = if path == "-" {
        Box::new(BufReader::new(io::stdin().lock()))
    } else {
        Box::new(BufReader::new(
            File::open(path).map_err(|error| Failure::Io(format!("{path}: {error}")))?,
        ))
    };
    let unit = usize::from(channels).div_ceil(8);
    let mut buffer = vec![
        0u8;
        usize::try_from(chunk)
            .unwrap_or(usize::MAX)
            .saturating_mul(unit)
    ];
    let mut start = Tick::ZERO;
    let push = |pipeline: &mut uiharu_core::Pipeline,
                output: &mut Output,
                item: SignalItem|
     -> Result<(), Failure> {
        let mut item = item;
        loop {
            match pipeline.push(source, item) {
                Ok(()) => return Ok(()),
                Err(PushError::Full(back)) => {
                    item = back;
                    pipeline.run();
                    output.drain().map_err(Failure::from)?;
                }
                Err(error) => return Err(Failure::Io(error.to_string())),
            }
        }
    };
    loop {
        let read = read_full(&mut reader, &mut buffer)
            .map_err(|error| Failure::Io(format!("{path}: {error}")))?;
        let samples = read / unit;
        if samples == 0 {
            break;
        }
        let block = LogicBlock::from_interleaved(start, channels, &buffer[..samples * unit])
            .map_err(|error| Failure::Io(error.to_string()))?;
        push(
            &mut pipeline,
            &mut output,
            SignalItem::Data(Arc::new(block)),
        )?;
        start = start.offset(samples as u64);
        if read < buffer.len() {
            break;
        }
    }
    push(&mut pipeline, &mut output, SignalItem::End(start))?;
    loop {
        pipeline.run();
        output.drain()?;
        if pipeline.is_finished() && output.is_ended() {
            break;
        }
    }
    output.out.flush()?;
    if args.flag("stats") {
        let stats = pipeline.stats();
        let counts: Vec<String> = output
            .writers
            .iter()
            .map(|(writer, _)| format!("{} {} events", writer.event_count(), writer.name()))
            .collect();
        eprintln!("{start} samples; {}", counts.join(", "));
        for queue in stats.queues {
            eprintln!(
                "{}: {} of {} items at most",
                queue.name, queue.high_water, queue.capacity
            );
        }
    }
    Ok(())
}
