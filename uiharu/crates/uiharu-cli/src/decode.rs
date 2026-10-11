// SPDX-License-Identifier: Apache-2.0

//! `uiharu decode`: replays a raw binary capture through the pipeline and writes NDJSON.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::sync::Arc;

use uiharu_core::{LogicBlock, PushError, SignalItem, Tick};

use crate::Failure;
use crate::args::{Args, UsageError};
use crate::ndjson::StreamWriter;
use crate::setup::{self, Setup};

/// The options of the command, beyond the shared ones.
pub const OPTIONS: &[&str] = &["chunk", "stream", "progress", "stats"];

/// The flags among [`OPTIONS`].
pub const FLAGS: &[&str] = &["progress", "stats"];

pub const USAGE: &str = "\
uiharu decode [options] <capture>

  Replays a raw binary capture (sample by sample, ⌈channels / 8⌉ bytes each, bit i of byte j
  being channel 8 j + i; `-` for standard input) through the pipeline and writes every event
  as one line of JSON on standard output.

      --chunk <SAMPLES>         samples per block fed to the pipeline (default 65536)
      --stream <spi|nor>        write only this stream; may be repeated (default: all)
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

/// The writers of the exposed streams and whether each is written or only read.
struct Output<W: Write> {
    out: W,
    progress: bool,
    spi: (StreamWriter<uiharu_protocols::spi::SpiEvent>, bool),
    nor: Option<(StreamWriter<uiharu_protocols::spi_nor::NorEvent>, bool)>,
}

impl<W: Write> Output<W> {
    fn drain(&mut self) -> io::Result<()> {
        let (spi, written) = &mut self.spi;
        if *written {
            spi.drain(&mut self.out, self.progress)?
        } else {
            spi.discard()
        }
        if let Some((nor, written)) = &mut self.nor {
            if *written {
                nor.drain(&mut self.out, self.progress)?
            } else {
                nor.discard()
            }
        }
        Ok(())
    }

    fn is_ended(&self) -> bool {
        self.spi.0.is_ended() && self.nor.as_ref().is_none_or(|(nor, _)| nor.is_ended())
    }
}

pub fn run(args: &Args) -> Result<(), Failure> {
    let mut known: Vec<&str> = setup::OPTIONS.to_vec();
    known.extend(OPTIONS);
    args.only(&known)?;
    let setup = Setup::from_args(args)?;
    let path = args.positional().get(1).ok_or_else(|| {
        UsageError("decode needs the capture to read, or - for standard input".into())
    })?;
    let chunk: u64 = args.parsed("chunk", 65_536)?;
    if chunk == 0 {
        return Err(UsageError("--chunk 0: a block holds at least one sample".into()).into());
    }
    let streams: Vec<&str> = args.values("stream").collect();
    for stream in &streams {
        if !matches!(*stream, "spi" | "nor") {
            return Err(UsageError(format!("--stream {stream}: expected spi or nor")).into());
        }
    }
    let wanted = |name: &str| streams.is_empty() || streams.contains(&name);

    let built = setup.build()?;
    let mut pipeline = built.pipeline;
    let source = built.source;
    let mut output = Output {
        out: BufWriter::new(io::stdout().lock()),
        progress: args.flag("progress"),
        spi: (StreamWriter::new("spi", built.spi), wanted("spi")),
        nor: built
            .nor
            .map(|nor| (StreamWriter::new("nor", nor), wanted("nor"))),
    };

    let mut reader: Box<dyn Read> = if path == "-" {
        Box::new(BufReader::new(io::stdin().lock()))
    } else {
        Box::new(BufReader::new(
            File::open(path).map_err(|error| Failure::Io(format!("{path}: {error}")))?,
        ))
    };
    let unit = usize::from(setup.channels).div_ceil(8);
    let mut buffer = vec![
        0u8;
        usize::try_from(chunk)
            .unwrap_or(usize::MAX)
            .saturating_mul(unit)
    ];
    let mut start = Tick::ZERO;
    let push = |pipeline: &mut uiharu_core::Pipeline,
                output: &mut Output<_>,
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
        let block = LogicBlock::from_interleaved(start, setup.channels, &buffer[..samples * unit])
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
        eprintln!(
            "{} samples, {} spi events, {} nor events",
            start,
            output.spi.0.events,
            output.nor.as_ref().map_or(0, |(nor, _)| nor.events)
        );
        for queue in stats.queues {
            eprintln!(
                "{}: {} of {} items at most",
                queue.name, queue.high_water, queue.capacity
            );
        }
    }
    Ok(())
}
